//! Account-wide reminder navigation, with only room-scoped opaque message IDs.
use super::{ButtonStyle, Desktop, LocalAction};
use gpui::{Context, Div, FontWeight, div, prelude::*};

impl Desktop {
    pub(super) fn account_reminders_body(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_2();
        let Some(view) = self
            .snapshot
            .account_reminders
            .as_ref()
            .filter(|view| Some(view.scope) == self.snapshot.active_account)
        else {
            return body.child(self.hint(self.l("Erinnerungen werden geladen …")));
        };
        if view.reminders.as_slice().is_empty() {
            body = body.child(self.hint(self.l("Keine anstehenden Erinnerungen.")));
        }
        for reminder in view.reminders.as_slice() {
            let Some(room) = self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|room| room.context == reminder.context)
            else {
                continue;
            };
            let time = self
                .settings_port
                .local_datetime(reminder.timestamp)
                .map(|value| value.as_str().to_owned())
                .unwrap_or_else(|| self.l("Zeit unbekannt").to_owned());
            body = body.child(
                self.control(
                    super::controls::scoped_id(
                        "account-reminder",
                        reminder.context,
                        reminder.message,
                        0,
                    ),
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::OpenReminder(reminder.context, reminder.message),
                    cx,
                )
                .w_full()
                .justify_start()
                .gap_3()
                .p_3()
                .child(self.scaled_icon("clock", 18., self.theme.accent))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_start()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_ellipsis()
                                .child(room.title.as_str().to_owned()),
                        )
                        .when_some(reminder.preview.as_ref(), |column, preview| {
                            column.child(
                                div()
                                    .w_full()
                                    .text_ellipsis()
                                    .child(preview.as_str().to_owned()),
                            )
                        })
                        .child(
                            div()
                                .text_size(self.sp(12.))
                                .text_color(self.theme.muted)
                                .child(time),
                        ),
                )
                .child(self.scaled_icon("arrow-right", 16., self.theme.muted)),
            );
        }
        if !view.complete {
            body = body.child(
                div()
                    .pt_2()
                    .child(self.hint(self.l("Einige Erinnerungen sind noch nicht verfügbar."))),
            );
        }
        body
    }
}
