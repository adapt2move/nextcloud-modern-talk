//! Compact, locally collapsible cards over already-resolved message links.
use super::{
    ButtonStyle, Desktop, LocalAction,
    controls::{Availability, scoped_id},
};
use gpui::{
    AnyElement, Context, Div, FontWeight, ImageSource, ObjectFit, div, img, prelude::*, px,
};
use std::time::Instant;
use talk_core::ui::{
    content::AssetTarget,
    presentation::{ConversationRef, MessageView, PresentationSnapshot, UiIntent},
};

type Key = (ConversationRef, u128, u16);
#[derive(Default)]
pub(super) struct ReferenceUi {
    hidden: Vec<Key>,
    arrival: Option<(Key, Instant)>,
}
impl Desktop {
    pub(super) fn observe_reference_previews(&mut self, next: &PresentationSnapshot) {
        if self.snapshot.active_account != next.active_account {
            self.reference_ui.hidden.clear();
        }
        if self.snapshot.selected == next.selected {
            if let Some(preview) = next.reference_previews.as_slice().iter().find(|preview| {
                !self
                    .snapshot
                    .reference_previews
                    .as_slice()
                    .iter()
                    .any(|old| {
                        old.context == preview.context
                            && old.message == preview.message
                            && old.ordinal == preview.ordinal
                    })
            }) {
                self.reference_ui.arrival = Some((
                    (preview.context, preview.message, preview.ordinal),
                    Instant::now(),
                ));
            }
        } else {
            self.reference_ui.arrival = None;
        }
        if self.snapshot.reference_previews != next.reference_previews {
            self.remeasure_reference_rows();
        }
    }
    fn remeasure_reference_rows(&mut self) {
        let anchor = self.message_ui.list.logical_scroll_top();
        let count = self.message_ui.list.item_count();
        self.message_ui.list.splice(0..count, count);
        self.message_ui.list.scroll_to(anchor);
    }
    pub(super) fn toggle_reference(
        &mut self,
        context: ConversationRef,
        message: u128,
        ordinal: u16,
    ) {
        if !self
            .snapshot
            .reference_previews
            .as_slice()
            .iter()
            .any(|p| p.context == context && p.message == message && p.ordinal == ordinal)
        {
            return;
        }
        let key = (context, message, ordinal);
        if let Some(index) = self
            .reference_ui
            .hidden
            .iter()
            .position(|item| *item == key)
        {
            self.reference_ui.hidden.remove(index);
        } else {
            if self.reference_ui.hidden.len() == 64 {
                self.reference_ui.hidden.remove(0);
            }
            self.reference_ui.hidden.push(key);
        }
        self.remeasure_reference_rows();
    }
    pub(super) fn open_reference(&mut self, context: ConversationRef, message: u128, ordinal: u16) {
        if !self
            .snapshot
            .reference_previews
            .as_slice()
            .iter()
            .any(|p| p.context == context && p.message == message && p.ordinal == ordinal)
        {
            return;
        }
        let Some(source) = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .find(|row| row.local_id == message)
            .map(|row| row.body.clone())
        else {
            return;
        };
        self.submit(UiIntent::OpenMessageLink {
            context,
            message,
            source,
            ordinal,
        });
    }
    pub(super) fn reference_cards(
        &self,
        message: &MessageView,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(context) = self.snapshot.selected else {
            return Vec::new();
        };
        self.snapshot
            .reference_previews
            .as_slice()
            .iter()
            .filter(|preview| preview.context == context && preview.message == message.local_id)
            .map(|preview| {
                let key = (context, message.local_id, preview.ordinal);
                let identity = scoped_id(
                    "reference-card",
                    context,
                    message.local_id,
                    u128::from(preview.ordinal),
                );
                if self.reference_ui.hidden.contains(&key) {
                    return self
                        .button(
                            identity,
                            "Vorschau anzeigen",
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::ToggleReference(
                                context,
                                message.local_id,
                                preview.ordinal,
                            ),
                            cx,
                        )
                        .into_any_element();
                }
                let target = AssetTarget::Reference {
                    context,
                    message: message.local_id,
                    ordinal: preview.ordinal,
                };
                let thumbnail: AnyElement = match self.asset_image(&target) {
                    Some(ImageSource::Render(image)) => img(image)
                        .w(px(64.))
                        .h(px(64.))
                        .rounded_md()
                        .object_fit(ObjectFit::Cover)
                        .into_any_element(),
                    _ => div()
                        .w(px(64.))
                        .h(px(64.))
                        .rounded_md()
                        .bg(self.theme.selected)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.scaled_icon("link", 24., self.theme.accent))
                        .into_any_element(),
                };
                let mut label = div().flex().flex_col().gap_1().min_w_0().flex_1().child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_ellipsis()
                        .child(preview.title.as_str().to_owned()),
                );
                if let Some(description) = preview
                    .description
                    .as_ref()
                    .filter(|description| !description.as_str().trim().is_empty())
                {
                    label =
                        label.child(div().text_ellipsis().child(self.hint(description.as_str())));
                }
                let card: Div = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .max_w(px(520.))
                    .rounded_lg()
                    .border_1()
                    .border_color(self.theme.border)
                    .child(thumbnail)
                    .child(
                        self.button(
                            scoped_id(
                                "reference-open",
                                context,
                                message.local_id,
                                u128::from(preview.ordinal),
                            ),
                            "",
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::OpenReference(context, message.local_id, preview.ordinal),
                            cx,
                        )
                        .flex_1()
                        .min_w_0()
                        .justify_start()
                        .child(label),
                    )
                    .child(self.icon_button(
                        scoped_id(
                            "reference-hide",
                            context,
                            message.local_id,
                            u128::from(preview.ordinal),
                        ),
                        ("x", "Vorschau verbergen"),
                        ButtonStyle::Quiet,
                        Availability::available(),
                        LocalAction::ToggleReference(context, message.local_id, preview.ordinal),
                        cx,
                    ));
                let arrived = self
                    .reference_ui
                    .arrival
                    .filter(|(item, _)| *item == key)
                    .map(|(_, at)| at);
                self.reveal_observation(identity, card, arrived)
            })
            .collect()
    }
}
