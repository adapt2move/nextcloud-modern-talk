//! Bounded visual observation for one automatically replaced call generation.
use std::time::{Duration, Instant};
use talk_core::ui::{
    CallScope,
    presentation::{CallStage, CallView},
};

#[derive(Default)]
pub(in crate::presentation) struct Recovery {
    scope: Option<CallScope>,
    waiting_since: Option<Instant>,
    observed_flow: bool,
}
impl Recovery {
    pub(in crate::presentation) fn observe(&mut self, call: Option<&CallView>, now: Instant) {
        let Some(call) = call.filter(|c| c.replacement_generation && c.stage == CallStage::Active)
        else {
            *self = Self::default();
            return;
        };
        let remote_present = call.participants.as_slice().iter().any(|p| !p.self_user);
        let video_flow = call.streams.as_slice().iter().any(|s| {
            s.flowing
                && call
                    .participants
                    .as_slice()
                    .iter()
                    .any(|p| !p.self_user && p.local_id == s.participant)
        });
        self.observe_values(
            call.scope,
            remote_present,
            call.remote_audio_received > 0 || video_flow,
            now,
        );
    }
    fn observe_values(&mut self, scope: CallScope, remote_present: bool, flow: bool, now: Instant) {
        if self.scope != Some(scope) {
            *self = Self {
                scope: Some(scope),
                ..Self::default()
            };
        }
        if flow {
            self.observed_flow = true;
        }
        if !remote_present || self.observed_flow {
            self.waiting_since = None;
        } else if self.waiting_since.is_none() {
            self.waiting_since = Some(now);
        }
    }
    pub(in crate::presentation) fn visible(&self, scope: CallScope, now: Instant) -> bool {
        self.scope == Some(scope)
            && self
                .waiting_since
                .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_secs(10))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{
        AccountId, AccountScope, CallGeneration, ConversationId, HostId, RuntimeGeneration,
    };
    fn scope(generation: u128) -> CallScope {
        let account = AccountId::from_local(1);
        CallScope {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: 2,
            },
            call: CallGeneration::from_local(generation),
        }
    }
    #[test]
    fn waits_for_remote_peer_and_suppresses_after_flow_or_scope_retirement() {
        let start = Instant::now();
        let mut state = Recovery::default();
        let first = scope(1);
        state.observe_values(first, false, false, start);
        assert!(!state.visible(first, start + Duration::from_secs(30)));
        state.observe_values(first, true, false, start + Duration::from_secs(30));
        assert!(!state.visible(first, start + Duration::from_secs(39)));
        assert!(state.visible(first, start + Duration::from_secs(40)));
        state.observe_values(first, true, true, start + Duration::from_secs(40));
        assert!(!state.visible(first, start + Duration::from_secs(60)));
        state.observe_values(first, true, false, start + Duration::from_secs(60));
        assert!(!state.visible(first, start + Duration::from_secs(90)));
        let second = scope(2);
        state.observe_values(second, true, false, start + Duration::from_secs(90));
        assert!(!state.visible(first, start + Duration::from_secs(110)));
        assert!(state.visible(second, start + Duration::from_secs(100)));
        state.observe(None, start + Duration::from_secs(110));
        assert!(!state.visible(second, start + Duration::from_secs(120)));
    }
}
