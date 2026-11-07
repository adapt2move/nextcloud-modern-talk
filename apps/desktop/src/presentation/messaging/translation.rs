//! Inline, explicitly requested server translation. No provider is chosen here.
use super::*;

impl Desktop {
    fn translation_language(&self, context: ConversationRef) -> Option<Text<64>> {
        let desired = match self.locale {
            talk_core::preferences::Locale::German => "de",
            talk_core::preferences::Locale::Spanish => "es",
            talk_core::preferences::Locale::English => "en",
        };
        let targets = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)?
            .translation_targets
            .as_ref()?;
        targets
            .as_slice()
            .iter()
            .find(|target| target.as_str().eq_ignore_ascii_case(desired))
            .or_else(|| {
                targets.as_slice().iter().find(|target| {
                    target
                        .as_str()
                        .split(['-', '_'])
                        .next()
                        .is_some_and(|base| base.eq_ignore_ascii_case(desired))
                })
            })
            .cloned()
    }

    pub(super) fn translation_available(
        &self,
        context: ConversationRef,
        message: &MessageView,
    ) -> bool {
        self.snapshot.selected == Some(context)
            && self.snapshot.content == ContentState::Ready
            && self.message_feature_enabled(MessageFeature::Translation)
            && self.translation_language(context).is_some()
            && message_actions_allowed(message)
            && !message.metadata.truncated
            && message.metadata.rich.as_slice().is_empty()
            && !message.body.as_str().trim().is_empty()
    }

    pub(super) fn current_translation<'a>(
        &'a self,
        message: &MessageView,
    ) -> Option<&'a talk_core::ui::messaging::Translation> {
        let context = self.snapshot.selected?;
        let language = self.translation_language(context)?;
        self.snapshot
            .message_panel
            .as_ref()?
            .translations
            .as_slice()
            .iter()
            .find(|item| {
                item.message == message.local_id
                    && item.source == message.body
                    && item.language == language
            })
    }

    pub(super) fn translate_message(&mut self, context: ConversationRef, id: u128) {
        let Some(message) = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .find(|message| message.local_id == id)
        else {
            return;
        };
        if !self.translation_available(context, message) {
            return;
        }
        if self.current_translation(message).is_some() {
            if self.message_ui.hidden_translations.contains(&id) {
                self.message_ui
                    .hidden_translations
                    .retain(|hidden| *hidden != id);
            } else if self.message_ui.hidden_translations.len() < 100 {
                self.message_ui.hidden_translations.push(id);
            }
            return;
        }
        let Some(language) = self.translation_language(context) else {
            return;
        };
        let source = message.body.clone();
        self.message_ui
            .hidden_translations
            .retain(|hidden| *hidden != id);
        self.submit_message(MessageAction::Translate {
            message: id,
            source,
            language,
        });
    }

    pub(super) fn translation_row(
        &self,
        context: ConversationRef,
        message: &MessageView,
        cx: &Context<Self>,
    ) -> Option<Div> {
        if !self.translation_available(context, message) {
            return None;
        }
        let panel = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)?;
        if panel.translation_pending == Some(message.local_id) {
            return Some(self.hint(self.l("Wird übersetzt …")));
        }
        if panel.translation_failed == Some(message.local_id) {
            return Some(self.hint(self.l("Die Übersetzung ist gerade nicht verfügbar.")));
        }
        if self
            .message_ui
            .hidden_translations
            .contains(&message.local_id)
        {
            return None;
        }
        let translation = self.current_translation(message)?;
        Some(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_1()
                .pl_3()
                .border_l_2()
                .border_color(self.theme.border)
                .child(self.hint(self.l("Übersetzung")))
                .child(
                    div()
                        .text_size(self.sp(13.))
                        .child(translation.text.as_str().to_owned()),
                )
                .child(self.button(
                    scoped_id("hide-translation", context, message.local_id, 0),
                    "Original anzeigen",
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::MessageIn(context, Action::HideTranslation(message.local_id)),
                    cx,
                )),
        )
    }
}
