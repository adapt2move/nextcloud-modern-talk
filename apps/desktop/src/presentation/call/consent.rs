//! Transient consent selection belongs to one visible call generation and gate.
use talk_core::ui::{
    CallScope,
    presentation::{CallStage, PresentationSnapshot, RequiredGate},
};

pub(in crate::presentation) fn scope(snapshot: &PresentationSnapshot) -> Option<CallScope> {
    snapshot
        .call
        .as_ref()
        .filter(|call| call.stage == CallStage::Required(RequiredGate::RecordingConsent))
        .map(|call| call.scope)
}

#[derive(Default)]
pub(in crate::presentation) struct State {
    scope: Option<CallScope>,
    selected: bool,
}

impl State {
    /// A new or retired gate always clears a prior selection.
    pub(in crate::presentation) fn observe(&mut self, scope: Option<CallScope>) -> bool {
        if self.scope == scope {
            return false;
        }
        self.scope = scope;
        self.selected = false;
        true
    }
    pub(in crate::presentation) fn reset(&mut self) {
        self.selected = false;
    }
    pub(in crate::presentation) fn toggle(&mut self, scope: CallScope) {
        if self.scope == Some(scope) {
            self.selected = !self.selected;
        }
    }
    pub(in crate::presentation) fn selected(&self, scope: CallScope) -> bool {
        self.scope == Some(scope) && self.selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ids::{
        AccountId, AccountScope, CallGeneration, ConversationId, HostId, RuntimeGeneration,
    };
    fn call(generation: u128) -> CallScope {
        let account = AccountId::from_local(1);
        CallScope {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: 1,
            },
            call: CallGeneration::from_local(generation),
        }
    }
    #[test]
    fn consent_is_unselected_and_never_crosses_a_call_or_retired_gate() {
        let mut state = State::default();
        let first = call(1);
        let second = call(2);
        assert!(state.observe(Some(first)));
        assert!(!state.selected(first));
        state.toggle(second);
        assert!(!state.selected(first));
        state.toggle(first);
        assert!(state.selected(first));
        assert!(state.observe(Some(second)));
        assert!(!state.selected(first) && !state.selected(second));
        state.toggle(second);
        assert!(state.observe(None));
        assert!(state.observe(Some(second)));
        assert!(!state.selected(second));
    }
}
