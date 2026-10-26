//! Native forward dialog. The session owns approval; views only project bounded data.
mod session;

use super::*;
pub(crate) use crate::presentation::messaging::forward::session::{Confirmation, Target};
use crate::presentation::messaging::forward::session::{
    Denial, ReturnSurface, Session, destinations, next_row, return_surface, source,
};
use crate::presentation::motion::DialogSurface;
use crate::presentation::{feedback::Feedback, theme::Theme, window_chrome};
use gpui::{ElementId, KeyDownEvent, ScrollHandle, SharedString, Subscription};

#[derive(Clone)]
pub(crate) enum Action {
    Select(Target, ConversationRef),
    Confirm(Confirmation),
    Cancel(Target),
}

struct ReturnFocus {
    context: ConversationRef,
    surface: ReturnSurface,
    control: ElementId,
}

#[derive(Debug, Eq, PartialEq)]
enum FocusReturn<T> {
    Skip,
    Invoker(T),
    Surface(ReturnSurface),
}

impl ReturnFocus {
    fn resolve<T>(
        &self,
        context: Option<ConversationRef>,
        overlay: Option<Overlay>,
        current: impl FnOnce(&ElementId) -> Option<T>,
    ) -> FocusReturn<T> {
        if context != Some(self.context) || overlay != surface_overlay(self.surface) {
            return FocusReturn::Skip;
        }
        // Replacing narrow CallChat with Forward unmounts its history and
        // retires its handles. Resolve the stable control only after the
        // restored surface prepaints, when a remount has registered its handle.
        current(&self.control).map_or(FocusReturn::Surface(self.surface), FocusReturn::Invoker)
    }
}

pub(in crate::presentation) struct Forward {
    session: Session,
    pub(in crate::presentation) search: Entity<TextInput>,
    _search_observer: Subscription,
    query: SharedString,
    selected_title: Option<Text<256>>,
    scroll: ScrollHandle,
    preview_scroll: ScrollHandle,
    body_scroll: ScrollHandle,
    return_focus: Option<ReturnFocus>,
    pending_entry: bool,
    pending_return: Option<ReturnFocus>,
}
impl Forward {
    pub fn new(theme: Theme, cx: &mut Context<Desktop>) -> Self {
        let search = cx.new(|cx| TextInput::new("Gespräche filtern …", 256, theme, cx));
        let observer = cx.observe(&search, |this, input, cx| {
            let query = input.read(cx).content.clone();
            if this.message_ui.forward.query != query {
                this.message_ui.forward.query = query;
                this.message_ui
                    .forward
                    .scroll
                    .set_offset(gpui::point(px(0.), px(0.)));
            }
            cx.notify();
        });
        Self {
            session: Session::default(),
            search,
            _search_observer: observer,
            query: "".into(),
            selected_title: None,
            scroll: ScrollHandle::new(),
            preview_scroll: ScrollHandle::new(),
            body_scroll: ScrollHandle::new(),
            return_focus: None,
            pending_entry: false,
            pending_return: None,
        }
    }
}

fn dialog_id(kind: &'static str, target: Target) -> ElementId {
    scoped_id(
        kind,
        target.context,
        target.message,
        u128::from(target.generation),
    )
}
fn destination_id(target: Target, destination: ConversationRef) -> ElementId {
    // Source message and generation distinguish callbacks when the same room
    // occurs in two successive dialogs. Destination identity survives filtering.
    scoped_id(
        "forward-destination",
        destination,
        target.message,
        u128::from(target.generation),
    )
}
fn overlay_surface(overlay: Option<Overlay>) -> Option<ReturnSurface> {
    match overlay {
        None => Some(ReturnSurface::Timeline),
        Some(Overlay::CallChat) => Some(ReturnSurface::CallChat),
        Some(Overlay::Thread) => Some(ReturnSurface::Thread),
        _ => None,
    }
}
fn surface_overlay(surface: ReturnSurface) -> Option<Overlay> {
    match surface {
        ReturnSurface::Timeline => None,
        ReturnSurface::CallChat => Some(Overlay::CallChat),
        ReturnSurface::Thread => Some(Overlay::Thread),
    }
}
fn denial_label(denial: Denial) -> &'static str {
    match denial {
        Denial::Source => "Weiterleiten ist derzeit nicht verfügbar.",
        Denial::Connection => "Die Verbindung ist derzeit nicht bereit.",
        Denial::Destination => "Das Zielgespräch ist nicht mehr verfügbar.",
        Denial::Selection => "Kein Ziel ausgewählt.",
    }
}

impl Desktop {
    fn forward_interrupted(&self) -> bool {
        self.update_restart_pending
            || self.projection_blocked
            || self.prejoin.is_some()
            || self.incoming_visible()
            || self.onboarding_open
            || self.account_removal.confirming()
            || self.navigation_open
    }

    pub(super) fn forward_source_available(&self, context: ConversationRef, message: u128) -> bool {
        !self.forward_interrupted()
            && source(&self.snapshot, context, message).is_some()
            && self
                .snapshot
                .message_panel
                .as_ref()
                .is_some_and(|panel| !panel.state.pending() || self.local_message_intents())
    }

    pub(super) fn open_forward_dialog(
        &mut self,
        context: ConversationRef,
        message: u128,
        return_control: ElementId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(surface) = overlay_surface(self.overlay) else {
            return;
        };
        if !self.forward_source_available(context, message) {
            return;
        }
        let Some(original) = source(&self.snapshot, context, message) else {
            return;
        };
        if self
            .message_ui
            .forward
            .session
            .open(context, original)
            .is_none()
        {
            return;
        }
        let forward = &mut self.message_ui.forward;
        forward.return_focus = Some(ReturnFocus {
            context,
            surface,
            control: return_control,
        });
        forward.pending_return = None;
        forward.selected_title = None;
        forward.pending_entry = true;
        forward.scroll.set_offset(gpui::point(px(0.), px(0.)));
        forward
            .preview_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        forward.body_scroll.set_offset(gpui::point(px(0.), px(0.)));
        forward.search.update(cx, |input, cx| {
            input.reset();
            cx.notify();
        });
        self.overlay = Some(Overlay::Forward);
        self.pending_modal_focus = false;
        self.hover_hints.cancel();
        self.modal_focus.focus(window);
        cx.notify();
    }

    /// Root Close/Escape uses the same retirement route as explicit Cancel.
    pub(in crate::presentation) fn close_forward_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let owns_overlay = self.overlay == Some(Overlay::Forward);
        if !owns_overlay && self.message_ui.forward.session.target().is_none() {
            return false;
        }
        let restore_focus = !self.forward_interrupted();
        let forward = &mut self.message_ui.forward;
        forward.session.close();
        forward.selected_title = None;
        forward.pending_entry = false;
        let prior = forward.return_focus.take();
        if owns_overlay {
            let destination = prior.as_ref().and_then(|prior| {
                return_surface(prior.surface, prior.context, self.snapshot.selected, true)
            });
            self.overlay = destination.and_then(surface_overlay);
            // A different selected context must never regain a source control.
            forward.pending_return = prior.filter(|_| destination.is_some() && restore_focus);
            self.pending_modal_focus = false;
            if restore_focus {
                if self.overlay.is_some() {
                    self.modal_focus.focus(window);
                } else {
                    self.navigation.history_focus().focus(window);
                }
            }
        }
        forward.search.update(cx, |input, cx| {
            input.interactive = false;
            input.reset();
            input.configure_caret(self.reduced_motion);
            cx.notify();
        });
        owns_overlay
    }

    pub(super) fn forward_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay != Some(Overlay::Forward) {
            return;
        }
        let target = match action {
            Action::Select(target, _) | Action::Cancel(target) => target,
            Action::Confirm(confirmation) => confirmation.target,
        };
        if self.message_ui.forward.session.target() != Some(target) {
            return;
        }
        if self.forward_interrupted()
            || !self
                .message_ui
                .forward
                .session
                .source_current(&self.snapshot)
        {
            self.close_forward_dialog(window, cx);
            return;
        }
        match action {
            Action::Cancel(target) if self.message_ui.forward.session.target() == Some(target) => {
                self.close_forward_dialog(window, cx);
            }
            Action::Select(target, destination) => {
                if let Some(room) = destinations(&self.snapshot, target.context)
                    .find(|room| room.context == destination)
                {
                    // Selection changes approval only; it never submits a message.
                    if self.message_ui.forward.session.select(target, destination) {
                        self.message_ui.forward.selected_title = Some(room.title.clone());
                    }
                }
            }
            Action::Confirm(confirmation) => {
                if self.message_ui.forward.search.read(cx).composing() {
                    return;
                }
                match self
                    .message_ui
                    .forward
                    .session
                    .confirm(confirmation, &self.snapshot)
                {
                    Ok(()) => {
                        // submit_message retains the typed owner route, including
                        // its bounded queue and backend source/policy validation.
                        // Admission is synchronous. Retire before returning to
                        // event dispatch so a second key-up/click cannot submit
                        // again; a rejected admission keeps the user's choice.
                        if self
                            .submit_message(MessageAction::Forward {
                                message: confirmation.target.message,
                                destination: confirmation.destination,
                            })
                            .is_some()
                        {
                            self.close_forward_dialog(window, cx);
                        }
                    }
                    Err(denial) => {
                        self.feedback = Some(
                            Feedback::info(denial_label(denial))
                                .in_conversation(confirmation.target.context),
                        );
                    }
                }
            }
            Action::Cancel(_) => {}
        }
    }

    /// Called by the existing frame preparation, before overlay projection. No task.
    pub(super) fn prepare_forward_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == Some(Overlay::Forward)
            && self.message_ui.forward.session.target().is_none()
            || self.message_ui.forward.session.target().is_some()
                && (self.overlay != Some(Overlay::Forward)
                    || self.forward_interrupted()
                    || !self
                        .message_ui
                        .forward
                        .session
                        .source_current(&self.snapshot))
        {
            self.close_forward_dialog(window, cx);
        }
        let interactive = self.overlay == Some(Overlay::Forward) && !self.forward_interrupted();
        let placeholder = self.l("Gespräche filtern …").to_owned();
        self.message_ui.forward.search.update(cx, |input, _| {
            input.theme = self.theme;
            input.text_scale = f32::from(self.scale) / 100.;
            input.clipboard_export_allowed = true;
            input.interactive = interactive;
            input.set_placeholder(&placeholder);
            input.configure_caret(self.reduced_motion);
        });
    }

    /// Invoke for every accepted snapshot, before it can be replaced again. An
    /// invalidated source cannot regain authority if a later snapshot resembles it.
    pub(in crate::presentation) fn revalidate_forward_session(&mut self) {
        self.message_ui.forward.session.reconcile(&self.snapshot);
    }

    pub(super) fn finish_forward_dialog_frame(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.message_ui.forward.pending_entry && self.overlay == Some(Overlay::Forward) {
            self.message_ui.forward.pending_entry = false;
            self.message_ui
                .forward
                .search
                .focus_handle(cx)
                .focus(window);
            cx.notify();
        }
        if let Some(prior) = self.message_ui.forward.pending_return.take() {
            match prior.resolve(self.snapshot.selected, self.overlay, |id| {
                self.control_focus.current(id)
            }) {
                FocusReturn::Skip => return,
                FocusReturn::Invoker(handle) => handle.focus(window),
                FocusReturn::Surface(ReturnSurface::Timeline) => {
                    self.navigation.history_focus().focus(window);
                }
                FocusReturn::Surface(ReturnSurface::CallChat | ReturnSurface::Thread) => {
                    self.modal_focus.focus(window);
                }
            }
            cx.notify();
        }
    }

    fn forward_rows(
        &self,
        target: Target,
        cx: &Context<Self>,
    ) -> Vec<&talk_core::ui::presentation::ConversationView> {
        let query = self
            .message_ui
            .forward
            .search
            .read(cx)
            .content
            .trim()
            .to_lowercase();
        destinations(&self.snapshot, target.context)
            .filter(|room| room.title.as_str().to_lowercase().contains(&query))
            .collect()
    }

    pub(super) fn forward_dialog_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(target) = self
            .message_ui
            .forward
            .session
            .target()
            .filter(|_| self.overlay == Some(Overlay::Forward))
        else {
            return false;
        };
        if self.message_ui.forward.search.read(cx).composing() {
            return false;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.control
            || modifiers.platform
            || modifiers.alt
            || modifiers.shift && key != "tab"
        {
            return false;
        }
        if key == "escape" {
            self.forward_action(Action::Cancel(target), window, cx);
            return true;
        }
        let search = self.message_ui.forward.search.focus_handle(cx);
        let rows: Vec<_> = self
            .forward_rows(target, cx)
            .iter()
            .enumerate()
            .filter_map(|(index, room)| {
                self.control_focus
                    .current(&destination_id(target, room.context))
                    .map(|focus| (index, focus))
            })
            .collect();
        if key == "tab" {
            let mut focus = Vec::with_capacity(rows.len() + 5);
            if let Some(close) = self
                .control_focus
                .current(&dialog_id("forward-close", target))
            {
                focus.push(close);
            }
            focus.push(search);
            focus.extend(rows.iter().map(|(_, focus)| focus.clone()));
            if self.feedback_visible()
                && let Some(dismiss) = self.control_focus.current(&"dismiss-feedback".into())
            {
                focus.push(dismiss);
            }
            if let Some(cancel) = self
                .control_focus
                .current(&dialog_id("forward-cancel", target))
            {
                focus.push(cancel);
            }
            if self.forward_confirmation().is_some_and(|confirmation| {
                self.message_ui
                    .forward
                    .session
                    .confirm(confirmation, &self.snapshot)
                    .is_ok()
            }) && let Some(confirm) = self
                .control_focus
                .current(&dialog_id("forward-confirm", target))
            {
                focus.push(confirm);
            }
            let current = focus.iter().position(|handle| handle.is_focused(window));
            if let Some(index) = next_row(
                current,
                focus.len(),
                if modifiers.shift { "up" } else { "down" },
            ) && let Some(handle) = focus.get(index)
            {
                handle.focus(window);
            }
            return true;
        }
        let current = rows.iter().position(|(_, focus)| focus.is_focused(window));
        if matches!(key, "up" | "down") && (search.is_focused(window) || current.is_some())
            || matches!(key, "home" | "end") && current.is_some()
        {
            if let Some(index) = next_row(current, rows.len(), key)
                && let Some((row, handle)) = rows.get(index)
            {
                handle.focus(window);
                self.message_ui.forward.scroll.scroll_to_item(*row);
            }
            return true;
        }
        // Enter in the search field never sends. Native controls activate on key-up.
        key == "enter" && search.is_focused(window)
    }

    fn forward_confirmation(&self) -> Option<Confirmation> {
        let approval = self.message_ui.forward.session.active.as_ref()?;
        Some(Confirmation {
            target: approval.target,
            destination: approval.destination?,
        })
    }

    pub(in crate::presentation) fn forward_dialog(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        let Some(approval) = self.message_ui.forward.session.active.as_ref() else {
            return div();
        };
        let target = approval.target;
        let t = self.theme;
        let rows = self.forward_rows(target, cx);
        let confirmation = self.forward_confirmation();
        let denied = confirmation.map_or(Some(Denial::Selection), |confirmation| {
            self.message_ui
                .forward
                .session
                .confirm(confirmation, &self.snapshot)
                .err()
        });
        let action =
            |action| LocalAction::MessageIn(target.context, super::Action::Forward(action));
        let mut list = div()
            .id(dialog_id("forward-list", target))
            .flex()
            .flex_col()
            .flex_none()
            .max_h(px(220.))
            .overflow_y_scroll()
            .track_scroll(&self.message_ui.forward.scroll)
            .gap_1();
        for room in &rows {
            let selected = approval.destination == Some(room.context);
            list = list.child(
                self.control(
                    destination_id(target, room.context),
                    ButtonStyle::Quiet,
                    Availability::available(),
                    action(Action::Select(target, room.context)),
                    cx,
                )
                .w_full()
                .justify_start()
                .min_h(self.sp(44.))
                .gap_3()
                .bg(if selected { t.selected } else { t.surface })
                .child(self.scaled_icon("chat", 18., t.muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_ellipsis()
                        .child(room.title.as_str().to_owned()),
                )
                .when(selected, |d| {
                    d.child(self.scaled_icon("check", 18., t.accent))
                }),
            );
        }
        if rows.is_empty() {
            list = list.child(self.hint(self.l("Keine Gespräche in diesem Filter.")));
        }
        let selected = approval.destination.and_then(|context| {
            self.snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|room| room.context == context)
        });
        let account = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|account| account.scope == target.context.account);
        let body = div()
            .id(dialog_id("forward-body", target))
            .min_h_0()
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.message_ui.forward.body_scroll)
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.hint(self.l("Weiterleiten als neue Nachricht. Ziel auswählen:")))
            .when_some(account, |body, account| {
                body.child(self.hint(&format!(
                    "{}: {} · {}",
                    self.l("Konto"),
                    account.label.as_str(),
                    account.server_label.as_str()
                )))
            })
            .child(
                div()
                    .id(dialog_id("forward-preview", target))
                    .max_h(px(140.))
                    .flex_none()
                    .overflow_y_scroll()
                    .track_scroll(&self.message_ui.forward.preview_scroll)
                    .rounded(px(8.))
                    .bg(t.subtle)
                    .p_3()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(approval.original.author.as_str().to_owned()),
                    )
                    .child(approval.original.body.as_str().to_owned()),
            )
            .child(
                self.input_surface(&self.message_ui.forward.search, cx)
                    .child(self.scaled_icon("search", 18., t.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.message_ui.forward.search.clone()),
                    ),
            )
            .child(self.hint(
                self.l("Es werden bis zu 100 geladene Gespräche desselben Kontos angezeigt."),
            ))
            .child(list)
            .child(
                div()
                    .rounded(px(8.))
                    .bg(t.subtle)
                    .p_3()
                    .child(self.hint(self.l("Ausgewählt")))
                    .child(
                        selected
                            .map(|room| room.title.as_str())
                            .or_else(|| {
                                self.message_ui
                                    .forward
                                    .selected_title
                                    .as_ref()
                                    .map(Text::as_str)
                            })
                            .unwrap_or_else(|| self.l("Kein Ziel ausgewählt."))
                            .to_owned(),
                    ),
            )
            .when_some(
                denied.filter(|denial| *denial != Denial::Selection),
                |body, denial| body.child(self.hint(self.l(denial_label(denial)))),
            );
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(t.scrim)
            .flex()
            .items_center()
            .justify_center()
            .p_5()
            .child(
                div()
                    .w_full()
                    .max_w(px(560.))
                    .max_h(
                        (window.viewport_size().height - px(window_chrome::HEIGHT + 48.))
                            .max(px(1.)),
                    )
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .bg(t.surface)
                    .rounded(px(13.))
                    .border_1()
                    .border_color(t.border)
                    .shadow_lg()
                    .track_focus(&self.modal_focus)
                    .tab_group()
                    .key_context("ForwardDialog")
                    .occlude()
                    // Keep unfinished search preedit out of application shortcuts.
                    // Root also includes search in its input_composing list.
                    .on_key_down(cx.listener(|this, _: &KeyDownEvent, _, cx| {
                        if this.message_ui.forward.search.read(cx).composing() {
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .px_5()
                            .py_4()
                            .border_b_1()
                            .border_color(t.border)
                            .child(self.title(self.l("Weiterleiten")))
                            .child(self.icon_button(
                                dialog_id("forward-close", target),
                                ("x", "Schließen"),
                                ButtonStyle::Quiet,
                                Availability::available(),
                                action(Action::Cancel(target)),
                                cx,
                            )),
                    )
                    .child(body)
                    .when(self.feedback_visible(), |dialog| {
                        dialog.child(
                            div()
                                .flex_none()
                                .px_5()
                                .pb_3()
                                .child(self.feedback_view(cx)),
                        )
                    })
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_wrap()
                            .justify_end()
                            .gap_3()
                            .px_5()
                            .py_4()
                            .border_t_1()
                            .border_color(t.border)
                            .child(self.button(
                                dialog_id("forward-cancel", target),
                                "Abbrechen",
                                ButtonStyle::Secondary,
                                Availability::available(),
                                action(Action::Cancel(target)),
                                cx,
                            ))
                            .child(
                                self.button_with_icon(
                                    dialog_id("forward-confirm", target),
                                    ("An dieses Gespräch senden", "arrow-right"),
                                    ButtonStyle::Primary,
                                    Availability::when(
                                        denied.is_none(),
                                        denied.map_or("", denial_label),
                                    )
                                    .explained_elsewhere(),
                                    action(Action::Confirm(confirmation.unwrap_or(Confirmation {
                                        target,
                                        destination: target.context,
                                    }))),
                                    cx,
                                ),
                            ),
                    )
                    .map(|mut panel| {
                        panel.interactivity().element_id =
                            Some(dialog_id("forward-dialog", target));
                        self.dialog_panel(DialogSurface::Overlay(Overlay::Forward), panel)
                    }),
            )
    }
}

#[cfg(test)]
mod tests;
