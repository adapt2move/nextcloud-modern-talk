//! One scoped pin selection and one bounded, deliberate cycling transition.
use super::*;
use gpui::{Animation, AnimationExt, AnyElement, ElementId};
use std::time::Duration;

const CYCLE: Duration = Duration::from_millis(170);

#[derive(Default)]
pub(super) struct Pins {
    context: Option<ConversationRef>,
    selected: Option<u128>,
    generation: u64,
    transition: Option<(Instant, bool)>,
}

impl Pins {
    fn reconcile(&mut self, context: Option<ConversationRef>, ids: &[u128]) {
        if context != self.context {
            self.context = context;
            self.selected = ids.first().copied();
            self.transition = None;
        } else if self.selected.is_none_or(|id| !ids.contains(&id)) {
            self.selected = ids.first().copied();
            self.transition = None;
        }
        if ids.is_empty() {
            self.transition = None;
        }
    }

    fn cycle(
        &mut self,
        context: ConversationRef,
        current: u128,
        forward: bool,
        ids: &[u128],
        motion: bool,
        now: Instant,
    ) -> bool {
        if self.context != Some(context) || self.selected != Some(current) || ids.len() < 2 {
            return false;
        }
        let Some(index) = ids.iter().position(|id| *id == current) else {
            return false;
        };
        let next = if forward {
            (index + 1) % ids.len()
        } else {
            (index + ids.len() - 1) % ids.len()
        };
        let Some(id) = ids.get(next).copied() else {
            return false;
        };
        self.selected = Some(id);
        self.transition = self.generation.checked_add(1).and_then(|generation| {
            self.generation = generation;
            motion.then_some((now, forward))
        });
        true
    }
}

impl Desktop {
    pub(super) fn refresh_pin_banner(&mut self) {
        let panel = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| Some(panel.context) == self.snapshot.selected);
        let ids: Vec<_> = panel
            .into_iter()
            .flat_map(|panel| panel.pinned_messages.as_slice())
            .filter(|message| !message.metadata.deleted)
            .map(|message| message.local_id)
            .collect();
        self.message_ui.pins.reconcile(self.snapshot.selected, &ids);
        if self.reduced_motion {
            self.message_ui.pins.transition = None;
        }
    }

    pub(super) fn cycle_pin(&mut self, context: ConversationRef, current: u128, forward: bool) {
        let Some(panel) =
            self.snapshot.message_panel.as_ref().filter(|panel| {
                panel.context == context && self.snapshot.selected == Some(context)
            })
        else {
            return;
        };
        let ids: Vec<_> = panel
            .pinned_messages
            .as_slice()
            .iter()
            .filter(|message| !message.metadata.deleted)
            .map(|message| message.local_id)
            .collect();
        self.message_ui.pins.cycle(
            context,
            current,
            forward,
            &ids,
            !self.reduced_motion,
            Instant::now(),
        );
    }

    pub(in crate::presentation) fn pin_banner(
        &self,
        context: ConversationRef,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let pins = &self.message_ui.pins;
        if pins.context != Some(context) {
            return None;
        }
        let selected = pins.selected?;
        let panel = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)?;
        let rows: Vec<_> = panel
            .pinned_messages
            .as_slice()
            .iter()
            .filter(|message| !message.metadata.deleted)
            .collect();
        let index = rows
            .iter()
            .position(|message| message.local_id == selected && !message.metadata.deleted)?;
        let message = rows.get(index)?;
        let theme = crate::presentation::shell::chat_header::theme(self.theme, self.palette);
        let caption = if message.body.as_str().trim().is_empty() {
            self.l("Anhang")
        } else {
            message.body.as_str()
        };
        let text = div()
            .min_w_0()
            .flex_1()
            .truncate()
            .text_color(theme.text)
            .text_size(self.sp(13.))
            .child(caption.to_owned());
        let animated: AnyElement = if !self.reduced_motion {
            if let Some((admitted, forward)) =
                pins.transition.filter(|(at, _)| at.elapsed() < CYCLE)
            {
                let frames = self.motion_frame_counter();
                text.with_animation(
                    ElementId::Name(format!("pin-cycle-{}", pins.generation).into()),
                    Animation::new(CYCLE),
                    move |element, _| {
                        crate::presentation::motion::count_frame(&frames);
                        let progress =
                            (admitted.elapsed().as_secs_f32() / CYCLE.as_secs_f32()).min(1.);
                        let eased = gpui::ease_out_quint()(progress);
                        element
                            .relative()
                            .left(px((if forward { 12. } else { -12. }) * (1. - eased)))
                            .opacity(0.55 + 0.45 * eased)
                    },
                )
                .into_any_element()
            } else {
                text.into_any_element()
            }
        } else {
            text.into_any_element()
        };
        Some(
            div()
                .flex_none()
                .min_h(px(44.))
                .px_6()
                .py_1()
                .flex()
                .items_center()
                .gap_3()
                .border_b_1()
                .border_color(theme.border.opacity(0.65))
                .bg(crate::presentation::shell::chat_header::surface(theme))
                .child(self.scaled_icon("pin", 15., theme.accent))
                .child(
                    self.control(
                        scoped_id("pin-banner-open", context, selected, 0),
                        ButtonStyle::Quiet,
                        Availability::available(),
                        LocalAction::MessageIn(context, Action::Locate(selected)),
                        cx,
                    )
                    .flex_1()
                    .min_w_0()
                    .justify_start()
                    .min_h(px(32.))
                    .px_1()
                    .py_0()
                    .child(animated),
                )
                .when(rows.len() > 1, |banner| {
                    banner
                        .child(
                            div()
                                .flex_none()
                                .font_family("Google Sans Code")
                                .text_size(self.sp(11.))
                                .text_color(theme.text)
                                .child(format!("{} / {}", index + 1, rows.len())),
                        )
                        .child(self.icon_button(
                            scoped_id("pin-banner-previous", context, 0, 0),
                            ("arrow-left", "Vorherige angeheftete Nachricht"),
                            ButtonStyle::Quiet,
                            Availability::available(),
                            LocalAction::MessageIn(
                                context,
                                Action::CyclePin {
                                    current: selected,
                                    forward: false,
                                },
                            ),
                            cx,
                        ))
                        .child(self.icon_button(
                            scoped_id("pin-banner-next", context, 0, 0),
                            ("arrow-right", "Nächste angeheftete Nachricht"),
                            ButtonStyle::Quiet,
                            Availability::available(),
                            LocalAction::MessageIn(
                                context,
                                Action::CyclePin {
                                    current: selected,
                                    forward: true,
                                },
                            ),
                            cx,
                        ))
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};

    fn context(room: u128) -> ConversationRef {
        let account = AccountId::from_local(1);
        ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: room,
            },
        }
    }

    #[test]
    fn cycles_wrap_and_obsolete_callbacks_cannot_change_the_current_selection() {
        let room = context(1);
        let mut pins = Pins::default();
        pins.reconcile(Some(room), &[7, 8, 9]);
        let now = Instant::now();
        assert!(pins.cycle(room, 7, false, &[7, 8, 9], true, now));
        assert_eq!(pins.selected, Some(9));
        assert!(!pins.cycle(room, 7, true, &[7, 8, 9], true, now));
        assert!(!pins.cycle(context(2), 9, true, &[7, 8, 9], true, now));
        assert!(pins.cycle(room, 9, true, &[7, 8, 9], false, now));
        assert_eq!(pins.selected, Some(7));
        assert!(pins.transition.is_none());
    }

    #[test]
    fn removal_and_room_replacement_retire_the_old_excerpt_and_animation() {
        let room = context(1);
        let mut pins = Pins::default();
        pins.reconcile(Some(room), &[7, 8]);
        assert!(pins.cycle(room, 7, true, &[7, 8], true, Instant::now()));
        pins.reconcile(Some(room), &[7]);
        assert_eq!(pins.selected, Some(7));
        assert!(pins.transition.is_none());
        pins.reconcile(Some(context(2)), &[]);
        assert!(pins.selected.is_none());
        assert!(!pins.cycle(room, 7, true, &[7, 8], true, Instant::now()));
    }
}
