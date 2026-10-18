//! Pure contract predicates with synthetic records. No journal/vault success is simulated.
use talk_core::identity::{
    AbandonChoice, Abandonment, CleanupMarker, CommittedRestoration, IdentityContractError,
    MaintenanceCommand, MaintenanceContext, MaintenanceOutcome, MaintenanceReport,
    PendingAttemptContext, RestoreDisposition, RetentionValidation, SessionOnlyCommit, Settlement,
    StoreIntent, SuppressionCause, can_restore,
};
use talk_core::ids::{
    AccountId, AccountScope, ClockEpoch, LoginAttemptId, OperationId, RuntimeGeneration,
    VaultGeneration,
};
use talk_core::lifecycle::{Admission, Cancellation};
use talk_core::policy::Moment;

fn now(millis: u64) -> Moment {
    Moment {
        epoch: ClockEpoch::from_local(1),
        millis,
    }
}

fn intent() -> StoreIntent {
    StoreIntent {
        account: AccountId::from_local(1),
        attempt: LoginAttemptId::from_local(2),
        operation: OperationId::from_local(3),
        attempted: VaultGeneration::from_local(4),
        retained: Some(VaultGeneration::from_local(5)),
    }
}

fn maintenance() -> MaintenanceContext {
    MaintenanceContext::for_supervisor(intent().account, OperationId::from_local(6), now(0))
        .unwrap()
}

fn pending() -> PendingAttemptContext {
    PendingAttemptContext {
        intent: intent(),
        deadline: now(10_000),
        cancellation: Cancellation::default(),
    }
}

fn request(choice: AbandonChoice) -> Abandonment {
    Abandonment::new(intent(), choice).unwrap()
}

// Synthetic reported commit used to test correlation, not an implementation of a commit.
fn acknowledgement(request: Abandonment, settlement: Settlement) -> MaintenanceReport {
    MaintenanceReport {
        operation: maintenance().operation(),
        outcome: MaintenanceOutcome::Abandoned {
            request,
            restoration: CommittedRestoration {
                account: request.intent().account,
                revision: 7,
                disposition: request.requested_disposition(),
            },
            settlement,
        },
    }
}

#[test]
fn abandonment_rejects_a_mixed_account_or_wrong_previous_generation() {
    let original = intent();
    let p = original.retained.unwrap();
    assert_eq!(
        Abandonment::new(
            StoreIntent {
                attempted: p,
                ..original
            },
            AbandonChoice::Cancel,
        ),
        Err(IdentityContractError::SameGeneration)
    );
    for (retained, generation) in [(None, p), (Some(p), original.attempted)] {
        assert_eq!(
            Abandonment::new(
                StoreIntent {
                    retained,
                    ..original
                },
                AbandonChoice::RetainPrevious {
                    generation,
                    validation: RetentionValidation::RevalidateStored,
                },
            ),
            Err(IdentityContractError::WrongRetainedGeneration)
        );
    }
    assert_eq!(
        Abandonment::new(
            original,
            AbandonChoice::RetainPrevious {
                generation: p,
                validation: RetentionValidation::CurrentRuntime(AccountScope {
                    account: AccountId::from_local(8),
                    runtime: RuntimeGeneration::from_local(9),
                }),
            },
        ),
        Err(IdentityContractError::WrongAccount)
    );
}

#[test]
fn uncertain_n_session_only_needs_the_exact_committed_suppression_ack() {
    let selected = request(AbandonChoice::SessionOnly);
    let report = acknowledgement(selected, Settlement::Unknown);
    let receipt = SessionOnlyCommit::from_acknowledgement(maintenance(), selected, report).unwrap();
    assert!(receipt.matches_live_attempt(&pending(), now(1)));
    assert_eq!(receipt.revision(), 7);

    let MaintenanceOutcome::Abandoned { restoration, .. } = report.outcome else {
        panic!("fixture must contain a committed abandonment");
    };
    for wrong_record in [
        CommittedRestoration {
            account: AccountId::from_local(8),
            ..restoration
        },
        CommittedRestoration {
            disposition: RestoreDisposition::Eligible(intent().attempted),
            ..restoration
        },
        CommittedRestoration {
            disposition: RestoreDisposition::Unresolved,
            ..restoration
        },
    ] {
        let wrong_ack = MaintenanceReport {
            outcome: MaintenanceOutcome::Abandoned {
                request: selected,
                restoration: wrong_record,
                settlement: Settlement::Unknown,
            },
            ..report
        };
        assert!(!wrong_ack.acknowledges_abandonment(maintenance(), selected));
        assert!(
            SessionOnlyCommit::from_acknowledgement(maintenance(), selected, wrong_ack).is_err()
        );
    }
    let wrong_operation = MaintenanceReport {
        operation: intent().operation,
        ..report
    };
    assert!(
        SessionOnlyCommit::from_acknowledgement(maintenance(), selected, wrong_operation).is_err()
    );

    let original = intent();
    for different in [
        StoreIntent {
            account: AccountId::from_local(8),
            ..original
        },
        StoreIntent {
            attempt: LoginAttemptId::from_local(8),
            ..original
        },
        StoreIntent {
            operation: OperationId::from_local(8),
            ..original
        },
        StoreIntent {
            attempted: VaultGeneration::from_local(8),
            ..original
        },
        StoreIntent {
            retained: None,
            ..original
        },
    ] {
        let different_request = Abandonment::new(different, AbandonChoice::SessionOnly).unwrap();
        assert!(
            SessionOnlyCommit::from_acknowledgement(maintenance(), different_request, report)
                .is_err()
        );
    }
}

#[test]
fn explicit_retention_can_acknowledge_only_named_p_and_cancel_does_not_infer_it() {
    let p = intent().retained.unwrap();
    let previous_scope = AccountScope {
        account: intent().account,
        runtime: RuntimeGeneration::from_local(9),
    };
    let previous_session = Admission::for_validated_scope(previous_scope);
    let replacement = pending();
    replacement.cancellation.cancel();
    let selected = request(AbandonChoice::RetainPrevious {
        generation: p,
        validation: RetentionValidation::CurrentRuntime(previous_scope),
    });
    let report = acknowledgement(selected, Settlement::WriterPending);
    assert!(previous_session.admits(previous_scope));
    assert!(report.acknowledges_abandonment(maintenance(), selected));
    let MaintenanceOutcome::Abandoned { restoration, .. } = report.outcome else {
        panic!("fixture must contain an abandonment");
    };
    assert!(can_restore(restoration.disposition, p));
    assert!(!can_restore(restoration.disposition, intent().attempted));
    assert!(SessionOnlyCommit::from_acknowledgement(maintenance(), selected, report).is_err());
    let cancel = request(AbandonChoice::Cancel);
    assert!(!report.acknowledges_abandonment(maintenance(), cancel));
    assert!(!can_restore(cancel.requested_disposition(), p));
    let cancelled = acknowledgement(cancel, Settlement::Unknown);
    assert!(cancelled.acknowledges_abandonment(maintenance(), cancel));
    assert!(SessionOnlyCommit::from_acknowledgement(maintenance(), cancel, cancelled).is_err());
}

#[test]
fn receipt_cannot_pass_attempt_cancellation_deadline_or_changed_identity() {
    let selected = request(AbandonChoice::SessionOnly);
    let receipt = SessionOnlyCommit::from_acknowledgement(
        maintenance(),
        selected,
        acknowledgement(selected, Settlement::Unknown),
    )
    .unwrap();
    let mut attempt = pending();
    attempt.cancellation.cancel();
    assert!(!receipt.matches_live_attempt(&attempt, now(1)));
    attempt = pending();
    assert!(!receipt.matches_live_attempt(&attempt, attempt.deadline));
    assert!(!receipt.matches_live_attempt(
        &attempt,
        Moment {
            epoch: ClockEpoch::from_local(2),
            millis: 1,
        }
    ));
    attempt.intent.attempt = LoginAttemptId::from_local(99);
    assert!(!receipt.matches_live_attempt(&attempt, now(1)));
}

#[test]
fn maintenance_is_bounded_after_runtime_closure_and_scoped_to_one_account() {
    let scope_a = AccountScope {
        account: intent().account,
        runtime: RuntimeGeneration::from_local(1),
    };
    let scope_b = AccountScope {
        account: AccountId::from_local(2),
        runtime: RuntimeGeneration::from_local(1),
    };
    let mut a = Admission::for_validated_scope(scope_a);
    let b = Admission::for_validated_scope(scope_b);
    let old_cancellation = a.cancellation();
    a.close();
    let authority = maintenance();
    let suppression = MaintenanceCommand::Suppress(SuppressionCause::AccountRejected(scope_a));
    assert!(old_cancellation.is_cancelled());
    assert!(authority.admits(scope_a.account, now(1)));
    assert!(suppression.is_scoped_to(authority.account()));
    assert!(!suppression.is_scoped_to(scope_b.account));
    assert!(!authority.admits(scope_b.account, now(1)));
    assert!(!authority.admits(scope_a.account, authority.deadline()));
    assert!(!authority.admits(
        scope_a.account,
        Moment {
            epoch: ClockEpoch::from_local(2),
            millis: 1,
        }
    ));
    assert!(
        MaintenanceContext::for_supervisor(
            scope_a.account,
            OperationId::from_local(1),
            now(u64::MAX)
        )
        .is_none()
    );
    assert!(!a.admits(scope_a));
    assert!(b.admits(scope_b));
    assert!(!b.cancellation().is_cancelled());
}

#[test]
fn late_settlement_cleanup_and_plain_suppression_cannot_be_activation_receipts() {
    let selected = request(AbandonChoice::SessionOnly);
    let restoration = CommittedRestoration {
        account: intent().account,
        revision: 7,
        disposition: RestoreDisposition::Suppressed,
    };
    for settlement in [
        Settlement::Unknown,
        Settlement::SettledPresent,
        Settlement::SettledAbsent,
    ] {
        for outcome in [
            MaintenanceOutcome::Reconciled {
                intent: intent(),
                restoration,
                settlement,
            },
            MaintenanceOutcome::Cleanup {
                marker: CleanupMarker {
                    intent: intent(),
                    settlement,
                },
                restoration,
            },
            MaintenanceOutcome::Suppressed {
                cause: SuppressionCause::AccountRemoval,
                restoration,
            },
        ] {
            let report = MaintenanceReport {
                operation: maintenance().operation(),
                outcome,
            };
            assert!(
                SessionOnlyCommit::from_acknowledgement(maintenance(), selected, report).is_err()
            );
        }
    }
}
