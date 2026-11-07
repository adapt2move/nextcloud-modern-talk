//! Outgoing attachment rows and composer admission; native I/O stays with composition.
use super::file_detail;
use crate::presentation::{
    ButtonStyle, Desktop, LocalAction,
    controls::{Availability, scoped_id},
    messaging,
};
use gpui::{Context, Div, ImageSource, ObjectFit, div, img, prelude::*, px};
use talk_core::ui::{
    Text,
    content::{AssetTarget, AttachmentRef},
    messaging::{MessageAction, MessageFeature, UploadFailure, UploadState, UploadView},
    presentation::{ConversationRef, RequestId},
};
pub(in crate::presentation) struct PendingSend {
    context: ConversationRef,
    request: RequestId,
    body: Text<4096>,
    editor: u64,
}
fn upload_label(state: UploadState) -> &'static str {
    match state {
        UploadState::Selected => "Bereit zum Senden",
        UploadState::Uploading => "Wird hochgeladen …",
        UploadState::Sharing => "Wird gesendet …",
        UploadState::Confirming => "Wird im Gespräch bestätigt …",
        UploadState::Shared => "Gesendet",
        UploadState::Uncertain => "Serverbestätigung fehlt. Das Gespräch wird abgeglichen …",
        UploadState::Failed(UploadFailure::TooMany) => {
            "Du kannst höchstens 8 Dateien auf einmal anhängen."
        }
        UploadState::Failed(UploadFailure::Missing) => "Datei nicht mehr gefunden.",
        UploadState::Failed(UploadFailure::TooLarge) => {
            "Die Datei ist zu groß. Maximal 32 MiB pro Datei."
        }
        UploadState::Failed(UploadFailure::Denied) => "Keine Berechtigung zum Senden dieser Datei.",
        UploadState::Failed(UploadFailure::Connection) => {
            "Datei konnte nicht gesendet werden. Erneut versuchen."
        }
        UploadState::Failed(UploadFailure::Unsupported) => {
            "Diese Datei kann nicht gesendet werden."
        }
    }
}
impl Desktop {
    pub(in crate::presentation) fn has_staged_attachments(&self) -> bool {
        self.snapshot.message_panel.as_ref().is_some_and(|panel| {
            Some(panel.context) == self.snapshot.selected
                && panel.uploads.as_slice().iter().any(|file| !file.submitted)
        })
    }
    pub(in crate::presentation) fn can_send_attachments(&self, cx: &Context<Self>) -> bool {
        self.snapshot.message_panel.as_ref().is_some_and(|panel| {
            Some(panel.context) == self.snapshot.selected
                && !panel.uploads_running
                && !panel
                    .uploads
                    .as_slice()
                    .iter()
                    .any(|file| file.state == UploadState::Uncertain)
                && panel
                    .uploads
                    .as_slice()
                    .iter()
                    .any(|file| !file.submitted && file.state == UploadState::Selected)
        }) && self.message_feature_enabled(MessageFeature::Files)
            && !self.media_offline()
            && self.message_ui.editing.is_none()
            && self.snapshot.selected.is_some_and(|context| {
                let target = self.drafts.target(context);
                !self.drafts.pending(context)
                    && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                        ((!target.silent && self.composer.read(cx).content.is_empty())
                            || panel.media_caption)
                            && (target.reply_to.is_none()
                                || self.message_feature_enabled(MessageFeature::Reply))
                            && (target.thread.is_none()
                                || self.message_feature_enabled(MessageFeature::Threads))
                    })
            })
    }
    pub(in crate::presentation) fn send_attachments(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.has_staged_attachments() {
            return false;
        }
        if !self.can_send_attachments(cx) {
            return true;
        }
        let Some(body) = self.composer_submission(cx) else {
            return true;
        };
        let context = self.snapshot.selected.expect("attachment context");
        let editor = self.composer.read(cx).edit_generation();
        if let Some(request) = self.submit_message(MessageAction::SendFiles {
            caption: body.clone(),
            target: self.drafts.target(context),
        }) {
            self.file_send = Some(PendingSend {
                context,
                request,
                body,
                editor,
            });
            self.message_ui.list.scroll_to(gpui::ListOffset {
                item_ix: self.message_ui.list.item_count(),
                offset_in_item: px(0.),
            });
        }
        true
    }
    pub(in crate::presentation) fn refresh_attachment_send(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.file_send.as_ref() else {
            return;
        };
        if self.snapshot.selected != Some(pending.context) {
            self.file_send = None;
            return;
        }
        if self
            .snapshot
            .message_panel
            .as_ref()
            .and_then(|panel| panel.files_queued)
            != Some(pending.request)
        {
            return;
        }
        let pending = self.file_send.take().expect("pending file caption");
        let input = self.composer.read(cx);
        if !input.composing()
            && input.edit_generation() == pending.editor
            && input.submitted_value() == Some(pending.body.as_str())
        {
            self.composer.update(cx, |input, _| input.reset());
            self.save_draft(cx);
        }
    }
    pub(in crate::presentation) fn upload_strip(&self, cx: &Context<Self>) -> Div {
        let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| Some(panel.context) == self.snapshot.selected)
        else {
            return div();
        };
        div().flex().flex_wrap().gap_2().children(
            panel
                .uploads
                .as_slice()
                .iter()
                .filter(|file| !file.submitted)
                .map(|file| {
                    let preview = self.asset_image(&Self::local_upload_target(panel.context, file));
                    div()
                        .w(px(280.))
                        .max_w_full()
                        .rounded(px(8.))
                        .bg(self.theme.subtle)
                        .p_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(if let Some(preview) = preview {
                            div()
                                .size(px(40.))
                                .flex_none()
                                .rounded(px(4.))
                                .overflow_hidden()
                                .child(img(preview).size_full().object_fit(ObjectFit::Cover))
                                .into_any_element()
                        } else {
                            self.scaled_icon("file", 22., self.theme.muted)
                                .into_any_element()
                        })
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(self.sp(13.))
                                        .child(file.name.as_str().to_owned()),
                                )
                                .child(self.hint(&if matches!(file.state, UploadState::Selected) { crate::presentation::localization::file_size(self.locale, file.bytes) } else { self.l(upload_label(file.state)).to_owned() })),
                        )
                        .when(
                            matches!(file.state, UploadState::Uploading | UploadState::Sharing),
                            |row| row.child(self.pending_dots(self.theme.muted)),
                        )
                        .when(matches!(file.state, UploadState::Failed(_)), |row| {
                            row.child(self.icon_button(
                                scoped_id("retry-file", panel.context, u128::from(file.id), 0),
                                ("refresh", "Erneut versuchen"),
                                ButtonStyle::Quiet,
                                Availability::when(
                                    !panel.uploads_running && !self.media_offline(),
                                    "Die Verbindung ist derzeit nicht bereit.",
                                ),
                                LocalAction::Message(messaging::Action::Command(
                                    MessageAction::RetryFile(file.id),
                                )),
                                cx,
                            ))
                        })
                        .child(self.icon_button(
                            scoped_id("remove-file", panel.context, u128::from(file.id), 0),
                            ("x", "Anhang entfernen"),
                            ButtonStyle::Quiet,
                            Availability::when(!panel.uploads_running, "Dateien werden gesendet …"),
                            LocalAction::Message(messaging::Action::Command(
                                MessageAction::RemoveFile(file.id),
                            )),
                            cx,
                        ))
                }),
        )
    }
    pub(in crate::presentation) fn local_upload_target(
        context: ConversationRef,
        file: &UploadView,
    ) -> AssetTarget {
        AssetTarget::Thumbnail(AttachmentRef {
            context,
            message: file.message_id(),
            placeholder: Text::new("upload").expect("static placeholder"),
        })
    }
    pub(in crate::presentation) fn confirmed_upload_image(
        &self,
        reference: &AttachmentRef,
    ) -> Option<ImageSource> {
        let panel = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| p.context == reference.context)?;
        let file = panel
            .uploads
            .as_slice()
            .iter()
            .find(|f| f.server_message == Some(reference.message))?;
        self.asset_image(&Self::local_upload_target(reference.context, file))
    }
    pub(in crate::presentation) fn pending_upload_card(
        &self,
        context: ConversationRef,
        file: &UploadView,
        cx: &Context<Self>,
    ) -> Div {
        let target = Self::local_upload_target(context, file);
        let image = self.asset_image(&target);
        let (width, height) = match &image {
            Some(ImageSource::Render(image)) => {
                let size = image.size(0);
                super::content::gallery::media_size(size.width.0 as f32, size.height.0 as f32)
            }
            _ => (280., 0.),
        };
        let pending = matches!(
            file.state,
            UploadState::Selected
                | UploadState::Uploading
                | UploadState::Sharing
                | UploadState::Confirming
        );
        let retry = matches!(file.state, UploadState::Failed(_))
            || file.state == UploadState::Uncertain && !file.uploaded;
        let running = self
            .snapshot
            .message_panel
            .as_ref()
            .is_some_and(|p| p.uploads_running);
        let label = if file.state == UploadState::Selected {
            "Wartet auf Hochladen …"
        } else if file.state == UploadState::Uncertain && !file.uploaded {
            "Hochladen unterbrochen. Erneut versuchen."
        } else {
            upload_label(file.state)
        };
        let mut card = div()
            .w(px(width))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(5.));
        card = card.child(if let Some(image) = image {
            div()
                .w_full()
                .h(px(height))
                .bg(self.theme.subtle)
                .rounded(px(8.))
                .overflow_hidden()
                .child(
                    img(image)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .rounded(px(8.)),
                )
        } else {
            div()
                .w_full()
                .rounded(px(8.))
                .bg(self.theme.subtle)
                .p_3()
                .flex()
                .items_center()
                .gap_2()
                .child(self.scaled_icon("file", 24., self.theme.muted))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .truncate()
                                .text_size(self.sp(13.))
                                .child(file.name.as_str().to_owned()),
                        )
                        .child(self.hint(&file_detail(self.locale, file.name.as_str(), file.bytes))),
                )
        });
        if file.state == UploadState::Uploading {
            let ratio = file
                .bytes
                .filter(|n| *n != 0)
                .map_or(0., |n| (file.transferred as f32 / n as f32).clamp(0., 1.));
            card = card.child(
                div()
                    .h(px(2.))
                    .w_full()
                    .rounded_full()
                    .bg(self.theme.border)
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(ratio))
                            .bg(self.theme.accent)
                            .rounded_full(),
                    ),
            );
        }
        card.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .child(self.hint(self.l(label)).text_size(self.sp(12.))),
                )
                .when(pending, |d| d.child(self.pending_dots(self.theme.muted)))
                .when(retry, |d| {
                    d.child(self.icon_button(
                        scoped_id("retry-upload-row", context, file.message_id(), 0),
                        ("refresh", "Erneut versuchen"),
                        ButtonStyle::Quiet,
                        Availability::when(
                            !running && !self.media_offline(),
                            "Die Verbindung ist derzeit nicht bereit.",
                        ),
                        LocalAction::MessageIn(
                            context,
                            messaging::Action::Command(MessageAction::RetryFile(file.id)),
                        ),
                        cx,
                    ))
                })
                .when(file.state == UploadState::Uncertain && file.uploaded, |d| {
                    d.child(self.icon_button(
                        scoped_id("check-upload-row", context, file.message_id(), 0),
                        ("refresh", "Gespräch aktualisieren"),
                        ButtonStyle::Quiet,
                        Availability::when(
                            !self.media_offline(),
                            "Die Verbindung ist derzeit nicht bereit.",
                        ),
                        LocalAction::MessageIn(
                            context,
                            messaging::Action::Command(MessageAction::Refresh),
                        ),
                        cx,
                    ))
                })
                .when(!pending, |d| {
                    d.child(self.icon_button(
                        scoped_id("hide-upload-row", context, file.message_id(), 0),
                        ("x", "Sendehinweis ausblenden"),
                        ButtonStyle::Quiet,
                        Availability::when(!running, "Dateien werden gesendet …"),
                        LocalAction::MessageIn(
                            context,
                            messaging::Action::Command(MessageAction::RemoveFile(file.id)),
                        ),
                        cx,
                    ))
                }),
        )
    }
}
