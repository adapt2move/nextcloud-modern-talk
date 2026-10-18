//! Interface regression only: every concrete identity path remains unconfigured.
use std::task::{Context, Poll, Waker};

use talk_adapters::identity::UnconfiguredIdentity;
use talk_core::identity::{
    AbandonChoice, Abandonment, AttemptCommand, CommittedRestoration, MaintenanceCommand,
    MaintenanceContext, MaintenanceOutcome, MaintenanceReport, PendingAttemptContext,
    RecoveryContext, RestoreDisposition, RetentionValidation, SessionOnlyCommit, Settlement,
    StoreIntent, SuppressionCause,
};
use talk_core::ids::{
    AccountId, AccountScope, ClockEpoch, LoginAttemptId, OperationId, RuntimeGeneration,
    VaultGeneration,
};
use talk_core::lifecycle::{Admission, Cancellation};
use talk_core::policy::Moment;
use talk_core::ports::{Availability, IdentityPort, OperationContext, PortError, Task};

fn assert_unconfigured<T>(mut task: Task<'_, T>) {
    let mut context = Context::from_waker(Waker::noop());
    match task.as_mut().poll(&mut context) {
        Poll::Ready(Err(error)) => assert_eq!(error, PortError::unconfigured()),
        _ => panic!("unconfigured identity must immediately return an error"),
    }
}

fn moment(millis: u64) -> Moment {
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
        retained: None,
    }
}

fn pending() -> PendingAttemptContext {
    PendingAttemptContext {
        intent: intent(),
        deadline: moment(10_000),
        cancellation: Cancellation::default(),
    }
}

fn maintenance() -> MaintenanceContext {
    MaintenanceContext::for_supervisor(intent().account, OperationId::from_local(5), moment(0))
        .unwrap()
}

#[test]
fn fresh_account_and_cold_restore_have_no_runtime_input_and_remain_unconfigured() {
    let mut adapter = UnconfiguredIdentity;
    assert_eq!(adapter.availability(), Availability::Unconfigured);
    for command in [
        AttemptCommand::Authorize,
        AttemptCommand::Stage,
        AttemptCommand::ActivateStored,
    ] {
        assert_unconfigured(adapter.attempt(pending(), command));
    }
    let request = Abandonment::new(intent(), AbandonChoice::SessionOnly).unwrap();
    // Synthetic committed-record input exercises the typed receipt, not a storage adapter.
    let report = MaintenanceReport {
        operation: maintenance().operation(),
        outcome: MaintenanceOutcome::Abandoned {
            request,
            restoration: CommittedRestoration {
                account: intent().account,
                revision: 1,
                disposition: RestoreDisposition::Suppressed,
            },
            settlement: Settlement::Unknown,
        },
    };
    let receipt = SessionOnlyCommit::from_acknowledgement(maintenance(), request, report).unwrap();
    assert_unconfigured(adapter.attempt(pending(), AttemptCommand::ActivateSessionOnly(receipt)));
    assert_unconfigured(adapter.restore(RecoveryContext {
        account: intent().account,
        operation: OperationId::from_local(6),
        deadline: moment(10_000),
        cancellation: Cancellation::default(),
    }));
}

#[test]
fn authenticated_and_post_cancellation_maintenance_paths_never_fake_success() {
    let mut adapter = UnconfiguredIdentity;
    let scope = AccountScope {
        account: intent().account,
        runtime: RuntimeGeneration::from_local(7),
    };
    let mut admission = Admission::for_validated_scope(scope);
    assert_unconfigured(adapter.resolve(OperationContext {
        scope,
        deadline: moment(10_000),
        cancellation: admission.cancellation(),
    }));
    admission.close();
    let replacement = StoreIntent {
        retained: Some(VaultGeneration::from_local(8)),
        ..intent()
    };
    for choice in [
        AbandonChoice::SessionOnly,
        AbandonChoice::Cancel,
        AbandonChoice::RetainPrevious {
            generation: replacement.retained.unwrap(),
            validation: RetentionValidation::CurrentRuntime(scope),
        },
        AbandonChoice::RetainPrevious {
            generation: replacement.retained.unwrap(),
            validation: RetentionValidation::RevalidateStored,
        },
    ] {
        let request = Abandonment::new(replacement, choice).unwrap();
        assert_unconfigured(adapter.maintain(maintenance(), MaintenanceCommand::Abandon(request)));
    }
    for command in [
        MaintenanceCommand::Suppress(SuppressionCause::AccountRejected(scope)),
        MaintenanceCommand::Suppress(SuppressionCause::Logout(scope)),
        MaintenanceCommand::Suppress(SuppressionCause::AccountRemoval),
        MaintenanceCommand::Reconcile(replacement),
        MaintenanceCommand::DeleteAbandonedAttempt(replacement),
    ] {
        assert_unconfigured(adapter.maintain(maintenance(), command));
    }
    assert!(!admission.admits(scope));
}
