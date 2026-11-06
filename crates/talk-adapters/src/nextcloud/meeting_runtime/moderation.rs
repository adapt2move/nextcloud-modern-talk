//! A single bounded lane for explicit, non-replayed call moderation requests.
use super::{Session, context};
use crate::nextcloud::transport::NetworkRuntime;
use nextcloud_talk_api::meeting::MeetingModerationWriter;
use std::sync::Arc;
use talk_core::lifecycle::Cancellation;
use tokio::sync::mpsc;

pub(super) enum Request {
    Ring { target: u128, attendee: u64 },
    EndEveryone,
}
pub(super) enum Result {
    Ring { target: u128, accepted: bool },
    EndEveryone { accepted: bool },
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
            Request::EndEveryone => Result::EndEveryone {
                accepted: writer.end_for_everyone(&ctx).await.is_ok(),
            },
        };
        if !session.live() || cancel.is_cancelled() {
            break;
        }
        let _ = results.try_send(result);
    }
}
