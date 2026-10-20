//! Authoritative room settings. Refresh before admission; never infer authority from UI.
use super::projection::Project;
use super::*;
use api::conversation::{Command as WireCommand, InvitationSource};
use nextcloud_talk_api as api;
use talk_core::protocol::conversation::*;
use talk_core::protocol::{FeatureSupport, Role, RoomKind};

pub(super) struct MemberBinding {
    pub id: MemberId,
    pub attendee: u64,
    pub actor_type: String,
    pub actor_id: String,
}
pub(super) struct RecipientBinding {
    pub id: RecipientId,
    pub source: RecipientSource,
    pub actor_id: String,
}
pub(super) struct IntegrationBinding {
    pub id: IntegrationId,
    pub server: u64,
    pub bot: bool,
}
pub(super) struct PendingAction {
    pub operation: OperationId,
    pub conversation: ConversationId,
    pub uncertain: bool,
}
pub(super) fn is_mutation(c: &ConversationCommand) -> bool {
    !matches!(
        c,
        ConversationCommand::Read
            | ConversationCommand::GuestLink
            | ConversationCommand::Members
            | ConversationCommand::Directory { .. }
            | ConversationCommand::Bots
            | ConversationCommand::Bans
            | ConversationCommand::PreviewEmailCsv(_)
            | ConversationCommand::BreakoutRooms
    )
}
fn feature(setting: Setting) -> &'static str {
    match setting {
        Setting::Metadata => "room-description",
        Setting::Avatar => "avatar",
        Setting::ReadOnly => "read-only-rooms",
        Setting::Notifications => "notification-levels",
        Setting::Important => "important-conversations",
        Setting::Sensitive => "sensitive-conversations",
        Setting::Expiration => "message-expiration",
        Setting::Preserve => "preserve-conversation",
        Setting::GuestAccess | Setting::Password => "conversation-v4",
        Setting::Discoverability => "listable-rooms",
        Setting::Mentions => "mention-permissions",
        Setting::Permissions => "conversation-permissions",
        Setting::Lobby => "webinary-lobby",
        Setting::RecordingConsent => "recording-consent",
        Setting::Delete | Setting::Members | Setting::Invite | Setting::Roles => "conversation-v4",
        Setting::ClearHistory => "clear-history",
        Setting::Keep => "unbind-conversation",
        Setting::Bots => "bots-v1",
        Setting::Bans => "ban-v1",
        Setting::Breakouts => "breakout-rooms-v1",
    }
}
fn source(s: &str) -> Option<RecipientSource> {
    match s {
        "users" => Some(RecipientSource::User),
        "groups" => Some(RecipientSource::Group),
        "teams" | "circles" => Some(RecipientSource::Team),
        "emails" | "guests" => Some(RecipientSource::Email),
        "federated_users" => Some(RecipientSource::Federated),
        "phones" => Some(RecipientSource::Phone),
        _ => None,
    }
}
fn wire_source(s: RecipientSource) -> InvitationSource {
    match s {
        RecipientSource::User => InvitationSource::User,
        RecipientSource::Group => InvitationSource::Group,
        RecipientSource::Team => InvitationSource::Team,
        RecipientSource::Email => InvitationSource::Email,
        RecipientSource::Federated => InvitationSource::Federated,
        RecipientSource::Phone => InvitationSource::Phone,
    }
}
fn moderator(room: &Conversation) -> bool {
    matches!(
        room.role,
        Role::Owner | Role::Moderator | Role::GuestModerator
    )
}
pub(super) fn control(
    setting: Setting,
    room: &Conversation,
    data: &Value,
    caps: &Capabilities,
) -> ActionAvailability {
    let server = caps.feature(feature(setting));
    let preserved = wire::integer(data.get("attributes")).is_some_and(|v| v & 2 != 0);
    let direct = matches!(
        room.kind,
        RoomKind::Direct | RoomKind::FormerDirect | RoomKind::NoteToSelf | RoomKind::Changelog
    );
    let mut allowed = if room.unknown_restrictions {
        Decision::Unknown
    } else if (matches!(
        setting,
        Setting::Notifications | Setting::Important | Setting::Sensitive
    ) || moderator(room))
    {
        Decision::Allowed
    } else {
        Decision::Denied
    };
    if matches!(
        setting,
        Setting::Metadata
            | Setting::Avatar
            | Setting::ReadOnly
            | Setting::GuestAccess
            | Setting::Password
            | Setting::Discoverability
            | Setting::Invite
            | Setting::Roles
            | Setting::Breakouts
    ) && direct
    {
        allowed = Decision::Denied
    }
    if setting == Setting::Members {
        allowed = room.actions.list_participants
    }
    if setting == Setting::Delete {
        allowed = match data.get("canDeleteConversation").and_then(Value::as_bool) {
            Some(true) if !room.unknown_restrictions => Decision::Allowed,
            Some(false) => Decision::Denied,
            _ => Decision::Unknown,
        }
    }
    if setting == Setting::Preserve && room.role != Role::Owner {
        allowed = Decision::Denied
    }
    if preserved
        && matches!(
            setting,
            Setting::Delete
                | Setting::ClearHistory
                | Setting::GuestAccess
                | Setting::Password
                | Setting::Discoverability
        )
    {
        allowed = Decision::Denied
    }
    if room.classified
        && matches!(
            setting,
            Setting::GuestAccess
                | Setting::Discoverability
                | Setting::Bots
                | Setting::RecordingConsent
                | Setting::Sensitive
        )
    {
        allowed = Decision::Denied
    }
    if setting == Setting::Keep
        && !matches!(
            data.get("objectType").and_then(Value::as_str),
            Some("event" | "phone" | "classified")
        )
    {
        allowed = Decision::Denied
    }
    if room.channel
        && matches!(
            setting,
            Setting::Breakouts | Setting::Lobby | Setting::RecordingConsent
        )
    {
        allowed = Decision::Denied
    }
    if setting == Setting::RecordingConsent && caps.recording_enabled != Some(true) {
        allowed = if caps.recording_enabled == Some(false) {
            Decision::Denied
        } else {
            Decision::Unknown
        }
    }
    if setting == Setting::Breakouts && caps.breakouts_enabled != Some(true) {
        allowed = if caps.breakouts_enabled == Some(false) {
            Decision::Denied
        } else {
            Decision::Unknown
        }
    }
    ActionAvailability {
        implemented: true,
        server,
        allowed,
    }
}
impl NextcloudClient {
    pub(super) fn opaque(&self) -> Result<u128, PortError> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| self.error(ErrorKind::Transport))?;
        Ok(u128::from_le_bytes(bytes))
    }
    pub(super) fn admit_action(
        &mut self,
        id: ConversationId,
        operation: OperationId,
    ) -> Result<(), PortError> {
        if let Some(old) = self.actions.iter().find(|a| a.operation == operation) {
            return Err(self.error(if old.conversation != id {
                ErrorKind::WrongScope
            } else if old.uncertain {
                ErrorKind::Uncertain
            } else {
                ErrorKind::Conflict
            }));
        }
        if self.actions.len() >= 100 {
            if let Some(i) = self.actions.iter().position(|a| !a.uncertain) {
                self.actions.remove(i);
            } else {
                return Err(self.error(ErrorKind::Busy));
            }
        }
        self.actions.push(PendingAction {
            operation,
            conversation: id,
            uncertain: true,
        });
        Ok(())
    }
    pub(super) fn settle_action(&mut self, operation: OperationId, error: Option<&PortError>) {
        if error.is_none_or(|e| e.kind != ErrorKind::Uncertain)
            && let Some(a) = self.actions.iter_mut().find(|a| a.operation == operation)
        {
            a.uncertain = false
        }
    }
    fn setting_allowed(
        &self,
        setting: Setting,
        room: &Conversation,
        data: &Value,
        caps: &Capabilities,
    ) -> Result<(), PortError> {
        let a = control(setting, room, data, caps);
        if !a.implemented {
            return Err(self.error(ErrorKind::Unconfigured));
        }
        if a.server != FeatureSupport::Supported {
            return Err(self.error(ErrorKind::Unsupported));
        }
        if a.allowed != Decision::Allowed {
            return Err(self.error(ErrorKind::PermissionDenied));
        }
        Ok(())
    }
    pub(super) fn conversation_details(
        &mut self,
        id: ConversationId,
        data: &Value,
        caps: &Capabilities,
    ) -> Result<ConversationDetails, PortError> {
        let mut room = wire::room(data, id, caps).map_err(|k| self.error(k))?;
        self.project_call(&mut room, data, caps)?;
        let mut controls = Page::empty();
        for setting in [
            Setting::Metadata,
            Setting::Avatar,
            Setting::ReadOnly,
            Setting::Notifications,
            Setting::Important,
            Setting::Sensitive,
            Setting::Expiration,
            Setting::Preserve,
            Setting::GuestAccess,
            Setting::Password,
            Setting::Discoverability,
            Setting::Mentions,
            Setting::Permissions,
            Setting::Lobby,
            Setting::RecordingConsent,
            Setting::Delete,
            Setting::ClearHistory,
            Setting::Keep,
            Setting::Members,
            Setting::Invite,
            Setting::Roles,
            Setting::Bots,
            Setting::Bans,
            Setting::Breakouts,
        ] {
            controls
                .try_push((setting, control(setting, &room, data, caps)))
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        let state = |key| match wire::integer(data.get(key)) {
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => None,
        };
        let bits = |key| {
            wire::integer(data.get(key))
                .filter(|p| p & !511 == 0)
                .and_then(|p| u16::try_from(p).ok())
        };
        Ok(ConversationDetails {
            conversation: room,
            description: data
                .get("description")
                .filter(|v| !v.is_null())
                .map(|v| wire::text(Some(v)))
                .transpose()
                .map_err(|k| self.error(k))?,
            read_only: state("readOnly"),
            notifications: match data.get("notificationLevel").and_then(Value::as_i64) {
                Some(0) => Some(NotificationLevel::Default),
                Some(1) => Some(NotificationLevel::Always),
                Some(2) => Some(NotificationLevel::Mention),
                Some(3) => Some(NotificationLevel::Never),
                _ => None,
            },
            call_notifications: state("notificationCalls"),
            important: data.get("isImportant").and_then(Value::as_bool),
            preserved: wire::integer(data.get("attributes")).map(|v| v & 2 != 0),
            message_expiration: wire::integer(data.get("messageExpiration"))
                .and_then(|n| u32::try_from(n).ok()),
            listable: match wire::integer(data.get("listable")) {
                Some(0) => Some(Discoverability::None),
                Some(1) => Some(Discoverability::Users),
                Some(2) => Some(Discoverability::Everyone),
                _ => None,
            },
            mention_all: state("mentionPermissions").map(|v| !v),
            default_permissions: bits("defaultPermissions"),
            server_default_permissions: caps.server_default_permissions,
            effective_permissions: bits("permissions"),
            lobby: state("lobbyState"),
            lobby_timer: wire::integer(data.get("lobbyTimer")).filter(|v| *v > 0),
            recording_consent: state("recordingConsent"),
            has_password: data.get("hasPassword").and_then(Value::as_bool),
            participant_count: None,
            controls,
        })
    }
    fn member_binding(
        &self,
        id: MemberId,
        room: ConversationId,
    ) -> Result<&MemberBinding, PortError> {
        if id.conversation != room {
            return Err(self.error(ErrorKind::WrongScope));
        }
        self.members
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| self.error(ErrorKind::NotFound))
    }
    fn integration(
        &self,
        id: IntegrationId,
        room: ConversationId,
        bot: bool,
    ) -> Result<u64, PortError> {
        if id.conversation != room {
            return Err(self.error(ErrorKind::WrongScope));
        }
        self.integrations
            .iter()
            .find(|i| i.id == id && i.bot == bot)
            .map(|i| i.server)
            .ok_or_else(|| self.error(ErrorKind::NotFound))
    }
    async fn members(
        &mut self,
        c: &OperationContext,
        id: ConversationId,
    ) -> Result<ConversationResult, PortError> {
        let token = api::models::RoomToken::parse(self.check_room(id)?)
            .map_err(|k| self.error(transport::kind(k)))?;
        let reply = self
            .transport
            .api_request(c, api::Request::Participants(token))
            .await?;
        let api::Reply::Participants(values) = reply else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let mut members = Page::empty();
        let mut bindings = Vec::new();
        for p in values {
            if p.room_token
                .as_deref()
                .is_some_and(|t| self.check_room(id).ok() != Some(t))
            {
                return Err(self.error(ErrorKind::WrongScope));
            }
            let attendee = p
                .attendee_id
                .filter(|n| *n > 0)
                .ok_or_else(|| self.error(ErrorKind::Malformed))?;
            let actor_id = p.actor_id.ok_or_else(|| self.error(ErrorKind::Malformed))?;
            let actor_type = p
                .actor_type
                .ok_or_else(|| self.error(ErrorKind::Malformed))?;
            let handle = MemberId {
                conversation: id,
                local: self.opaque()?,
            };
            let role = wire::role(p.participant_type.map(Value::from).as_ref());
            members
                .try_push(Member {
                    id: handle,
                    name: Text::new(p.display_name.as_deref().unwrap_or(""))
                        .map_err(|_| self.error(ErrorKind::TooLarge))?,
                    role,
                    source: source(&actor_type)
                        .ok_or_else(|| self.error(ErrorKind::Unsupported))?,
                    permissions: p.permissions.filter(|p| p & !511 == 0).map(|p| p as u16),
                    is_self: actor_type == "users" && actor_id == self.transport.user(),
                })
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
            bindings.push(MemberBinding {
                id: handle,
                attendee,
                actor_type,
                actor_id,
            });
        }
        self.members = bindings;
        Ok(ConversationResult::Members(members))
    }
    fn recipients(
        &mut self,
        c: &OperationContext,
        entries: Vec<api::conversation::DirectoryEntry>,
        requested: Option<RecipientSource>,
    ) -> Result<Page<Recipient, 100>, PortError> {
        let mut output = Page::empty();
        let mut bindings = Vec::new();
        for entry in entries {
            let Some(source) = source(&entry.source) else {
                continue;
            };
            if requested.is_some_and(|s| s != source) {
                continue;
            }
            let id = RecipientId {
                scope: c.scope,
                local: self.opaque()?,
            };
            output
                .try_push(Recipient {
                    id,
                    name: Text::new(&entry.label).map_err(|_| self.error(ErrorKind::TooLarge))?,
                    label: Text::new(entry.details.as_deref().unwrap_or(&entry.id))
                        .map_err(|_| self.error(ErrorKind::TooLarge))?,
                    source,
                })
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
            bindings.push(RecipientBinding {
                id,
                source,
                actor_id: entry.id,
            });
        }
        self.recipients = bindings;
        Ok(output)
    }
    pub(super) async fn settings(
        &mut self,
        c: &OperationContext,
        id: ConversationId,
        operation: OperationId,
        command: ConversationCommand,
        caps: &Capabilities,
    ) -> Result<ConversationResult, PortError> {
        use ConversationCommand as C;
        let data = self.room_data(c, id).await?;
        let room = wire::room(&data, id, caps).map_err(|k| self.error(k))?;
        if command == C::Read {
            return Ok(ConversationResult::Details(Box::new(
                self.conversation_details(id, &data, caps)?,
            )));
        }
        if command == C::Leave {
            self.allowed(room.actions.leave_conversation)?;
            self.admit_action(id, operation)?;
            let token = api::models::RoomToken::parse(self.check_room(id)?)
                .map_err(|k| self.error(transport::kind(k)))?;
            let reply = self
                .transport
                .api_request(c, api::Request::LeaveRoom(token))
                .await;
            self.settle_action(operation, reply.as_ref().err());
            reply?;
            self.rooms.retain(|r| r.id != id);
            return Ok(ConversationResult::Left);
        }
        if command == C::GuestLink {
            if room.unknown_restrictions || room.classified || room.kind != RoomKind::Public {
                return Err(self.error(ErrorKind::PermissionDenied));
            }
            return Ok(ConversationResult::GuestLink(
                Text::new(&self.transport.public_room_link(self.check_room(id)?)?)
                    .map_err(|_| self.error(ErrorKind::TooLarge))?,
            ));
        }
        let setting = match &command {
            C::SetName(_) | C::SetDescription(_) => Setting::Metadata,
            C::SetEmojiAvatar { .. } | C::RemoveAvatar => Setting::Avatar,
            C::SetReadOnly(_) => Setting::ReadOnly,
            C::SetNotifications(_) | C::SetCallNotifications(_) => Setting::Notifications,
            C::SetImportant(_) => Setting::Important,
            C::SetSensitive(_) => Setting::Sensitive,
            C::SetExpiration(_) => Setting::Expiration,
            C::SetPreserved(_) => Setting::Preserve,
            C::SetGuestAccess { .. } => Setting::GuestAccess,
            C::SetPassword(_) => Setting::Password,
            C::SetDiscoverability(_) => Setting::Discoverability,
            C::SetMentionAll(_) => Setting::Mentions,
            C::SetDefaultPermissions(_) | C::SetMemberPermissions { .. } => Setting::Permissions,
            C::SetLobby { .. } => Setting::Lobby,
            C::SetRecordingConsent(_) => Setting::RecordingConsent,
            C::Delete => Setting::Delete,
            C::ClearHistory => Setting::ClearHistory,
            C::Keep => Setting::Keep,
            C::Members => Setting::Members,
            C::Directory { .. }
            | C::Invite(_)
            | C::ResendInvitations(_)
            | C::PreviewEmailCsv(_) => Setting::Invite,
            C::RemoveMember(_) | C::SetRole { .. } => Setting::Roles,
            C::Bots | C::SetBotEnabled { .. } => Setting::Bots,
            C::Bans | C::Ban { .. } | C::Unban(_) => Setting::Bans,
            _ => Setting::Breakouts,
        };
        self.setting_allowed(setting, &room, &data, caps)?;
        if command == C::Members {
            return self.members(c, id).await;
        }
        if let C::PreviewEmailCsv(text) = &command {
            if !caps.has("email-csv-import") || room.classified {
                return Err(self.error(ErrorKind::Unsupported));
            }
            let mut entries = Vec::new();
            let mut duplicate_count = 0;
            let mut invalid_count = 0;
            for line in text.as_str().lines() {
                let email = line.trim().trim_matches('"');
                if email.is_empty() {
                    continue;
                }
                if email.len() > 254
                    || email.matches('@').count() != 1
                    || !email.contains('.')
                    || email.chars().any(char::is_whitespace)
                    || email.contains([',', ';', '<', '>', '"'])
                {
                    invalid_count += 1;
                    continue;
                }
                if entries
                    .iter()
                    .any(|e: &api::conversation::DirectoryEntry| e.id.eq_ignore_ascii_case(email))
                {
                    duplicate_count += 1;
                    continue;
                }
                if entries.len() >= 100 {
                    return Err(self.error(ErrorKind::TooLarge));
                }
                entries.push(api::conversation::DirectoryEntry {
                    id: email.into(),
                    label: email.into(),
                    source: "emails".into(),
                    details: None,
                });
            }
            return Ok(ConversationResult::EmailCsvPreview {
                recipients: self.recipients(c, entries, None)?,
                duplicate_count,
                invalid_count,
            });
        }
        let read = matches!(
            command,
            C::Directory { .. } | C::Bots | C::Bans | C::BreakoutRooms
        );
        let requested = if let C::Directory { source, .. } = &command {
            Some(*source)
        } else {
            None
        };
        let delete = command == C::Delete;
        let clear = command == C::ClearHistory;
        let keep = command == C::Keep;
        let wire = match command {
            C::SetName(v) => WireCommand::Rename(v.as_str().into()),
            C::SetDescription(v) => WireCommand::Description(v.as_str().into()),
            C::SetEmojiAvatar { emoji, color } => WireCommand::EmojiAvatar {
                emoji: emoji.as_str().into(),
                color: color.map(|c| c.as_str().into()),
            },
            C::RemoveAvatar => WireCommand::DeleteAvatar,
            C::SetReadOnly(v) => WireCommand::ReadOnly(v),
            C::SetNotifications(v) => WireCommand::NotificationLevel(match v {
                NotificationLevel::Default => 0,
                NotificationLevel::Always => 1,
                NotificationLevel::Mention => 2,
                NotificationLevel::Never => 3,
            }),
            C::SetCallNotifications(v) => {
                if !caps.has("notification-calls") {
                    return Err(self.error(ErrorKind::Unsupported));
                }
                WireCommand::CallNotifications(v)
            }
            C::SetImportant(v) => WireCommand::Important(v),
            C::SetSensitive(v) => WireCommand::Sensitive(v),
            C::SetExpiration(v) => WireCommand::Expiration(v),
            C::SetPreserved(v) => WireCommand::Preserve(v),
            C::SetGuestAccess { enabled, password } => WireCommand::Public {
                enabled,
                password: password.as_str().into(),
            },
            C::SetPassword(v) => WireCommand::Password(v.as_str().into()),
            C::SetDiscoverability(v) => WireCommand::Listable(match v {
                Discoverability::None => 0,
                Discoverability::Users => 1,
                Discoverability::Everyone => 2,
            }),
            C::SetMentionAll(v) => WireCommand::MentionAll(v),
            C::SetDefaultPermissions(v) => WireCommand::DefaultPermissions(v),
            C::SetMemberPermissions {
                member,
                permissions,
            } => WireCommand::AttendeePermissions {
                attendee: self.member_binding(member, id)?.attendee,
                permissions,
            },
            C::SetLobby { enabled, timer } => WireCommand::Lobby { enabled, timer },
            C::SetRecordingConsent(v) => WireCommand::RecordingConsent(v),
            C::Delete => WireCommand::Delete,
            C::ClearHistory => WireCommand::ClearHistory,
            C::Keep => WireCommand::Keep,
            C::Directory { search, source } => {
                if room.classified
                    && !matches!(
                        source,
                        RecipientSource::User | RecipientSource::Group | RecipientSource::Team
                    )
                {
                    return Err(self.error(ErrorKind::PermissionDenied));
                }
                WireCommand::Directory {
                    search: search.as_str().into(),
                    source: wire_source(source),
                }
            }
            C::Invite(recipient) => {
                if recipient.scope != c.scope {
                    return Err(self.error(ErrorKind::WrongScope));
                }
                let r = self
                    .recipients
                    .iter()
                    .find(|r| r.id == recipient)
                    .ok_or_else(|| self.error(ErrorKind::NotFound))?;
                if room.classified
                    && !matches!(
                        r.source,
                        RecipientSource::User | RecipientSource::Group | RecipientSource::Team
                    )
                {
                    return Err(self.error(ErrorKind::PermissionDenied));
                }
                WireCommand::Invite {
                    source: wire_source(r.source),
                    id: r.actor_id.clone(),
                }
            }
            C::RemoveMember(member) => {
                WireCommand::RemoveAttendee(self.member_binding(member, id)?.attendee)
            }
            C::SetRole { member, role } => {
                if !matches!(role, Role::Owner | Role::Moderator | Role::Member) {
                    return Err(self.error(ErrorKind::Unsupported));
                }
                if role == Role::Owner && room.role != Role::Owner {
                    return Err(self.error(ErrorKind::PermissionDenied));
                }
                if role == Role::Owner && !caps.has("promote-demote-owner") {
                    return Err(self.error(ErrorKind::Unsupported));
                }
                WireCommand::Role {
                    attendee: self.member_binding(member, id)?.attendee,
                    moderator: role == Role::Moderator,
                    owner: role == Role::Owner,
                }
            }
            C::Bots => WireCommand::Bots,
            C::SetBotEnabled { bot, enabled } => WireCommand::Bot {
                id: self.integration(bot, id, true)?,
                enabled,
            },
            C::Bans => WireCommand::Bans,
            C::Ban { member, note } => {
                let m = self.member_binding(member, id)?;
                WireCommand::Ban {
                    actor_type: m.actor_type.clone(),
                    actor_id: m.actor_id.clone(),
                    note: note.as_str().into(),
                }
            }
            C::Unban(ban) => WireCommand::Unban(self.integration(ban, id, false)?),
            C::ResendInvitations(member) => WireCommand::ResendInvitations(
                member
                    .map(|m| self.member_binding(m, id).map(|m| m.attendee))
                    .transpose()?,
            ),
            C::BreakoutRooms => WireCommand::BreakoutRooms,
            C::ConfigureBreakouts { amount, automatic } => {
                WireCommand::ConfigureBreakouts { amount, automatic }
            }
            C::StartBreakouts => WireCommand::StartBreakouts,
            C::StopBreakouts => WireCommand::StopBreakouts,
            C::RemoveBreakouts => WireCommand::RemoveBreakouts,
            _ => return Err(self.error(ErrorKind::Unsupported)),
        };
        let token = api::models::RoomToken::parse(self.check_room(id)?)
            .map_err(|k| self.error(transport::kind(k)))?;
        if !read {
            self.admit_action(id, operation)?
        }
        let result = self
            .transport
            .api_request(
                c,
                api::Request::Conversation {
                    room: token,
                    command: wire,
                },
            )
            .await;
        if !read && result.is_err() {
            self.settle_action(operation, result.as_ref().err())
        }
        let result = result?;
        let result = (|| match result {
            api::Reply::Conversation(api::conversation::Response::Directory(entries)) => Ok(
                ConversationResult::Directory(self.recipients(c, entries, requested)?),
            ),
            api::Reply::Conversation(api::conversation::Response::Bots(bots)) => {
                self.integrations.retain(|b| !b.bot);
                let mut out = Page::empty();
                for b in bots {
                    let handle = IntegrationId {
                        conversation: id,
                        local: self.opaque()?,
                    };
                    out.try_push(RoomBot {
                        id: handle,
                        name: Text::new(&b.name).map_err(|_| self.error(ErrorKind::TooLarge))?,
                        description: Text::new(b.description.as_deref().unwrap_or(""))
                            .map_err(|_| self.error(ErrorKind::TooLarge))?,
                        enabled: match b.state {
                            0 => Some(false),
                            1 => Some(true),
                            _ => None,
                        },
                    })
                    .map_err(|_| self.error(ErrorKind::TooLarge))?;
                    self.integrations.push(IntegrationBinding {
                        id: handle,
                        server: b.id,
                        bot: true,
                    });
                }
                Ok(ConversationResult::Bots(out))
            }
            api::Reply::Conversation(api::conversation::Response::Bans(bans)) => {
                self.integrations.retain(|b| b.bot);
                let mut out = Page::empty();
                for b in bans {
                    let handle = IntegrationId {
                        conversation: id,
                        local: self.opaque()?,
                    };
                    out.try_push(RoomBan {
                        id: handle,
                        name: Text::new(&b.name).map_err(|_| self.error(ErrorKind::TooLarge))?,
                        note: Text::new(&b.note).map_err(|_| self.error(ErrorKind::TooLarge))?,
                        timestamp: b.timestamp,
                    })
                    .map_err(|_| self.error(ErrorKind::TooLarge))?;
                    self.integrations.push(IntegrationBinding {
                        id: handle,
                        server: b.id,
                        bot: false,
                    });
                }
                Ok(ConversationResult::Bans(out))
            }
            api::Reply::Rooms(rooms) => {
                if rooms.len() > 20 {
                    return Err(self.error(ErrorKind::TooLarge));
                }
                let mut out = Page::empty();
                for room in rooms {
                    let data = room.project();
                    let rid = self.bind_room(&data)?;
                    let mut room = wire::room(&data, rid, caps).map_err(|k| self.error(k))?;
                    self.project_call(&mut room, &data, caps)?;
                    out.try_push(room)
                        .map_err(|_| self.error(ErrorKind::TooLarge))?;
                }
                Ok(ConversationResult::BreakoutRooms(out))
            }
            _ if delete => {
                self.rooms.retain(|r| r.id != id);
                Ok(ConversationResult::Deleted)
            }
            _ if clear => Ok(ConversationResult::HistoryCleared),
            api::Reply::Room(room) => {
                let data = room.project();
                if keep
                    && self
                        .conversation_details(id, &data, caps)?
                        .conversation
                        .classified
                    && data["objectType"].as_str() != Some("classified_persist")
                {
                    return Err(self.error(ErrorKind::Uncertain));
                }
                Ok(ConversationResult::Details(Box::new(
                    self.conversation_details(id, &data, caps)?,
                )))
            }
            _ if !read => Ok(ConversationResult::Changed),
            _ => Err(self.error(ErrorKind::Malformed)),
        })();
        let result = if read {
            result
        } else {
            result.map_err(|_| self.error(ErrorKind::Uncertain))
        };
        if !read {
            self.settle_action(operation, result.as_ref().err())
        }
        result
    }
}
