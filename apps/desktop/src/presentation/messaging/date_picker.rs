//! Keyboard-operable month grid and time stepper. Civil values stay local until
//! the settings owner resolves the system timezone and daylight-saving choice.
use super::*;

#[derive(Clone, Copy)]
pub(crate) enum PickerAction {
    Month(i8),
    Select(u8),
    Move(i8),
    Minutes(i16),
}

fn day_id(context: ConversationRef, day: u8) -> gpui::ElementId {
    scoped_id("deadline-calendar-day", context, u128::from(day), 0)
}

impl Desktop {
    pub(super) fn focus_date_picker(&self, window: &mut Window, cx: &Context<Self>) {
        let Some(context) = self.snapshot.selected else {
            return;
        };
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |this, cx| {
                if this.snapshot.selected == Some(context)
                    && this.message_ui.time_target.is_some()
                    && let Some(day) = this.message_ui.picker.map(|picker| picker.day())
                    && let Some(focus) = this.control_focus(day_id(context, day), cx)
                {
                    focus.focus(window);
                }
            });
        });
    }

    pub(super) fn date_picker_action(
        &mut self,
        context: ConversationRef,
        action: PickerAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot.selected != Some(context) || self.message_ui.time_target.is_none() {
            return;
        }
        let Some(picker) = self.message_ui.picker.as_mut() else {
            return;
        };
        let changed = match action {
            PickerAction::Month(delta) => picker.month(delta),
            PickerAction::Select(day) => picker.select(day),
            PickerAction::Move(delta) => picker.shift_days(delta),
            PickerAction::Minutes(delta) => picker.shift_minutes(delta),
        };
        if changed {
            self.message_ui.time_error = None;
            if matches!(action, PickerAction::Move(_)) {
                self.focus_date_picker(window, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn date_picker(&self, context: ConversationRef, cx: &Context<Self>) -> Div {
        let Some(picker) = self.message_ui.picker else {
            return div().child(self.hint(self.l(
                "Die Systemzeitzone ist gerade nicht verfügbar. Bitte gleich erneut versuchen.",
            )));
        };
        let months = [
            "Januar",
            "Februar",
            "März",
            "April",
            "Mai",
            "Juni",
            "Juli",
            "August",
            "September",
            "Oktober",
            "November",
            "Dezember",
        ];
        let mut grid = div().flex_none().flex().flex_col().gap_1().w(px(280.));
        grid = grid.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(self.icon_button(
                    "calendar-previous",
                    ("chevron-left", "Vorheriger Monat"),
                    ButtonStyle::Quiet,
                    Availability::when(
                        !(picker.year() == 1970 && picker.month_number() == 1),
                        "Vorheriger Monat",
                    ),
                    LocalAction::MessageIn(context, Action::DatePicker(PickerAction::Month(-1))),
                    cx,
                ))
                .child(div().font_weight(FontWeight::MEDIUM).child(format!(
                    "{} {}",
                    self.l(months[usize::from(picker.month_number() - 1)]),
                    picker.year()
                )))
                .child(self.icon_button(
                    "calendar-next",
                    ("arrow-right", "Nächster Monat"),
                    ButtonStyle::Quiet,
                    Availability::when(
                        !(picker.year() == 2100 && picker.month_number() == 12),
                        "Nächster Monat",
                    ),
                    LocalAction::MessageIn(context, Action::DatePicker(PickerAction::Month(1))),
                    cx,
                )),
        );
        grid = grid.child(
            div()
                .flex()
                .children(
                    ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"]
                        .into_iter()
                        .map(|key| {
                            div()
                                .w(px(40.))
                                .text_center()
                                .text_size(self.sp(11.))
                                .text_color(self.theme.muted)
                                .child(self.l(key))
                        }),
                ),
        );
        for week in picker.grid().chunks(7) {
            if week.iter().all(Option::is_none) {
                continue;
            }
            let mut row = div().flex();
            for &day in week {
                if let Some(day) = day {
                    let id = day_id(context, day);
                    let focus = self.control_focus(id.clone(), cx);
                    let button = self
                        .button(
                            id,
                            day.to_string(),
                            if day == picker.day() {
                                ButtonStyle::Primary
                            } else {
                                ButtonStyle::Quiet
                            },
                            true,
                            LocalAction::MessageIn(
                                context,
                                Action::DatePicker(PickerAction::Select(day)),
                            ),
                            cx,
                        )
                        .w(px(40.))
                        .min_h(px(30.))
                        .h(px(32.))
                        .justify_center()
                        .when_some(focus, |button, focus| {
                            button.track_focus(&focus.tab_stop(day == picker.day()))
                        })
                        .on_key_down(cx.listener(
                            move |this, event: &gpui::KeyDownEvent, window, cx| {
                                if event.keystroke.modifiers.control
                                    || event.keystroke.modifiers.alt
                                    || event.keystroke.modifiers.platform
                                {
                                    return;
                                }
                                let delta = match event.keystroke.key.as_str() {
                                    "left" => -1,
                                    "right" => 1,
                                    "up" => -7,
                                    "down" => 7,
                                    _ => return,
                                };
                                this.date_picker_action(
                                    context,
                                    PickerAction::Move(delta),
                                    window,
                                    cx,
                                );
                                cx.stop_propagation();
                            },
                        ));
                    row = row.child(button);
                } else {
                    row = row.child(div().w(px(40.)).h(px(30.)));
                }
            }
            grid = grid.child(row);
        }
        let step = |id, icon, label, delta| {
            self.icon_button(
                id,
                (icon, label),
                ButtonStyle::Quiet,
                Availability::available(),
                LocalAction::MessageIn(context, Action::DatePicker(PickerAction::Minutes(delta))),
                cx,
            )
        };
        grid.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap_1()
                .pt_1()
                .child(step("hour-down", "chevron-down", "Stunden verringern", -60))
                .child(
                    div()
                        .font_family("Google Sans Code")
                        .child(format!("{:02}", picker.hour())),
                )
                .child(step("hour-up", "chevron-up", "Stunden erhöhen", 60))
                .child(div().child(":"))
                .child(step(
                    "minute-down",
                    "chevron-down",
                    "Minuten verringern",
                    -5,
                ))
                .child(
                    div()
                        .font_family("Google Sans Code")
                        .child(format!("{:02}", picker.minute())),
                )
                .child(step("minute-up", "chevron-up", "Minuten erhöhen", 5)),
        )
    }
}
