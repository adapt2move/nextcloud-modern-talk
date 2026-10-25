//! One bounded removal: retire admission, run the identity owner's credential
//! cleanup, then forget the durable row only after confirmed absence.
use super::*;
use talk_core::ui::presentation::{AccountRemovalFailure, AccountRemovalResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Step {
    Retire,
    Cleanup,
    Forget,
}
impl Step {
    fn diagnostic(self) -> DiagnosticOperation {
        match self {
            Self::Retire => DiagnosticOperation::AccountRetire,
            Self::Cleanup => DiagnosticOperation::CredentialCleanup,
            Self::Forget => DiagnosticOperation::AccountForget,
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct Removal {
    request: RequestId,
    account: AccountId,
    step: Step,
    deadline: Instant,
    next_at: Instant,
    awaiting_settlement: bool,
}

fn removal_failure(error: IdentityError) -> AccountRemovalFailure {
    use AccountRemovalFailure as F;
    match error {
        IdentityError::Storage {
            backend: StorageBackend::PrivateFile,
            failure: StorageFailure::AccessDenied,
        } => F::LocalDenied,
        IdentityError::Storage {
            backend: StorageBackend::PrivateFile,
            ..
        } => F::LocalFile,
        IdentityError::Storage { failure, .. } => match failure {
            StorageFailure::Locked => F::NativeLocked,
            StorageFailure::AccessDenied => F::NativeDenied,
            StorageFailure::Cancelled => F::NativeCancelled,
            StorageFailure::Missing | StorageFailure::Unsupported | StorageFailure::Unavailable => {
                F::NativeUnavailable
            }
            StorageFailure::TemporarilyUnavailable => F::Busy,
            _ => F::NativeFailed,
        },
        IdentityError::RecoveryRequired => F::LocalState,
        IdentityError::SuppressionUnconfirmed => F::SuppressionUnconfirmed,
        IdentityError::WorkerUnavailable => F::Unavailable,
        IdentityError::Request(error) => match error {
            ErrorKind::Deadline => F::TimedOut,
            ErrorKind::Busy => F::Busy,
            ErrorKind::Cancelled => F::Cancelled,
            ErrorKind::WrongScope | ErrorKind::Conflict => F::Changed,
            _ => F::Unavailable,
        },
    }
}
fn removal_event(
    request: RequestId,
    operation: DiagnosticOperation,
    outcome: DiagnosticOutcome,
    failure: Option<AccountRemovalFailure>,
) -> Diagnostic {
    use AccountRemovalFailure as F;
    let mut event = Diagnostic::new(operation, outcome);
    event.request = request.0;
    event.error = failure.map(|failure| match failure {
        F::TimedOut => ErrorKind::Deadline,
        F::Busy => ErrorKind::Busy,
        F::Cancelled => ErrorKind::Cancelled,
        F::Changed => ErrorKind::WrongScope,
        F::Unavailable => ErrorKind::Unconfigured,
        F::CleanupPending => ErrorKind::Uncertain,
        _ => ErrorKind::StorageUnavailable,
    });
    event
}
fn failure_event(
    request: RequestId,
    operation: DiagnosticOperation,
    outcome: DiagnosticOutcome,
    error: IdentityError,
) -> Diagnostic {
    let mut event = removal_event(request, operation, outcome, Some(removal_failure(error)));
    event.error = Some(diagnostic_error(error));
    if let IdentityError::Storage { backend, failure } = error {
        event.storage = Some((backend, failure));
    }
    event
}

impl Accounts {
    pub(super) fn remove_account(
        &mut self,
        request: RequestId,
        account: AccountId,
        scope: Option<AccountScope>,
    ) -> Result<(), SubmitError> {
        let result = self.start_removal(request, account, scope);
        let (outcome, failure) = match result {
            Ok(()) => (DiagnosticOutcome::Started, None),
            Err(error) => (
                DiagnosticOutcome::Rejected,
                Some(match error {
                    SubmitError::Busy => AccountRemovalFailure::Busy,
                    SubmitError::StaleScope => AccountRemovalFailure::Changed,
                    _ => AccountRemovalFailure::Unavailable,
                }),
            ),
        };
        diagnostics::record(removal_event(
            request,
            DiagnosticOperation::AccountRemoval,
            outcome,
            failure,
        ));
        result
    }
    fn start_removal(
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
        // Priority local closure and explicit authority to retire this account's
        // eligible credentials. RetireAll does not itself run provider cleanup.
        self.admit(
            RuntimeCommand::RetireAll { account },
            Purpose::Removal(request, account, Step::Retire),
        )
        .map_err(submit_error)?;
        let now = Instant::now();
        self.removal = Some(Removal {
            request,
            account,
            step: Step::Retire,
            deadline: now + Duration::from_secs(10),
            next_at: now,
            awaiting_settlement: false,
        });
        diagnostics::record(removal_event(
            request,
            Step::Retire.diagnostic(),
            DiagnosticOutcome::Started,
            None,
        ));
        self.preferences.retain(|p| p.scope.account != account);
        self.history.remove_account(account);
        self.outbox.retire_account(account);
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

    fn finish_removal(&mut self, removal: Removal, result: Result<(), AccountRemovalFailure>) {
        diagnostics::record(removal_event(
            removal.request,
            DiagnosticOperation::AccountRemoval,
            if result.is_ok() {
                DiagnosticOutcome::Accepted
            } else {
                DiagnosticOutcome::Failed
            },
            result.err(),
        ));
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
        let Some(removal) = self
            .removal
            .filter(|r| r.request == request && r.account == account && r.step == step)
        else {
            return;
        };
        diagnostics::record(match &result {
            Ok(_) => removal_event(
                request,
                step.diagnostic(),
                DiagnosticOutcome::Accepted,
                None,
            ),
            Err(error) => failure_event(
                request,
                step.diagnostic(),
                DiagnosticOutcome::Failed,
                *error,
            ),
        });
        match (step, result) {
            (Step::Retire | Step::Cleanup, Ok(RuntimeReply::State(summary)))
                if summary.account == account =>
            {
                if summary.suppression_unconfirmed {
                    self.finish_removal(
                        removal,
                        Err(AccountRemovalFailure::SuppressionUnconfirmed),
                    );
                } else if summary.scope.is_some() {
                    self.finish_removal(removal, Err(AccountRemovalFailure::Changed));
                } else {
                    let awaiting_settlement = step == Step::Cleanup && summary.cleanup_pending;
                    self.removal = Some(Removal {
                        step: if step == Step::Cleanup && !summary.cleanup_pending {
                            Step::Forget
                        } else {
                            Step::Cleanup
                        },
                        next_at: Instant::now()
                            + if awaiting_settlement {
                                Duration::from_millis(100)
                            } else {
                                Duration::ZERO
                            },
                        awaiting_settlement,
                        ..removal
                    });
                }
            }
            (Step::Forget, Ok(RuntimeReply::Forgotten(removed))) if removed == account => {
                self.finish_removal(removal, Ok(()))
            }
            (_, Err(IdentityError::Request(ErrorKind::Busy))) => {
                // The identity owner still charges any in-flight provider work.
                // Retry admission within this removal's original fixed deadline.
                self.removal = Some(Removal {
                    next_at: Instant::now() + Duration::from_millis(100),
                    ..removal
                });
            }
            (_, Err(error)) => self.finish_removal(removal, Err(removal_failure(error))),
            _ => self.finish_removal(removal, Err(AccountRemovalFailure::Unavailable)),
        }
    }
    pub(super) fn tick_removal(&mut self) {
        let Some(removal) = self.removal else {
            return;
        };
        let now = Instant::now();
        if now >= removal.deadline {
            let failure = if removal.awaiting_settlement {
                AccountRemovalFailure::CleanupPending
            } else {
                AccountRemovalFailure::TimedOut
            };
            diagnostics::record(removal_event(
                removal.request,
                removal.step.diagnostic(),
                DiagnosticOutcome::Failed,
                Some(failure),
            ));
            self.finish_removal(removal, Err(failure));
            return;
        }
        if now < removal.next_at || self.pending.iter().any(|p| matches!(p.purpose, Purpose::Removal(request, account, _) if request == removal.request && account == removal.account)) {
            return;
        }
        let command = match removal.step {
            Step::Retire => RuntimeCommand::RetireAll {
                account: removal.account,
            },
            Step::Cleanup => RuntimeCommand::Maintain {
                account: removal.account,
            },
            Step::Forget => RuntimeCommand::ForgetAccount {
                account: removal.account,
            },
        };
        match self.admit(
            command,
            Purpose::Removal(removal.request, removal.account, removal.step),
        ) {
            Ok(_) => diagnostics::record(removal_event(
                removal.request,
                removal.step.diagnostic(),
                DiagnosticOutcome::Started,
                None,
            )),
            Err(IdentityError::Request(ErrorKind::Busy)) => {
                self.removal = Some(Removal {
                    next_at: now + Duration::from_millis(100),
                    ..removal
                });
            }
            Err(error) => {
                let failure = removal_failure(error);
                diagnostics::record(failure_event(
                    removal.request,
                    removal.step.diagnostic(),
                    DiagnosticOutcome::Rejected,
                    error,
                ));
                self.finish_removal(removal, Err(failure));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retired(account: AccountId) -> AccountSummary {
        AccountSummary {
            account,
            navigation_key: talk_core::preferences::navigation::NavigationAccountKey::new(
                "a1:00000000000000000000000000000001:00000000000000000000000000000002",
            )
            .unwrap(),
            server_label: None,
            connected_once: true,
            account_label: None,
            attempt: None,
            scope: None,
            phase: talk_adapters::identity::AccountPhase::Disconnected,
            display_name: text(""),
            storage: StoredState::Suppressed,
            cleanup_pending: true,
            suppression_unconfirmed: false,
            connection_degraded: false,
            connection_error: None,
            recovery: None,
            revision: 2,
        }
    }

    #[test]
    fn removal_dispatches_cleanup_instead_of_waiting_for_an_unscheduled_worker() {
        let mut model = super::super::tests::model();
        let account = AccountId::from_local(23);
        let request = RequestId(47);
        model.removal = Some(Removal {
            request,
            account,
            step: Step::Retire,
            deadline: Instant::now() + Duration::from_secs(10),
            next_at: Instant::now(),
            awaiting_settlement: false,
        });
        let summary = retired(account);
        model.complete_removal(
            request,
            account,
            Step::Retire,
            Ok(RuntimeReply::State(summary.clone())),
        );
        model.tick_removal();
        // This fixture has no connection worker. An actual cleanup submission
        // must report that failure now, not silently wait for a 10-second timeout.
        assert!(
            model.snapshot.account_removal.is_some(),
            "retirement must schedule credential cleanup"
        );
        assert!(model.removal.is_none());
    }
    fn removing(step: Step) -> (Accounts, AccountId, RequestId) {
        let mut model = super::super::tests::model();
        let account = AccountId::from_local(23);
        let request = RequestId(47);
        model.removal = Some(Removal {
            account,
            request,
            step,
            deadline: Instant::now() + Duration::from_secs(10),
            next_at: Instant::now(),
            awaiting_settlement: false,
        });
        (model, account, request)
    }
    #[test]
    fn removal_waits_for_owned_cleanup_and_forget_replies_and_rejects_stale_results() {
        let (mut model, account, request) = removing(Step::Retire);
        let summary = retired(account);
        model.complete_removal(
            request,
            account,
            Step::Retire,
            Ok(RuntimeReply::State(summary.clone())),
        );
        assert_eq!(model.removal.unwrap().step, Step::Cleanup);
        model.pending.push(Pending {
            id: ConnectionRequestId(1),
            purpose: Purpose::Removal(request, account, Step::Cleanup),
        });
        model.tick_removal();
        assert!(
            model.snapshot.account_removal.is_none(),
            "in-flight cleanup must not be duplicated or skipped"
        );
        model.pending.clear();
        model.complete_removal(
            request,
            account,
            Step::Cleanup,
            Ok(RuntimeReply::State(summary.clone())),
        );
        assert_eq!(model.removal.unwrap().step, Step::Cleanup);
        assert!(model.removal.unwrap().awaiting_settlement);
        model.tick_removal();
        assert!(
            model.snapshot.account_removal.is_none(),
            "late writer wait is throttled"
        );
        let mut clean = summary;
        clean.cleanup_pending = false;
        model.complete_removal(
            RequestId(99),
            account,
            Step::Cleanup,
            Ok(RuntimeReply::State(clean.clone())),
        );
        assert_eq!(model.removal.unwrap().step, Step::Cleanup);
        model.complete_removal(
            request,
            account,
            Step::Cleanup,
            Ok(RuntimeReply::State(clean)),
        );
        assert_eq!(model.removal.unwrap().step, Step::Forget);
        assert!(
            model.snapshot.account_removal.is_none(),
            "provider cleanup alone is not account removal"
        );
        model.complete_removal(
            request,
            AccountId::from_local(99),
            Step::Forget,
            Ok(RuntimeReply::Forgotten(account)),
        );
        assert!(model.snapshot.account_removal.is_none());
        model.complete_removal(
            request,
            account,
            Step::Forget,
            Ok(RuntimeReply::Forgotten(account)),
        );
        assert_eq!(model.snapshot.account_removal.unwrap().result, Ok(()));
        assert!(model.removal.is_none());
    }
    #[test]
    fn removal_preserves_provider_errors_and_never_forgets_an_unsettled_writer() {
        for failure in [
            StorageFailure::Locked,
            StorageFailure::AccessDenied,
            StorageFailure::Cancelled,
        ] {
            let (mut model, account, request) = removing(Step::Cleanup);
            model.complete_removal(
                request,
                account,
                Step::Cleanup,
                Err(IdentityError::Storage {
                    backend: StorageBackend::MacKeychain,
                    failure,
                }),
            );
            assert_eq!(
                model.snapshot.account_removal.unwrap().result,
                Err(match failure {
                    StorageFailure::Locked => AccountRemovalFailure::NativeLocked,
                    StorageFailure::AccessDenied => AccountRemovalFailure::NativeDenied,
                    _ => AccountRemovalFailure::NativeCancelled,
                })
            );
            assert!(model.pending.is_empty());
        }
        let (mut model, account, request) = removing(Step::Cleanup);
        model.complete_removal(
            request,
            account,
            Step::Cleanup,
            Ok(RuntimeReply::State(retired(account))),
        );
        model.removal.as_mut().unwrap().deadline = Instant::now();
        model.tick_removal();
        assert_eq!(
            model.snapshot.account_removal.unwrap().result,
            Err(AccountRemovalFailure::CleanupPending)
        );
        assert!(model.pending.is_empty());
    }
    #[test]
    fn removal_busy_backoff_keeps_the_original_deadline_and_times_out_without_a_connection() {
        let (mut model, account, request) = removing(Step::Cleanup);
        let deadline = model.removal.unwrap().deadline;
        model.complete_removal(
            request,
            account,
            Step::Cleanup,
            Err(IdentityError::Request(ErrorKind::Busy)),
        );
        assert_eq!(model.removal.unwrap().deadline, deadline);
        model.tick_removal();
        assert!(model.snapshot.account_removal.is_none());
        model.removal.as_mut().unwrap().deadline = Instant::now();
        model.tick_removal();
        assert_eq!(
            model.snapshot.account_removal.unwrap().result,
            Err(AccountRemovalFailure::TimedOut)
        );
        // Late cleanup completion cannot overwrite the retained timeout or a later request.
        model.complete_removal(
            request,
            account,
            Step::Cleanup,
            Ok(RuntimeReply::State(retired(account))),
        );
        assert_eq!(
            model.snapshot.account_removal.unwrap().result,
            Err(AccountRemovalFailure::TimedOut)
        );
    }
    #[test]
    fn removal_log_keeps_the_stage_request_and_exact_safe_storage_failure() {
        let event = failure_event(
            RequestId(47),
            Step::Cleanup.diagnostic(),
            DiagnosticOutcome::Failed,
            IdentityError::Storage {
                backend: StorageBackend::MacKeychain,
                failure: StorageFailure::Locked,
            },
        );
        let line = diagnostics::test_line(event);
        assert!(line.contains("\"request\":47"));
        assert!(line.contains("\"operation\":\"credential_cleanup\""));
        assert!(line.contains("\"storage_backend\":\"mac_keychain\""));
        assert!(line.contains("\"storage_failure\":\"locked\""));
        assert_eq!(
            line.matches(':').count(),
            11,
            "only the two allowlisted storage enums extend the scalar schema"
        );
        assert!(
            !line.contains("account_id") && !line.contains("password") && !line.contains("http")
        );
    }
}
