//! Account-wide federated invitations, with actions on scoped opaque IDs.
use super::{
    ButtonStyle, Desktop, LocalAction,
    controls::{Availability, BannerRole},
};
use gpui::{Context, Div, ElementId, FontWeight, div, prelude::*};
use talk_core::ui::presentation::UiIntent;

impl Desktop {
    pub(super) fn federation_body(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_3();
        let Some(view) = self
            .snapshot
            .federation_invitations
            .as_ref()
            .filter(|view| Some(view.scope) == self.snapshot.active_account)
        else {
            return body;
        };
        body = body.child(self.hint(
            self.l("Föderierte Gespräche unterstützen Textnachrichten. Anrufe und weitere Funktionen sind noch nicht verfügbar."),
        ));
        if view.failed {
            body = body.child(self.banner(
                BannerRole::Error,
                self.l("Die Einladung konnte nicht bearbeitet werden."),
            ));
        }
        if view.pending {
            body = body.child(self.loading_row(self.l("Einladung wird überprüft …")));
        } else if view.invitations.as_slice().is_empty() {
            body = body.child(self.empty_row(self.l("Keine offenen Einladungen.")));
        }
        for item in view.invitations.as_slice() {
            let action = |accept| {
                LocalAction::Intent(UiIntent::ChangeFederationInvitation {
                    scope: view.scope,
                    invitation: item.id,
                    accept,
                })
            };
            body = body.child(
                self.style_list_row(div(), super::controls::RowSize::Regular)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .py_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(item.room.as_str().to_owned()),
                            )
                            .child(self.hint(&self.lf(
                                "{name} hat dich eingeladen",
                                &[("name", item.inviter.as_str())],
                            ))),
                    )
                    .child(
                        self.button(
                            ElementId::Name(
                                format!(
                                    "federation-decline-{:032x}-{:032x}-{:032x}",
                                    view.scope.account.as_local(),
                                    view.scope.runtime.as_local(),
                                    item.id
                                )
                                .into(),
                            ),
                            "Ablehnen",
                            ButtonStyle::Quiet,
                            if view.pending {
                                Availability::pending()
                            } else {
                                Availability::available()
                            },
                            action(false),
                            cx,
                        ),
                    )
                    .child(
                        self.button(
                            ElementId::Name(
                                format!(
                                    "federation-accept-{:032x}-{:032x}-{:032x}",
                                    view.scope.account.as_local(),
                                    view.scope.runtime.as_local(),
                                    item.id
                                )
                                .into(),
                            ),
                            "Annehmen",
                            ButtonStyle::Secondary,
                            if view.pending {
                                Availability::pending()
                            } else {
                                Availability::available()
                            },
                            action(true),
                            cx,
                        ),
                    ),
            );
        }
        body
    }
}
