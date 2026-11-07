//! Bounded editor for account-owned status messages. Server writes remain in the owner.
use super::{ButtonStyle, Desktop, LocalAction, controls::Availability, probes::Probe};
use gpui::{Context, Div, Focusable, Window, div, prelude::*, px};
use talk_core::preferences::{
    RequestId,
    account::{
        AccountPreferenceChange, AccountPreferenceCommand, AccountPreferenceError,
        AccountPreferenceState, StatusClearAfter, StatusMessage, StatusMessageChange,
    },
};
use talk_core::ui::Text;

impl Desktop {
    pub(super) fn visible_status_message(&self) -> Option<&StatusMessage> {
        let scope = self.snapshot.active_account?;
        let snapshot = self
            .account_preferences
            .as_ref()
            .filter(|p| p.scope == scope)?;
        if let Some((pending_scope, request, value)) = &self.own_message_optimistic
            && *pending_scope == scope
            && snapshot.state == (AccountPreferenceState::Saving { request: *request })
        {
            return value.as_ref();
        }
        snapshot
            .values
            .as_ref()
            .filter(|v| v.scope == scope && v.user_status_enabled == Some(true))?
            .status_message
            .as_ref()
    }

    pub(super) fn edit_status_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.visible_status_message();
        let icon = message
            .and_then(|m| m.icon.as_ref())
            .map_or("", |s| s.as_str())
            .to_owned();
        let body = message
            .and_then(|m| m.message.as_ref())
            .map_or("", |s| s.as_str())
            .to_owned();
        self.own_status_message_input.update(cx, |input, _| {
            input.set_context_value(&body);
        });
        self.own_status_icon_input.update(cx, |input, _| {
            input.set_context_value(&icon);
        });
        self.own_status_clear_after = StatusClearAfter::Never;
        self.own_status_message_error = false;
        self.own_status_editing = true;
        self.own_status_emoji_open = false;
        self.own_status_expiry_open = false;
        self.own_status_message_input.focus_handle(cx).focus(window);
    }

    fn submit_status_message(
        &mut self,
        change: StatusMessageChange,
        value: Option<StatusMessage>,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.account_preferences.as_ref().filter(|p| {
            Some(p.scope) == self.snapshot.active_account
                && p.values
                    .as_ref()
                    .is_some_and(|v| v.user_status_enabled == Some(true))
                && matches!(
                    p.state,
                    AccountPreferenceState::Ready | AccountPreferenceState::Loading { .. }
                )
        }) else {
            return;
        };
        let scope = snapshot.scope;
        if self.development && matches!(self.probe, Probe::Status | Probe::StatusMessage) {
            if let Some(values) = self
                .account_preferences
                .as_mut()
                .and_then(|p| p.values.as_mut())
            {
                values.status_message = value;
            }
            self.own_status_editing = false;
            cx.notify();
            return;
        }
        let Some(number) = self.account_preference_request.checked_add(1) else {
            return;
        };
        self.account_preference_request = number;
        let request = RequestId(number);
        match self
            .settings_port
            .submit_account(AccountPreferenceCommand::Set {
                scope,
                request,
                change: AccountPreferenceChange::StatusMessage(change),
            }) {
            Ok(()) => {
                self.account_preference_error = None;
                self.own_message_optimistic = Some((scope, request, value));
                self.own_status_editing = false;
            }
            Err(AccountPreferenceError::Busy) => {}
            Err(error) => self.account_preference_error = Some(error),
        }
        self.refresh_account_preferences(cx);
    }

    pub(super) fn save_status_message(&mut self, cx: &mut Context<Self>) {
        let message_input = self.own_status_message_input.read(cx);
        let icon_input = self.own_status_icon_input.read(cx);
        let (Some(message), Some(icon)) = (
            message_input.submitted_value(),
            icon_input.submitted_value(),
        ) else {
            return;
        };
        let message = message.trim();
        let emoji_supported = self
            .account_preferences
            .as_ref()
            .and_then(|p| p.values.as_ref())
            .is_some_and(|v| v.user_status_emoji == Some(true));
        let icon = if emoji_supported { icon.trim() } else { "" };
        if message.chars().count() > 80
            || message.chars().any(char::is_control)
            || icon.chars().any(char::is_control)
        {
            self.own_status_message_error = true;
            cx.notify();
            return;
        }
        if message.is_empty() && icon.is_empty() {
            self.clear_status_message(cx);
            return;
        }
        let Ok(message) = Text::new(message) else {
            self.own_status_message_error = true;
            return;
        };
        let icon = if icon.is_empty() {
            None
        } else {
            Text::new(icon).ok()
        };
        let change = StatusMessageChange::Custom {
            icon: icon.clone(),
            message: message.clone(),
            clear_after: self.own_status_clear_after,
        };
        let value = Some(StatusMessage {
            icon,
            message: (!message.as_str().is_empty()).then_some(message),
            clear_at: None,
        });
        self.submit_status_message(change, value, cx);
    }
    pub(super) fn clear_status_message(&mut self, cx: &mut Context<Self>) {
        self.submit_status_message(StatusMessageChange::Clear, None, cx);
    }
    pub(super) fn select_predefined_status(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self
            .account_preferences
            .as_ref()
            .filter(|p| Some(p.scope) == self.snapshot.active_account)
            .and_then(|p| p.values.as_ref())
            .and_then(|v| v.predefined_statuses.as_ref())
            .and_then(|v| v.as_slice().get(index))
            .cloned()
        else {
            return;
        };
        let value = Some(StatusMessage {
            icon: Some(item.icon),
            message: Some(item.message),
            clear_at: None,
        });
        let change = StatusMessageChange::Predefined {
            id: item.id,
            clear_after: self.own_status_clear_after,
        };
        self.submit_status_message(change, value, cx);
    }

    pub(super) fn status_message_editor(&self, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let values = self
            .account_preferences
            .as_ref()
            .filter(|p| Some(p.scope) == self.snapshot.active_account)
            .and_then(|p| p.values.as_ref());
        let ready = self.account_preferences.as_ref().is_some_and(|p| {
            matches!(
                p.state,
                AccountPreferenceState::Ready | AccountPreferenceState::Loading { .. }
            )
        });
        let has_draft = !self
            .own_status_message_input
            .read(cx)
            .content
            .trim()
            .is_empty()
            || values.is_some_and(|v| v.user_status_emoji == Some(true))
                && !self
                    .own_status_icon_input
                    .read(cx)
                    .content
                    .trim()
                    .is_empty();
        let can_save = ready && (has_draft || self.visible_status_message().is_some());
        let mut body = div().flex().flex_col().gap_2().p_3().child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_size(self.sp(13.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(self.l("Statusnachricht")),
                )
                .child(self.icon_button(
                    "status-message-back",
                    ("x", "Zurück"),
                    ButtonStyle::Quiet,
                    Availability::available(),
                    LocalAction::BackStatusMenu,
                    cx,
                )),
        );
        if let Some(choices) = values.and_then(|v| v.predefined_statuses.as_ref()) {
            if !choices.as_slice().is_empty() {
                body = body.child(self.hint(self.l("Vorschläge")));
            }
            for (index, choice) in choices.as_slice().iter().enumerate() {
                body = body.child(
                    self.button(
                        ("predefined-status", index),
                        "",
                        ButtonStyle::Quiet,
                        ready,
                        LocalAction::SelectPredefinedStatus(index),
                        cx,
                    )
                    .w_full()
                    .justify_start()
                    .child(format!(
                        "{} {}",
                        choice.icon.as_str(),
                        choice.message.as_str()
                    )),
                );
            }
        }
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .min_h(px(40.))
                .rounded(px(8.))
                .border_1()
                .border_color(t.input_border)
                .bg(t.subtle)
                .when(
                    values.is_some_and(|v| v.user_status_emoji == Some(true)),
                    |d| {
                        let icon = self.own_status_icon_input.read(cx).content.clone();
                        d.child(
                            self.button(
                                "status-emoji-picker",
                                if icon.is_empty() {
                                    "💬".to_owned()
                                } else {
                                    icon.to_string()
                                },
                                ButtonStyle::Quiet,
                                ready,
                                LocalAction::ToggleStatusEmoji,
                                cx,
                            )
                            .min_h(px(32.))
                            .px_1(),
                        )
                    },
                )
                .child(
                    self.input_surface(&self.own_status_message_input, cx)
                        .flex_1()
                        .border_0()
                        .child(self.own_status_message_input.clone()),
                ),
        );
        if self.own_status_emoji_open {
            body = body.child(
                div().flex().flex_wrap().gap_1().children(
                    ["💬", "😊", "🏠", "🎯", "☕", "🌴"]
                        .into_iter()
                        .enumerate()
                        .map(|(i, emoji)| {
                            self.button(
                                ("status-emoji", i),
                                emoji,
                                ButtonStyle::Quiet,
                                ready,
                                LocalAction::SelectStatusEmoji(emoji),
                                cx,
                            )
                        }),
                ),
            );
        }
        if self.own_status_message_error {
            body = body.child(
                self.hint(self.l("Bitte verwende höchstens 80 Zeichen und keine Zeilenumbrüche.")),
            );
        }
        let expiry_label = match self.own_status_clear_after {
            StatusClearAfter::Never => "Nie",
            StatusClearAfter::ThirtyMinutes => "30 Minuten",
            StatusClearAfter::OneHour => "1 Stunde",
            StatusClearAfter::FourHours => "4 Stunden",
        };
        body = body
            .child(
                self.button(
                    "status-expiry-toggle",
                    self.lf("Löschen nach: {time}", &[("time", self.l(expiry_label))]),
                    ButtonStyle::Secondary,
                    ready,
                    LocalAction::ToggleStatusExpiry,
                    cx,
                )
                .child(self.scaled_icon("chevron-down", 16., t.muted)),
            )
            .when(self.own_status_expiry_open, |body| {
                body.child(
                    div().flex().flex_wrap().gap_1().children(
                        [
                            (StatusClearAfter::Never, "Nie"),
                            (StatusClearAfter::ThirtyMinutes, "30 Minuten"),
                            (StatusClearAfter::OneHour, "1 Stunde"),
                            (StatusClearAfter::FourHours, "4 Stunden"),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(i, (choice, label))| {
                            self.button(
                                ("status-expiry", i),
                                label,
                                if choice == self.own_status_clear_after {
                                    ButtonStyle::Secondary
                                } else {
                                    ButtonStyle::Quiet
                                },
                                ready,
                                LocalAction::SelectStatusExpiry(choice),
                                cx,
                            )
                        }),
                    ),
                )
            });
        body = body.child(
            div()
                .flex()
                .gap_1()
                .child(self.button(
                    "save-status-message",
                    "Speichern",
                    ButtonStyle::Primary,
                    Availability::when(can_save, "Kein Status gesetzt."),
                    LocalAction::SaveStatusMessage,
                    cx,
                ))
                .when(self.visible_status_message().is_some(), |d| {
                    d.child(self.button(
                        "clear-status-message",
                        "Entfernen",
                        ButtonStyle::Quiet,
                        Availability::when(
                            ready && self.visible_status_message().is_some(),
                            "Kein Status gesetzt.",
                        ),
                        LocalAction::ClearStatusMessage,
                        cx,
                    ))
                }),
        );
        div()
            .absolute()
            .bottom(px(58.))
            .left_0()
            .right_0()
            .occlude()
            .rounded(px(12.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .shadow_lg()
            .child(
                self.scroll_area(super::navigation::Region::Overlay)
                    .max_h(px(480.))
                    .child(body),
            )
    }
}
