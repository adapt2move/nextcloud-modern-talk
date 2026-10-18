// Independent temporary behavior probe, not a candidate source or native-client test.
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use talk_adapters::{identity::UnconfiguredIdentity, media::UnconfiguredMedia,
                    nextcloud::UnconfiguredNextcloud};
use talk_core::application::Bootstrap;
use talk_core::identity::{PersistenceState, StoreIntent};
use talk_core::ids::*;
use talk_core::lifecycle::Cancellation;
use talk_core::policy::Moment;
use talk_core::ports::*;
use talk_core::ui::*;

struct NoWake;
impl Wake for NoWake { fn wake(self: Arc<Self>) {} }
fn rejected<T>(mut task: Task<'_, T>) -> bool {
    let waker = Waker::from(Arc::new(NoWake));
    matches!(task.as_mut().poll(&mut Context::from_waker(&waker)),
             Poll::Ready(Err(PortError { kind: ErrorKind::Unconfigured, account: None, retry: Retry::Never })))
}

fn main() {
    let mut assertions = 0;
    macro_rules! check { ($condition:expr) => { assert!($condition); assertions += 1; }; }
    let account = AccountId::from_local(1);
    let scope = AccountScope { account, runtime: RuntimeGeneration::from_local(1) };
    let call = CallScope { account: scope, conversation: ConversationId { account, host: HostId::from_local(1), local: 1 }, call: CallGeneration::from_local(1) };
    let deadline = Moment { epoch: ClockEpoch::from_local(1), millis: 1000 };
    let context = || OperationContext { scope, deadline, cancellation: Cancellation::default() };
    let store = StoreIntent { account, attempt: LoginAttemptId::from_local(1), operation: OperationId::from_local(1), attempted: VaultGeneration::from_local(2), retained: Some(VaultGeneration::from_local(1)) };
    let mut protocol = UnconfiguredNextcloud;
    let mut identity = UnconfiguredIdentity;
    let mut media = UnconfiguredMedia;
    for request in [ProtocolRequest::ProbeAccount, ProtocolRequest::RefreshCallPolicy(call), ProtocolRequest::ReconcileConversation(call.conversation), ProtocolRequest::Leave(call)] {
        check!(rejected(protocol.request(context(), request)));
    }
    for command in [IdentityCommand::Stage(store), IdentityCommand::Activate(store), IdentityCommand::Resolve(scope), IdentityCommand::Reconcile(store), IdentityCommand::DeleteAbandonedAttempt(store), IdentityCommand::SuppressRestoration(account)] {
        check!(rejected(identity.transition(context(), command)));
    }
    for reason in [StopReason::UserLeave, StopReason::AccountRejected, StopReason::PolicyExpired, StopReason::RestrictionObserved, StopReason::Sleep, StopReason::Shutdown] {
        media.close_publication(scope, reason);
        check!(rejected(media.stop(call, reason, deadline)));
    }
    let mut application = Bootstrap::new(protocol, identity, media);
    let state = application.integration_state();
    check!(state.protocol == Availability::Unconfigured && state.identity == Availability::Unconfigured && state.media == Availability::Unconfigured);
    let first = application.take_snapshot().expect("initial snapshot");
    check!(first.account.is_none() && first.availability == Some(UnavailableReason::Unconfigured) && first.persistence == PersistenceState::Unconfigured && first.conversations.as_slice().is_empty() && first.messages.as_slice().is_empty());
    check!(application.take_snapshot().is_none());
    for intent in [UserIntent::SelectConversation(call.conversation), UserIntent::PrepareConference(call), UserIntent::AcceptDirect {call, media: MediaIntent::MicrophoneOffCameraOff}, UserIntent::AcceptDirect {call, media: MediaIntent::MicrophoneOnCameraOff}, UserIntent::AcceptDirect {call, media: MediaIntent::MicrophoneOnCameraOn}, UserIntent::JoinPrepared {call, media: MediaIntent::MicrophoneOnCameraOff}, UserIntent::Leave(call), UserIntent::Logout(account)] {
        check!(application.try_submit(intent) == Err(SubmitError::Unavailable(UnavailableReason::Unconfigured)));
    }
    println!("Independent unconfigured adapter/composition probe: {assertions} behavioral assertions passed; no native I/O or media exercised.");
}
