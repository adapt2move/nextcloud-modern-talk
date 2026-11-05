//! Composer attachments and native file gestures. Only typed intents cross to I/O owners.
use super::{
    ButtonStyle, Desktop, LocalAction, content,
    controls::{Availability, scoped_id},
    feedback::Feedback,
    messaging,
};
use gpui::{
    Context, Div, ExternalPaths, Focusable, Render, SharedString, Window, div, prelude::*, px,
};
use talk_core::ui::{
    Page, Text,
    content::AttachmentRef,
    messaging::{
        ComposeTarget, MessageAction, MessageFeature, RichObject, UploadFailure, UploadState,
    },
    presentation::{ConversationRef, RequestId, UiIntent},
};

pub(super) struct PendingSend {
    context: ConversationRef,
    request: RequestId,
    body: Text<4096>,
    editor: u64,
}
struct DragLabel {
    name: String,
}
impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
            .w(px(240.))
            .p_3()
            .rounded_lg()
            .bg(gpui::rgb(0xf3f5f7))
            .text_color(gpui::rgb(0x26354b))
            .truncate()
            .child(self.name.clone())
    }
}
pub(super) fn previewable(object: &RichObject) -> bool {
    matches!(object.mime_type.as_ref().map(|mime| mime.as_str()), Some(mime) if mime.starts_with("image/") || mime == "video/mp4" || mime == "video/webm" || mime == "application/pdf")
}
pub(super) fn file_detail(name: &str, bytes: Option<u64>) -> String {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 12
                && extension.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .unwrap_or("FILE")
        .to_ascii_uppercase();
    match bytes {
        Some(bytes) if bytes >= 1024 * 1024 => {
            format!("{extension} · {:.1} MiB", bytes as f64 / (1024. * 1024.))
        }
        Some(bytes) if bytes >= 1024 => format!("{extension} · {:.1} KiB", bytes as f64 / 1024.),
        Some(bytes) => format!("{extension} · {bytes} B"),
        None => extension,
    }
}
fn upload_label(state: UploadState) -> &'static str {
    match state {
        UploadState::Selected => "Bereit zum Senden",
        UploadState::Uploading => "Wird hochgeladen …",
        UploadState::Sharing => "Wird gesendet …",
        UploadState::Uncertain => "Sendestatus unklar. Bitte im Gespräch prüfen.",
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
    pub(super) fn can_send_attachments(&self) -> bool {
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
                    .any(|file| file.state == UploadState::Selected)
        }) && self.message_feature_enabled(MessageFeature::Files)
            && !self.media_offline()
            && self.message_ui.editing.is_none()
            && self.snapshot.selected.is_some_and(|context| {
                !self.drafts.pending(context)
                    && self.drafts.target(context) == ComposeTarget::default()
            })
    }
    pub(super) fn send_attachments(&mut self, cx: &mut Context<Self>) -> bool {
        if !self
            .snapshot
            .message_panel
            .as_ref()
            .is_some_and(|panel| !panel.uploads.as_slice().is_empty())
        {
            return false;
        }
        if !self.can_send_attachments() {
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
    pub(super) fn refresh_attachment_send(&mut self, cx: &mut Context<Self>) {
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
            .and_then(|panel| panel.files_sent)
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
    pub(super) fn stage_dropped_files(
        &mut self,
        context: ConversationRef,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot.selected != Some(context)
            || self.overlay.is_some()
            || self.background_inert
            || !self.message_feature_enabled(MessageFeature::Files)
        {
            return;
        }
        let mut selected = Page::empty();
        for path in paths.paths() {
            let Some(path) = path.to_str().and_then(|path| Text::new(path).ok()) else {
                self.feedback = Some(
                    Feedback::error("Diese Dateiauswahl wird nicht unterstützt.")
                        .in_conversation(context),
                );
                cx.notify();
                return;
            };
            if selected.try_push(path).is_err() {
                self.feedback = Some(
                    Feedback::error("Du kannst höchstens 8 Dateien auf einmal anhängen.")
                        .in_conversation(context),
                );
                cx.notify();
                return;
            }
        }
        if self
            .submit_message(MessageAction::StageFiles(selected))
            .is_some()
        {
            self.composer.focus_handle(cx).focus(window);
        }
        cx.notify();
    }
    pub(super) fn upload_strip(&self, cx: &Context<Self>) -> Div {
        let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| Some(panel.context) == self.snapshot.selected)
        else {
            return div();
        };
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .children(panel.uploads.as_slice().iter().map(|file| {
                div()
                    .w(px(280.))
                    .max_w_full()
                    .rounded(px(8.))
                    .bg(self.theme.subtle)
                    .p_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.scaled_icon("file", 22., self.theme.muted))
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
                            .child(self.hint(self.l(upload_label(file.state)))),
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
            }))
    }
    pub(in crate::presentation) fn exportable_card(
        &self,
        card: Div,
        reference: AttachmentRef,
        source: &RichObject,
        cx: &Context<Self>,
    ) -> Div {
        let id = scoped_id(
            "file-drag",
            reference.context,
            reference.message,
            super::content::placeholder_key(reference.placeholder.as_str()),
        );
        let source = source.clone();
        let weak = cx.entity().downgrade();
        let enabled = self.download_available().is_enabled() && !self.media_offline();
        div().max_w_full().child(
            card.id(id)
                .when(enabled && cfg!(target_os = "macos"), |card| {
                    card.on_drag(
                        (reference, source),
                        move |(reference, source), _, window, cx| {
                            let reference = reference.clone();
                            let source = source.clone();
                            let name = source.name.as_str().to_owned();
                            let weak = weak.clone();
                            window.start_file_drag(
                                SharedString::from(name.clone()),
                                cx,
                                move |destination, cx| {
                                    let destination =
                                        destination.to_str().and_then(|path| Text::new(path).ok());
                                    Self::wait_for_native_export(
                                        weak,
                                        reference,
                                        source,
                                        destination,
                                        cx,
                                    )
                                },
                            );
                            cx.new(|_| DragLabel { name })
                        },
                    )
                }),
        )
    }
    /// A foreground native-promise lifetime waiter; it performs no file or network I/O.
    /// The platform owns/cancels this task. The composition result is scalar and
    /// survives row removal; its acknowledgement releases the single export slot.
    fn wait_for_native_export(
        weak: gpui::WeakEntity<Self>,
        reference: AttachmentRef,
        source: RichObject,
        destination: Option<Text<16384>>,
        cx: &mut gpui::AsyncApp,
    ) -> gpui::Task<bool> {
        cx.spawn(async move |cx| {
            let Some(destination) = destination else {
                return false;
            };
            let context = reference.context;
            let mut captured = Page::empty();
            if captured.try_push(source).is_err() {
                return false;
            }
            let request = weak
                .update(cx, |view, _| {
                    if view.snapshot.selected != Some(context) {
                        return None;
                    }
                    view.submit_message(MessageAction::ExportFile {
                        attachment: reference,
                        source: captured,
                        destination,
                    })
                })
                .ok()
                .flatten();
            let Some(request) = request else {
                return false;
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            let saved = loop {
                match weak.update(cx, |view, _| {
                    view.snapshot
                        .file_export
                        .filter(|result| result.request == request && result.context == context)
                        .and_then(|result| result.saved)
                }) {
                    Ok(Some(saved)) => break saved,
                    Err(_) => break false,
                    _ if std::time::Instant::now() >= deadline => break false,
                    _ => {
                        gpui::Timer::after(std::time::Duration::from_millis(50)).await;
                    }
                }
            };
            let _ = weak.update(cx, |view, _| {
                view.submit(UiIntent::FinishFileExport { request, context });
            });
            saved
        })
    }
    pub(in crate::presentation) fn file_card(
        &self,
        reference: &AttachmentRef,
        source: &RichObject,
        preview: bool,
        cx: &Context<Self>,
    ) -> Div {
        let id = scoped_id(
            "file-card",
            reference.context,
            reference.message,
            super::content::placeholder_key(reference.placeholder.as_str()),
        );
        let group: SharedString = format!("{id:?}").into();
        let action = if preview {
            content::Action::Preview(reference.clone())
        } else {
            content::Action::Download(reference.clone())
        };
        div()
            .group(group.clone())
            .w(px(340.))
            .max_w_full()
            .rounded(px(8.))
            .bg(self.theme.subtle)
            .flex()
            .items_center()
            .pr_2()
            .child(
                self.button(
                    id,
                    "",
                    ButtonStyle::Quiet,
                    if preview {
                        Availability::available()
                    } else {
                        self.download_available()
                    },
                    LocalAction::Content(action),
                    cx,
                )
                .min_w_0()
                .flex_1()
                .p_3()
                .gap_3()
                .child(
                    div()
                        .size(px(36.))
                        .flex_none()
                        .rounded(px(6.))
                        .bg(self.theme.surface)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.scaled_icon("file", 22., self.theme.muted)),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_1()
                        .child(
                            div()
                                .w_full()
                                .truncate()
                                .text_size(self.sp(13.))
                                .child(source.name.as_str().to_owned()),
                        )
                        .child(self.hint(&file_detail(source.name.as_str(), source.size))),
                ),
            )
            .child(self.attachment_download(reference, group, cx))
    }
    pub(in crate::presentation) fn attachment_download(
        &self,
        reference: &AttachmentRef,
        group: SharedString,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        self.icon_button(
            scoped_id(
                "attachment-download",
                reference.context,
                reference.message,
                super::content::placeholder_key(reference.placeholder.as_str()),
            ),
            ("download", "Herunterladen"),
            ButtonStyle::Secondary,
            self.download_available(),
            LocalAction::Content(content::Action::Download(reference.clone())),
            cx,
        )
        .opacity(0.)
        .group_hover(group, |style| style.opacity(1.))
        .focus(|style| style.opacity(1.))
    }
}
