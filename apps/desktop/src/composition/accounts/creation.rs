//! Explicit creation uses the same admitted owner and never guesses a directory identity.
use super::*;
use talk_core::ui::conversation::{
    AccountPerson, AccountPersonRef, CreateAction, CreatePanel, CreationResult, RemoteState,
};
#[derive(Clone, Copy)]
pub(super) enum Work {
    Permissions,
    People,
    Create,
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
        let write = matches!(action, CreateAction::Group(_) | CreateAction::Direct(_));
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
            CreateAction::Refresh => (ProtocolRequest::InspectAccount, Work::Permissions),
            CreateAction::FindPeople(query) => {
                (ProtocolRequest::FindPeople { query }, Work::People)
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
        let completion = match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::Account(probe)))
                if probe.scope == account && matches!(work, Work::Permissions) =>
            {
                panel.group = probe.group_creation;
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
            Ok(RuntimeReply::Protocol(ProtocolReply::Conversation(room)))
                if room.id.account == account.account && matches!(work, Work::Create) =>
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
                Completion::ServerAccepted
            }
            Err(error) => completion_error(error),
            _ => Completion::Failed(UnavailableReason::PolicyStale),
        };
        finish_creation(panel, request, work, completion, created);
        if let Some(room) = created_room {
            self.observe_created_room(account, room);
        }
        if let Some(context) = created {
            let _ = self.open(context);
        }
        self.touch();
    }
}
fn begin_creation(panel: &mut CreatePanel, request: RequestId, work: Work) {
    panel.result = None;
    panel.state = if matches!(work, Work::Create) {
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
) {
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
    }
    panel.state = match completion {
        Completion::Queued | Completion::ServerAccepted => RemoteState::Idle,
        Completion::Failed(reason) => RemoteState::Failed(request, reason),
        Completion::Uncertain => RemoteState::Uncertain(request),
    };
    if matches!(work, Work::Create) {
        panel.uncertain = completion == Completion::Uncertain;
    }
    if matches!(work, Work::Create | Work::People) {
        panel.result = Some(CreationResult {
            request,
            completion,
            conversation: created,
        });
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
        );
        assert_eq!(
            panel.result,
            Some(CreationResult {
                request: RequestId(2),
                completion: Completion::ServerAccepted,
                conversation: None
            })
        );
        assert!(
            panel.uncertain,
            "A directory read does not resolve an uncertain creation"
        );
        begin_creation(&mut panel, RequestId(3), Work::People);
        finish_creation(
            &mut panel,
            RequestId(3),
            Work::People,
            Completion::Failed(UnavailableReason::PermissionDenied),
            None,
        );
        assert!(
            panel.people.as_slice().is_empty(),
            "Known access loss retires retained directory identities"
        );
        assert!(!panel.people_loaded);
    }
}
