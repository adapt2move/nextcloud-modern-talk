//! A single bounded lane for explicit, non-replayed call moderation requests.
use super::{Session, context};
use crate::nextcloud::transport::NetworkRuntime;
use nextcloud_talk_api::meeting::MeetingModerationWriter;
use std::sync::Arc;
use talk_core::lifecycle::Cancellation;
use tokio::sync::mpsc;

pub(super) enum Request {
    Ring {
        target: u128,
        attendee: u64,
    },
    Mute {
        target: u128,
        peer: nextcloud_talk_api::signaling::PeerSession,
    },
    EndEveryone,
    Recording {
        start: bool,
    },
}
pub(super) enum Result {
    Ring { target: u128, accepted: bool },
    Mute { target: u128, accepted: bool },
    EndEveryone { accepted: bool },
    Recording { start: bool, accepted: bool },
}

pub(super) async fn run(
    mut writer: MeetingModerationWriter,
    mut requests: mpsc::Receiver<Request>,
    results: mpsc::Sender<Result>,
    session: Arc<Session>,
    network: NetworkRuntime,
    cancel: Cancellation,
) {
    while session.live() && !cancel.is_cancelled() {
        let Some(request) = requests.recv().await else {
            break;
        };
        if !session.live() || cancel.is_cancelled() {
            break;
        }
        let ctx = context(session.scope, &network, &cancel, 12000);
        let result = match request {
            Request::Ring { target, attendee } => Result::Ring {
                target,
                accepted: writer.ring(&ctx, attendee).await.is_ok(),
            },
            Request::Mute { target, peer } => Result::Mute {
                target,
                accepted: writer.mute_participant(&ctx, &peer).await.is_ok(),
            },
            Request::EndEveryone => Result::EndEveryone {
                accepted: writer.end_for_everyone(&ctx).await.is_ok(),
            },
            Request::Recording { start } => Result::Recording {
                start,
                accepted: writer.set_recording(&ctx, start).await.is_ok(),
            },
        };
        if !session.live() || cancel.is_cancelled() {
            break;
        }
        let _ = results.try_send(result);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecordingResolution {
    Pending,
    Confirmed,
    Uncertain,
}

/// Do not display or settle a recording state from an expired or invalidated
/// conversation observation, even if the last refresh completed successfully.
pub(super) fn current_recording(
    observed: talk_core::policy::Observed<talk_core::policy::RecordingState>,
    scope: talk_core::ids::CallScope,
    now: talk_core::policy::Moment,
    policy_ready: bool,
) -> talk_core::policy::RecordingState {
    use talk_core::policy::{Observed, RecordingState};
    match observed {
        Observed::Known { value, lease } if policy_ready && lease.is_fresh(scope, now) => value,
        _ => RecordingState::Unknown,
    }
}

/// A receipt alone is not a recording state. Only a later fresh observation
/// settles the request; expiry never replays it or claims that it stopped.
pub(super) fn recording_resolution(
    start: bool,
    accepted: bool,
    fresh: bool,
    observed: talk_core::policy::RecordingState,
    expired: bool,
) -> RecordingResolution {
    use talk_core::policy::RecordingState;
    if expired {
        return RecordingResolution::Uncertain;
    }
    if !accepted || !fresh {
        return RecordingResolution::Pending;
    }
    if observed == RecordingState::Failed {
        return RecordingResolution::Uncertain;
    }
    if (start && matches!(observed, RecordingState::Starting | RecordingState::Active))
        || (!start && observed == RecordingState::Inactive)
    {
        RecordingResolution::Confirmed
    } else {
        RecordingResolution::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::{
        ids::{
            AccountId, AccountScope, CallGeneration, CallScope, ClockEpoch, ConversationId, HostId,
            RuntimeGeneration,
        },
        policy::{Lease, Moment, Observed, RecordingState, Source},
    };

    #[test]
    fn recording_status_expires_and_cannot_cross_call_scope() {
        let account = AccountId::from_local(1);
        let scope = CallScope {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: 1,
            },
            call: CallGeneration::from_local(1),
        };
        let now = Moment {
            epoch: ClockEpoch::from_local(1),
            millis: 10,
        };
        let lease = Lease::from_qualified_refresh(scope, Source::Conversation, now, now).unwrap();
        let observed = Observed::Known {
            value: RecordingState::Active,
            lease,
        };
        assert_eq!(
            current_recording(observed, scope, now, true),
            RecordingState::Active
        );
        assert_eq!(
            current_recording(observed, scope, now, false),
            RecordingState::Unknown
        );
        assert_eq!(
            current_recording(
                observed,
                CallScope {
                    call: CallGeneration::from_local(2),
                    ..scope
                },
                now,
                true
            ),
            RecordingState::Unknown
        );
        assert_eq!(
            current_recording(
                observed,
                scope,
                Moment {
                    millis: lease.expires().millis,
                    ..now
                },
                true
            ),
            RecordingState::Unknown
        );
    }

    #[test]
    fn recording_receipts_never_fabricate_or_replay_a_result() {
        assert_eq!(
            recording_resolution(false, true, false, RecordingState::Inactive, false),
            RecordingResolution::Pending
        );
        assert_eq!(
            recording_resolution(false, true, true, RecordingState::Active, false),
            RecordingResolution::Pending
        );
        assert_eq!(
            recording_resolution(false, true, true, RecordingState::Inactive, false),
            RecordingResolution::Confirmed
        );
        assert_eq!(
            recording_resolution(true, true, true, RecordingState::Starting, false),
            RecordingResolution::Confirmed
        );
        assert_eq!(
            recording_resolution(true, true, true, RecordingState::Failed, false),
            RecordingResolution::Uncertain
        );
        assert_eq!(
            recording_resolution(false, false, false, RecordingState::Unknown, true),
            RecordingResolution::Uncertain
        );
    }
}
