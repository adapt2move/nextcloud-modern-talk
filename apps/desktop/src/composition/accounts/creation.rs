//! Explicit creation uses the same admitted owner and never guesses a directory identity.
use super::*;
use talk_core::ui::conversation::{
    AccountPerson, AccountPersonRef, CreateAction, CreatePanel, CreationPreset, CreationResult,
    RemoteState,
};
#[derive(Clone, Copy)]
pub(super) enum Work {
    Permissions,
    People,
    Stage(AccountPersonRef),
    Unstage(AccountPersonRef),
    Create,
    Private(ConversationRef),
}
impl Accounts {
    pub(super) fn create_action(
        &mut self,
        request: RequestId,
        account: AccountScope,
        action: CreateAction,
    ) -> Result<(), SubmitError> {
        self.current(account)?;
        if self.snapshot.active_account != Some(account) {
            return Err(SubmitError::StaleScope);
        }
        if self
            .pending
            .iter()
            .any(|p| matches!(p.purpose,Purpose::Create(_,scope,_)if scope==account))
        {
            return Err(SubmitError::Busy);
        }
        let write = matches!(
            action,
            CreateAction::Group(_)
                | CreateAction::Direct(_)
                | CreateAction::DirectMember(_)
                | CreateAction::Conversation { .. }
                | CreateAction::PrivateReply { .. }
        );
        if write
            && self
                .snapshot
                .create_panel
                .as_ref()
                .is_some_and(|p| p.uncertain)
        {
            return Err(SubmitError::Busy);
        }
        let (command, work) = match action {
            CreateAction::Refresh => (ProtocolRequest::InspectCreation, Work::Permissions),
            CreateAction::FindPeople(query) => {
                (ProtocolRequest::FindPeople { query }, Work::People)
            }
            CreateAction::AddInvite(person) => {
                let panel = self
                    .snapshot
                    .create_panel
                    .as_ref()
                    .ok_or(SubmitError::StaleScope)?;
                if person.account != account
                    || !panel.people.as_slice().iter().any(|p| p.id == person)
                    || panel.invites.as_slice().len() >= 20
                {
                    return Err(SubmitError::StaleScope);
                }
                (
                    ProtocolRequest::StageCreationPerson(protocol::conversation::RecipientId {
                        scope: account,
                        local: person.local,
                    }),
                    Work::Stage(person),
                )
            }
            CreateAction::RemoveInvite(person) => {
                let panel = self
                    .snapshot
                    .create_panel
                    .as_ref()
                    .ok_or(SubmitError::StaleScope)?;
                if person.account != account
                    || !panel.invites.as_slice().iter().any(|p| p.id == person)
                {
                    return Err(SubmitError::StaleScope);
                }
                (
                    ProtocolRequest::UnstageCreationPerson(protocol::conversation::RecipientId {
                        scope: account,
                        local: person.local,
                    }),
                    Work::Unstage(person),
                )
            }
            CreateAction::Conversation {
                name,
                description,
                public,
                preset,
            } => {
                let panel = self
                    .snapshot
                    .create_panel
                    .as_ref()
                    .ok_or(SubmitError::StaleScope)?;
                if panel.group != Decision::Allowed
                    || name.as_str().trim().is_empty()
                    || (public
                        && (panel.public != FeatureSupport::Supported || !panel.public_allowed))
                    || (!public && !panel.private_allowed)
                    || (panel.extended != FeatureSupport::Supported
                        && (!description.as_str().is_empty()
                            || preset.is_some()
                            || !panel.invites.as_slice().is_empty()))
                    || preset
                        .as_ref()
                        .is_some_and(|id| !panel.presets.as_slice().iter().any(|p| &p.id == id))
                {
                    return Err(SubmitError::Unavailable(
                        UnavailableReason::PermissionDenied,
                    ));
                }
                (
                    ProtocolRequest::CreateConversation(protocol::creation::Room {
                        name,
                        description,
                        public,
                        preset,
                        participants: page(panel.invites.as_slice().iter().map(|p| {
                            protocol::conversation::RecipientId {
                                scope: account,
                                local: p.id.local,
                            }
                        })),
                    }),
                    Work::Create,
                )
            }
            CreateAction::Group(name) => {
                if self
                    .snapshot
                    .create_panel
                    .as_ref()
                    .is_none_or(|p| p.group != Decision::Allowed)
                    || name.as_str().trim().is_empty()
                {
                    return Err(SubmitError::Unavailable(
                        UnavailableReason::PermissionDenied,
                    ));
                }
                (ProtocolRequest::CreateGroup { name }, Work::Create)
            }
            CreateAction::Direct(person) => {
                if person.account != account
                    || self
                        .snapshot
                        .create_panel
                        .as_ref()
                        .is_none_or(|p| !p.people.as_slice().iter().any(|v| v.id == person))
                {
                    return Err(SubmitError::StaleScope);
                }
                let operation = self
                    .connection
                    .as_ref()
                    .map_err(|e| submit_error(*e))?
                    .new_operation_id(account)
                    .map_err(submit_error)?;
                (
                    ProtocolRequest::CreateDirect {
                        operation,
                        recipient: protocol::conversation::RecipientId {
                            scope: account,
                            local: person.local,
                        },
                    },
                    Work::Create,
                )
            }
            CreateAction::DirectMember(person) => {
                if person.context.account != account
                    || self.snapshot.selected != Some(person.context)
                    || self
                        .snapshot
                        .conversation_panel
                        .as_ref()
                        .is_none_or(|panel| {
                            panel.context != person.context
                                || !panel.members_loaded
                                || panel.details_stale
                                || !panel.members.as_slice().iter().any(|member| {
                                    member.id == person
                                        && !member.self_user
                                        && member.source
                                            == talk_core::ui::conversation::DirectorySource::User
                                })
                        })
                {
                    return Err(SubmitError::StaleScope);
                }
                let operation = self
                    .connection
                    .as_ref()
                    .map_err(|e| submit_error(*e))?
                    .new_operation_id(account)
                    .map_err(submit_error)?;
                (
                    ProtocolRequest::CreateDirectForMember {
                        operation,
                        member: protocol::conversation::MemberId {
                            conversation: person.context.conversation,
                            local: person.local,
                        },
                    },
                    Work::Create,
                )
            }
            CreateAction::PrivateReply { context, message } => {
                if context.account != account || self.snapshot.selected != Some(context) {
                    return Err(SubmitError::StaleScope);
                }
                let allowed = self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                    panel.context == context
                        && panel
                            .availability(talk_core::ui::messaging::MessageFeature::PrivateReply)
                            .enabled()
                }) && self.snapshot.conversations.as_slice().iter().any(|room| {
                    room.context == context
                        && room.navigation_key.is_some()
                        && room.draft_clipboard == Decision::Allowed
                }) && self
                    .snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .chain(
                        self.snapshot
                            .message_panel
                            .as_ref()
                            .filter(|p| p.context == context)
                            .into_iter()
                            .flat_map(|p| p.thread_messages.as_slice()),
                    )
                    .any(|row| {
                        row.local_id == message
                            && !row.own
                            && !row.metadata.deleted
                            && !row.metadata.system
                            && row.metadata.replyable
                            && row
                                .metadata
                                .author_key
                                .as_ref()
                                .is_some_and(|key| key.as_str().starts_with("5:users:"))
                    });
                if !allowed {
                    return Err(SubmitError::Unavailable(
                        UnavailableReason::PermissionDenied,
                    ));
                }
                let sequence = u64::try_from(message)
                    .ok()
                    .filter(|id| *id > 0)
                    .ok_or(SubmitError::StaleScope)?;
                let operation = self
                    .connection
                    .as_ref()
                    .map_err(|e| submit_error(*e))?
                    .new_operation_id(account)
                    .map_err(submit_error)?;
                (
                    ProtocolRequest::CreatePrivateReply {
                        operation,
                        source: protocol::MessageId {
                            conversation: context.conversation,
                            sequence,
                        },
                    },
                    Work::Private(context),
                )
            }
        };
        self.request(account, command, Purpose::Create(request, account, work))?;
        let panel = self
            .snapshot
            .create_panel
            .get_or_insert_with(|| CreatePanel::new(account));
        // Retained rows remain displayable during a new search. Presentation
        // requires its matching result before enabling a recipient action.
        begin_creation(panel, request, work);
        self.touch();
        Ok(())
    }
    pub(super) fn complete_creation(
        &mut self,
        request: RequestId,
        account: AccountScope,
        work: Work,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(account).is_err() || self.snapshot.active_account != Some(account) {
            return;
        }
        let Some(panel) = self.snapshot.create_panel.as_mut().filter(|p| {
            p.account == account
                && matches!(p.state,RemoteState::Loading(r)|RemoteState::Saving(r)if r==request)
        }) else {
            return;
        };
        let mut created = None;
        let mut created_room = None;
        let incomplete = matches!(
            &result,
            Ok(RuntimeReply::Protocol(ProtocolReply::CreatedConversation {
                incomplete_invitations: true,
                ..
            }))
        );
        let completion = match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::CreationOptions(options)))
                if matches!(work, Work::Permissions) =>
            {
                panel.group = options.group;
                panel.extended = support(options.extended);
                panel.public = support(options.public);
                panel.private_allowed = options.private_allowed;
                panel.public_allowed = options.public_allowed;
                panel.presets = page(options.presets.as_slice().iter().map(|p| CreationPreset {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    description: p.description.clone(),
                }));
                Completion::ServerAccepted
            }
            Ok(RuntimeReply::Protocol(ProtocolReply::CreationPerson(id))) if matches!(work, Work::Stage(person) if person.account == id.scope && person.local == id.local) => {
                if let Some(person) = panel
                    .people
                    .as_slice()
                    .iter()
                    .find(|p| p.id.account == id.scope && p.id.local == id.local)
                    .cloned()
                {
                    if !panel.invites.as_slice().iter().any(|p| p.id == person.id) {
                        panel.invites = page(
                            panel
                                .invites
                                .as_slice()
                                .iter()
                                .cloned()
                                .chain(std::iter::once(person)),
                        );
                    }
                    Completion::ServerAccepted
                } else {
                    Completion::Failed(UnavailableReason::PolicyStale)
                }
            }
            Ok(RuntimeReply::Protocol(ProtocolReply::CreationPerson(id))) if matches!(work, Work::Unstage(person) if person.account == id.scope && person.local == id.local) =>
            {
                panel.invites = page(
                    panel
                        .invites
                        .as_slice()
                        .iter()
                        .filter(|p| p.id.account != id.scope || p.id.local != id.local)
                        .cloned(),
                );
                Completion::ServerAccepted
            }
            Ok(RuntimeReply::Protocol(ProtocolReply::People(people)))
                if matches!(work, Work::People) =>
            {
                if people.as_slice().iter().any(|p| p.id.scope != account) {
                    Completion::Failed(UnavailableReason::PolicyStale)
                } else {
                    panel.people = page(people.as_slice().iter().map(|p| AccountPerson {
                        id: AccountPersonRef {
                            account,
                            local: p.id.local,
                        },
                        name: p.name.clone(),
                        label: p.label.clone(),
                    }));
                    panel.people_loaded = true;
                    Completion::ServerAccepted
                }
            }
            Ok(RuntimeReply::Protocol(
                ProtocolReply::Conversation(room)
                | ProtocolReply::CreatedConversation {
                    room,
                    incomplete_invitations: false,
                },
            )) if room.id.account == account.account
                && matches!(work, Work::Create | Work::Private(_)) =>
            {
                let row = conversation(&room, account);
                created = Some(row.context);
                self.snapshot.conversations = page(
                    std::iter::once(row).chain(
                        self.snapshot
                            .conversations
                            .as_slice()
                            .iter()
                            .filter(|r| Some(r.context) != created)
                            .take(99)
                            .cloned(),
                    ),
                );
                self.load_rooms = Some(account);
                created_room = Some(room);
                panel.invites = Page::empty();
                Completion::ServerAccepted
            }
            Ok(RuntimeReply::Protocol(ProtocolReply::CreatedConversation {
                room,
                incomplete_invitations: true,
            })) if room.id.account == account.account && matches!(work, Work::Create) => {
                let row = conversation(&room, account);
                created = Some(row.context);
                self.snapshot.conversations = page(
                    std::iter::once(row).chain(
                        self.snapshot
                            .conversations
                            .as_slice()
                            .iter()
                            .filter(|r| Some(r.context) != created)
                            .take(99)
                            .cloned(),
                    ),
                );
                self.load_rooms = Some(account);
                created_room = Some(room);
                panel.invites = Page::empty();
                Completion::ServerAccepted
            }
            Err(error) => completion_error(error),
            _ => Completion::Failed(UnavailableReason::PolicyStale),
        };
        finish_creation(panel, request, work, completion, created, incomplete);
        if let Some(room) = created_room {
            self.observe_created_room(account, room);
        }
        if let Some(context) = created
            && match work {
                Work::Private(source) => self.snapshot.selected == Some(source),
                _ => true,
            }
        {
            let _ = self.open(context);
        }
        self.touch();
    }
}
fn begin_creation(panel: &mut CreatePanel, request: RequestId, work: Work) {
    panel.result = None;
    panel.state = if matches!(work, Work::Create | Work::Private(_)) {
        RemoteState::Saving(request)
    } else {
        RemoteState::Loading(request)
    };
}
fn finish_creation(
    panel: &mut CreatePanel,
    request: RequestId,
    work: Work,
    completion: Completion,
    created: Option<ConversationRef>,
    incomplete_invitations: bool,
) {
    if matches!(work, Work::Permissions) && !matches!(completion, Completion::ServerAccepted) {
        panel.group = Decision::Stale;
        panel.extended = FeatureSupport::Unknown;
        panel.public = FeatureSupport::Unknown;
        panel.private_allowed = false;
        panel.public_allowed = false;
        panel.presets = Page::empty();
    }
    if matches!(work, Work::People)
        && matches!(
            completion,
            Completion::Failed(
                UnavailableReason::PermissionDenied | UnavailableReason::ReauthenticationRequired
            )
        )
    {
        panel.people = Page::empty();
        panel.people_loaded = false;
        panel.invites = Page::empty();
    }
    panel.state = match completion {
        Completion::Queued | Completion::ServerAccepted => RemoteState::Idle,
        Completion::Failed(reason) => RemoteState::Failed(request, reason),
        Completion::Uncertain => RemoteState::Uncertain(request),
    };
    if matches!(work, Work::Create | Work::Private(_)) {
        panel.uncertain = completion == Completion::Uncertain;
    }
    if matches!(work, Work::Create | Work::Private(_) | Work::People) {
        panel.result = Some(CreationResult {
            request,
            completion,
            conversation: created,
            incomplete_invitations,
        });
    }
}
fn support(value: protocol::FeatureSupport) -> FeatureSupport {
    match value {
        protocol::FeatureSupport::Supported => FeatureSupport::Supported,
        protocol::FeatureSupport::Unsupported => FeatureSupport::Unsupported,
        protocol::FeatureSupport::Unknown => FeatureSupport::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn people_refresh_retains_rows_and_exposes_its_matching_read_result_without_creating_a_room() {
        let account = super::super::super::fixtures::scope();
        let mut panel = CreatePanel::new(account);
        panel.people_loaded = true;
        panel.uncertain = true;
        panel.people = page([AccountPerson {
            id: AccountPersonRef { account, local: 7 },
            name: text("Synthetic person"),
            label: text("example.test"),
        }]);
        let rows = panel.people.clone();
        begin_creation(&mut panel, RequestId(1), Work::People);
        assert_eq!(panel.people, rows);
        assert!(panel.people_loaded);
        assert_eq!(panel.state, RemoteState::Loading(RequestId(1)));
        finish_creation(
            &mut panel,
            RequestId(1),
            Work::People,
            Completion::Failed(UnavailableReason::Connection(
                ConnectionFailure::Unreachable,
            )),
            None,
            false,
        );
        assert_eq!(panel.people, rows);
        assert_eq!(panel.result.unwrap().request, RequestId(1));
        assert!(panel.uncertain);
        begin_creation(&mut panel, RequestId(2), Work::People);
        assert!(panel.result.is_none());
        assert_eq!(panel.people, rows);
        finish_creation(
            &mut panel,
            RequestId(2),
            Work::People,
            Completion::ServerAccepted,
            None,
            false,
        );
        assert_eq!(
            panel.result,
            Some(CreationResult {
                request: RequestId(2),
                completion: Completion::ServerAccepted,
                conversation: None,
                incomplete_invitations: false,
            })
        );
        assert!(
            panel.uncertain,
            "A directory read does not resolve an uncertain creation"
        );
        begin_creation(&mut panel, RequestId(3), Work::People);
        panel.invites = page(panel.people.as_slice().iter().cloned());
        finish_creation(
            &mut panel,
            RequestId(3),
            Work::People,
            Completion::Failed(UnavailableReason::PermissionDenied),
            None,
            false,
        );
        assert!(
            panel.people.as_slice().is_empty(),
            "Known access loss retires retained directory identities"
        );
        assert!(!panel.people_loaded);
        assert!(panel.invites.as_slice().is_empty());
    }
}
