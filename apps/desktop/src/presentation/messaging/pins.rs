//! One scoped pin selection and one bounded, deliberate cycling transition.
use super::*;
use gpui::{AnyElement, FocusHandle};
mod locate;
mod motion;
mod overlay;
use crate::presentation::messaging::pins::motion::{Activity, Kind, Motion, Ticket};
use std::time::Duration;

const CYCLE: Duration = Duration::from_millis(170);

#[derive(Default)]
pub(super) struct Pins {
    context: Option<ConversationRef>,
    selected: Option<u128>,
    motion: Rc<RefCell<Motion>>,
    viewport: Option<gpui::Size<gpui::Pixels>>,
    focus: Option<FocusHandle>,
}

impl Pins {
    pub(super) fn visibility(&self, visible: bool, reduced: bool) {
        if !visible || reduced {
            self.motion.borrow_mut().clear();
        }
    }
    pub(super) fn cancellation(&self) -> impl Fn() + use<> {
        let owner = self.motion.clone();
        move || owner.borrow_mut().clear()
    }
    fn reconcile(&mut self, context: Option<ConversationRef>, ids: &[u128]) {
        if context != self.context {
            self.context = context;
            self.selected = ids.first().copied();
            self.motion.borrow_mut().clear();
        } else if self.selected.is_none_or(|id| !ids.contains(&id)) {
            self.selected = ids.first().copied();
            let mut owner = self.motion.borrow_mut();
            if matches!(owner.active, Some(Activity::Cycle { .. })) {
                owner.cancel();
            }
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
        self.focus = None;
        let mut owner = self.motion.borrow_mut();
        if owner.next().is_some() && motion {
            owner.active = Some(Activity::Cycle {
                started: now,
                forward,
            });
        }
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
        self.reconcile_pin_motion();
        if self.reduced_motion {
            self.message_ui.pins.motion.borrow_mut().cancel();
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
        let animated: AnyElement = {
            let owner = pins.motion.borrow();
            if let Some(Activity::Cycle { started, forward }) = owner.active.as_ref() {
                let progress = (started.elapsed().as_secs_f32() / CYCLE.as_secs_f32()).min(1.);
                let eased = gpui::ease_out_quint()(progress);
                text.relative()
                    .left(px((if *forward { 12. } else { -12. }) * (1. - eased)))
                    .opacity(0.55 + 0.45 * eased)
                    .into_any_element()
            } else {
                text.into_any_element()
            }
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
                .relative()
                .child(self.pin_header_anchor(context, selected))
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

fn fingerprint(message: &MessageView) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&message.local_id, &mut hasher);
    std::hash::Hash::hash(message.body.as_str(), &mut hasher);
    std::hash::Hash::hash(message.author.as_str(), &mut hasher);
    std::hash::Hash::hash(&message.metadata.timestamp, &mut hasher);
    std::hash::Hash::hash(&message.metadata.edited, &mut hasher);
    for object in message.metadata.rich.as_slice() {
        std::hash::Hash::hash(object.placeholder.as_str(), &mut hasher);
        std::hash::Hash::hash(object.name.as_str(), &mut hasher);
    }
    std::hash::Hasher::finish(&hasher)
}

impl Desktop {
    fn pin_source(&self, context: ConversationRef, source: u128) -> Option<&MessageView> {
        if self.snapshot.selected != Some(context) {
            return None;
        }
        self.snapshot
            .messages
            .as_slice()
            .iter()
            .find(|m| m.local_id == source)
            .or_else(|| {
                self.snapshot
                    .message_panel
                    .as_ref()
                    .filter(|p| p.context == context)?
                    .pinned_messages
                    .as_slice()
                    .iter()
                    .find(|m| m.local_id == source)
            })
            .filter(|m| !m.metadata.deleted)
    }

    pub(super) fn admit_pin_intent(
        &mut self,
        context: ConversationRef,
        request: RequestId,
        source: u128,
        pin: bool,
    ) {
        let owner = self.message_ui.pins.motion.clone();
        let mut motion = owner.borrow_mut();
        let generation = motion.next(); // Latest admitted intent replaces the one decorative slot.
        self.message_ui.pins.focus = None;
        if self.reduced_motion {
            return;
        }
        let Some(generation) = generation else {
            return;
        };
        let Some(message) = self.pin_source(context, source) else {
            return;
        };
        let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| p.context == context)
        else {
            return;
        };
        if !motion.retain_pin_authority(panel.pin_change, panel.availability(MessageFeature::Pins))
        {
            return;
        }
        let already_pinned = panel
            .pinned_messages
            .as_slice()
            .iter()
            .any(|m| m.local_id == source && !m.metadata.deleted);
        if already_pinned == pin {
            return;
        }
        let head = self
            .media_span(source)
            .and_then(|span| self.snapshot.messages.as_slice().get(span.start))
            .map_or(source, |message| message.local_id);
        let from = (!pin)
            .then(|| {
                motion
                    .header
                    .filter(|(id, _)| *id == source)
                    .map(|(_, b)| b)
                    .or_else(|| {
                        motion
                            .sources
                            .iter()
                            .find(|(id, _)| *id == head)
                            .map(|(_, b)| *b)
                    })
            })
            .flatten();
        motion.active = Some(Activity::Copy(motion::CopyMotion {
            ticket: Ticket {
                generation,
                request,
                context,
                source,
                fingerprint: fingerprint(message),
            },
            kind: if pin { Kind::Pin } else { Kind::Unpin },
            head,
            excerpt: motion::excerpt(if message.body.as_str().trim().is_empty() {
                self.l("Anhang")
            } else {
                message.body.as_str()
            }),
            admitted: Instant::now(),
            observed: false,
            started: None,
            from,
            to: None,
            error_baseline: panel.action_error.map(|(id, _)| id),
        }));
    }

    fn reconcile_pin_motion(&mut self) {
        let owner = self.message_ui.pins.motion.clone();
        let mut motion = owner.borrow_mut();
        let Some(Activity::Copy(copy)) = motion.active.as_ref() else {
            return;
        };
        let ticket = copy.ticket;
        let error_baseline = copy.error_baseline;
        let current_fingerprint = self
            .pin_source(ticket.context, ticket.source)
            .map(fingerprint);
        if !motion.retain_source(self.snapshot.selected, current_fingerprint) {
            return;
        }
        if !matches!(
            self.snapshot.content,
            ContentState::Ready | ContentState::Offline
        ) {
            motion.clear();
            return;
        }
        let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| p.context == ticket.context)
        else {
            motion.clear();
            return;
        };
        if !motion.retain_pin_authority(panel.pin_change, panel.availability(MessageFeature::Pins))
        {
            return;
        }
        let rejected = panel.result.is_some_and(|(request, result)| {
            request == ticket.request
                && matches!(result, Completion::Failed(_) | Completion::Uncertain)
        }) || panel.action_error.map(|(id, _)| id) != error_baseline;
        let pinned = panel
            .pinned_messages
            .as_slice()
            .iter()
            .any(|m| m.local_id == ticket.source && !m.metadata.deleted);
        // Pinning can split an existing media run. Resolve its *current*
        // painted head again instead of flying from its former neighbour.
        if let Some(Activity::Copy(copy)) = motion.active.as_mut() {
            copy.head = self
                .media_span(ticket.source)
                .and_then(|span| self.snapshot.messages.as_slice().get(span.start))
                .map_or(ticket.source, |message| message.local_id);
        }
        if motion.observe(ticket, pinned, rejected) && pinned {
            self.message_ui.pins.selected = Some(ticket.source);
        }
    }

    pub(super) fn settle_pin_scroll_for_reduced_motion(&self) {
        if !self.reduced_motion {
            return;
        }
        let owner = self.message_ui.pins.motion.borrow();
        if let Some(Activity::Scroll(scroll)) = owner.active.as_ref()
            && self.snapshot.selected == Some(scroll.context)
            && let Some(index) = self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .position(|m| m.local_id == scroll.source && !m.metadata.deleted)
        {
            let index = self
                .media_span(scroll.source)
                .map_or(index, |span| span.start);
            self.message_ui.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: if index == 0 {
                    px(0.)
                } else {
                    -self.message_ui.geometry.borrow().header_inset
                },
            });
        }
    }

    pub(in crate::presentation) fn prepare_pin_motion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reconcile_pin_motion();
        let focus = window.focused(cx);
        let incoming = self.incoming_visible();
        let pins = &mut self.message_ui.pins;
        let mut owner = pins.motion.borrow_mut();
        let focused_elsewhere = pins
            .focus
            .as_ref()
            .is_some_and(|previous| Some(previous) != focus.as_ref());
        let visible = window.is_window_active()
            && !self.reduced_motion
            && !self.projection_blocked
            && !self.update_restart_pending
            && self.snapshot.selected.is_some()
            && !self.navigation_open
            && !incoming
            && !self.onboarding_open
            && self.prejoin.is_none()
            && !self.account_removal.confirming()
            && matches!(self.overlay, None | Some(Overlay::CallChat));
        let viewport = window.viewport_size();
        let resized = pins.viewport.is_some_and(|previous| previous != viewport);
        pins.viewport = Some(viewport);
        if !visible || focused_elsewhere || resized {
            owner.clear();
        }
        if owner.active.is_some() {
            pins.focus = focus;
        } else {
            pins.focus = None;
        }
        owner.begin_frame();
        let now = Instant::now();
        drop(owner);
        self.advance_pin_locate(now);
        let mut owner = self.message_ui.pins.motion.borrow_mut();
        owner.expire(now);
        if owner.active.is_some() {
            window.request_animation_frame();
        }
    }

    pub(super) fn pin_source_anchor(&self, context: ConversationRef, id: u128) -> impl IntoElement {
        let owner = self.message_ui.pins.motion.clone();
        let frame = owner.borrow().frame;
        let current = self.snapshot.selected == Some(context);
        gpui::canvas(
            move |bounds, _, _| {
                let mut owner = owner.borrow_mut();
                if current {
                    owner.source(frame, id, bounds);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
    fn pin_header_anchor(&self, context: ConversationRef, id: u128) -> impl IntoElement {
        let owner = self.message_ui.pins.motion.clone();
        let frame = owner.borrow().frame;
        let current = self.message_ui.pins.context == Some(context);
        gpui::canvas(
            move |bounds, _, _| {
                let mut owner = owner.borrow_mut();
                if current && owner.frame == frame && motion::usable(bounds) {
                    owner.header = Some((id, bounds));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
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
        assert!(pins.motion.borrow().active.is_none());
    }

    #[test]
    fn removal_and_room_replacement_retire_the_old_excerpt_and_animation() {
        let room = context(1);
        let mut pins = Pins::default();
        pins.reconcile(Some(room), &[7, 8]);
        assert!(pins.cycle(room, 7, true, &[7, 8], true, Instant::now()));
        pins.reconcile(Some(room), &[7]);
        assert_eq!(pins.selected, Some(7));
        assert!(pins.motion.borrow().active.is_none());
        pins.reconcile(Some(context(2)), &[]);
        assert!(pins.selected.is_none());
        assert!(!pins.cycle(room, 7, true, &[7, 8], true, Instant::now()));
    }
}
