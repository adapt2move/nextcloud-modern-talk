//! A single bounded lane for explicit, non-replayed call moderation requests.
use super::{Session, context};
use crate::nextcloud::transport::NetworkRuntime;
use nextcloud_talk_api::meeting::MeetingModerationWriter;
use std::sync::Arc;
use talk_core::lifecycle::Cancellation;
use tokio::sync::mpsc;

pub(super) struct Ring {
    pub target: u128,
    pub attendee: u64,
}
pub(super) struct RingResult {
    pub target: u128,
    pub accepted: bool,
}

pub(super) async fn run(
    mut writer: MeetingModerationWriter,
    mut requests: mpsc::Receiver<Ring>,
    results: mpsc::Sender<RingResult>,
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
        let accepted = writer.ring(&ctx, request.attendee).await.is_ok();
        if !session.live() || cancel.is_cancelled() {
            break;
        }
        let _ = results.try_send(RingResult {
            target: request.target,
            accepted,
        });
    }
}
