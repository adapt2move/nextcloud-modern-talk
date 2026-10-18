use talk_core::bounded::{Page, Text};
use talk_core::identity::{
    CleanupMarker, RestoreDisposition, Settlement, StoreIntent, can_restore,
};
use talk_core::ids::{
    AccountId, AccountScope, CallGeneration, CallScope, ClockEpoch, ConversationId, HostId,
    LoginAttemptId, OperationId, RuntimeGeneration, VaultGeneration,
};
use talk_core::lifecycle::{Admission, RejectionAction, RejectionEvidence, classify};
use talk_core::policy::{Decision, Fact, Gate, Lease, Moment, PolicyError, Source};

fn scope(account: u128, runtime: u128, call: u128) -> CallScope {
    let account_id = AccountId::from_local(account);
    CallScope {
        account: AccountScope {
            account: account_id,
            runtime: RuntimeGeneration::from_local(runtime),
        },
        conversation: ConversationId {
            account: account_id,
            host: HostId::from_local(1),
            local: 1,
        },
        call: CallGeneration::from_local(call),
    }
}

fn time(millis: u64) -> Moment {
    Moment {
        epoch: ClockEpoch::from_local(1),
        millis,
    }
}

#[test]
fn closed_account_rejects_late_work_without_cancelling_another_account() {
    let a = scope(1, 1, 1).account;
    let b = scope(2, 1, 1).account;
    let mut gate_a = Admission::for_validated_scope(a);
    let gate_b = Admission::for_validated_scope(b);
    let outstanding = gate_a.cancellation();
    gate_a.close();
    assert!(outstanding.is_cancelled());
    assert!(!gate_a.admits(a));
    assert!(gate_b.admits(b));
    let replacement = Admission::for_validated_scope(scope(1, 2, 1).account);
    assert!(!replacement.admits(a));
}

#[test]
fn action_and_transport_errors_do_not_invalidate_the_account() {
    assert_eq!(
        classify(RejectionEvidence::ActionDenied),
        RejectionAction::RestrictAction
    );
    assert_eq!(
        classify(RejectionEvidence::TransportFailure),
        RejectionAction::DegradeAndProbe
    );
    assert_eq!(
        classify(RejectionEvidence::ConfirmedAccountProbe),
        RejectionAction::ShutDownAccount
    );
}

#[test]
fn delayed_read_cannot_extend_lease_from_receipt_time() {
    let call = scope(1, 1, 1);
    let lease = Lease::from_qualified_refresh(call, Source::Conversation, time(5_000), time(8_000))
        .unwrap();
    assert_eq!(lease.expires(), time(18_000));
    assert!(lease.is_fresh(call, time(17_999)));
    assert!(!lease.is_fresh(call, time(18_000)));
    assert_eq!(
        Lease::from_qualified_refresh(call, Source::Conversation, time(5_000), time(8_001)),
        Err(PolicyError::RequestExpired)
    );
}

#[test]
fn lease_cannot_cross_accounts_calls_runtime_generations_or_sleep_epochs() {
    let call = scope(1, 1, 1);
    let lease =
        Lease::from_qualified_refresh(call, Source::LocalCapabilities, time(0), time(1)).unwrap();
    assert!(!lease.is_fresh(call, time(0)));
    for other in [scope(2, 1, 1), scope(1, 2, 1), scope(1, 1, 2)] {
        assert!(!lease.is_fresh(other, time(2)));
    }
    assert!(!lease.is_fresh(
        call,
        Moment {
            epoch: ClockEpoch::from_local(2),
            millis: 2,
        }
    ));
    let mut mixed = call;
    mixed.conversation.account = AccountId::from_local(2);
    assert_eq!(
        Lease::from_qualified_refresh(mixed, Source::Conversation, time(0), time(1)),
        Err(PolicyError::WrongScope)
    );
}

#[test]
fn a_capability_alone_never_grants_permission_and_expiry_remains_denied() {
    let call = scope(1, 1, 1);
    let lease =
        Lease::from_qualified_refresh(call, Source::Conversation, time(0), time(1)).unwrap();
    let mut gate = Gate {
        capability: Fact::Yes,
        configured: Fact::Yes,
        effective_permission: Fact::Unknown,
        invariant_allows: Fact::Yes,
    };
    assert_eq!(gate.resolve(Some(lease), call, time(2)), Decision::Unknown);
    gate.effective_permission = Fact::Yes;
    assert_eq!(gate.resolve(Some(lease), call, time(2)), Decision::Allowed);
    assert_eq!(
        gate.resolve(Some(lease), call, time(13_000)),
        Decision::Stale
    );
    gate.invariant_allows = Fact::No;
    assert_eq!(gate.resolve(None, call, time(2)), Decision::Denied);
}

#[test]
fn abandonment_cleanup_never_deletes_a_retained_item_or_trusts_early_absence() {
    let previous = VaultGeneration::from_local(1);
    let attempted = VaultGeneration::from_local(2);
    let account = AccountId::from_local(1);
    let mut marker = CleanupMarker {
        intent: StoreIntent {
            account,
            attempt: LoginAttemptId::from_local(1),
            operation: OperationId::from_local(1),
            attempted,
            retained: Some(previous),
        },
        settlement: Settlement::WriterPending,
    };
    assert!(!marker.complete());
    assert!(!marker.may_delete(account, attempted));
    marker.settlement = Settlement::SettledPresent;
    assert!(marker.may_delete(account, attempted));
    assert!(!marker.may_delete(account, previous));
    assert!(!marker.may_delete(AccountId::from_local(2), attempted));
    assert!(!can_restore(RestoreDisposition::Unresolved, previous));
    assert!(!can_restore(RestoreDisposition::Suppressed, attempted));
    assert!(can_restore(
        RestoreDisposition::Eligible(previous),
        previous
    ));
    assert!(!can_restore(
        RestoreDisposition::Eligible(previous),
        attempted
    ));
    marker.settlement = Settlement::SettledAbsent;
    assert!(marker.complete());
}

#[test]
fn boundary_limits_count_utf8_bytes_and_refuse_page_overflow() {
    assert!(Text::<3>::new("äö").is_err());
    let mut page = Page::<Text<4>, 1>::empty();
    page.try_push(Text::new("äö").unwrap()).unwrap();
    assert!(page.try_push(Text::new("x").unwrap()).is_err());
    assert_eq!(page.as_slice()[0].as_str(), "äö");
}
