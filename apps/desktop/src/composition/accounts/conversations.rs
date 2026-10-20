//! Account-owned settings bridge. No speculative mutation is projected as confirmed state.
use super::*;
use talk_core::protocol::conversation as wire;
use talk_core::ui::conversation as view;

impl Accounts {
    pub(super) fn operation(&self, context: ConversationRef) -> Result<OperationId, SubmitError> {
        self.current(context.account)?;
        if self.snapshot.selected != Some(context) {
            return Err(SubmitError::StaleScope);
        }
        self.connection
            .as_ref()
            .map_err(|e| submit_error(*e))?
            .new_operation_id(context.account)
            .map_err(submit_error)
    }
    pub(super) fn room_action(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        action: view::ConversationAction,
    ) -> Result<(), SubmitError> {
        let read = action.reads();
        if self
            .pending
            .iter()
            .any(|p| matches!(p.purpose, Purpose::Room(_, c, _, _) if c == context))
        {
            return Err(SubmitError::Busy);
        }
        if !read
            && self.snapshot.conversation_panel.as_ref().is_some_and(|p| {
                p.context == context && matches!(p.state, view::RemoteState::Uncertain(_))
            })
        {
            return Err(SubmitError::Busy);
        }
        let operation = self.operation(context)?;
        let invited = if let view::ConversationAction::Invite(person) = &action {
            Some(*person)
        } else {
            None
        };
        let command = command(context, action)?;
        self.request(
            context.account,
            ProtocolRequest::ConversationSettings {
                conversation: context.conversation,
                operation,
                command,
            },
            Purpose::Room(request, context, operation, read),
        )?;
        if let Some(person) = invited {
            self.room_invite = Some((request, person));
        }
        let panel = self
            .snapshot
            .conversation_panel
            .get_or_insert_with(|| view::ConversationPanel::new(context));
        panel.result = None;
        panel.state = if read {
            view::RemoteState::Loading(request)
        } else {
            view::RemoteState::Saving(request)
        };
        self.touch();
        Ok(())
    }
    pub(super) fn complete_room(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        operation: OperationId,
        read: bool,
        reply: Result<RuntimeReply, IdentityError>,
    ) {
        if self.snapshot.selected != Some(context) || self.current(context.account).is_err() {
            return;
        }
        let result = match reply {
            Ok(RuntimeReply::Protocol(ProtocolReply::ConversationSettings {
                conversation,
                operation: returned,
                result,
            })) if conversation == context.conversation && returned == operation => Ok(result),
            Err(error) => Err(completion_error(error)),
            _ => Err(Completion::Failed(UnavailableReason::PolicyStale)),
        };
        let Some(panel) = self
            .snapshot
            .conversation_panel
            .as_mut()
            .filter(|p| p.context == context)
        else {
            return;
        };
        if !matches!(panel.state, view::RemoteState::Loading(r) | view::RemoteState::Saving(r) if r == request)
        {
            return;
        }
        let mut reload = false;
        let mut invalidate_history = None;
        let mut reload_messages = false;
        let mut load_members = false;
        let completion = match result {
            Ok(result) => {
                match result {
                    wire::ConversationResult::Details(details) => {
                        if details.conversation.actions.read_messages != Decision::Allowed {
                            invalidate_history = Some(
                                if details.conversation.actions.read_messages == Decision::Denied {
                                    ContentState::Denied
                                } else {
                                    ContentState::Failed
                                },
                            );
                        } else if self.snapshot.content == ContentState::Denied {
                            reload_messages = true;
                        }
                        let mut row = conversation(&details.conversation, context.account);
                        row.participants = details
                            .participant_count
                            .and_then(|count| u16::try_from(count).ok())
                            .or_else(|| {
                                self.snapshot
                                    .conversations
                                    .as_slice()
                                    .iter()
                                    .find(|existing| existing.context == context)
                                    .and_then(|existing| existing.participants)
                            });
                        self.snapshot.conversations =
                            page(self.snapshot.conversations.as_slice().iter().map(|r| {
                                if r.context == context {
                                    row.clone()
                                } else {
                                    r.clone()
                                }
                            }));
                        load_members = details.participant_count.is_none()
                            && !matches!(
                                row.kind,
                                ConversationKind::Direct | ConversationKind::NoteToSelf
                            )
                            && details.controls.as_slice().iter().any(|(key, value)| {
                                *key == wire::Setting::Members
                                    && value.implemented
                                    && value.allowed == Decision::Allowed
                            });
                        panel.details = Some(details_view(*details));
                    }
                    wire::ConversationResult::Members(people) => {
                        panel.members = page(people.as_slice().iter().map(|m| view::Member {
                            id: person(context, m.id.local),
                            name: m.name.clone(),
                            role: role_view(m.role),
                            source: source_view(m.source),
                            permissions: m.permissions,
                            self_user: m.is_self,
                        }));
                        panel.members_loaded = true;
                        // This endpoint is authoritative and unpaged; the adapter rejects overflow.
                        let count = u16::try_from(panel.members.as_slice().len()).ok();
                        self.snapshot.conversations =
                            page(self.snapshot.conversations.as_slice().iter().map(|row| {
                                let mut row = row.clone();
                                if row.context == context {
                                    row.participants = count;
                                }
                                row
                            }));
                    }
                    wire::ConversationResult::Directory(people) => {
                        panel.directory = recipients(context, people);
                        panel.directory_loaded = true;
                    }
                    wire::ConversationResult::EmailCsvPreview {
                        recipients: people,
                        duplicate_count,
                        invalid_count,
                    } => {
                        panel.directory = recipients(context, people);
                        panel.directory_loaded = true;
                        panel.csv_duplicates = duplicate_count;
                        panel.csv_invalid = invalid_count;
                    }
                    wire::ConversationResult::Bots(bots) => {
                        panel.bots = page(bots.as_slice().iter().map(|b| view::Bot {
                            id: person(context, b.id.local),
                            name: b.name.clone(),
                            description: b.description.clone(),
                            enabled: b.enabled,
                        }));
                        panel.bots_loaded = true;
                    }
                    wire::ConversationResult::Bans(bans) => {
                        panel.bans = page(bans.as_slice().iter().map(|b| view::Ban {
                            id: person(context, b.id.local),
                            name: b.name.clone(),
                            note: b.note.clone(),
                        }));
                        panel.bans_loaded = true;
                    }
                    wire::ConversationResult::Deleted | wire::ConversationResult::Left => {
                        invalidate_history = Some(ContentState::Denied);
                        self.load_rooms = Some(context.account);
                        self.snapshot.messages = Page::empty();
                        self.snapshot.files = Page::empty();
                        self.snapshot.participants = Page::empty();
                        self.snapshot.content = ContentState::Denied;
                        panel.members = Page::empty();
                        panel.directory = Page::empty();
                        panel.bots = Page::empty();
                        panel.bans = Page::empty();
                    }
                    wire::ConversationResult::HistoryCleared => {
                        invalidate_history = Some(ContentState::Loading);
                        reload_messages = true;
                        self.snapshot.messages = Page::empty();
                        reload = true;
                    }
                    wire::ConversationResult::Changed => {
                        reload = true;
                        panel.bots_loaded = false;
                        panel.bans_loaded = false;
                    }
                    wire::ConversationResult::BreakoutRooms(_) => (),
                    wire::ConversationResult::GuestLink(link) => panel.guest_link = Some(link),
                }
                Completion::ServerAccepted
            }
            Err(error) => error,
        };
        panel.state = match completion {
            Completion::ServerAccepted => view::RemoteState::Idle,
            Completion::Failed(reason) => view::RemoteState::Failed(request, reason),
            Completion::Uncertain => view::RemoteState::Uncertain(request),
        };
        // Denial invalidates confidential membership data, including directory candidates.
        if completion == Completion::Failed(UnavailableReason::PermissionDenied) {
            if let Some(details) = panel.details.as_mut() {
                details.controls = page(details.controls.as_slice().iter().map(|(key, value)| {
                    (
                        *key,
                        view::Availability {
                            allowed: Decision::Denied,
                            ..*value
                        },
                    )
                }));
                details.leave = Decision::Denied;
            }
            panel.members = Page::empty();
            panel.members_loaded = false;
            self.snapshot.conversations =
                page(self.snapshot.conversations.as_slice().iter().map(|row| {
                    let mut row = row.clone();
                    if row.context == context {
                        row.participants = None;
                    }
                    row
                }));
            panel.directory = Page::empty();
            panel.directory_loaded = false;
            panel.bots = Page::empty();
            panel.bans = Page::empty();
        }
        if !read {
            if let Some((invitation, person)) = self.room_invite
                && invitation == request
                && person.context == context
            {
                if completion == Completion::ServerAccepted {
                    panel.directory = page(
                        panel
                            .directory
                            .as_slice()
                            .iter()
                            .filter(|p| p.id != person)
                            .cloned(),
                    );
                }
                self.room_invite = None;
            }
            panel.result = Some(view::RoomResult {
                request,
                completion,
            });
        }
        if let Some(state) = invalidate_history {
            self.invalidate_history(context, state);
        }
        if reload_messages {
            let _ = self.start_messages(context);
        }
        if reload {
            let _ = self.room_action(request, context, view::ConversationAction::Read);
        }
        if load_members {
            let _ = self.room_action(request, context, view::ConversationAction::Members);
        }
        self.touch();
    }
}
fn recipients(
    context: ConversationRef,
    people: Page<wire::Recipient, 100>,
) -> Page<view::Recipient, 100> {
    page(people.as_slice().iter().map(|p| view::Recipient {
        id: person(context, p.id.local),
        name: p.name.clone(),
        label: p.label.clone(),
        source: source_view(p.source),
    }))
}
fn person(context: ConversationRef, local: u128) -> view::PersonRef {
    view::PersonRef { context, local }
}
fn member(context: ConversationRef, id: view::PersonRef) -> Result<wire::MemberId, SubmitError> {
    if context != id.context {
        return Err(SubmitError::StaleScope);
    }
    Ok(wire::MemberId {
        conversation: context.conversation,
        local: id.local,
    })
}
fn integration(
    context: ConversationRef,
    id: view::PersonRef,
) -> Result<wire::IntegrationId, SubmitError> {
    member(context, id).map(|id| wire::IntegrationId {
        conversation: id.conversation,
        local: id.local,
    })
}
fn command(
    context: ConversationRef,
    action: view::ConversationAction,
) -> Result<wire::ConversationCommand, SubmitError> {
    use view::ConversationAction as V;
    use wire::ConversationCommand as W;
    Ok(match action {
        V::Read => W::Read,
        V::SetName(v) => W::SetName(v),
        V::SetDescription(v) => W::SetDescription(v),
        V::SetEmojiAvatar(emoji) => W::SetEmojiAvatar { emoji, color: None },
        V::RemoveAvatar => W::RemoveAvatar,
        V::SetReadOnly(v) => W::SetReadOnly(v),
        V::SetNotifications(v) => W::SetNotifications(match v {
            view::NotificationLevel::Default => wire::NotificationLevel::Default,
            view::NotificationLevel::Always => wire::NotificationLevel::Always,
            view::NotificationLevel::Mention => wire::NotificationLevel::Mention,
            view::NotificationLevel::Never => wire::NotificationLevel::Never,
        }),
        V::SetCallNotifications(v) => W::SetCallNotifications(v),
        V::SetImportant(v) => W::SetImportant(v),
        V::SetSensitive(v) => W::SetSensitive(v),
        V::SetExpiration(v) => W::SetExpiration(v),
        V::GuestLink => W::GuestLink,
        V::SetGuestAccess(enabled) => W::SetGuestAccess {
            enabled,
            password: text(""),
        },
        V::SetPassword(v) => W::SetPassword(v),
        V::SetDefaultPermissions(v) => W::SetDefaultPermissions(v),
        V::SetLobby(enabled) => W::SetLobby {
            enabled,
            timer: None,
        },
        V::Delete => W::Delete,
        V::ClearHistory => W::ClearHistory,
        V::Leave => W::Leave,
        V::Members => W::Members,
        V::Directory { search, source } => W::Directory {
            search,
            source: source_wire(source),
        },
        V::Invite(id) => {
            member(context, id)?;
            W::Invite(wire::RecipientId {
                scope: context.account,
                local: id.local,
            })
        }
        V::RemoveMember(id) => W::RemoveMember(member(context, id)?),
        V::SetRole { member: id, role } => W::SetRole {
            member: member(context, id)?,
            role: role_wire(role),
        },
        V::Bots => W::Bots,
        V::SetBotEnabled { bot, enabled } => W::SetBotEnabled {
            bot: integration(context, bot)?,
            enabled,
        },
        V::Bans => W::Bans,
        V::Unban(id) => W::Unban(integration(context, id)?),
        V::PreviewCsv(csv) => W::PreviewEmailCsv(csv),
        V::ConfigureBreakouts { amount, automatic } => W::ConfigureBreakouts { amount, automatic },
    })
}
pub(super) fn availability(a: wire::ActionAvailability) -> view::Availability {
    view::Availability {
        implemented: a.implemented,
        server: support(a.server),
        allowed: a.allowed,
    }
}
fn details_view(d: wire::ConversationDetails) -> view::RoomDetails {
    view::RoomDetails {
        name: d.conversation.name,
        description: d.description,
        read_only: d.read_only,
        notifications: d.notifications.map(|v| match v {
            wire::NotificationLevel::Default => view::NotificationLevel::Default,
            wire::NotificationLevel::Always => view::NotificationLevel::Always,
            wire::NotificationLevel::Mention => view::NotificationLevel::Mention,
            wire::NotificationLevel::Never => view::NotificationLevel::Never,
        }),
        call_notifications: d.call_notifications,
        important: d.important,
        sensitive: d.conversation.sensitive,
        classified: d.conversation.classified,
        preserved: d.preserved,
        message_expiration: d.message_expiration,
        default_permissions: d.default_permissions,
        server_default_permissions: d.server_default_permissions,
        effective_permissions: d.effective_permissions,
        lobby: d.lobby,
        has_password: d.has_password,
        participant_count: d.participant_count,
        guest_access: Some(d.conversation.kind == RoomKind::Public),
        leave: d.conversation.actions.leave_conversation,
        controls: page(
            d.controls
                .as_slice()
                .iter()
                .map(|(key, a)| (setting(*key), availability(*a))),
        ),
    }
}
fn setting(s: wire::Setting) -> view::RoomSetting {
    use view::RoomSetting as V;
    use wire::Setting as W;
    match s {
        W::Metadata => V::Metadata,
        W::Avatar => V::Avatar,
        W::ReadOnly => V::ReadOnly,
        W::Notifications => V::Notifications,
        W::Important => V::Important,
        W::Sensitive => V::Sensitive,
        W::Expiration => V::Expiration,
        W::Preserve => V::Preserve,
        W::GuestAccess => V::GuestAccess,
        W::Password => V::Password,
        W::Discoverability => V::Discoverability,
        W::Mentions => V::Mentions,
        W::Permissions => V::Permissions,
        W::Lobby => V::Lobby,
        W::RecordingConsent => V::RecordingConsent,
        W::Delete => V::Delete,
        W::ClearHistory => V::ClearHistory,
        W::Keep => V::Keep,
        W::Members => V::Members,
        W::Invite => V::Invite,
        W::Roles => V::Roles,
        W::Bots => V::Bots,
        W::Bans => V::Bans,
        W::Breakouts => V::Breakouts,
    }
}
fn source_view(s: wire::RecipientSource) -> view::DirectorySource {
    match s {
        wire::RecipientSource::User => view::DirectorySource::User,
        wire::RecipientSource::Group => view::DirectorySource::Group,
        wire::RecipientSource::Team => view::DirectorySource::Team,
        wire::RecipientSource::Email => view::DirectorySource::Email,
        wire::RecipientSource::Federated => view::DirectorySource::Federated,
        wire::RecipientSource::Phone => view::DirectorySource::Phone,
    }
}
fn source_wire(s: view::DirectorySource) -> wire::RecipientSource {
    match s {
        view::DirectorySource::User => wire::RecipientSource::User,
        view::DirectorySource::Group => wire::RecipientSource::Group,
        view::DirectorySource::Team => wire::RecipientSource::Team,
        view::DirectorySource::Email => wire::RecipientSource::Email,
        view::DirectorySource::Federated => wire::RecipientSource::Federated,
        view::DirectorySource::Phone => wire::RecipientSource::Phone,
    }
}
fn role_view(r: protocol::Role) -> view::MemberRole {
    match r {
        protocol::Role::Owner => view::MemberRole::Owner,
        protocol::Role::Moderator => view::MemberRole::Moderator,
        protocol::Role::Member => view::MemberRole::Member,
        protocol::Role::Guest => view::MemberRole::Guest,
        protocol::Role::Following => view::MemberRole::Following,
        protocol::Role::GuestModerator => view::MemberRole::GuestModerator,
        protocol::Role::Unknown => view::MemberRole::Unknown,
    }
}
fn role_wire(r: view::MemberRole) -> protocol::Role {
    match r {
        view::MemberRole::Owner => protocol::Role::Owner,
        view::MemberRole::Moderator => protocol::Role::Moderator,
        view::MemberRole::Member => protocol::Role::Member,
        view::MemberRole::Guest => protocol::Role::Guest,
        view::MemberRole::Following => protocol::Role::Following,
        view::MemberRole::GuestModerator => protocol::Role::GuestModerator,
        view::MemberRole::Unknown => protocol::Role::Unknown,
    }
}
