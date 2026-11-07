//! A call remains owned by the call runtime while this window shows other content.
use crate::presentation::{ButtonStyle, Desktop, LocalAction, controls::Availability};
use gpui::{Context, Div, FontWeight, div, prelude::*, px};
use talk_core::ui::{
    UserIntent,
    presentation::{CallStage, MediaState, UiIntent},
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
        for participant in call
            .participants
            .as_slice()
            .iter()
            .filter(|person| person.speaking)
            .chain(
                call.participants
                    .as_slice()
                    .iter()
                    .filter(|person| !person.speaking),
            )
            .take(3)
        {
            avatars = avatars.child(
                div()
                    .rounded_full()
                    .border_2()
                    .border_color(if participant.speaking {
                        t.success
                    } else {
                        t.subtle
                    })
                    .child(self.person_avatar(participant.initials.as_str(), 22.)),
            );
        }
        let media = |state: MediaState| {
            !call.end_pending
                && matches!(
                    state,
                    MediaState::Off | MediaState::Active | MediaState::Requested
                )
        };
        div()
            .flex_none()
            .w_full()
            .h(px(64.))
            .px_3()
            .rounded(px(10.))
            .bg(t.subtle)
            .flex()
            .items_center()
            .gap_2()
            .child(div().size(px(8.)).rounded_full().bg(t.success))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(self.sp(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_ellipsis()
                            .child(format!("{} · {}", call.title.as_str(), status)),
                    )
                    .child(avatars),
            )
            .child(
                self.icon_button(
                    "dock-mic",
                    (
                        if call.microphone == MediaState::Active {
                            "mic"
                        } else {
                            "mic-off"
                        },
                        super::media_label(true, call.microphone),
                    ),
                    ButtonStyle::Quiet,
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
                .when(call.microphone != MediaState::Active, |d| {
                    d.bg(gpui::rgb(0x9e3e32)).text_color(gpui::white())
                }),
            )
            .child(self.icon_button(
                "dock-leave",
                ("phone-down", "Anruf verlassen"),
                ButtonStyle::StageDanger,
                Availability::available().labelled(),
                LocalAction::Intent(UiIntent::Legacy(UserIntent::Leave(call.scope))),
                cx,
            ))
            .child(self.icon_button(
                "dock-expand",
                ("arrow-up-right", "Anruf öffnen"),
                ButtonStyle::Quiet,
                Availability::available().labelled(),
                LocalAction::ExpandCall(call.scope),
                cx,
            ))
    }
}
