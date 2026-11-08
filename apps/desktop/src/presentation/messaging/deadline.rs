//! Reminder and scheduled-send choices use the same bounded local-clock owner.
use super::*;
use crate::presentation::controls::BannerRole;
use talk_core::preferences::time::{DeadlineChoice, LocalTimeError, RepeatedTime};

impl Desktop {
    pub(in crate::presentation) fn retire_thread_deadline(&mut self) {
        if self.message_ui.time_thread.is_some() {
            self.message_ui.time_target = None;
            self.message_ui.time_thread = None;
            self.message_ui.picker = None;
            self.message_ui.time_error = None;
        }
    }

    pub(super) fn open_scheduled_edit(
        &mut self,
        id: u128,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(context) = self.snapshot.selected else {
            return;
        };
        let Some(item) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)
            .and_then(|panel| {
                panel
                    .scheduled
                    .as_slice()
                    .iter()
                    .find(|item| item.id == id && item.editable)
            })
            .cloned()
        else {
            return;
        };
        if !self.message_feature_enabled(MessageFeature::Schedules) {
            return;
        }
        self.message_ui.scheduled_edit = Some((id, item.silent));
        self.message_ui.scheduled_input.update(cx, |input, cx| {
            input.set_context_value(item.body.as_str());
            cx.notify();
        });
        self.open_message_deadline(None, None, window, cx);
        if let Some(local) = (if item.timestamp == 0 {
            item.failed_original_timestamp
        } else {
            Some(item.timestamp)
        })
        .and_then(|timestamp| self.settings_port.local_datetime(timestamp))
        {
            self.message_ui.picker =
                talk_core::preferences::time::calendar::Calendar::parse(local.as_str());
        }
        self.message_ui
            .scheduled_input
            .focus_handle(cx)
            .focus(window);
    }
    pub(in crate::presentation) fn message_deadline_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.message_ui.time_target.is_none()
            || !matches!(self.overlay, None | Some(Overlay::Thread))
            || self.message_ui.time_thread.is_some_and(|root| {
                self.overlay != Some(Overlay::Thread)
                    || self.thread_parent != self.snapshot.selected.map(|context| (context, root))
            })
            || self.message_ui.scheduled_input.read(cx).composing()
        {
            return false;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt {
            return false;
        }
        let action = match event.keystroke.key.as_str() {
            "escape" => Action::CancelTime,
            _ => return false,
        };
        self.message_action(action, window, cx);
        cx.stop_propagation();
        true
    }
    pub(super) fn open_message_deadline(
        &mut self,
        target: Option<u128>,
        thread: Option<u128>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if thread.is_some_and(|root| {
            target.is_none()
                || self.overlay != Some(Overlay::Thread)
                || self.thread_parent != self.snapshot.selected.map(|context| (context, root))
        }) {
            return;
        }
        self.content_ui.menu = false;
        self.message_ui.menu.clear();
        self.message_ui.time_target = Some(target);
        self.message_ui.time_thread = thread;
        self.message_ui.time_error = None;
        // Editing a plan starts on its exact time; otherwise the quick choices come first.
        self.message_ui.custom_time = self.message_ui.scheduled_edit.is_some();
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
        self.message_ui.picker = value
            .as_ref()
            .and_then(|value| {
                talk_core::preferences::time::calendar::Calendar::parse(value.as_str())
            })
            .or_else(|| {
                self.development
                    .then(|| {
                        talk_core::preferences::time::calendar::Calendar::parse("2027-01-16 09:00")
                    })
                    .flatten()
            });
        if self.message_ui.scheduled_edit.is_none() {
            self.focus_date_picker(window, cx);
        }
    }

    pub(super) fn submit_custom_deadline(
        &mut self,
        repeated: RepeatedTime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.message_ui.scheduled_input.read(cx).composing() {
            return;
        }
        if let Some(local) = self.message_ui.picker.and_then(|picker| picker.local()) {
            self.submit_message_deadline(DeadlineChoice::Custom { local, repeated }, window, cx);
        } else {
            self.message_ui.time_error = Some(LocalTimeError::Unavailable);
        }
    }

    pub(super) fn submit_message_deadline(
        &mut self,
        choice: DeadlineChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
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
        } else if let Some((id, silent)) = self.message_ui.scheduled_edit {
            self.message_ui
                .scheduled_input
                .read(cx)
                .submitted_value()
                .filter(|body| !body.trim().is_empty())
                .and_then(|body| Text::new(body).ok())
                .map(|body| MessageAction::EditScheduled {
                    id,
                    body,
                    timestamp: moment.timestamp,
                    silent,
                })
        } else if !self.composer.read(cx).composing()
            && !self.has_staged_attachments()
            && self.message_ui.editing.is_none()
        {
            self.composer_submission(cx)
                .filter(|body| !body.as_str().trim().is_empty())
                .map(|body| MessageAction::Schedule {
                    body,
                    timestamp: moment.timestamp,
                    target: self.drafts.target(context),
                })
        } else {
            None
        };
        let scheduled_draft = matches!(command, Some(MessageAction::Schedule { .. }));
        if let Some(command) = command
            && self.submit_message(command).is_some()
        {
            if scheduled_draft {
                // The words now wait in the schedule; a failure comes back through
                // the failed-message recovery, which can restore them for editing.
                self.composer.update(cx, |input, cx| {
                    input.reset();
                    cx.notify();
                });
                let _ = self.drafts.save(context, "");
            }
            let thread = self.message_ui.time_thread;
            self.message_ui.time_target = None;
            self.message_ui.time_thread = None;
            self.message_ui.picker = None;
            self.message_ui.scheduled_edit = None;
            self.restore_deadline_focus(thread, window, cx);
        }
    }

    pub(super) fn restore_deadline_focus(
        &self,
        thread: Option<u128>,
        window: &mut Window,
        cx: &Context<Self>,
    ) {
        if thread.is_some_and(|root| {
            self.overlay == Some(Overlay::Thread)
                && self.thread_parent == self.snapshot.selected.map(|context| (context, root))
        }) {
            self.thread_composer.focus_handle(cx).focus(window);
        } else if self.composer_available() {
            self.composer.focus_handle(cx).focus(window);
        } else {
            self.navigation.history_focus().focus(window);
        }
    }

    pub(in crate::presentation) fn message_deadline_card_on(
        &self,
        thread: bool,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let target = self.message_ui.time_target?;
        let context = self.snapshot.selected?;
        if self.message_ui.time_thread.is_some_and(|root| {
            self.overlay != Some(Overlay::Thread) || self.thread_parent != Some((context, root))
        }) {
            return None;
        }
        let in_thread = self.message_ui.time_thread.is_some();
        if thread != in_thread {
            return None;
        }
        let reminder = target.is_some();
        let custom = self.message_ui.custom_time;
        let editing = !reminder && self.message_ui.scheduled_edit.is_some();
        let available = self.message_available(if reminder {
            MessageFeature::Reminders
        } else {
            MessageFeature::Schedules
        });
        let available = if editing
            && available.is_enabled()
            && self
                .message_ui
                .scheduled_input
                .read(cx)
                .submitted_value()
                .is_none_or(|body| body.trim().is_empty())
        {
            Availability::when(false, "Gib eine Nachricht ein.")
        } else if !reminder
            && !editing
            && available.is_enabled()
            && (self.has_staged_attachments()
                || self.message_ui.editing.is_some()
                || self.message_ui.composer_load_failed == Some(context)
                || self
                    .composer
                    .read(cx)
                    .submitted_value()
                    .is_none_or(|body| body.trim().is_empty()))
        {
            Availability::when(false, "Zum Planen eine Nachricht ohne Dateien verfassen.")
        } else {
            available
        };
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
                            } else if editing {
                                "Geplante Nachricht bearbeiten"
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
                .when(editing, |d| {
                    d.child(
                        self.input_surface(&self.message_ui.scheduled_input, cx)
                            .w_full()
                            .child(self.message_ui.scheduled_input.clone()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .children(
                            [
                                ("In 20 Minuten", DeadlineChoice::InTwentyMinutes),
                                ("In einer Stunde", DeadlineChoice::InOneHour),
                                ("In 3 Stunden", DeadlineChoice::InThreeHours),
                                ("Morgen um 09:00", DeadlineChoice::TomorrowMorning),
                                ("Montag um 09:00", DeadlineChoice::NextMondayMorning),
                            ]
                            .into_iter()
                            .enumerate()
                            .filter(|(index, _)| reminder || *index >= 3)
                            .map(|(index, (label, choice))| {
                                self.button(
                                    ("message-time-preset", index),
                                    label,
                                    ButtonStyle::Quiet,
                                    available,
                                    LocalAction::MessageIn(context, Action::ChooseTime(choice)),
                                    cx,
                                )
                                .rounded_full()
                                .bg(self.theme.subtle)
                            }),
                        )
                        .child(self.button_with_icon(
                            "message-time-custom",
                            (
                                "Eigene Zeit …",
                                if custom { "chevron-up" } else { "chevron-down" },
                            ),
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::MessageIn(context, Action::ToggleCustomTime),
                            cx,
                        )),
                )
                .when(custom, |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_start()
                            .gap_4()
                            .child(self.date_picker(context, cx))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(160.))
                                    .flex()
                                    .flex_col()
                                    .items_end()
                                    .justify_end()
                                    .h_full()
                                    .child(self.button(
                                        "submit-message-time",
                                        if reminder {
                                            "Erinnern"
                                        } else if editing {
                                            "Änderungen speichern"
                                        } else {
                                            "Planen"
                                        },
                                        ButtonStyle::Primary,
                                        if self.message_ui.picker.is_some() {
                                            available
                                        } else {
                                            Availability::when(
                                                false,
                                                "Die Systemzeitzone ist gerade nicht verfügbar. Bitte gleich erneut versuchen.",
                                            )
                                        },
                                        LocalAction::MessageIn(context, Action::SubmitTime),
                                        cx,
                                    )),
                            ),
                    )
                })
                .when_some(self.message_ui.time_error, |d, error| {
                    d.child(
                        self.banner(BannerRole::Error, self.l(error_label(error))),
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
