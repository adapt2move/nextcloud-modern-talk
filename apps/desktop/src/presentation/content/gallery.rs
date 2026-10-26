//! Bounded media tiles and a room-scoped viewer. No remote routes or byte I/O.
use super::*;
use crate::presentation::motion::DialogSurface;
use talk_core::ui::{content::PlaybackState, messaging::RichObject};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::presentation) enum Kind {
    Image,
    Video,
    Pdf,
    File,
}

pub(in crate::presentation) fn kind(object: &RichObject) -> Kind {
    match object.mime_type.as_ref().map(|m| m.as_str()).unwrap_or("") {
        mime if mime.starts_with("image/") => Kind::Image,
        mime if mime.starts_with("video/") => Kind::Video,
        "application/pdf" => Kind::Pdf,
        _ => Kind::File,
    }
}
pub(super) fn entries(
    snapshot: &PresentationSnapshot,
    current: Option<&AttachmentRef>,
) -> Vec<AttachmentRef> {
    window::entries(snapshot, current)
}
pub(super) fn adjacent(
    items: &[AttachmentRef],
    current: &AttachmentRef,
    offset: i8,
) -> Option<AttachmentRef> {
    let index = items.iter().position(|item| item == current)?;
    let next = match offset {
        -1 => index.checked_sub(1)?,
        1 => index.checked_add(1)?,
        _ => return None,
    };
    items
        .get(next)
        .filter(|r| r.context == current.context)
        .cloned()
}
#[derive(Default)]
pub(super) struct Swipe {
    distance: f32,
    blocked: bool,
    last: Option<std::time::Instant>,
}
impl Swipe {
    fn update(&mut self, x: f32, phase: gpui::TouchPhase, now: std::time::Instant) -> Option<i8> {
        if self.last.is_none_or(|last| {
            now.saturating_duration_since(last) > std::time::Duration::from_millis(200)
        }) || matches!(phase, gpui::TouchPhase::Started)
        {
            self.distance = 0.;
            self.blocked = false;
        }
        self.last = Some(now);
        if matches!(phase, gpui::TouchPhase::Ended) {
            self.distance = 0.;
            self.blocked = false;
            return None;
        }
        if self.blocked || !x.is_finite() {
            return None;
        }
        self.distance = (self.distance + x).clamp(-200., 200.);
        if self.distance.abs() < 72. {
            return None;
        }
        self.blocked = true;
        Some(if self.distance < 0. { 1 } else { -1 })
    }
}
impl Desktop {
    pub(in crate::presentation) fn bare_media_body(
        &self,
        m: &talk_core::ui::presentation::MessageView,
    ) -> bool {
        let media: Vec<_> = m
            .metadata
            .rich
            .as_slice()
            .iter()
            .filter(|o| o.kind.as_str() == "file" && matches!(kind(o), Kind::Image | Kind::Video))
            .take(32)
            .collect();
        !media.is_empty()
            && (m.body.as_str().trim().is_empty()
                || media.len() == 1
                    && (m.body.as_str().trim() == media[0].name.as_str()
                        || m.body.as_str().trim()
                            == format!("{{{}}}", media[0].placeholder.as_str())))
    }
    pub(in crate::presentation) fn media_spans(&self) -> &[groups::Span] {
        &self.content_ui.media_groups
    }
    pub(in crate::presentation) fn media_span(&self, id: u128) -> Option<groups::Span> {
        let index = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .position(|m| m.local_id == id)?;
        self.content_ui
            .media_groups
            .iter()
            .find(|s| s.start <= index && index < s.end)
            .copied()
    }
    pub(in crate::presentation) fn media_attachments<'a>(
        &'a self,
        message: &'a talk_core::ui::presentation::MessageView,
    ) -> Vec<(AttachmentRef, &'a RichObject)> {
        let Some(context) = self.snapshot.selected else {
            return Vec::new();
        };
        let rows: &[talk_core::ui::presentation::MessageView] =
            if let Some(span) = self.media_span(message.local_id) {
                self.snapshot
                    .messages
                    .as_slice()
                    .get(span.start..span.end)
                    .unwrap_or(&[])
            } else {
                std::slice::from_ref(message)
            };
        rows.iter()
            .flat_map(|m| {
                m.metadata
                    .rich
                    .as_slice()
                    .iter()
                    .filter(|o| {
                        o.kind.as_str() == "file" && matches!(kind(o), Kind::Image | Kind::Video)
                    })
                    .map(move |o| {
                        (
                            AttachmentRef {
                                context,
                                message: m.local_id,
                                placeholder: o.placeholder.clone(),
                            },
                            o,
                        )
                    })
            })
            .take(32)
            .collect()
    }
    pub(in crate::presentation) fn media_offline(&self) -> bool {
        self.snapshot.content == ContentState::Offline
            || self.snapshot.selected.is_some_and(|c| {
                c.account.runtime.as_local() == 0
                    || c.conversation.host.as_local() == 0
                    || self
                        .account_connection_failure(c.account)
                        .is_some_and(crate::presentation::connectivity::transient)
            })
    }
    pub(super) fn open_media_preview(
        &mut self,
        reference: AttachmentRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot.selected != Some(reference.context)
            || self.attachment_object(&reference).is_none()
        {
            return;
        }
        if !self.media_offline()
            && self
                .submit_content(ContentAction::Preview(reference.clone()))
                .is_none()
        {
            return;
        }
        self.content_ui.gallery = entries(&self.snapshot, Some(&reference));
        self.content_ui.gallery_position = window::global_position(&self.snapshot, &reference);
        self.content_ui.preview = Some(reference);
        if self.overlay != Some(Overlay::ContentPreview) {
            self.content_ui.return_overlay = self.overlay;
            self.handle(LocalAction::Open(Overlay::ContentPreview), window, cx);
        }
    }
    pub(in crate::presentation) fn attachment_card(
        &self,
        reference: AttachmentRef,
        object: &RichObject,
        cx: &Context<Self>,
    ) -> Div {
        let media = matches!(kind(object), Kind::Image | Kind::Video);
        let target = AssetTarget::Thumbnail(reference.clone());
        let preview = self.asset_image(&target);
        let mut card = self
            .button(
                scoped_id(
                    "content-preview",
                    reference.context,
                    reference.message,
                    placeholder_key(reference.placeholder.as_str()),
                ),
                "",
                ButtonStyle::Quiet,
                Availability::available(),
                LocalAction::Content(Action::Preview(reference.clone())),
                cx,
            )
            .w_full()
            .p_0()
            .flex_col()
            .items_start()
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(if media { 136. } else { 160. }))
                    .bg(self.theme.subtle)
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if let Some(image) = preview {
                        let maximum = match &image {
                            ImageSource::Render(image) => {
                                let s = image.size(0);
                                (s.width.0 as f32 / 2., s.height.0 as f32 / 2.)
                            }
                            _ => (200., 136.),
                        };
                        img(image)
                            .size_full()
                            .max_w(px(maximum.0))
                            .max_h(px(maximum.1))
                            .object_fit(ObjectFit::Contain)
                            .rounded(px(8.))
                            .into_any_element()
                    } else {
                        self.scaled_icon(
                            if kind(object) == Kind::Video {
                                "play"
                            } else {
                                "file"
                            },
                            28.,
                            self.theme.muted,
                        )
                        .into_any_element()
                    })
                    .when(kind(object) == Kind::Video, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom_2()
                                .right_2()
                                .child(self.scaled_icon("play", 18., self.theme.text)),
                        )
                    }),
            );
        if !media {
            card = card.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(self.scaled_icon("file", 16., self.theme.muted))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(self.sp(13.))
                            .child(object.name.as_str().to_owned()),
                    ),
            );
        }
        div()
            .relative()
            .w(px(if media { 200. } else { 280. }))
            .max_w_full()
            .child(card)
            .when(media && self.media_span(reference.message).is_some(), |d| {
                d.child(
                    self.attachment_actions(&reference, cx)
                        .absolute()
                        .top_1()
                        .right_1(),
                )
            })
    }
    pub(in crate::presentation) fn attachment_grid(
        &self,
        entries: &[(AttachmentRef, &RichObject)],
        cx: &Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .max_w(px(408.))
            .children(
                entries
                    .iter()
                    .map(|(reference, object)| self.attachment_card(reference.clone(), object, cx)),
            )
    }
    pub(in crate::presentation) fn media_view(&self, _window: &Window, cx: &Context<Self>) -> Div {
        let reference = self
            .content_ui
            .preview
            .as_ref()
            .filter(|r| Some(r.context) == self.snapshot.selected);
        let current = reference.cloned();
        let swipe_reference = current.clone();
        let mut dialog = div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(self.theme.scrim)
            .p_5()
            .flex()
            .flex_col()
            .track_focus(&self.modal_focus)
            .tab_group();
        dialog = dialog.on_key_down(cx.listener(
            move |this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                let offset = match event.keystroke.key.as_str() {
                    "left" => -1,
                    "right" => 1,
                    _ => return,
                };
                if let Some(from) = current.clone() {
                    this.content_action(Action::GalleryMove { from, offset }, window, cx);
                    cx.stop_propagation();
                }
            },
        ));
        dialog = dialog.on_scroll_wheel(cx.listener(
            move |this, event: &gpui::ScrollWheelEvent, window, cx| {
                let delta = event.delta.pixel_delta(px(20.));
                if delta.x.abs() <= delta.y.abs() {
                    return;
                }
                if let Some(offset) = this.content_ui.swipe.update(
                    f32::from(delta.x),
                    event.touch_phase,
                    std::time::Instant::now(),
                ) && let Some(from) = swipe_reference.clone()
                {
                    this.content_action(Action::GalleryMove { from, offset }, window, cx);
                }
                cx.stop_propagation();
            },
        ));
        let mut stage = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .items_center()
            .justify_center();
        if let Some(reference) = reference {
            let offline = self.media_offline();
            let target = if offline {
                AssetTarget::Thumbnail(reference.clone())
            } else {
                AssetTarget::Image(reference.clone())
            };
            let image = self.asset_image(&target);
            if let Some(image) = image {
                stage = stage.child(img(image).size_full().object_fit(ObjectFit::Contain));
            } else {
                let loading = !offline
                    && matches!(
                        self.asset(&target).map(|a| a.state),
                        None | Some(AssetState::Loading | AssetState::Ready { .. })
                    );
                if loading {
                    stage = stage.child(self.pending_dots(self.theme.muted));
                } else {
                    let label = if offline {
                        "Vorschau nicht verfügbar"
                    } else {
                        match self.asset(&target).map(|a| a.state) {
                            Some(AssetState::Denied) => {
                                "Du hast keinen Zugriff auf diese Vorschau."
                            }
                            Some(AssetState::Failed) => "Die Vorschau konnte nicht geladen werden.",
                            _ => {
                                "Keine native Vorschau verfügbar. Öffne die Datei in der Cloud-Webansicht."
                            }
                        }
                    };
                    stage = stage.child(self.hint(self.l(label)));
                }
            }
            if adjacent(&self.content_ui.gallery, reference, -1).is_some() {
                stage = stage.child(
                    self.icon_button(
                        "gallery-previous",
                        ("arrow-left", "Vorherige Datei"),
                        ButtonStyle::Quiet,
                        Availability::available(),
                        LocalAction::Content(Action::GalleryMove {
                            from: reference.clone(),
                            offset: -1,
                        }),
                        cx,
                    )
                    .absolute()
                    .left_2(),
                );
            }
            if adjacent(&self.content_ui.gallery, reference, 1).is_some() {
                stage = stage.child(
                    self.icon_button(
                        "gallery-next",
                        ("arrow-right", "Nächste Datei"),
                        ButtonStyle::Quiet,
                        Availability::available(),
                        LocalAction::Content(Action::GalleryMove {
                            from: reference.clone(),
                            offset: 1,
                        }),
                        cx,
                    )
                    .absolute()
                    .right_2(),
                );
            }
            if let Some(video) = self
                .snapshot
                .content_panel
                .playback
                .as_ref()
                .filter(|v| v.attachment == *reference)
            {
                let playing = video.state == PlaybackState::Playing;
                stage = stage.child(
                    self.button(
                        "gallery-play",
                        if playing { "Pause" } else { "Abspielen" },
                        ButtonStyle::Quiet,
                        !offline
                            && matches!(
                                video.state,
                                PlaybackState::Paused
                                    | PlaybackState::Playing
                                    | PlaybackState::Ended
                            ),
                        LocalAction::Content(Action::Command(if playing {
                            ContentAction::PausePreview {
                                attachment: reference.clone(),
                                generation: video.generation,
                            }
                        } else {
                            ContentAction::PlayPreview {
                                attachment: reference.clone(),
                                generation: video.generation,
                            }
                        })),
                        cx,
                    )
                    .absolute()
                    .bottom_3(),
                );
            }
        }
        let mut footer = div()
            .flex_none()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_3()
            .pt_3()
            .text_size(self.sp(12.))
            .text_color(self.theme.muted);
        if let Some(reference) = reference {
            let name = self
                .attachment_object(reference)
                .map_or(self.l("Datei"), |o| o.name.as_str());
            footer = footer.child(div().flex_1().min_w_0().truncate().child(name.to_owned()));
            if self.media_offline() {
                footer = footer.child(self.hint(self.l("Kleine Vorschau")));
            }
            if let Some(pages) = self
                .asset(&AssetTarget::Image(reference.clone()))
                .and_then(|a| a.document_pages)
            {
                footer = footer.child(self.hint(&self.lf(
                    "Erste PDF-Seite · {pages} Seiten",
                    &[("pages", &pages.to_string())],
                )));
            }
            if self
                .snapshot
                .content_panel
                .playback
                .as_ref()
                .is_some_and(|v| {
                    v.attachment == *reference && matches!(v.state, PlaybackState::Failed(_))
                })
            {
                footer = footer.child(self.hint(self.l("Videowiedergabe fehlgeschlagen.")));
            }
            if self
                .snapshot
                .content_panel
                .playback
                .as_ref()
                .is_some_and(|v| v.attachment == *reference && v.stop_unconfirmed)
            {
                footer = footer
                    .child(self.hint(self.l("Videowiedergabe konnte noch nicht beendet werden.")));
            }
            if !self.media_offline()
                && (self
                    .asset(&AssetTarget::Image(reference.clone()))
                    .is_some_and(|a| a.state == AssetState::Failed)
                    || self
                        .snapshot
                        .content_panel
                        .playback
                        .as_ref()
                        .is_some_and(|v| {
                            v.attachment == *reference
                                && matches!(v.state, PlaybackState::Failed(_))
                        }))
            {
                footer = footer.child(self.button(
                    "preview-retry",
                    "Vorschau erneut laden",
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Content(Action::Preview(reference.clone())),
                    cx,
                ));
            }
            if let Some((index, total)) = self.content_ui.gallery_position {
                footer = footer.child(self.hint(&format!("{} / {}", index + 1, total)));
            }
            footer = footer
                .child(self.button(
                    "preview-download",
                    "Herunterladen",
                    ButtonStyle::Quiet,
                    self.download_available(),
                    LocalAction::Content(Action::Download(reference.clone())),
                    cx,
                ))
                .child(self.button_with_icon(
                    "preview-cloud",
                    ("In der Cloud-Webansicht öffnen", "external"),
                    ButtonStyle::Quiet,
                    !self.media_offline(),
                    LocalAction::Content(Action::Command(ContentAction::OpenCloud(
                        reference.clone(),
                    ))),
                    cx,
                ));
        }
        dialog
            .child(
                self.dialog_panel(
                    DialogSurface::Overlay(Overlay::ContentPreview),
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(stage)
                        .child(footer)
                        .children(self.download_status(cx)),
                ),
            )
            .child(
                self.control_with_geometry(
                    "close-overlay",
                    ButtonStyle::Quiet,
                    Availability::available().help("Schließen"),
                    LocalAction::Close,
                    crate::presentation::controls::ControlGeometry::bare(),
                    cx,
                )
                .size(px(32.))
                .min_h(px(32.))
                .p_0()
                .child(self.scaled_icon("x", 20., self.theme.text))
                .absolute()
                .top_2()
                .right_2(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_swipe_advances_once_per_gesture_and_recovers_after_a_gap() {
        use gpui::TouchPhase::{Ended, Moved, Started};
        let now = std::time::Instant::now();
        let mut swipe = Swipe::default();
        assert_eq!(swipe.update(-40., Started, now), None);
        assert_eq!(swipe.update(-40., Moved, now), Some(1));
        assert_eq!(swipe.update(-120., Moved, now), None);
        assert_eq!(swipe.update(120., Ended, now), None);
        assert_eq!(swipe.update(80., Started, now), Some(-1));
        assert_eq!(swipe.update(f32::NAN, Moved, now), None);
        assert_eq!(
            swipe.update(-80., Moved, now + std::time::Duration::from_millis(201)),
            Some(1)
        );
    }

    #[test]
    fn navigation_requires_the_exact_room_and_file_and_does_not_wrap() {
        use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};
        let account = AccountId::from_local(1);
        let context = ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: 1,
            },
        };
        let first = AttachmentRef {
            context,
            message: 1,
            placeholder: Text::new("a").unwrap(),
        };
        let second = AttachmentRef {
            context,
            message: 1,
            placeholder: Text::new("b").unwrap(),
        };
        let files = [first.clone(), second.clone()];
        assert_eq!(adjacent(&files, &first, 1), Some(second.clone()));
        assert_eq!(adjacent(&files, &first, -1), None);
        assert_eq!(adjacent(&files, &second, 1), None);
        let mut stale = first;
        stale.context.account.runtime = talk_core::ui::RuntimeGeneration::from_local(99);
        assert_eq!(adjacent(&files, &stale, 1), None);
    }
}
