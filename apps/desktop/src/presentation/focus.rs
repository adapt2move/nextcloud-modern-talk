//! Finite native focus ownership. Keys identify projected items, never their positions.
use gpui::{ElementId, FocusHandle};
use std::collections::HashMap;
use talk_core::ui::AccountScope;
use talk_core::ui::presentation::ConversationRef;

// Projection ceiling: 100 room rows + 100 × (8 attachments × 2 controls
// + 1 thread) + 100 file rows + 100 participant rows + 16 account controls
// + 16 call participants + 300 local-search results + 256 fixed controls
// = 2588 (conservative simultaneous sum, even for exclusive panels). 4096 leaves explicit
// headroom for composed surfaces. Old and new frames coexist: peak <= 8192.
const MAX_CONTROLS: usize = 4096;
const MAX_RETAINED: usize = MAX_CONTROLS * 2;

pub(super) fn account_id(kind: &'static str, scope: AccountScope) -> ElementId {
    ElementId::Name(
        format!(
            "{kind}:{:032x}:{:032x}",
            scope.account.as_local(),
            scope.runtime.as_local()
        )
        .into(),
    )
}

/// Fixed-length local scope components, with no title, route or wire identity.
/// Occurrence disambiguates the same file shown in different message groups.
pub(super) fn scoped_id(
    kind: &'static str,
    context: ConversationRef,
    occurrence: u128,
    item: u128,
) -> ElementId {
    ElementId::Name(
        format!(
            "{kind}:{:032x}:{:032x}:{:032x}:{:032x}:{:032x}:{occurrence:032x}:{item:032x}",
            context.account.account.as_local(),
            context.account.runtime.as_local(),
            context.conversation.account.as_local(),
            context.conversation.host.as_local(),
            context.conversation.local
        )
        .into(),
    )
}

// Generic only to test ownership/lifetime without manufacturing GPUI handles.
struct FrameCache<T> {
    entries: HashMap<ElementId, (T, bool)>,
    admitted: usize,
}
impl<T> Default for FrameCache<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            admitted: 0,
        }
    }
}
impl<T: Clone> FrameCache<T> {
    fn begin(&mut self) {
        self.admitted = 0;
        for (_, seen) in self.entries.values_mut() {
            *seen = false;
        }
    }
    fn get(&mut self, id: ElementId, make: impl FnOnce() -> T) -> Option<T> {
        if let Some((value, seen)) = self.entries.get_mut(&id) {
            if !*seen {
                if self.admitted == MAX_CONTROLS {
                    return None;
                }
                self.admitted += 1;
                *seen = true;
            }
            return Some(value.clone());
        }
        if self.admitted == MAX_CONTROLS || self.entries.len() == MAX_RETAINED {
            // Stable overload: this control remains non-focusable/inert for the
            // frame. Never allocate mismatched ring/action/tooltip handles.
            return None;
        }
        let value = make();
        self.admitted += 1;
        self.entries.insert(id, (value.clone(), true));
        Some(value)
    }
    fn finish(&mut self, mut removed: impl FnMut(&T)) {
        self.entries.retain(|_, (value, seen)| {
            if !*seen {
                removed(value);
            }
            *seen
        });
    }
}
#[derive(Default)]
pub(super) struct FocusStates(std::cell::RefCell<FrameCache<FocusHandle>>);
impl FocusStates {
    pub fn begin_frame(&self) {
        self.0.borrow_mut().begin();
    }
    pub fn get(&self, id: ElementId, cx: &gpui::App) -> Option<FocusHandle> {
        // The bounded projection fits comfortably; overload still cannot abort
        // rendering or attach a different item's action to an old handle.
        self.0.borrow_mut().get(id, || cx.focus_handle())
    }
    pub fn live(&self, id: &ElementId) -> bool {
        self.0
            .borrow()
            .entries
            .get(id)
            .is_some_and(|(_, seen)| *seen)
    }
    pub fn current(&self, id: &ElementId) -> Option<FocusHandle> {
        self.0
            .borrow()
            .entries
            .get(id)
            .filter(|(_, seen)| *seen)
            .map(|(h, _)| h.clone())
    }
    /// Finite fixture diagnostics only: no user labels, item IDs or allocation.
    pub fn named_action(&self, window: &gpui::Window) -> Option<&'static str> {
        let state = self.0.borrow();
        [
            "call",
            "room-info",
            "send",
            "attach",
            "join-prepared",
            "prejoin-back",
            "return-conversation",
            "rejoin-call",
            "reopen-browser",
            "login",
            "accept",
            "decline",
            "leave-call",
            "microphone",
            "camera",
            "screen-share",
        ]
        .into_iter()
        .find(|id| {
            state
                .entries
                .get(&ElementId::from(*id))
                .is_some_and(|(handle, _)| handle.is_focused(window))
        })
    }
    pub fn finish_frame(&self, window: &mut gpui::Window, fallback: &FocusHandle) {
        self.0.borrow_mut().finish(|handle| {
            if handle.is_focused(window) {
                fallback.focus(window);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};
    fn context(account: u128, runtime: u128, host: u128) -> ConversationRef {
        ConversationRef {
            account: AccountScope {
                account: AccountId::from_local(account),
                runtime: RuntimeGeneration::from_local(runtime),
            },
            conversation: ConversationId {
                account: AccountId::from_local(account),
                host: HostId::from_local(host),
                local: 7,
            },
        }
    }
    #[test]
    fn reorder_keeps_item_ownership_but_removal_and_scope_change_retire_it() {
        let a = scoped_id("room", context(1, 1, 1), 0, 1);
        let b = scoped_id("room", context(1, 1, 1), 0, 2);
        let mut cache = FrameCache::default();
        assert_eq!(cache.get(a.clone(), || 10), Some(10));
        assert_eq!(cache.get(b.clone(), || 20), Some(20));
        cache.finish(|_| panic!("present item retired"));
        cache.begin();
        assert_eq!(cache.get(b.clone(), || 30), Some(20));
        assert_eq!(cache.get(a.clone(), || 40), Some(10));
        cache.finish(|_| panic!("reorder retired an item"));
        cache.begin();
        cache.get(b, || 50);
        let mut retired = Vec::new();
        cache.finish(|v| retired.push(*v));
        assert_eq!(retired, [10]);
        assert_eq!(cache.get(a.clone(), || 60), Some(60));
        for scope in [context(2, 1, 1), context(1, 2, 1), context(1, 1, 2)] {
            let key = scoped_id("room", scope, 0, 1);
            assert_ne!(key, a);
            assert_eq!(cache.get(key, || 70), Some(70));
        }
        assert_ne!(
            account_id("account", context(1, 1, 1).account),
            account_id("account", context(1, 2, 1).account)
        );
        assert_ne!(
            scoped_id("file", context(1, 1, 1), 1, 1),
            scoped_id("file", context(1, 1, 1), 2, 1)
        );
    }
    #[test]
    fn budget_is_finite_and_recovers_without_reusing_another_items_identity() {
        let mut cache = FrameCache::default();
        for i in 0..MAX_CONTROLS {
            assert_eq!(cache.get(("test", i).into(), || i), Some(i));
        }
        assert_eq!(cache.get("overflow".into(), || 999), None);
        assert_eq!(
            cache.get("overflow".into(), || panic!("no unstable overflow handle")),
            None
        );
        cache.begin();
        // A fully replaced page fits while the previous generation is retained.
        for i in 0..MAX_CONTROLS {
            assert_eq!(cache.get(("new", i).into(), || i), Some(i));
        }
        assert_eq!(cache.entries.len(), MAX_RETAINED);
        cache.finish(|_| {});
        cache.begin();
        assert_eq!(cache.get(("new", 3usize).into(), || 900), Some(3));
        cache.finish(|_| {});
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.get("overflow".into(), || 999), Some(999));
    }
}
