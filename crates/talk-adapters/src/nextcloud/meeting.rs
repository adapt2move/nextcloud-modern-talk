//! Admission belongs to the authenticated account owner. UI-supplied correlation
//! is checked against the owner's room table before any network/native work.
use super::{
    NextcloudClient,
    call_policy::{Bundle, Sources},
    meeting_runtime::{DEVICES, LiveCall, SUPPORT},
    transport,
};
use crate::media::MediaService;
use nextcloud_talk_api::{
    self as api,
    meeting::{Observation, ObservedReply},
    models::{ParticipantFlags, RoomToken},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use talk_core::{
    calls::{
        CallSession, Command, DeviceState, Entry, Gate, MediaCounters, MediaWishes, Phase, Snapshot,
    },
    ids::{CallGeneration, CallScope, ConversationId},
    policy::RecordingState,
    ports::{ErrorKind, OperationContext, PortError, ProtocolReply},
};
pub(super) enum CallOwner {
    Pending {
        call: CallSession,
        last: Instant,
    },
    Live(LiveCall),
    Ended {
        snapshot: Snapshot,
        cleanup: bool,
        restart: bool,
    },
}
impl NextcloudClient {
    pub(super) async fn poll_incoming_calls(
        &mut self,
        context: &OperationContext,
    ) -> Result<ProtocolReply, PortError> {
        self.transport.validate(context)?;
        let empty = || talk_core::calls::IncomingCalls {
            scope: context.scope,
            call: None,
            title: None,
            eligible: false,
        };
        if self.media_service.is_none() {
            return Err(self.error(ErrorKind::Unconfigured));
        }
        self.settle_call(context).await?;
        if matches!(
            &self.meeting_owner,
            Some(CallOwner::Live(_) | CallOwner::Ended { cleanup: true, .. })
        ) || matches!(&self.meeting_owner,Some(CallOwner::Pending{call,..}) if call.accepted())
        {
            return Ok(ProtocolReply::IncomingCalls(empty()));
        }
        let invitations = self.transport.call_invitations(context).await?;
        for invitation in invitations {
            if self.dismissed_calls.contains(&invitation.id()) {
                continue;
            }
            let api::Reply::Room(room) = self
                .transport
                .api_request(context, api::Request::Room(invitation.room().clone()))
                .await?
            else {
                return Err(self.error(ErrorKind::Malformed));
            };
            if room.kind != Some(1)
                || room.remote
                || room.has_call != Some(true)
                || room.read_only != Some(0)
                || !room.permissions.is_some_and(|p| p & 4 != 0)
            {
                continue;
            }
            if !self
                .transport
                .invitation_current(context, &invitation)
                .await?
            {
                continue;
            }
            let conversation =
                self.bind_room(&serde_json::json!({"token":room.token,"id":room.server_id}))?;
            let existing = match &self.meeting_owner {
                Some(CallOwner::Pending { call, .. })
                    if call.entry() == Entry::IncomingDirect
                        && self
                            .incoming_invitation
                            .as_ref()
                            .is_some_and(|i| i.id() == invitation.id()) =>
                {
                    Some(call.scope())
                }
                _ => None,
            };
            let scope = if let Some(scope) = existing {
                scope
            } else {
                let previous = self
                    .call_scope_for(conversation)?
                    .ok_or_else(|| self.error(ErrorKind::Unconfigured))?;
                self.rotate_call(previous)?
            };
            let call = CallSession::new(scope, Entry::IncomingDirect)
                .map_err(|_| self.error(ErrorKind::WrongScope))?;
            let value = talk_core::calls::IncomingCalls {
                scope: context.scope,
                call: Some(pending_snapshot(&call)),
                title: Some(
                    talk_core::bounded::Text::new(&room.name)
                        .map_err(|_| self.error(ErrorKind::Malformed))?,
                ),
                eligible: invitation.eligible() && room.notification_calls == Some(1),
            };
            self.incoming_invitation = Some(invitation);
            self.meeting_owner = Some(CallOwner::Pending {
                call,
                last: Instant::now(),
            });
            return Ok(ProtocolReply::IncomingCalls(value));
        }
        self.incoming_invitation = None;
        if matches!(&self.meeting_owner,Some(CallOwner::Pending{call,..}) if call.entry()==Entry::IncomingDirect&&!call.accepted())
        {
            self.meeting_owner = None;
        }
        Ok(ProtocolReply::IncomingCalls(empty()))
    }

    /// This only exposes the join action. Complete fresh policy is still required
    /// after explicit intent; the room projection never authorizes capture.
    pub(super) fn project_call(
        &mut self,
        room: &mut talk_core::protocol::Conversation,
        data: &serde_json::Value,
        caps: &super::wire::Capabilities,
    ) -> Result<(), PortError> {
        use talk_core::{
            policy::Decision,
            protocol::{RoomKind, ServerFamily},
        };
        room.call_scope = self.call_scope_for(room.id)?;
        if room.call_scope.is_none() {
            return Ok(());
        }
        let permissions = super::wire::integer(data.get("permissions"));
        let readonly = super::wire::integer(data.get("readOnly"));
        room.actions.join_call = if room.channel
            || matches!(
                room.kind,
                RoomKind::NoteToSelf | RoomKind::Changelog | RoomKind::FormerDirect
            )
            || readonly == Some(1)
            || caps.call_enabled == Decision::Denied
            || permissions.is_some_and(|p| p & 4 == 0)
        {
            Decision::Denied
        } else if room.unknown_restrictions
            || readonly.is_none()
            || permissions.is_none()
            || !matches!(
                caps.family,
                ServerFamily::Nextcloud34 | ServerFamily::Nextcloud35
            )
            || caps.call_enabled != Decision::Allowed
        {
            Decision::Unknown
        } else if room.call_active == Some(true)
            || (data
                .get("canStartCall")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && permissions.is_some_and(|p| p & 2 != 0))
        {
            Decision::Allowed
        } else {
            Decision::Denied
        };
        Ok(())
    }

    pub(crate) fn set_media_service(&mut self, service: Option<Arc<MediaService>>) {
        self.media_service = service;
    }
    pub(super) fn call_scope_for(
        &mut self,
        conversation: ConversationId,
    ) -> Result<Option<CallScope>, PortError> {
        if self.media_service.is_none() {
            return Ok(None);
        }
        self.check_room(conversation)?;
        if let Some((_, scope)) = self
            .call_bindings
            .iter()
            .find(|(id, _)| *id == conversation)
        {
            return Ok(Some(*scope));
        }
        if self.call_bindings.len() >= 100 {
            return Err(self.error(ErrorKind::Busy));
        }
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| self.error(ErrorKind::Transport))?;
        let scope = CallScope {
            account: self.transport.scope(),
            conversation,
            call: CallGeneration::from_local(u128::from_le_bytes(bytes)),
        };
        self.call_bindings.push((conversation, scope));
        Ok(Some(scope))
    }
    pub(super) async fn policy_sources(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<Bundle, PortError> {
        let room = RoomToken::parse(self.check_room(call.conversation)?)
            .map_err(|k| self.error(transport::kind(k)))?;
        let own = self.transport.own_session(context, &room)?;
        let mut sources = Sources::new(call, own);
        for (observation, request) in [
            (Observation::Account, api::Request::Account),
            (Observation::Capabilities, api::Request::Capabilities),
            (Observation::Room, api::Request::Room(room.clone())),
            (
                Observation::ConnectedParticipants,
                api::Request::Participants(room.clone()),
            ),
            (
                Observation::SignalingSettings,
                api::Request::SignalingSettings(room.clone()),
            ),
            (
                Observation::CallParticipants,
                api::Request::CallParticipants(room.clone()),
            ),
        ] {
            let started = self.transport.api_now();
            let reply = self.transport.api_request(context, request).await?;
            sources.add(
                observation,
                ObservedReply {
                    started,
                    received: self.transport.api_now(),
                    reply,
                },
            );
        }
        sources
            .finish(false)
            .map_err(|k| self.error(transport::kind(k)))
    }
    async fn prepare_meeting(
        &mut self,
        context: &OperationContext,
        mut call: CallSession,
    ) -> Result<CallOwner, PortError> {
        let scope = call.scope();
        let room = RoomToken::parse(self.check_room(scope.conversation)?)
            .map_err(|k| self.error(transport::kind(k)))?;
        if self
            .active
            .as_ref()
            .is_none_or(|a| a.id != scope.conversation)
        {
            if let Some(active) = &self.active {
                let previous = RoomToken::parse(self.check_room(active.id)?)
                    .map_err(|k| self.error(transport::kind(k)))?;
                self.transport
                    .api_request(context, api::Request::DeactivateRoom(previous))
                    .await?;
                self.active = None;
            }
            let reply = self
                .transport
                .api_request(context, api::Request::ActivateRoom(room.clone()))
                .await?;
            let api::Reply::RoomSession(session) = reply else {
                return Err(self.error(ErrorKind::Malformed));
            };
            self.active = Some(super::ActiveConversation {
                id: scope.conversation,
                session: session.into_secret(),
            });
        } else {
            // HPB session retirement also retires its backend room session.
            // ActivateRoom(force=false) refreshes a selected room's session
            // before a new explicit call; it never starts media or a call.
            let reply = self
                .transport
                .api_request(context, api::Request::ActivateRoom(room.clone()))
                .await?;
            let api::Reply::RoomSession(session) = reply else {
                return Err(self.error(ErrorKind::Malformed));
            };
            self.active = Some(super::ActiveConversation {
                id: scope.conversation,
                session: session.into_secret(),
            });
        }
        let bundle = self.policy_sources(context, scope).await?;
        if call
            .advance(
                &bundle.inputs,
                bundle.exists,
                SUPPORT,
                DEVICES,
                self.transport.network().now(),
            )
            .is_err()
        {
            return Ok(CallOwner::Pending {
                call,
                last: Instant::now(),
            });
        }
        let wishes = call
            .grant()
            .ok_or_else(|| self.error(ErrorKind::PermissionDenied))?
            .publication();
        let flags = ParticipantFlags::new(wishes.microphone, wishes.camera);
        let (reader, writer, stop) = self
            .transport
            .begin_meeting(
                context,
                room,
                flags,
                call.recording_consent(),
                &bundle.settings,
            )
            .await?;
        let service = self
            .media_service
            .clone()
            .ok_or_else(|| self.error(ErrorKind::Unconfigured))?;
        LiveCall::start(
            service,
            call,
            bundle,
            reader,
            writer,
            stop,
            self.transport.network(),
            self.cancellation(),
            self.account_rejection.clone(),
        )
        .map(CallOwner::Live)
        .map_err(|k| self.error(transport::kind(k)))
    }
    async fn install_call(
        &mut self,
        context: &OperationContext,
        call: CallSession,
    ) -> Result<(), PortError> {
        let mut fallback = pending_snapshot(&call);
        fallback.phase = Phase::Waiting(Gate::PolicyStale);
        match self.prepare_meeting(context, call).await {
            Ok(owner) => {
                self.meeting_owner = Some(owner);
                Ok(())
            }
            Err(error) => {
                self.meeting_owner = Some(CallOwner::Ended {
                    snapshot: fallback,
                    cleanup: self.active.is_some(),
                    restart: false,
                });
                Err(error)
            }
        }
    }
    fn rotate_call(&mut self, old: CallScope) -> Result<CallScope, PortError> {
        self.call_bindings.retain(|(_, scope)| *scope != old);
        self.call_scope_for(old.conversation)?
            .ok_or_else(|| self.error(ErrorKind::Unconfigured))
    }
    async fn settle_call(&mut self, context: &OperationContext) -> Result<(), PortError> {
        if self
            .meeting_owner
            .as_ref()
            .is_some_and(|owner| matches!(owner,CallOwner::Live(live) if live.closed()))
        {
            let Some(CallOwner::Live(live)) = self.meeting_owner.take() else {
                return Err(self.error(ErrorKind::Uncertain));
            };
            let mut snapshot = live
                .snapshot()
                .map_err(|k| self.error(transport::kind(k)))?;
            snapshot.microphone = DeviceState::Off;
            snapshot.camera = DeviceState::Off;
            snapshot.screen = DeviceState::Off;
            if matches!(snapshot.phase, Phase::Connecting | Phase::Active) {
                snapshot.phase = Phase::Reconnecting;
            }
            drop(live);
            self.meeting_owner = Some(CallOwner::Ended {
                snapshot,
                cleanup: true,
                restart: false,
            });
        }
        let pending = match &self.meeting_owner {
            Some(CallOwner::Ended {
                snapshot,
                cleanup: true,
                ..
            }) => Some(snapshot.call),
            _ => None,
        };
        if let Some(scope) = pending {
            let room = RoomToken::parse(self.check_room(scope.conversation)?)
                .map_err(|k| self.error(transport::kind(k)))?;
            match self.transport.finish_meeting(context, room).await {
                Ok(()) => {
                    if let Some(CallOwner::Ended { cleanup, .. }) = &mut self.meeting_owner {
                        *cleanup = false;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind,
                        ErrorKind::Busy
                            | ErrorKind::Conflict
                            | ErrorKind::Transport
                            | ErrorKind::Deadline
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    pub(super) async fn meeting(
        &mut self,
        context: &OperationContext,
        command: Command,
    ) -> Result<ProtocolReply, PortError> {
        let mut scope = command.scope();
        self.transport.validate(context)?;
        if scope.account != context.scope
            || !scope.is_consistent()
            || !self.call_bindings.iter().any(|(_, s)| *s == scope)
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        if self.media_service.is_none() {
            return Err(self.error(ErrorKind::Unconfigured));
        }
        self.settle_call(context).await?;
        if let Command::Start { entry, wishes, .. } = command {
            if entry == Entry::IncomingDirect {
                let invitation = self
                    .incoming_invitation
                    .as_ref()
                    .ok_or_else(|| self.error(ErrorKind::NotFound))?;
                if !self
                    .transport
                    .invitation_current(context, invitation)
                    .await?
                {
                    self.incoming_invitation = None;
                    self.meeting_owner = None;
                    return Err(self.error(ErrorKind::NotFound));
                }
                if !matches!(&self.meeting_owner,Some(CallOwner::Pending{call,..}) if call.scope()==scope&&call.entry()==Entry::IncomingDirect&&!call.accepted())
                {
                    return Err(self.error(ErrorKind::WrongScope));
                }
            }
            match &self.meeting_owner {
                None => {}
                Some(CallOwner::Ended { cleanup: false, .. }) => {
                    scope = self.rotate_call(scope)?;
                    self.meeting_owner = None;
                }
                Some(CallOwner::Pending { call, .. })
                    if call.scope() == scope && !call.accepted() =>
                {
                    self.meeting_owner = None;
                }
                _ => return Err(self.error(ErrorKind::Busy)),
            }
            let mut call =
                CallSession::new(scope, entry).map_err(|_| self.error(ErrorKind::WrongScope))?;
            call.accept(scope, wishes)
                .map_err(|_| self.error(ErrorKind::WrongScope))?;
            self.install_call(context, call).await?;
            self.incoming_invitation = None;
        } else {
            let owner = self
                .meeting_owner
                .take()
                .ok_or_else(|| self.error(ErrorKind::NotFound))?;
            let owned_scope = match &owner {
                CallOwner::Pending { call, .. } => call.scope(),
                CallOwner::Live(live) => {
                    live.snapshot()
                        .map_err(|k| self.error(transport::kind(k)))?
                        .call
                }
                CallOwner::Ended { snapshot, .. } => snapshot.call,
            };
            if owned_scope != scope {
                self.meeting_owner = Some(owner);
                return Err(self.error(ErrorKind::WrongScope));
            }
            match owner {
                CallOwner::Pending { mut call, last } => {
                    let mut prepare = false;
                    match command {
                        Command::Stop(_) => {
                            call.ended(scope)
                                .map_err(|_| self.error(ErrorKind::WrongScope))?;
                            if let Some(invitation) = self.incoming_invitation.take() {
                                if self.dismissed_calls.len() == 32 {
                                    self.dismissed_calls.remove(0);
                                }
                                self.dismissed_calls.push(invitation.id());
                                let _ =
                                    self.transport.decline_invitation(context, invitation).await;
                            }
                        }
                        Command::Consent(_) => {
                            if call.consent(scope).is_err() {
                                self.meeting_owner = Some(CallOwner::Pending { call, last });
                                return Err(self.error(ErrorKind::PermissionDenied));
                            }
                            prepare = true;
                        }
                        Command::Change { wishes, .. } => {
                            if call.set_wishes(scope, wishes).is_err() {
                                self.meeting_owner = Some(CallOwner::Pending { call, last });
                                return Err(self.error(ErrorKind::PermissionDenied));
                            }
                            prepare = true;
                        }
                        Command::Observe(_)
                            if call.accepted() && last.elapsed() >= Duration::from_secs(2) =>
                        {
                            prepare = true
                        }
                        Command::Observe(_)
                            if call.entry() == Entry::IncomingDirect
                                && !call.accepted()
                                && last.elapsed() >= Duration::from_secs(2) =>
                        {
                            let current = if let Some(invitation) = &self.incoming_invitation {
                                self.transport
                                    .invitation_current(context, invitation)
                                    .await
                                    .unwrap_or(false)
                            } else {
                                false
                            };
                            if !current {
                                call.ended(scope)
                                    .map_err(|_| self.error(ErrorKind::WrongScope))?;
                                self.incoming_invitation = None;
                            }
                        }
                        _ => {}
                    }
                    if call.phase() == Phase::Ended {
                        self.meeting_owner = Some(CallOwner::Ended {
                            snapshot: pending_snapshot(&call),
                            cleanup: false,
                            restart: false,
                        });
                    } else if prepare {
                        self.install_call(context, call).await?;
                    } else {
                        self.meeting_owner = Some(CallOwner::Pending { call, last });
                    }
                }
                CallOwner::Live(live) => {
                    if matches!(command, Command::Reconnect(_) | Command::Stop(_)) {
                        live.close();
                        let mut snapshot = live
                            .snapshot()
                            .map_err(|k| self.error(transport::kind(k)))?;
                        drop(live);
                        let restart = matches!(command, Command::Reconnect(_));
                        snapshot.phase = if restart {
                            Phase::Reconnecting
                        } else {
                            Phase::Ended
                        };
                        snapshot.microphone = DeviceState::Off;
                        snapshot.camera = DeviceState::Off;
                        snapshot.screen = DeviceState::Off;
                        self.meeting_owner = Some(CallOwner::Ended {
                            snapshot,
                            cleanup: true,
                            restart,
                        });
                    } else {
                        let result = live.command(command);
                        self.meeting_owner = Some(CallOwner::Live(live));
                        result.map_err(|k| self.error(transport::kind(k)))?;
                    }
                }
                CallOwner::Ended {
                    mut snapshot,
                    cleanup,
                    mut restart,
                } => {
                    if matches!(command, Command::Reconnect(_)) {
                        restart = true;
                        snapshot.phase = Phase::Reconnecting;
                    }
                    if matches!(command, Command::Stop(_)) {
                        restart = false;
                        snapshot.phase = Phase::Ended;
                    }
                    self.meeting_owner = Some(CallOwner::Ended {
                        snapshot,
                        cleanup,
                        restart,
                    });
                }
            }
        }
        self.settle_call(context).await?;
        if matches!(
            self.meeting_owner,
            Some(CallOwner::Ended {
                cleanup: false,
                restart: true,
                ..
            })
        ) {
            // A fresh room session also isolates destructive signaling from
            // messages queued for the previous peer connections.
            if let Some(active) = &self.active {
                let room = RoomToken::parse(self.check_room(active.id)?)
                    .map_err(|k| self.error(transport::kind(k)))?;
                self.transport
                    .api_request(context, api::Request::DeactivateRoom(room))
                    .await?;
                self.active = None;
            }
            let scope = self.rotate_call(scope)?;
            let mut call = CallSession::new(scope, Entry::PreparedConference)
                .map_err(|_| self.error(ErrorKind::WrongScope))?;
            call.accept(scope, MediaWishes::default())
                .map_err(|_| self.error(ErrorKind::WrongScope))?;
            self.install_call(context, call).await?;
        }
        let snapshot = match self
            .meeting_owner
            .as_ref()
            .ok_or_else(|| self.error(ErrorKind::Uncertain))?
        {
            CallOwner::Pending { call, .. } => pending_snapshot(call),
            CallOwner::Live(live) => live
                .snapshot()
                .map_err(|k| self.error(transport::kind(k)))?,
            CallOwner::Ended { snapshot, .. } => snapshot.clone(),
        };
        Ok(ProtocolReply::Meeting(snapshot))
    }
}
fn pending_snapshot(call: &CallSession) -> Snapshot {
    Snapshot {
        call: call.scope(),
        entry: call.entry(),
        participants: talk_core::bounded::Page::empty(),
        phase: call.phase(),
        microphone: DeviceState::Off,
        camera: DeviceState::Off,
        screen: DeviceState::Unavailable,
        peers: 0,
        recording: RecordingState::Unknown,
        counters: MediaCounters::default(),
        revision: 0,
    }
}
