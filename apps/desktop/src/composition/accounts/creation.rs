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
        panel.result = None;
        if matches!(work, Work::People) {
            panel.people = Page::empty();
            panel.people_loaded = false;
        }
        panel.state = if write {
            RemoteState::Saving(request)
        } else {
            RemoteState::Loading(request)
        };
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
                Completion::ServerAccepted
            }
            Err(error) => completion_error(error),
            _ => Completion::Failed(UnavailableReason::PolicyStale),
        };
        panel.state = match completion {
            Completion::Queued | Completion::ServerAccepted => RemoteState::Idle,
            Completion::Failed(reason) => RemoteState::Failed(request, reason),
            Completion::Uncertain => RemoteState::Uncertain(request),
        };
        if matches!(work, Work::Create) {
            panel.uncertain = completion == Completion::Uncertain;
            panel.result = Some(CreationResult {
                request,
                completion,
                conversation: created,
            });
        }
        if let Some(context) = created {
            let _ = self.open(context);
        }
        self.touch();
    }
}
