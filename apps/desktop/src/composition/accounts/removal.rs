//! One bounded removal. The existing identity owner closes admission, retires
//! credentials and reconciles cleanup before the durable row can be forgotten.
use super::*;
use talk_core::ui::presentation::AccountRemovalResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Step {
    Retire,
    Cleanup,
    Forget,
}
#[derive(Clone, Copy)]
pub(super) struct Removal {
    request: RequestId,
    account: AccountId,
    step: Step,
    deadline: Instant,
}

impl Accounts {
    pub(super) fn remove_account(
        &mut self,
        request: RequestId,
        account: AccountId,
        scope: Option<AccountScope>,
    ) -> Result<(), SubmitError> {
        if self.removal.is_some() || self.snapshot.account_removal.is_some() {
            return Err(SubmitError::Busy);
        }
        if !self
            .snapshot
            .saved_accounts
            .as_slice()
            .iter()
            .any(|row| row.id == account)
        {
            return Err(SubmitError::StaleScope);
        }
        if let Some(scope) = scope {
            if scope.account != account {
                return Err(SubmitError::StaleScope);
            }
            self.current(scope)?;
        } else if self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .any(|row| row.scope.account == account)
        {
            return Err(SubmitError::StaleScope);
        }
        // RetireAll has priority local closure inside the identity owner. It is
        // explicit removal authority, including dormant eligible credentials.
        self.admit(
            RuntimeCommand::RetireAll { account },
            Purpose::Removal(request, account, Step::Retire),
        )
        .map_err(submit_error)?;
        self.removal = Some(Removal {
            request,
            account,
            step: Step::Retire,
            deadline: Instant::now() + Duration::from_secs(10),
        });
        self.preferences.retain(|p| p.scope.account != account);
        self.history.remove_account(account);
        if self
            .login
            .as_ref()
            .and_then(|login| login.owner)
            .is_some_and(|owner| owner.0 == account)
        {
            self.cancel_login();
        }
        if self
            .snapshot
            .active_account
            .is_some_and(|scope| scope.account == account)
        {
            self.select(None);
        }
        Ok(())
    }

    fn finish_removal(&mut self, removal: Removal, result: Completion) {
        self.snapshot.account_removal = Some(AccountRemovalResult {
            request: removal.request,
            account: removal.account,
            result,
        });
        self.removal = None;
        self.touch();
    }

    pub(super) fn complete_removal(
        &mut self,
        request: RequestId,
        account: AccountId,
        step: Step,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        let Some(removal) = self.removal.filter(|removal| {
            removal.request == request && removal.account == account && removal.step == step
        }) else {
            return;
        };
        match (step, result) {
            (Step::Retire, Ok(RuntimeReply::State(summary))) if summary.account == account => {
                self.removal = Some(Removal {
                    step: Step::Cleanup,
                    ..removal
                });
            }
            (Step::Forget, Ok(RuntimeReply::Forgotten(removed))) if removed == account => {
                self.finish_removal(removal, Completion::ServerAccepted);
            }
            (
                _,
                Err(
                    IdentityError::RecoveryRequired
                    | IdentityError::SuppressionUnconfirmed
                    | IdentityError::Storage { .. },
                ),
            ) => {
                self.finish_removal(removal, Completion::Uncertain);
            }
            (_, Err(error)) => self.finish_removal(removal, completion_error(error)),
            _ => self.finish_removal(removal, Completion::Uncertain),
        }
    }

    pub(super) fn tick_removal(&mut self, snapshot: &ConnectionSnapshot) {
        let Some(removal) = self.removal else {
            return;
        };
        if Instant::now() >= removal.deadline {
            self.finish_removal(removal, Completion::Uncertain);
            return;
        }
        if removal.step != Step::Cleanup {
            return;
        }
        let Some(row) = snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|row| row.summary.account == removal.account)
        else {
            self.finish_removal(removal, Completion::Uncertain);
            return;
        };
        if row.summary.suppression_unconfirmed {
            self.finish_removal(removal, Completion::Uncertain);
        } else if !row.summary.cleanup_pending && row.summary.scope.is_none() {
            match self.admit(
                RuntimeCommand::ForgetAccount {
                    account: removal.account,
                },
                Purpose::Removal(removal.request, removal.account, Step::Forget),
            ) {
                Ok(_) => {
                    self.removal = Some(Removal {
                        step: Step::Forget,
                        ..removal
                    })
                }
                Err(IdentityError::Request(ErrorKind::Busy)) => {}
                Err(error) => self.finish_removal(removal, completion_error(error)),
            }
        }
    }
}
