//! A call remains owned by the call runtime while this window shows other content.
use crate::presentation::{ButtonStyle, Desktop, LocalAction, controls::Availability};
use gpui::{Context, Div, FontWeight, div, prelude::*, px};
use talk_core::ui::{
    UserIntent,
    content::AssetTarget,
    presentation::{CallStage, CallView, ConversationRef, MediaState, ParticipantView, UiIntent},
};

impl Desktop {
    pub(in crate::presentation) fn docked_call(&self) -> bool {
        self.snapshot.call.as_ref().is_some_and(|call| {
            self.call_minimized == Some(call.scope)
                && matches!(
                    call.stage,
                    CallStage::Outgoing
                        | CallStage::Connecting
                        | CallStage::Active
                        | CallStage::Reconnecting
                        | CallStage::Required(_)
                )
        })
    }

    pub(in crate::presentation) fn call_dock(&self, cx: &Context<Self>) -> Div {
        let Some(call) = self.snapshot.call.as_ref().filter(|_| self.docked_call()) else {
            return div();
        };
        let t = self.theme;
        let status = match call.stage {
            CallStage::Active => self
                .call_started
                .filter(|(scope, _)| *scope == call.scope)
                .map(|(_, started)| {
                    let seconds = started.elapsed().as_secs();
                    format!("{}:{:02}", seconds / 60, seconds % 60)
                })
                .unwrap_or_else(|| self.l("Im Gespräch").to_owned()),
            CallStage::Reconnecting => self.l("Verbindung unterbrochen").to_owned(),
            CallStage::Required(_) => self.l("Beitritt wartet auf Freigabe").to_owned(),
            _ => self.l("Verbindung wird aufgebaut …").to_owned(),
        };
        let mut avatars = div().flex().items_center().gap_1();
        // Whoever is heard comes first. Your own voice shows at the microphone,
        // so it neither rings nor moves your picture.
        let heard = |person: &ParticipantView| person.speaking && !person.self_user;
        for participant in call
            .participants
            .as_slice()
            .iter()
            .filter(|person| heard(person))
            .chain(
                call.participants
                    .as_slice()
                    .iter()
                    .filter(|person| !heard(person)),
            )
            .take(3)
        {
            let level = if heard(participant) {
                participant.speaking_level.max(1)
            } else {
                0
            };
            avatars = avatars.child(
                div()
                    .relative()
                    .p(px(2.))
                    .child(self.call_avatar(call, participant, 22.))
                    // The picture is a rounded square; the ring keeps its shape.
                    .children(
                        speaking_ring_overlay(level, speaking_ring(t), Some(2.))
                            .map(|ring| ring.rounded(px(6. + f32::from(level.min(4))))),
                    ),
            );
        }
        let media = |state: MediaState| {
            !call.end_pending
                && matches!(
                    state,
                    MediaState::Off | MediaState::Active | MediaState::Requested
                )
        };
        // A calm indicator that you are being heard: a ring around your own
        // microphone while you speak. Nothing moves when you do not.
        let mic_on = call.microphone == MediaState::Active;
        let own_level = call
            .participants
            .as_slice()
            .iter()
            .find(|person| mic_on && person.self_user)
            .map_or(0, |person| person.speaking_level);
        self.style_floating_surface(
            div().relative(),
            t.subtle,
            t.dark,
            10.,
            crate::presentation::theme::Elevation::Raised,
        )
        .flex_none()
        .w_full()
        .px_3()
        .py_2()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .relative()
                .flex_none()
                .child(
                    self.icon_button(
                        "dock-mic",
                        (
                            if mic_on { "mic" } else { "mic-off" },
                            super::media_label(true, call.microphone),
                        ),
                        if mic_on {
                            ButtonStyle::Quiet
                        } else {
                            ButtonStyle::Secondary
                        },
                        Availability::when(
                            media(call.microphone),
                            super::media_reason(call.microphone),
                        )
                        .labelled(),
                        LocalAction::Intent(UiIntent::SetMicrophone {
                            call: call.scope,
                            enabled: !matches!(
                                call.microphone,
                                MediaState::Active | MediaState::Requested
                            ),
                        }),
                        cx,
                    )
                    .rounded_full(),
                )
                // The ring grows with the voice and is gone when nothing is heard.
                .children(speaking_ring_overlay(own_level, speaking_ring(t), Some(0.))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div().flex().child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(self.sp(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_ellipsis()
                            .child(call.title.as_str().to_owned()),
                    ),
                )
                .child(
                    div().flex().items_center().gap_2().child(avatars).child(
                        div()
                            .flex_none()
                            .text_size(self.sp(12.))
                            .text_color(t.muted)
                            .child(status),
                    ),
                ),
        )
        .child(self.icon_button(
            "dock-expand",
            ("arrow-up-right", "Anruf öffnen"),
            ButtonStyle::Quiet,
            Availability::available().labelled(),
            LocalAction::ExpandCall(call.scope),
            cx,
        ))
        .child(self.icon_button(
            "dock-leave",
            ("phone-down", "Anruf verlassen"),
            ButtonStyle::StageDanger,
            Availability::available().labelled(),
            LocalAction::Intent(UiIntent::Legacy(UserIntent::Leave(call.scope))),
            cx,
        ))
    }
}

impl Desktop {
    /// A call participant's picture from the account's avatar store, where the
    /// call establishes who it is: oneself, and the other person of a one-to-one
    /// call. A group call names its participants without a store identity, so
    /// they keep their initials.
    pub(in crate::presentation) fn call_avatar(
        &self,
        call: &CallView,
        participant: &ParticipantView,
        size: f32,
    ) -> Div {
        let target = if self.snapshot.active_account != Some(call.scope.account) {
            None
        } else if participant.self_user {
            Some(AssetTarget::SelfAvatar)
        } else if call.direct {
            Some(AssetTarget::ConversationAvatar(ConversationRef {
                account: call.scope.account,
                conversation: call.scope.conversation,
            }))
        } else {
            None
        };
        match target {
            Some(target) => self.content_avatar(target, participant.initials.as_str(), size),
            None => self.person_avatar(participant.initials.as_str(), size),
        }
    }
}

/// A ring in its relative parent whose weight follows the loudness step:
/// nothing while silent, one to four pixels while speaking. With `around` it
/// hugs content that lies that many pixels inside the parent and grows outwards;
/// without, it lies over the parent's edge and grows inwards. It never takes
/// part in layout, so a changing voice moves nothing.
pub(in crate::presentation) fn speaking_ring_overlay(
    level: u8,
    color: gpui::Hsla,
    around: Option<f32>,
) -> Option<Div> {
    let inset = around.map_or(0., |gap| gap - f32::from(level.min(4)));
    let ring = div()
        .absolute()
        .top(px(inset))
        .left(px(inset))
        .right(px(inset))
        .bottom(px(inset))
        .rounded_full()
        .border_color(color);
    Some(match level {
        0 => return None,
        1 => ring.border_1(),
        2 => ring.border_2(),
        3 => ring.border_3(),
        _ => ring.border_4(),
    })
}

/// Ring for "someone is speaking" on the dock's light or dark surface.
fn speaking_ring(theme: crate::presentation::Theme) -> gpui::Hsla {
    theme.success
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::theme::{Appearance, Palette, Theme};
    fn luminance(color: gpui::Hsla) -> f32 {
        let c = gpui::Rgba::from(color);
        let linear = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
    }
    fn contrast(a: gpui::Hsla, b: gpui::Hsla) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn the_dock_keeps_text_and_the_speaking_ring_readable_in_every_theme() {
        for palette in [Palette::Blue, Palette::Sage] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::resolve(palette, appearance, gpui::WindowAppearance::Light);
                let label = format!("{palette:?}/{appearance:?}");
                assert!(contrast(t.text, t.subtle) >= 4.5, "{label} title");
                assert!(contrast(t.muted, t.subtle) >= 4.5, "{label} timer");
                assert!(
                    contrast(speaking_ring(t), t.subtle) >= 3.,
                    "{label} speaking ring"
                );
            }
        }
    }
}
