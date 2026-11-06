//! Native file gestures and incoming file cards. Typed intents cross to I/O owners.
mod uploads;
use super::{
    ButtonStyle, Desktop, LocalAction, content,
    controls::{Availability, scoped_id},
    feedback::Feedback,
};
pub(super) use crate::presentation::attachments::uploads::PendingSend;
use gpui::{
    Context, Div, ExternalPaths, Focusable, MouseButton, Render, SharedString, Window, div,
    prelude::*, px,
};
use talk_core::ui::{
    Page, Text,
    content::AttachmentRef,
    messaging::{MessageAction, MessageFeature, RichObject},
    presentation::{ConversationRef, UiIntent},
};

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
impl Desktop {
    pub(super) fn paste_clipboard_image(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let primary = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control
        };
        let paste = (key == "v" && primary && !modifiers.alt && !modifiers.shift)
            || (key == "insert"
                && modifiers.shift
                && !modifiers.control
                && !modifiers.alt
                && !modifiers.platform);
        if !paste
            || self.overlay.is_some()
            || self.background_inert
            || !self.composer.focus_handle(cx).is_focused(window)
            || self.input_composing(cx)
            || !self.composer_available()
            || !self.message_feature_enabled(MessageFeature::Files)
        {
            return false;
        }
        let Some(context) = self.snapshot.selected else {
            return false;
        };
        let (extension, image) = match super::input::read_pasted_image(cx) {
            super::input::PastedImage::None => return false,
            super::input::PastedImage::Unsupported => {
                self.feedback = Some(
                    Feedback::error("Dieses Bildformat wird nicht unterstützt.")
                        .in_conversation(context),
                );
                return true;
            }
            super::input::PastedImage::TooLarge => {
                self.feedback = Some(
                    Feedback::error("Das Bild aus der Zwischenablage ist zu groß.")
                        .in_conversation(context),
                );
                return true;
            }
            super::input::PastedImage::Image(extension, image) => (extension, image),
        };
        if let Ok(name) = Text::new(&format!("clipboard.{extension}")) {
            self.submit_message(MessageAction::StageClipboardImage { name, image });
        }
        true
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
                    card.drag_threshold(px(8.)).on_drag(
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
        // This action is inside the file's drag surface. Do not arm that
        // ancestor on a download click (trackpads can move a few pixels).
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .opacity(0.)
        .group_hover(group, |style| style.opacity(1.))
        .focus(|style| style.opacity(1.))
    }
}
