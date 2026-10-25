//! Reminder and scheduled-send choices use the same bounded local-clock owner.
use super::*;
use talk_core::preferences::time::{DeadlineChoice, LocalTimeError, RepeatedTime};

impl Desktop {
    pub(in crate::presentation) fn message_deadline_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.message_ui.time_target.is_none()
            || self.overlay.is_some()
            || self.message_ui.time_input.read(cx).composing()
        {
            return false;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt {
            return false;
        }
        let action = match event.keystroke.key.as_str() {
            "escape" => Action::CancelTime,
            "enter"
                if !modifiers.shift
                    && self
                        .message_ui
                        .time_input
                        .focus_handle(cx)
                        .is_focused(window) =>
            {
                Action::SubmitTime
            }
            _ => return false,
        };
        self.message_action(action, window, cx);
        cx.stop_propagation();
        true
    }
    pub(super) fn open_message_deadline(
        &mut self,
        target: Option<u128>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.content_ui.menu = false;
        self.message_ui.menu.clear();
        self.message_ui.time_target = Some(target);
        self.message_ui.time_error = None;
        let value = target
            .and_then(|message| {
                self.snapshot
                    .message_panel
                    .as_ref()
                    .and_then(|panel| {
                        panel
                            .reminders
                            .as_slice()
                            .iter()
                            .find(|reminder| reminder.message == message)
                    })
                    .and_then(|reminder| self.settings_port.local_datetime(reminder.timestamp))
            })
            .or_else(|| {
                self.settings_port
                    .resolve_deadline(DeadlineChoice::InOneHour)
                    .ok()
                    .map(|moment| moment.local)
            });
        self.message_ui.time_input.update(cx, |input, cx| {
            input.set_context_value(value.as_ref().map_or("", Text::as_str));
            cx.notify();
        });
        self.message_ui.time_input.focus_handle(cx).focus(window);
    }

    pub(super) fn submit_custom_deadline(
        &mut self,
        repeated: RepeatedTime,
        window: &mut Window,
        cx: &Context<Self>,
    ) {
        if self.message_ui.time_input.read(cx).composing() {
            return;
        }
        let local = Text::new(self.message_ui.time_input.read(cx).content.as_str());
        if let Ok(local) = local {
            self.submit_message_deadline(DeadlineChoice::Custom { local, repeated }, window, cx);
        } else {
            self.message_ui.time_error = Some(LocalTimeError::Invalid);
        }
    }

    pub(super) fn submit_message_deadline(
        &mut self,
        choice: DeadlineChoice,
        window: &mut Window,
        cx: &Context<Self>,
    ) {
        let Some(target) = self.message_ui.time_target else {
            return;
        };
        let Some(context) = self.snapshot.selected else {
            return;
        };
        let moment = match self.settings_port.resolve_deadline(choice) {
            Ok(moment) => moment,
            Err(error) => {
                self.message_ui.time_error = Some(error);
                return;
            }
        };
        self.message_ui.time_error = None;
        let command = if let Some(message) = target {
            Some(MessageAction::Reminder {
                message,
                timestamp: Some(moment.timestamp),
            })
        } else if !self.composer.read(cx).composing() {
            Text::new(self.composer.read(cx).content.as_str())
                .ok()
                .filter(|body| !body.as_str().trim().is_empty())
                .map(|body| MessageAction::Schedule {
                    body,
                    timestamp: moment.timestamp,
                    target: self.drafts.target(context),
                })
        } else {
            None
        };
        if let Some(command) = command
            && self.submit_message(command).is_some()
        {
            self.message_ui.time_target = None;
            self.restore_deadline_focus(window, cx);
        }
    }

    pub(super) fn restore_deadline_focus(&self, window: &mut Window, cx: &Context<Self>) {
        if self.composer_available() {
            self.composer.focus_handle(cx).focus(window);
        } else {
            self.navigation.history_focus().focus(window);
        }
    }

    pub(super) fn message_deadline_card(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let target = self.message_ui.time_target?;
        let context = self.snapshot.selected?;
        let reminder = target.is_some();
        let available = self.message_available(if reminder {
            MessageFeature::Reminders
        } else {
            MessageFeature::Schedules
        });
        Some(
            self.composer_card()
                .p_3()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(self.l(
                            if reminder {
                                "Später erinnern"
                            } else {
                                "Nachricht planen"
                            },
                        )))
                        .child(self.icon_button(
                            "cancel-message-time",
                            ("x", "Abbrechen"),
                            ButtonStyle::Quiet,
                            Availability::available(),
                            LocalAction::MessageIn(context, Action::CancelTime),
                            cx,
                        )),
                )
                .child(
                    div().flex().flex_wrap().gap_2().children(
                        [
                            ("In 30 Minuten", DeadlineChoice::InThirtyMinutes),
                            ("In einer Stunde", DeadlineChoice::InOneHour),
                            ("Morgen um 09:00", DeadlineChoice::TomorrowMorning),
                            (
                                "Nächsten Montag um 09:00",
                                DeadlineChoice::NextMondayMorning,
                            ),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(index, (label, choice))| {
                            self.button(
                                ("message-time-preset", index),
                                label,
                                ButtonStyle::Secondary,
                                available,
                                LocalAction::MessageIn(context, Action::ChooseTime(choice)),
                                cx,
                            )
                        }),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            self.input_surface(&self.message_ui.time_input, cx)
                                .flex_1()
                                .min_w_0()
                                .child(self.message_ui.time_input.clone()),
                        )
                        .child(self.button(
                            "submit-message-time",
                            if reminder { "Erinnern" } else { "Planen" },
                            ButtonStyle::Primary,
                            available,
                            LocalAction::MessageIn(context, Action::SubmitTime),
                            cx,
                        )),
                )
                .child(self.hint(self.l("Datum und Uhrzeit: JJJJ-MM-TT HH:MM")))
                .when_some(self.message_ui.time_error, |d, error| {
                    d.child(
                        self.hint(self.l(error_label(error)))
                            .text_color(self.theme.danger),
                    )
                    .when(error == LocalTimeError::Repeated, |d| {
                        d.child(
                            div().flex().gap_2().children(
                                [
                                    ("Erste Uhrzeit verwenden", RepeatedTime::First),
                                    ("Zweite Uhrzeit verwenden", RepeatedTime::Second),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(
                                    |(index, (label, repeated))| {
                                        self.button(
                                            ("message-time-repeated", index),
                                            label,
                                            ButtonStyle::Secondary,
                                            available,
                                            LocalAction::MessageIn(
                                                context,
                                                Action::SubmitRepeatedTime(repeated),
                                            ),
                                            cx,
                                        )
                                    },
                                ),
                            ),
                        )
                    })
                })
                .into_any_element(),
        )
    }
}

fn error_label(error: LocalTimeError) -> &'static str {
    match error {
        LocalTimeError::Unavailable => {
            "Die Systemzeitzone ist gerade nicht verfügbar. Bitte gleich erneut versuchen."
        }
        LocalTimeError::Invalid => "Bitte ein gültiges Datum mit Uhrzeit eingeben.",
        LocalTimeError::Past => "Bitte einen Zeitpunkt in der Zukunft wählen.",
        LocalTimeError::Skipped => {
            "Diese Uhrzeit wird bei der Zeitumstellung übersprungen. Bitte eine andere wählen."
        }
        LocalTimeError::Repeated => {
            "Diese Uhrzeit kommt bei der Zeitumstellung zweimal vor. Welche möchtest du verwenden?"
        }
    }
}
