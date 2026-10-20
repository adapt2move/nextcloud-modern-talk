//! The API account owner retains the socket and mailbox. Only bounded, exact-scope
//! invalidations cross into composition; room tokens confer no local authority.
use nextcloud_talk_api::events::{LiveEvent, LiveEventBatch, LiveState};
use talk_core::{
    bounded::Page,
    ids::AccountScope,
    ports::ErrorKind,
    protocol::{AccountChange, AccountLiveEvents, AccountLiveState},
};

pub(super) fn project(
    scope: AccountScope,
    batch: LiveEventBatch,
) -> Result<AccountLiveEvents, ErrorKind> {
    if batch.scope.account.key() != scope.account.as_local()
        || batch.scope.runtime.key() != scope.runtime.as_local()
    {
        return Err(ErrorKind::WrongScope);
    }
    if batch.events.len() > 32 {
        return Err(ErrorKind::TooLarge);
    }
    let mut rooms = false;
    let mut messages = false;
    let mut calls = false;
    for event in batch.events {
        match event {
            LiveEvent::RefreshAll | LiveEvent::MembershipChanged { .. } => {
                rooms = true;
                messages = true;
                calls = true;
            }
            LiveEvent::RoomChanged { .. } => rooms = true,
            LiveEvent::ChatAvailable { .. } => {
                rooms = true;
                messages = true;
            }
            LiveEvent::CallChanged { .. } => {
                rooms = true;
                calls = true;
            }
        }
    }
    let mut changes = Page::empty();
    for (changed, kind) in [
        (calls, AccountChange::IncomingCalls),
        (messages, AccountChange::Messages),
        (rooms, AccountChange::Conversations),
    ] {
        if changed {
            changes.try_push(kind).map_err(|_| ErrorKind::TooLarge)?;
        }
    }
    Ok(AccountLiveEvents {
        scope,
        state: match batch.state {
            LiveState::Internal => AccountLiveState::Internal,
            LiveState::Connecting => AccountLiveState::Connecting,
            LiveState::Connected => AccountLiveState::Connected,
            LiveState::Reconnecting => AccountLiveState::Reconnecting,
            LiveState::Stopped(kind) => AccountLiveState::Stopped(super::transport::kind(kind)),
        },
        first_transport_failure: batch.first_transport_failure.map(super::transport::kind),
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nextcloud_talk_api as api;
    use talk_core::ids::{AccountId, RuntimeGeneration};

    fn scope() -> AccountScope {
        AccountScope {
            account: AccountId::from_local(1),
            runtime: RuntimeGeneration::from_local(2),
        }
    }
    fn batch(events: Vec<LiveEvent>) -> LiveEventBatch {
        LiveEventBatch {
            scope: api::scope::SessionScope {
                account: api::scope::BindingId::from_key(1),
                runtime: api::scope::LeaseId::from_key(2),
            },
            state: LiveState::Connected,
            first_transport_failure: None,
            events,
        }
    }
    #[test]
    fn every_hint_maps_to_coalesced_authoritative_http_slots() {
        use AccountChange::{Conversations, IncomingCalls, Messages};
        let room = api::models::RoomToken::parse("unboundroom").unwrap();
        for (event, expected) in [
            (
                LiveEvent::RefreshAll,
                vec![IncomingCalls, Messages, Conversations],
            ),
            (
                LiveEvent::RoomChanged { room: room.clone() },
                vec![Conversations],
            ),
            (
                LiveEvent::ChatAvailable { room: room.clone() },
                vec![Messages, Conversations],
            ),
            (
                LiveEvent::CallChanged { room: room.clone() },
                vec![IncomingCalls, Conversations],
            ),
            (
                LiveEvent::MembershipChanged { room },
                vec![IncomingCalls, Messages, Conversations],
            ),
        ] {
            let result = project(scope(), batch(vec![event; 32])).unwrap();
            assert_eq!(result.scope, scope());
            assert_eq!(result.changes.as_slice(), expected.as_slice());
        }
        let result = project(
            scope(),
            batch(vec![LiveEvent::RefreshAll, LiveEvent::RefreshAll]),
        )
        .unwrap();
        assert_eq!(
            result.changes.as_slice(),
            &[IncomingCalls, Messages, Conversations]
        );
    }
    #[test]
    fn wrong_account_runtime_or_oversized_batches_cannot_invalidate() {
        for wrong_account in [true, false] {
            let mut input = batch(vec![LiveEvent::RefreshAll]);
            if wrong_account {
                input.scope.account = api::scope::BindingId::from_key(9);
            } else {
                input.scope.runtime = api::scope::LeaseId::from_key(9);
            }
            assert_eq!(project(scope(), input), Err(ErrorKind::WrongScope));
        }
        assert_eq!(
            project(scope(), batch(vec![LiveEvent::RefreshAll; 33])),
            Err(ErrorKind::TooLarge)
        );
    }
    #[test]
    fn transport_states_and_first_failure_remain_informational() {
        for (state, expected) in [
            (LiveState::Internal, AccountLiveState::Internal),
            (LiveState::Connecting, AccountLiveState::Connecting),
            (LiveState::Connected, AccountLiveState::Connected),
            (LiveState::Reconnecting, AccountLiveState::Reconnecting),
            (
                LiveState::Stopped(api::ErrorKind::PermissionDenied),
                AccountLiveState::Stopped(ErrorKind::PermissionDenied),
            ),
        ] {
            let mut input = batch(Vec::new());
            input.state = state;
            input.first_transport_failure = Some(api::ErrorKind::Transport);
            let result = project(scope(), input).unwrap();
            assert_eq!(result.state, expected);
            assert_eq!(result.first_transport_failure, Some(ErrorKind::Transport));
            assert!(result.changes.as_slice().is_empty());
        }
    }
}
