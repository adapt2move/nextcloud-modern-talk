//! Message-row geometry, sender grouping and truthful delivery presentation.
use super::{Action, compact_system_message, menu, message_actions_allowed};
use crate::presentation::{
    ButtonStyle, Desktop, LocalAction,
    controls::{Availability, ControlGeometry, scoped_id},
};
use gpui::{Context, Div, ElementId, FontWeight, Stateful, div, prelude::*, px};
use talk_core::ui::{
    messaging::MessageFeature,
    presentation::{DeliveryState, MessageView},
};

impl Desktop {
    pub(in crate::presentation) fn attachment_actions(
        &self,
        reference: &talk_core::ui::content::AttachmentRef,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        self.icon_button(
            scoped_id("attachment-menu", reference.context, reference.message, 0),
            ("more", "Nachrichtenaktionen"),
            ButtonStyle::Quiet,
            Availability::available(),
            LocalAction::MessageIn(reference.context, Action::Menu(reference.message)),
            cx,
        )
        .child(self.message_menu_anchor(
            reference.context,
            reference.message,
            menu::Source::Actions,
        ))
    }
    pub(super) fn message_row(
        &self,
        m: &MessageView,
        continuation: bool,
        cx: &Context<Self>,
    ) -> Div {
        let t = self.theme;
        // A tombstone remains an authored row even beside the same sender.
        let continuation = continuation && !m.metadata.deleted;
        let context = self.snapshot.selected.expect("message context");
        let action_group: gpui::SharedString = scoped_id("message-actions", context, m.local_id, 0)
            .to_string()
            .into();
        if compact_system_message(m) {
            return div().child(
                div()
                    .id(scoped_id("message", context, m.local_id, 0))
                    .px_6()
                    .py_2()
                    .flex()
                    .justify_center()
                    .gap_2()
                    .text_color(t.muted)
                    .text_size(self.sp(12.))
                    .child(self.message_text(m, cx))
                    .child(self.hint(m.time_label.as_str())),
            );
        }
        let delivery = self.message_delivery(m);
        let pinned = self.snapshot.message_panel.as_ref().is_some_and(|panel| {
            panel.context == context
                && panel
                    .pinned_messages
                    .as_slice()
                    .iter()
                    .any(|pin| pin.local_id == m.local_id)
        });
        let metadata = |text: &str| {
            self.hint(text)
                .text_size(self.sp(13.))
                .line_height(self.sp(20.))
                .flex_none()
        };
        let mut content = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .when(pinned && !m.metadata.deleted, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_color(t.muted)
                        .child(self.scaled_icon("pin", 12., t.muted))
                        .child(self.hint(self.l("Angeheftet")))
                        .text_size(self.sp(11.)),
                )
            })
            .when(!continuation, |d| {
                d.child(
                    div()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .pr(px(76.))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(self.sp(13.))
                                .line_height(self.sp(20.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(m.author.as_str().to_owned()),
                        )
                        .when(m.own, |d| d.child(metadata(self.l("Du"))))
                        .child(metadata(m.time_label.as_str()))
                        .when(m.metadata.edited, |d| {
                            d.child(metadata(self.l("Bearbeitet")))
                        }),
                )
            })
            .when_some(
                m.metadata.parent.as_ref().filter(|_| !m.metadata.deleted),
                |d, parent| {
                    d.child(self.quote_preview(
                        if parent.unavailable {
                            self.l("Ursprüngliche Nachricht nicht mehr verfügbar.")
                        } else {
                            parent.author.as_str()
                        },
                        (!parent.unavailable).then_some(parent.text.as_str()),
                    ))
                },
            );
        if m.metadata.deleted {
            content = content.child(self.hint(self.l("Nachricht gelöscht.")));
        } else if !self.bare_media_body(m) {
            content = content.child(
                div().flex().items_start().gap_2().pr(px(76.)).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(self.message_text(m, cx))
                        .when(continuation && m.metadata.edited, |d| {
                            d.child(self.hint(self.l("Bearbeitet")))
                        }),
                ),
            );
        }
        let media = self.media_attachments(m);
        if !m.metadata.deleted && !media.is_empty() {
            content = content.child(self.attachment_grid(&media, cx));
        }
        for object in m.metadata.rich.as_slice().iter().filter(|r| {
            !m.metadata.deleted
                && !(r.kind.as_str() == "file"
                    && matches!(
                        crate::presentation::content::gallery::kind(r),
                        crate::presentation::content::gallery::Kind::Image
                            | crate::presentation::content::gallery::Kind::Video
                    ))
                && !matches!(
                    r.kind.as_str(),
                    "user" | "guest" | "group" | "email" | "call"
                )
        }) {
            content = if object.kind.as_str() == "file" {
                content.child(self.attachment_card(
                    talk_core::ui::content::AttachmentRef {
                        context,
                        message: m.local_id,
                        placeholder: object.placeholder.clone(),
                    },
                    object,
                    cx,
                ))
            } else if matches!(object.kind.as_str(), "poll" | "talk-poll")
                && let Some(id) = object.id.as_ref().and_then(|id| id.as_str().parse().ok())
            {
                content.child(self.poll_card(id, cx))
            } else {
                content.child(
                    div()
                        .p_3()
                        .rounded_lg()
                        .bg(t.subtle)
                        .child(object.name.as_str().to_owned())
                        .child(self.hint(self.l(match object.kind.as_str() {
                            "file" => "Datei",
                            "poll" => "Umfrage",
                            _ => "Anhang",
                        }))),
                )
            };
        }
        if !m.metadata.deleted && !m.metadata.reactions.as_slice().is_empty() {
            let reaction_height = self.sp(24.);
            content = content.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(2.))
                    .children(m.metadata.reactions.as_slice().iter().map(|r| {
                        // The emoji is the server reaction key. Its control identity
                        // survives count changes and other emoji being inserted.
                        let id = gpui::ElementId::Name(
                            format!(
                                "{}:{}",
                                scoped_id("reaction", context, m.local_id, 0),
                                r.emoji.as_str()
                            )
                            .into(),
                        );
                        self.control_with_geometry(
                            id,
                            ButtonStyle::Quiet,
                            self.reaction_available(m.local_id),
                            LocalAction::MessageIn(
                                context,
                                Action::React(m.local_id, r.emoji.clone()),
                            ),
                            ControlGeometry::inset_background(
                                0.,
                                7.,
                                4.,
                                if r.own { t.selected } else { t.subtle },
                            ),
                            cx,
                        )
                        .min_w(px(32.))
                        .min_h(reaction_height + px(8.))
                        .p_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .px(px(6.))
                                .h(reaction_height)
                                .text_color(if r.own { t.accent } else { t.muted })
                                .text_size(self.sp(11.))
                                .line_height(self.sp(16.))
                                .child(r.emoji.as_str().to_owned())
                                .child(r.count.to_string()),
                        )
                    }))
                    .child(self.reaction_picker_button(m, menu::Source::Badges, cx)),
            );
        }
        if !m.metadata.deleted && m.metadata.truncated {
            content = content.child(self.hint(
                self.l("Lange Nachricht gekürzt angezeigt. Bearbeiten ist hier nicht verfügbar."),
            ));
        }
        div().child(
            div()
                .id(scoped_id("message", context, m.local_id, 0))
                .group(action_group.clone())
                .relative()
                .px_6()
                .py(px(if continuation { 3. } else { 12. }))
                .when(m.own, |d| d.min_h(px(34.)))
                .flex()
                .gap_3()
                .when(
                    self.highlighted_message == Some((context, m.local_id)),
                    |d| d.bg(t.selected),
                )
                .child(if continuation {
                    div()
                        .w(px(36.))
                        .flex_none()
                        .text_size(self.sp(10.))
                        .line_height(self.sp(20.))
                        .text_color(t.muted)
                        .child(
                            m.time_label
                                .as_str()
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .to_owned(),
                        )
                        .opacity(0.)
                        .group_hover(action_group.clone(), |s| s.opacity(1.))
                        .into_any_element()
                } else {
                    self.content_avatar(
                        talk_core::ui::content::AssetTarget::MessageAvatar {
                            context,
                            message: m.local_id,
                        },
                        m.initials.as_str(),
                        36.,
                    )
                    .into_any_element()
                })
                .child(content)
                .when_some(delivery, |d, indicator| {
                    let mut status = self.delivery_indicator(
                        indicator,
                        scoped_id("message-delivery", context, m.local_id, 0),
                        cx,
                    );
                    status.style().align_self = Some(gpui::AlignSelf::FlexEnd);
                    d.child(status)
                })
                // Hover/menu controls never enter the message's flex layout.
                // Revealing them cannot move text, metadata or delivery status.
                .when(
                    !m.metadata.deleted && self.media_span(m.local_id).is_none(),
                    |d| {
                        d.child(
                        div()
                            .absolute()
                            .when(!m.own, |d| d.top(px(8.)))
                            .when(m.own, |d| {
                                // Centre the 34 px action beside the 20 sp status.
                                d.bottom(
                                    px(if continuation { 3. } else { 12. }) + self.sp(10.)
                                        - px(17.),
                                )
                            })
                            // Own-message status keeps the far-right column even
                            // on compact continuations; actions sit to its left.
                            .right(if m.own {
                                px(24.) + self.delivery_width() + self.sp(8.)
                            } else {
                                px(8.)
                            })
                            .flex()
                            .gap_1()
                            .when(message_actions_allowed(m), |d| {
                                let picker_open = self.message_ui.menu.is_for(context, m.local_id);
                                d.children((0..3).map(|index| {
                                    self.quick_reaction_button(m, index, cx)
                                        .opacity(0.)
                                        .group_hover(action_group.clone(), |s| s.opacity(1.))
                                        .focus(|s| s.opacity(1.))
                                }))
                                .child(
                                    self.reaction_picker_button(m, menu::Source::Reactions, cx)
                                        .opacity(if picker_open { 1. } else { 0. })
                                        .group_hover(action_group.clone(), |s| s.opacity(1.))
                                        .focus(|s| s.opacity(1.)),
                                )
                            })
                            .when(!m.own, |d| {
                                d.child(
                                    self.icon_button(
                                        scoped_id("message-reply", context, m.local_id, 0),
                                        ("reply", "Antworten"),
                                        ButtonStyle::Quiet,
                                        Availability::when(
                                            self.can_reply_to(m),
                                            "Auf diese Nachricht kann nicht geantwortet werden.",
                                        ),
                                        LocalAction::MessageIn(context, Action::Reply(m.local_id)),
                                        cx,
                                    )
                                    .opacity(0.)
                                    .group_hover(action_group.clone(), |s| s.opacity(1.))
                                    .focus(|s| s.opacity(1.)),
                                )
                            })
                            .child(
                                self.icon_button(
                                    scoped_id("message-menu", context, m.local_id, 0),
                                    ("more", "Nachrichtenaktionen"),
                                    ButtonStyle::Quiet,
                                    {
                                        let available = Availability::when(
                                            message_actions_allowed(m),
                                            "Für diese Nachricht sind keine Aktionen verfügbar.",
                                        );
                                        if self.message_ui.menu.is_for(context, m.local_id) {
                                            available.explained_elsewhere()
                                        } else {
                                            available
                                        }
                                    },
                                    LocalAction::MessageIn(context, Action::Menu(m.local_id)),
                                    cx,
                                )
                                .child(self.message_menu_anchor(
                                    context,
                                    m.local_id,
                                    menu::Source::Actions,
                                ))
                                .opacity(if self.message_ui.menu.is_for(context, m.local_id) {
                                    1.
                                } else {
                                    0.
                                })
                                .group_hover(action_group, |s| s.opacity(1.))
                                .focus(|s| s.opacity(1.)),
                            ),
                    )
                    },
                ),
        )
    }
    fn message_delivery(&self, message: &MessageView) -> Option<DeliveryIndicator> {
        let panel = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| Some(panel.context) == self.snapshot.selected);
        delivery_indicator(
            message,
            panel.is_some_and(|panel| panel.availability(MessageFeature::ReadReceipts).enabled()),
            panel.and_then(|panel| panel.common_read),
        )
    }

    fn delivery_width(&self) -> gpui::Pixels {
        px(14. * (f32::from(self.scale) / 100.).sqrt()) + self.sp(5.)
    }

    fn delivery_indicator(
        &self,
        indicator: DeliveryIndicator,
        id: ElementId,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let color = match indicator {
            DeliveryIndicator::Read => t.accent,
            DeliveryIndicator::Failed => t.danger,
            DeliveryIndicator::Uncertain => t.warning,
            DeliveryIndicator::Pending | DeliveryIndicator::Accepted => t.muted,
        };
        let name = match indicator {
            DeliveryIndicator::Read | DeliveryIndicator::Accepted => "check",
            DeliveryIndicator::Pending => "clock",
            DeliveryIndicator::Failed | DeliveryIndicator::Uncertain => "info",
        };
        let glyph = px(14. * (f32::from(self.scale) / 100.).sqrt());
        self.status_icon(id, (name, indicator.label()), (14., color), None, cx)
            .w(self.delivery_width())
            .h(self.sp(20.))
            .when(indicator != DeliveryIndicator::Read, |d| d.justify_end())
            .when(indicator == DeliveryIndicator::Read, |d| {
                d.child(
                    self.scaled_icon("check", 14., color)
                        .absolute()
                        .top((self.sp(20.) - glyph) * 0.5)
                        .left(self.sp(5.)),
                )
            })
    }
}

/// Presentation groups never create protocol threads. Match a known actor key,
/// not a display name. The clock owner supplies the displayed calendar boundary.
pub(super) fn grouped_message(
    previous: &MessageView,
    current: &MessageView,
    same_local_day: bool,
) -> bool {
    let eligible = |message: &MessageView| {
        !message.metadata.system
            && !message.metadata.deleted
            && message.metadata.parent.is_none()
            && message.metadata.thread.is_none()
            && message.reply_count == 0
            && message.metadata.timestamp != 0
    };
    eligible(previous)
        && eligible(current)
        && previous.own == current.own
        && previous
            .metadata
            .author_key
            .as_ref()
            .filter(|key| !key.as_str().is_empty())
            .is_some_and(|key| current.metadata.author_key.as_ref() == Some(key))
        && same_local_day
        && current
            .metadata
            .timestamp
            .checked_sub(previous.metadata.timestamp)
            .is_some_and(|elapsed| elapsed <= 120)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeliveryIndicator {
    Pending,
    Accepted,
    Read,
    Failed,
    Uncertain,
}
impl DeliveryIndicator {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "Wird gesendet …",
            Self::Accepted => "Vom Server angenommen",
            Self::Read => "Gelesen (öffentliche Lesebestätigungen)",
            Self::Failed => "Senden fehlgeschlagen",
            Self::Uncertain => {
                "Sendestatus unklar. Vor erneutem Senden muss der Status abgeglichen werden."
            }
        }
    }
}

fn delivery_indicator(
    message: &MessageView,
    receipts_allowed: bool,
    common_read: Option<u128>,
) -> Option<DeliveryIndicator> {
    if !message.own || message.metadata.deleted || message.metadata.system {
        return None;
    }
    Some(match message.delivery {
        DeliveryState::Pending => DeliveryIndicator::Pending,
        DeliveryState::Received | DeliveryState::ServerAccepted => {
            if receipts_allowed && common_read.is_some_and(|id| message.local_id <= id) {
                DeliveryIndicator::Read
            } else {
                DeliveryIndicator::Accepted
            }
        }
        DeliveryState::Failed => DeliveryIndicator::Failed,
        DeliveryState::Uncertain => DeliveryIndicator::Uncertain,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::Text;

    fn authored_message(id: u128, key: Option<&str>, timestamp: u64) -> MessageView {
        MessageView {
            local_id: id,
            author: Text::new("Same visible name").unwrap(),
            initials: Text::new("S").unwrap(),
            time_label: Text::new("09:00").unwrap(),
            body: Text::new("Fixture").unwrap(),
            own: false,
            delivery: DeliveryState::Received,
            attachments: talk_core::ui::Page::empty(),
            reply_count: 0,
            metadata: talk_core::ui::messaging::MessageMetadata {
                author_key: key.map(|key| Text::new(key).unwrap()),
                timestamp,
                ..Default::default()
            },
        }
    }

    #[test]
    fn grouping_uses_actor_identity_and_at_most_two_minutes_between_messages() {
        let previous = authored_message(1, Some("users:one"), 32_400);
        let mut current = authored_message(2, Some("users:one"), 32_520);
        assert!(grouped_message(&previous, &current, true));
        current.metadata.timestamp += 1;
        assert!(!grouped_message(&previous, &current, true));
        current.metadata.timestamp = previous.metadata.timestamp - 1;
        assert!(!grouped_message(&previous, &current, true));
        current.metadata.timestamp = previous.metadata.timestamp;
        assert!(grouped_message(&previous, &current, true));
        current.metadata.author_key = Some(Text::new("users:two").unwrap());
        assert!(
            !grouped_message(&previous, &current, true),
            "equal names are not identity"
        );
        current.metadata.author_key = None;
        assert!(!grouped_message(&previous, &current, true));
        let unknown = authored_message(3, None, 32_400);
        assert!(!grouped_message(&unknown, &unknown, true));
        let empty = authored_message(4, Some(""), 32_400);
        assert!(!grouped_message(&empty, &empty, true));
    }

    #[test]
    fn grouping_keeps_dates_replies_threads_system_and_deleted_boundaries() {
        let previous = authored_message(1, Some("users:one"), 86_399);
        let current = authored_message(2, Some("users:one"), 86_400);
        assert!(!grouped_message(&previous, &current, false));
        assert!(
            grouped_message(&previous, &current, true),
            "a UTC midnight is not a local calendar boundary"
        );
        let previous = authored_message(1, Some("users:one"), 32_400);
        let current = authored_message(2, Some("users:one"), 32_460);
        for boundary in 0..7 {
            let mut changed = current.clone();
            match boundary {
                0 => changed.metadata.system = true,
                1 => changed.metadata.deleted = true,
                2 => changed.metadata.thread = Some(5),
                3 => {
                    changed.metadata.parent = Some(talk_core::ui::messaging::Reply {
                        id: 5,
                        author: Text::new("Fixture").unwrap(),
                        text: Text::new("Parent").unwrap(),
                        unavailable: false,
                    })
                }
                4 => changed.reply_count = 1,
                5 => changed.own = true,
                _ => changed.metadata.timestamp = 0,
            }
            assert!(!grouped_message(&previous, &changed, true));
            assert!(
                !grouped_message(&changed, &current, true),
                "both sides preserve a semantic boundary"
            );
        }
    }

    #[test]
    fn receipt_indicators_require_confirmed_own_message_permission_and_common_marker() {
        let mut message = authored_message(10, Some("users:one"), 32_400);
        assert_eq!(delivery_indicator(&message, true, Some(10)), None);
        message.own = true;
        for delivery in [DeliveryState::Received, DeliveryState::ServerAccepted] {
            message.delivery = delivery;
            assert_eq!(
                delivery_indicator(&message, false, Some(10)),
                Some(DeliveryIndicator::Accepted)
            );
            assert_eq!(
                delivery_indicator(&message, true, None),
                Some(DeliveryIndicator::Accepted)
            );
            assert_eq!(
                delivery_indicator(&message, true, Some(9)),
                Some(DeliveryIndicator::Accepted)
            );
            assert_eq!(
                delivery_indicator(&message, true, Some(10)),
                Some(DeliveryIndicator::Read)
            );
            assert_eq!(
                delivery_indicator(&message, true, Some(11)),
                Some(DeliveryIndicator::Read)
            );
        }
        for (delivery, indicator) in [
            (DeliveryState::Pending, DeliveryIndicator::Pending),
            (DeliveryState::Failed, DeliveryIndicator::Failed),
            (DeliveryState::Uncertain, DeliveryIndicator::Uncertain),
        ] {
            message.delivery = delivery;
            assert_eq!(
                delivery_indicator(&message, true, Some(11)),
                Some(indicator)
            );
        }
        message.metadata.deleted = true;
        assert_eq!(delivery_indicator(&message, true, Some(11)), None);
        message.metadata.deleted = false;
        message.metadata.system = true;
        assert_eq!(delivery_indicator(&message, true, Some(11)), None);
    }
}
