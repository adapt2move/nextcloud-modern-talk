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
        restart: Option<Restart>,
    },
}
/// One fresh admission after cleanup, never a replay of a room/media command.
#[derive(Clone, Copy)]
pub(super) struct Restart {
    wishes: MediaWishes,
    entry: Entry,
    continuity: bool,
}
impl Restart {
    /// Used by both live and retired owners; disposal timing cannot change the
    /// authority of an explicit consent rejoin or transfer old capture intent.
    fn explicit(snapshot: &Snapshot) -> Self {
        let consent_rejoin = snapshot.phase == Phase::Waiting(Gate::RecordingConsentRejoin);
        Self {
            wishes: MediaWishes::default(),
            entry: if consent_rejoin {
                snapshot.entry
            } else {
                Entry::PreparedConference
            },
            continuity: consent_rejoin,
        }
    }
}
/// Automatic restarts of a call whose owner stopped on its own: the policy
/// evidence went stale or a control request failed. Each restart is a fresh
/// admission that requires the call to still exist; the budget keeps a call
/// that cannot be held from restarting forever.
#[derive(Default)]
pub(super) struct CallRecovery {
    attempts: u8,
    last: Option<Instant>,
    retry_at: Option<Instant>,
}
impl CallRecovery {
    const ATTEMPTS: u8 = 4;
    const EPISODE: Duration = Duration::from_secs(120);
    fn admit(&mut self, now: Instant) -> bool {
        if self
            .last
            .is_none_or(|last| now.duration_since(last) >= Self::EPISODE)
        {
            self.attempts = 0;
        }
        if self.attempts >= Self::ATTEMPTS {
            return false;
        }
        self.attempts += 1;
        self.last = Some(now);
        true
    }
    /// The wait before the next attempt after `attempts` have been used.
    fn delay(&self) -> Duration {
        Duration::from_secs(1 << self.attempts.min(3))
    }
    fn reset(&mut self) {
        *self = Self::default();
    }
}
/// Whether a stopped owner may be replaced without the user asking: only a
/// lost connection or lapsed evidence, never a decision about the call
/// (consent, encryption, removal, call end).
fn recoverable(phase: Phase) -> bool {
    matches!(
        phase,
        Phase::Reconnecting | Phase::Waiting(Gate::PolicyStale | Gate::PolicyUnavailable(_))
    )
}
/// The network, a slow server or momentarily exhausted local request slots.
fn transient(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Transport
            | ErrorKind::Deadline
            | ErrorKind::ServerUnavailable
            | ErrorKind::RateLimited
            | ErrorKind::Busy
    )
}
fn media_recovery_restart(snapshot: &Snapshot) -> Option<Restart> {
    if snapshot.phase != Phase::Active {
        return None;
    }
    let mut wishes = snapshot.wishes;
    wishes.screen = false;
    Some(Restart {
        wishes,
        entry: snapshot.entry,
        continuity: true,
    })
}
impl NextcloudClient {
    pub(super) async fn poll_incoming_calls(
        &mut self,
        context: &OperationContext,
    ) -> Result<ProtocolReply, PortError> {
        let result = self.incoming_calls_inner(context).await;
        let Some(source) = result
            .as_ref()
            .err()
            .and_then(super::notifications::source_failure)
        else {
            return result;
        };
        self.transport.validate(context)?;
        Ok(ProtocolReply::IncomingCalls(
            talk_core::calls::IncomingCalls {
                scope: context.scope,
                source,
                call: None,
                title: None,
                eligible: false,
            },
        ))
    }
    async fn incoming_calls_inner(
        &mut self,
        context: &OperationContext,
    ) -> Result<ProtocolReply, PortError> {
        self.transport.validate(context)?;
        let empty = || talk_core::calls::IncomingCalls {
            scope: context.scope,
            source: talk_core::protocol::NotificationSourceState::Available,
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
                source: talk_core::protocol::NotificationSourceState::Available,
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
        use talk_core::{policy::Decision, protocol::RoomKind};
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
            || permissions.is_some_and(|p| p & 4 == 0)
        {
            Decision::Denied
        } else if room.unknown_restrictions
            || readonly.is_none()
            || permissions.is_none()
            || !caps.has("conversation-v4")
            || !caps.has("signaling-v3")
        {
            Decision::Unknown
        } else if room.call_active == Some(true)
            || (data
                .get("canStartCall")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && permissions.is_some_and(|p| p & 2 != 0)
                && caps.call_enabled == Decision::Allowed)
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
        // One inert action identity per admitted room, not one media session.
        // Match the room registry so an ordinary full listing cannot exhaust
        // call projection before any call has even been joined.
        if self.call_bindings.len() >= 256 {
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
    ) -> Result<Bundle, (PortError, talk_core::calls::PolicyEvidence)> {
        let room = RoomToken::parse(
            self.check_room(call.conversation)
                .map_err(|e| (e, talk_core::calls::PolicyEvidence::Room))?,
        )
        .map_err(|k| {
            (
                self.error(transport::kind(k)),
                talk_core::calls::PolicyEvidence::Room,
            )
        })?;
        let own = self
            .transport
            .own_session(context, &room)
            .map_err(|e| (e, talk_core::calls::PolicyEvidence::Participants))?;
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
            let reply = self
                .transport
                .api_request(context, request)
                .await
                .map_err(|e| (e, super::call_policy::evidence(observation)))?;
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
            .map_err(|evidence| (self.error(ErrorKind::Unsupported), evidence))
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
            let reply = match self
                .transport
                .api_request(context, api::Request::ActivateRoom(room.clone()))
                .await
            {
                Ok(reply) => reply,
                Err(error)
                    if matches!(
                        error.kind,
                        ErrorKind::Transport
                            | ErrorKind::Deadline
                            | ErrorKind::ServerUnavailable
                            | ErrorKind::RateLimited
                            | ErrorKind::Busy
                    ) =>
                {
                    // force=false activation is idempotent and has not joined a
                    // call. Retry the room session without capture authority.
                    call.wait_for_policy(talk_core::calls::PolicyEvidence::Participants);
                    return Ok(CallOwner::Pending {
                        call,
                        last: Instant::now(),
                    });
                }
                Err(error) => return Err(error),
            };
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
            let reply = match self
                .transport
                .api_request(context, api::Request::ActivateRoom(room.clone()))
                .await
            {
                Ok(reply) => reply,
                Err(error)
                    if matches!(
                        error.kind,
                        ErrorKind::Transport
                            | ErrorKind::Deadline
                            | ErrorKind::ServerUnavailable
                            | ErrorKind::RateLimited
                            | ErrorKind::Busy
                    ) =>
                {
                    // force=false activation is idempotent and has not joined a
                    // call. Retry the room session without capture authority.
                    call.wait_for_policy(talk_core::calls::PolicyEvidence::Participants);
                    return Ok(CallOwner::Pending {
                        call,
                        last: Instant::now(),
                    });
                }
                Err(error) => return Err(error),
            };
            let api::Reply::RoomSession(session) = reply else {
                return Err(self.error(ErrorKind::Malformed));
            };
            self.active = Some(super::ActiveConversation {
                id: scope.conversation,
                session: session.into_secret(),
            });
        }
        let bundle = match self.policy_sources(context, scope).await {
            Ok(bundle) => bundle,
            Err((error, evidence))
                if !matches!(
                    error.kind,
                    ErrorKind::ReauthenticationRequired
                        | ErrorKind::Cancelled
                        | ErrorKind::WrongScope
                ) =>
            {
                // No join has been dispatched. Keep this generation and explicit
                // intent pending; the existing two-second observation lane retries.
                // No lease/grant is issued by a failed or partial observation.
                call.wait_for_policy(evidence);
                return Ok(CallOwner::Pending {
                    call,
                    last: Instant::now(),
                });
            }
            Err((error, _)) => return Err(error),
        };
        if let Err(gate) = call.advance(
            &bundle.inputs,
            bundle.exists,
            SUPPORT,
            DEVICES,
            self.transport.network().now(),
        ) {
            if gate == Gate::CallEnded {
                return Ok(CallOwner::Ended {
                    snapshot: pending_snapshot(&call),
                    cleanup: false,
                    restart: None,
                });
            }
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
                    restart: None,
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
            let restart = if live.session_replaced()
                || recoverable(snapshot.phase) && self.call_recovery.admit(Instant::now())
            {
                self.call_recovery.retry_at = None;
                let mut wishes = snapshot.wishes;
                wishes.screen = false;
                Some(Restart {
                    wishes,
                    entry: snapshot.entry,
                    continuity: true,
                })
            } else {
                None
            };
            if restart.is_some() {
                snapshot.phase = Phase::Reconnecting;
            }
            drop(live);
            self.meeting_owner = Some(CallOwner::Ended {
                snapshot,
                cleanup: true,
                restart,
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
        if matches!(
            command,
            Command::Start { .. } | Command::Reconnect(_) | Command::Stop(_)
        ) {
            self.call_recovery.reset();
        }
        if matches!(command, Command::Start { .. }) {
            let room = RoomToken::parse(self.check_room(scope.conversation)?)
                .map_err(|kind| self.error(transport::kind(kind)))?;
            self.transport
                .retire_stopped_external_meeting(context, &room)
                .await?;
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
                        Command::RecoverMedia(_)
                        | Command::SetHand { .. }
                        | Command::React { .. }
                        | Command::Ring { .. }
                        | Command::MuteParticipant { .. }
                        | Command::EndEveryone(_)
                        | Command::SetRecording { .. } => {
                            self.meeting_owner = Some(CallOwner::Pending { call, last });
                            return Err(self.error(ErrorKind::PermissionDenied));
                        }
                        Command::Reconnect(_) if call.accepted() => prepare = true,
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
                            restart: None,
                        });
                    } else if prepare {
                        self.install_call(context, call).await?;
                    } else {
                        self.meeting_owner = Some(CallOwner::Pending { call, last });
                    }
                }
                CallOwner::Live(live) => {
                    if matches!(
                        command,
                        Command::Reconnect(_) | Command::RecoverMedia(_) | Command::Stop(_)
                    ) {
                        let mut snapshot = live
                            .snapshot()
                            .map_err(|k| self.error(transport::kind(k)))?;
                        if matches!(command, Command::RecoverMedia(_))
                            && snapshot.phase != Phase::Active
                        {
                            self.meeting_owner = Some(CallOwner::Live(live));
                            return Err(self.error(ErrorKind::PermissionDenied));
                        }
                        live.close();
                        drop(live);
                        let restart = match command {
                            Command::Reconnect(_) => Some(Restart::explicit(&snapshot)),
                            Command::RecoverMedia(_) => media_recovery_restart(&snapshot),
                            _ => None,
                        };
                        if restart.is_none_or(|restart| !restart.continuity) {
                            snapshot.wishes = MediaWishes::default();
                        }
                        snapshot.phase = if restart.is_some() {
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
                    if matches!(
                        command,
                        Command::RecoverMedia(_)
                            | Command::SetHand { .. }
                            | Command::React { .. }
                            | Command::Ring { .. }
                            | Command::MuteParticipant { .. }
                            | Command::EndEveryone(_)
                            | Command::SetRecording { .. }
                    ) {
                        self.meeting_owner = Some(CallOwner::Ended {
                            snapshot,
                            cleanup,
                            restart,
                        });
                        return Err(self.error(ErrorKind::NotFound));
                    }
                    if matches!(command, Command::Reconnect(_)) {
                        restart = Some(Restart::explicit(&snapshot));
                        snapshot.phase = Phase::Reconnecting;
                    }
                    if matches!(command, Command::Stop(_)) {
                        restart = None;
                        snapshot.wishes = MediaWishes::default();
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
                restart: Some(_),
                ..
            })
        ) && self
            .call_recovery
            .retry_at
            .is_none_or(|at| Instant::now() >= at)
        {
            // A fresh room session also isolates destructive signaling from
            // messages queued for the previous peer connections.
            if let Some(active) = &self.active {
                let room = RoomToken::parse(self.check_room(active.id)?)
                    .map_err(|k| self.error(transport::kind(k)))?;
                // A signaling session that ran out of reconnects still holds
                // the room on the server; settle that before the fresh join.
                let _ = self
                    .transport
                    .retire_stopped_external_meeting(context, &room)
                    .await;
                match self
                    .transport
                    .api_request(context, api::Request::DeactivateRoom(room))
                    .await
                {
                    Ok(_) => self.active = None,
                    Err(error) if transient(error.kind) => {
                        // Still offline: the restart stays pending.
                        self.call_recovery.retry_at = Some(Instant::now() + Duration::from_secs(2));
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        if self.active.is_none()
            && matches!(
                self.meeting_owner,
                Some(CallOwner::Ended {
                    cleanup: false,
                    restart: Some(_),
                    ..
                })
            )
            && self
                .call_recovery
                .retry_at
                .is_none_or(|at| Instant::now() >= at)
        {
            let restart = match &self.meeting_owner {
                Some(CallOwner::Ended {
                    restart: Some(restart),
                    ..
                }) => *restart,
                _ => return Err(self.error(ErrorKind::Uncertain)),
            };
            let scope = self.rotate_call(scope)?;
            let mut call = if restart.continuity {
                CallSession::recover(scope, restart.entry, restart.wishes)
            } else {
                CallSession::new(scope, restart.entry)
            }
            .map_err(|_| self.error(ErrorKind::WrongScope))?;
            if !restart.continuity {
                call.accept(scope, restart.wishes)
                    .map_err(|_| self.error(ErrorKind::WrongScope))?;
            }
            // install_call retains a scoped, blocked candidate on failure. Its
            // new generation must reach the UI even when fresh admission fails.
            self.call_recovery.retry_at = None;
            if let Err(error) = self.install_call(context, call).await
                && restart.continuity
                && transient(error.kind)
                && self.call_recovery.admit(Instant::now())
                && let Some(CallOwner::Ended {
                    snapshot,
                    restart: again,
                    ..
                }) = &mut self.meeting_owner
            {
                // The network was not back yet. Keep the restart and try the
                // admission again after a growing pause.
                snapshot.phase = Phase::Reconnecting;
                *again = Some(restart);
                self.call_recovery.retry_at = Some(Instant::now() + self.call_recovery.delay());
            }
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
        wishes: call.wishes(),
        call: call.scope(),
        entry: call.entry(),
        participants: talk_core::bounded::Page::empty(),
        phase: call.phase(),
        microphone: DeviceState::Off,
        local_speaking: false,
        local_level: 0,
        camera: DeviceState::Off,
        screen: DeviceState::Unavailable,
        sharing: talk_core::calls::SharingState::Idle,
        peers: 0,
        recording: RecordingState::Unknown,
        recording_control: None,
        recording_pending: false,
        recording_uncertain: false,
        encrypted: false,
        hand_supported: false,
        hand_raised: false,
        reactions: talk_core::bounded::Page::empty(),
        latest_reaction: None,
        invitees_loaded: false,
        invitees: talk_core::bounded::Page::empty(),
        ring_status: None,
        mute_status: None,
        can_end_everyone: false,
        end_pending: false,
        end_failed: false,
        counters: MediaCounters::default(),
        revision: 0,
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use talk_core::ids::{
        AccountId, AccountScope, CallGeneration, ConversationId, HostId, RuntimeGeneration,
    };
    #[test]
    fn a_stopped_call_restarts_by_itself_a_bounded_number_of_times() {
        let start = Instant::now();
        let mut recovery = CallRecovery::default();
        for _ in 0..CallRecovery::ATTEMPTS {
            assert!(recovery.admit(start));
        }
        // A call that cannot be held asks the user instead of looping.
        assert!(!recovery.admit(start + Duration::from_secs(60)));
        // A later, unrelated interruption gets a fresh budget.
        assert!(recovery.admit(start + CallRecovery::EPISODE + Duration::from_secs(1)));
        recovery.reset();
        assert!(recovery.admit(start));
        assert!(recovery.delay() > Duration::ZERO);
    }
    #[test]
    fn only_a_lost_connection_or_lapsed_evidence_restarts_without_the_user() {
        for phase in [
            Phase::Reconnecting,
            Phase::Waiting(Gate::PolicyStale),
            Phase::Waiting(Gate::PolicyUnavailable(
                talk_core::calls::PolicyEvidence::CallParticipants,
            )),
        ] {
            assert!(recoverable(phase));
        }
        for phase in [
            Phase::Ended,
            Phase::Ending,
            Phase::Waiting(Gate::CallEnded),
            Phase::Waiting(Gate::RecordingConsent),
            Phase::Waiting(Gate::RecordingConsentRejoin),
            Phase::Waiting(Gate::RequiredEncryption),
            Phase::Waiting(Gate::ClassifiedRecording),
        ] {
            assert!(!recoverable(phase));
        }
    }
    #[test]
    fn explicit_media_and_consent_rejoins_preserve_only_eligible_intent() {
        let account = AccountId::from_local(1);
        let scope = CallScope {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(2),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(3),
                local: 4,
            },
            call: CallGeneration::from_local(5),
        };
        let mut session = CallSession::new(scope, Entry::PreparedConference).unwrap();
        session
            .accept(
                scope,
                MediaWishes {
                    microphone: false,
                    camera: true,
                    screen: true,
                },
            )
            .unwrap();
        let mut snapshot = pending_snapshot(&session);
        assert!(media_recovery_restart(&snapshot).is_none());
        snapshot.phase = Phase::Active;
        let restart = media_recovery_restart(&snapshot).unwrap();
        assert!(!restart.wishes.microphone && restart.wishes.camera && !restart.wishes.screen);
        assert!(restart.continuity);
        let recovered = CallSession::recover(scope, restart.entry, restart.wishes).unwrap();
        assert!(!recovered.recording_consent());
        assert_eq!(recovered.wishes(), restart.wishes);
        snapshot.phase = Phase::Waiting(Gate::RecordingConsentRejoin);
        snapshot.entry = Entry::IncomingDirect;
        let consent_rejoin = Restart::explicit(&snapshot);
        assert!(consent_rejoin.continuity);
        assert_eq!(consent_rejoin.entry, Entry::IncomingDirect);
        assert_eq!(consent_rejoin.wishes, MediaWishes::default());
        let recovered =
            CallSession::recover(scope, consent_rejoin.entry, consent_rejoin.wishes).unwrap();
        assert!(!recovered.recording_consent());
    }
}
