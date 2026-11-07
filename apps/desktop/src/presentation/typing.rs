//! Composer activity is local, bounded and disposable. No keystroke text or
//! signaling identifier crosses the presentation boundary.
use super::{Desktop, UiIntent};
use gpui::{Context, Div, Focusable, Window, div, prelude::*, px};
use std::time::{Duration, Instant};
use talk_core::ui::{Decision, presentation::ConversationRef};

#[derive(Default)]
pub(super) struct Input {
    context: Option<ConversationRef>,
    generation: u64,
    last_edit: Option<Instant>,
    last_sent: Option<Instant>,
    active: bool,
}
impl Input {
    fn change(
        &mut self,
        context: Option<ConversationRef>,
        generation: u64,
        eligible: bool,
        now: Instant,
    ) -> Option<(ConversationRef, bool)> {
        if self.context != context {
            let old = self.context.filter(|_| self.active);
            self.context = context;
            self.generation = generation;
            self.last_edit = None;
            self.last_sent = None;
            self.active = false;
            return old.map(|context| (context, false));
        }
        let context = context?;
        let changed = self.generation != generation;
        if changed {
            self.generation = generation;
            self.last_edit = Some(now);
        }
        let recent = self
            .last_edit
            .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(4));
        if !eligible || !recent {
            self.last_edit = None;
            if self.active {
                self.active = false;
                return Some((context, false));
            }
        } else if changed
            && (!self.active
                || self
                    .last_sent
                    .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1)))
        {
            self.active = true;
            self.last_sent = Some(now);
            return Some((context, true));
        }
        None
    }
    pub fn stop(&mut self) -> Option<ConversationRef> {
        self.last_edit = None;
        if !self.active {
            return None;
        }
        self.active = false;
        self.context
    }
}
impl Desktop {
    pub(super) fn prepare_typing(&mut self, window: &Window, cx: &Context<Self>) {
        let context = self.snapshot.selected;
        let input = self.composer.read(cx);
        let generation = input.edit_generation();
        let eligible = window.is_window_active()
            && self.composer.focus_handle(cx).is_focused(window)
            && !input.composing()
            && !input.content.trim().is_empty()
            && self.composer_available()
            && self.message_ui.editing.is_none()
            && self.message_feature_enabled(talk_core::ui::messaging::MessageFeature::Typing)
            && self
                .selected()
                .is_some_and(|row| row.send == Decision::Allowed);
        if let Some((context, active)) =
            self.typing_input
                .change(context, generation, eligible, Instant::now())
        {
            let _ = self.port.try_submit(UiIntent::Typing { context, active });
        }
    }
    pub(super) fn stop_local_typing(&mut self) {
        if let Some(context) = self.typing_input.stop() {
            let _ = self.port.try_submit(UiIntent::Typing {
                context,
                active: false,
            });
        }
    }
    pub(super) fn typing_people(
        &self,
        context: ConversationRef,
    ) -> &[talk_core::ui::messaging::TypingPerson] {
        self.snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)
            .map_or(&[], |panel| panel.typing.as_slice())
    }
    pub(super) fn typing_slot(&self, context: ConversationRef) -> Div {
        let people = self.typing_people(context);
        let label = match people {
            [] => None,
            [one] => Some(self.lf("{name} schreibt", &[("name", one.name.as_str())])),
            [first, second] => Some(self.lf(
                "{first} und {second} schreiben",
                &[
                    ("first", first.name.as_str()),
                    ("second", second.name.as_str()),
                ],
            )),
            _ => Some(self.l("Mehrere Personen schreiben").to_owned()),
        };
        div()
            .flex_none()
            .h(px(18.))
            .pl(px(48.))
            .line_height(self.sp(18.))
            .flex()
            .items_center()
            .gap_2()
            .text_size(self.sp(12.))
            .text_color(self.theme.muted)
            .when_some(label, |slot, label| {
                slot.child(label).child(self.typing_dots(context))
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activity_is_throttled_and_stops_on_idle_blur_or_scope_change() {
        let scope = talk_core::ui::AccountScope {
            account: talk_core::ui::AccountId::from_local(1),
            runtime: talk_core::ui::RuntimeGeneration::from_local(1),
        };
        let context = |local| ConversationRef {
            account: scope,
            conversation: talk_core::ui::ConversationId {
                account: scope.account,
                host: talk_core::ui::HostId::from_local(1),
                local,
            },
        };
        let a = context(1);
        let b = context(2);
        let now = Instant::now();
        let mut state = Input::default();
        assert_eq!(state.change(Some(a), 1, true, now), None);
        assert_eq!(state.change(Some(a), 2, true, now), Some((a, true)));
        assert_eq!(
            state.change(Some(a), 3, true, now + Duration::from_millis(500)),
            None
        );
        assert_eq!(
            state.change(Some(a), 4, true, now + Duration::from_secs(1)),
            Some((a, true))
        );
        assert_eq!(
            state.change(Some(a), 4, false, now + Duration::from_secs(2)),
            Some((a, false))
        );
        assert_eq!(
            state.change(Some(a), 5, true, now + Duration::from_secs(3)),
            Some((a, true))
        );
        assert_eq!(
            state.change(Some(b), 1, true, now + Duration::from_secs(3)),
            Some((a, false))
        );
        assert_eq!(
            state.change(Some(b), 2, true, now + Duration::from_secs(4)),
            Some((b, true))
        );
        assert_eq!(
            state.change(Some(b), 2, true, now + Duration::from_secs(8)),
            Some((b, false))
        );
    }
}
