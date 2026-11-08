//! One bounded, call-owned activity lane. It cannot delay media or policy writes.
use super::{Session, context};
use crate::nextcloud::transport::NetworkRuntime;
use nextcloud_talk_api::{
    meeting::MeetingActivityWriter,
    signaling::{PeerSession, activity::Activity},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use talk_core::lifecycle::Cancellation;
use tokio::sync::{mpsc, watch};

#[derive(Clone)]
pub(super) struct Message {
    pub peers: Vec<PeerSession>,
    pub activity: Activity,
    pub created: Instant,
}

fn merge_hands(previous: Option<Message>, next: Option<Message>) -> Option<Message> {
    let mut next = next?;
    if let Some(previous) = previous.filter(|previous| previous.activity == next.activity) {
        for peer in previous.peers {
            if next.peers.len() < 16 && !next.peers.contains(&peer) {
                next.peers.push(peer);
            }
        }
    }
    Some(next)
}

/// Hand changes are latest-value state; reactions are a small, lossy queue.
/// No send task survives the call, and stale transient reactions are discarded.
pub(super) async fn run(
    mut writer: MeetingActivityWriter,
    mut hands: watch::Receiver<Option<Message>>,
    mut reactions: mpsc::Receiver<Message>,
    mut encryption: mpsc::Receiver<(PeerSession, serde_json::Value)>,
    session: Arc<Session>,
    network: NetworkRuntime,
    cancel: Cancellation,
) {
    let mut pending_hand: Option<Message> = None;
    let mut retry_at = Instant::now();
    let mut backoff = Duration::from_millis(300);
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while session.live() && !cancel.is_cancelled() {
        tokio::select! {
            changed = hands.changed() => {
                if changed.is_err() { break; }
                pending_hand = merge_hands(pending_hand, hands.borrow_and_update().clone());
                retry_at = Instant::now();
                backoff = Duration::from_millis(300);
            }
            message = encryption.recv() => {
                // Key-exchange messages go out at once; the exchange owns their timeouts.
                if let Some((peer, payload)) = message {
                    let ctx = context(session.scope, &network, &cancel, 1500);
                    let _ = writer.send_encryption(&ctx, &peer, &payload).await;
                }
            }
            _ = tick.tick() => {}
        }
        if !session.live() || cancel.is_cancelled() {
            break;
        }
        if let Some(message) = pending_hand.clone() {
            if Instant::now() < retry_at {
                continue;
            }
            let ctx = context(session.scope, &network, &cancel, 1500);
            tokio::select! {
                result = writer.send(&ctx, &message.peers, &message.activity) => {
                    if result.is_ok() { pending_hand = None; }
                    else {
                        retry_at = Instant::now() + backoff;
                        backoff = (backoff * 2).min(Duration::from_secs(5));
                    }
                }
                changed = hands.changed() => {
                    if changed.is_err() { break; }
                    pending_hand = merge_hands(pending_hand, hands.borrow_and_update().clone());
                    retry_at = Instant::now();
                    backoff = Duration::from_millis(300);
                }
            }
        } else if let Ok(message) = reactions.try_recv()
            && message.created.elapsed() < Duration::from_secs(3)
        {
            let ctx = context(session.scope, &network, &cancel, 1500);
            let _ = writer.send(&ctx, &message.peers, &message.activity).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalescing_keeps_pending_targets_but_never_replays_an_older_state() {
        let first = PeerSession::parse("first").unwrap();
        let second = PeerSession::parse("second").unwrap();
        let message = |state, timestamp, peers| Message {
            peers,
            activity: Activity::RaisedHand { state, timestamp },
            created: Instant::now(),
        };
        let pending = message(true, 1, vec![first.clone()]);
        let joined = message(true, 1, vec![second.clone()]);
        let merged = merge_hands(Some(pending), Some(joined)).unwrap();
        assert!(merged.peers == vec![second.clone(), first.clone()]);
        let lower = message(false, 2, vec![first, second]);
        let latest = merge_hands(Some(merged), Some(lower)).unwrap();
        assert_eq!(
            latest.activity,
            Activity::RaisedHand {
                state: false,
                timestamp: 2
            }
        );
        assert!(merge_hands(Some(latest), None).is_none());
    }
}
