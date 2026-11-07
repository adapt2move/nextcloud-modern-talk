//! A small, dismissible summary offer and result above the composer.
use super::*;
use talk_core::ui::messaging::Summary;

impl Desktop {
    pub(super) fn summary_card(&self, context: ConversationRef, cx: &Context<Self>) -> Option<Div> {
        let panel = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)?;
        let summary = panel.summary.as_ref()?;
        if matches!(summary, Summary::Offer { .. })
            && (!self.message_feature_enabled(MessageFeature::Summary)
                || self.snapshot.content != ContentState::Ready)
        {
            return None;
        }
        let mut card = self.composer_card().px_3().py_2().gap_1();
        let title = match summary {
            Summary::Offer { unread, .. } => {
                self.lf("{n} ungelesene Nachrichten", &[("n", &unread.to_string())])
            }
            Summary::Pending { .. } => self.l("Zusammenfassung wird erstellt …").to_owned(),
            _ => self.l("Zusammenfassung").to_owned(),
        };
        let mut heading = div().flex().items_center().gap_2().child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(self.sp(13.))
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        );
        if matches!(summary, Summary::Offer { .. }) {
            heading = heading.child(self.button(
                "summary-create",
                "Zusammenfassung erstellen",
                ButtonStyle::Quiet,
                true,
                LocalAction::MessageIn(context, Action::Command(MessageAction::Summarize)),
                cx,
            ));
        }
        heading = heading.child(self.icon_button(
            "summary-dismiss",
            ("x", "Schließen"),
            ButtonStyle::Quiet,
            Availability::available(),
            LocalAction::MessageIn(context, Action::Command(MessageAction::DismissSummary)),
            cx,
        ));
        card = card.child(heading);
        match summary {
            Summary::Pending { parts } | Summary::Ready { parts, .. }
                if !parts.as_slice().is_empty() =>
            {
                let mut content = div()
                    .id(scoped_id("summary-content", context, 0, 0))
                    .max_h(px(150.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2();
                for part in parts.as_slice() {
                    content = content.child(
                        div()
                            .text_size(self.sp(13.))
                            .child(part.as_str().to_owned()),
                    );
                }
                card = card.child(content);
            }
            Summary::Empty => {
                card = card
                    .child(self.hint(self.l("Es gibt keine Nachrichten für eine Zusammenfassung.")))
            }
            Summary::Failed => {
                card =
                    card.child(self.hint(self.l("Die Zusammenfassung ist gerade nicht verfügbar.")))
            }
            _ => (),
        }
        if matches!(summary, Summary::Ready { partial: true, .. }) {
            card =
                card.child(self.hint(self.l("Weitere Nachrichten wurden nicht zusammengefasst.")));
        }
        Some(card)
    }
}
