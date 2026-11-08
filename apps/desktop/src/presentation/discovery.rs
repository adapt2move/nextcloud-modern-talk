//! Open-room browsing stays responsive while the account owner queries the server.
use super::{
    ButtonStyle, Desktop, LocalAction,
    controls::{Availability, BannerRole},
};
use gpui::{Context, Div, FontWeight, Window, div, prelude::*, px};
use talk_core::ui::{
    Decision, Text,
    conversation::RemoteState,
    discover::Action,
    presentation::{ConversationRef, FeatureSupport, RequestId, UiIntent},
};

impl Desktop {
    fn discovery_intent(&mut self, action: Action) -> bool {
        let Some(account) = self.snapshot.active_account else {
            return false;
        };
        self.discovery_request = self.discovery_request.saturating_add(1);
        self.submit(UiIntent::Discover {
            account,
            request: RequestId(self.discovery_request),
            action,
        })
    }
    pub(super) fn open_discovery(&mut self, cx: &Context<Self>) {
        if self.snapshot.discover_panel.as_ref().is_none_or(|p| {
            !p.loaded && !p.state.pending() && p.support != FeatureSupport::Unsupported
        }) {
            self.search_discovery(cx);
        }
    }
    pub(super) fn search_discovery(&mut self, cx: &Context<Self>) {
        if !self.active_account_connected() {
            return;
        }
        if let Ok(query) = Text::new(self.discovery_search.read(cx).content.trim()) {
            let _ = self.discovery_intent(Action::List(query));
        }
    }
    pub(super) fn join_discovery(
        &mut self,
        context: ConversationRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_account_connected()
            && self.save_draft(cx)
            && self.discovery_intent(Action::Join(context))
        {
            self.discovery_join = Some(context);
            self.overlay = None;
            self.root_focus.focus(window);
        }
    }
    pub(super) fn refresh_discovery(&mut self) {
        let Some(context) = self.discovery_join else {
            return;
        };
        if self.snapshot.discover_panel.as_ref().is_some_and(|p| {
            !p.uncertain && p.state == RemoteState::Idle && p.joining == Some(context)
        }) {
            self.discovery_join = None;
        } else if let Some(panel) = self
            .snapshot
            .discover_panel
            .as_ref()
            .filter(|p| p.account == context.account && p.joining == Some(context))
            && matches!(
                panel.state,
                RemoteState::Failed(..) | RemoteState::Uncertain(..)
            )
        {
            if matches!(
                panel.state,
                RemoteState::Failed(
                    _,
                    talk_core::ui::UnavailableReason::PasswordRequired
                        | talk_core::ui::UnavailableReason::Lobby
                )
            ) {
                self.discovery_join = None;
                return;
            }
            self.feedback = Some(if panel.uncertain {
                super::feedback::Feedback::warning("Der Beitritt wird überprüft.")
            } else {
                super::feedback::Feedback::error(
                    "Das Gespräch konnte nicht geöffnet werden. Bitte versuche es erneut.",
                )
            });
            self.discovery_join = None;
        }
    }
    pub(super) fn discover_body(&self, cx: &Context<Self>) -> Div {
        let panel = self
            .snapshot
            .discover_panel
            .as_ref()
            .filter(|p| Some(p.account) == self.snapshot.active_account);
        let busy = panel.is_some_and(|p| p.state.pending() || p.uncertain);
        let unsupported = panel.is_some_and(|p| p.support == FeatureSupport::Unsupported);
        let raw_query = self.discovery_search.read(cx).content.trim();
        let query = if raw_query.contains("://") {
            String::new()
        } else {
            raw_query.to_lowercase()
        };
        let mut body = div().flex().flex_col().gap_4().child(
            div()
                .flex()
                .gap_2()
                .child(
                    self.input_surface(&self.discovery_search, cx)
                        .flex_1()
                        .min_w_0()
                        .child(self.discovery_search.clone()),
                )
                .child(self.button(
                    "discover-search",
                    "Suchen",
                    ButtonStyle::Primary,
                    self.active_account_connected() && !busy && !unsupported,
                    LocalAction::DiscoverSearch,
                    cx,
                )),
        );
        if !self.active_account_connected() {
            body = body.child(self.banner(
                BannerRole::Warning,
                self.l("Das Verzeichnis benötigt eine Verbindung."),
            ));
        }
        let Some(panel) = panel else {
            return body.child(
                self.collection_skeleton(super::controls::Skeleton::Rows, "discover-initial"),
            );
        };
        if panel.uncertain {
            body = body.child(self.loading_row(self.l("Der Beitritt wird überprüft.")));
        } else if matches!(panel.state, RemoteState::Failed(..)) {
            body = body.child(self.banner(
                BannerRole::Error,
                self.l(if unsupported {
                    "Das Gesprächsverzeichnis ist auf diesem Server nicht verfügbar."
                } else {
                    "Das Gesprächsverzeichnis ist derzeit nicht verfügbar."
                }),
            ));
        } else if panel.state.pending() && !panel.loaded {
            body = body.child(
                self.collection_skeleton(super::controls::Skeleton::Rows, "discover-pending"),
            );
        }
        let mut shown = 0;
        for room in panel.rooms.as_slice().iter().filter(|room| {
            query.is_empty()
                || room.name.as_str().to_lowercase().contains(&query)
                || room
                    .description
                    .as_ref()
                    .is_some_and(|d| d.as_str().to_lowercase().contains(&query))
        }) {
            shown += 1;
            let joining = panel.joining == Some(room.context) && busy;
            let reason = if !self.active_account_connected() {
                "Das Verzeichnis benötigt eine Verbindung."
            } else if room.join != Decision::Allowed {
                "Der Beitritt ist für dieses Gespräch nicht verfügbar."
            } else {
                "Ein Beitritt läuft bereits."
            };
            let mut details = div().flex().flex_col().gap_1().flex_1().min_w_0().child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_ellipsis()
                    .child(if room.name.as_str().trim().is_empty() {
                        self.l("Gespräch über Link").to_owned()
                    } else {
                        room.name.as_str().to_owned()
                    }),
            );
            if let Some(description) = room
                .description
                .as_ref()
                .filter(|d| !d.as_str().trim().is_empty())
            {
                details =
                    details.child(div().text_ellipsis().child(self.hint(description.as_str())));
            }
            if let Some(count) = room.participants {
                details =
                    details.child(self.hint(
                        &self.count(super::localization::Count::Participants, u64::from(count)),
                    ));
            }
            body = body.child(
                self.style_card(
                    self.style_list_row(div(), super::controls::RowSize::Detailed),
                    super::controls::CardStyle::Raised,
                )
                .flex()
                .items_center()
                .gap_3()
                .p_3()
                .child(details)
                .child(
                    self.button(
                        super::controls::scoped_id("discover-join", room.context, 0, 0),
                        if joining {
                            "Beitritt läuft …"
                        } else {
                            "Beitreten"
                        },
                        ButtonStyle::Secondary,
                        Availability::when(
                            self.active_account_connected()
                                && !busy
                                && room.join == Decision::Allowed,
                            reason,
                        ),
                        LocalAction::DiscoverJoin(room.context),
                        cx,
                    )
                    .border_1()
                    .border_color(self.theme.input_border)
                    .rounded(px(8.)),
                ),
            );
        }
        if panel.loaded && shown == 0 && !panel.state.pending() {
            body = body.child(self.empty_row(self.l("Keine offenen Gespräche gefunden.")));
        }
        body
    }
}
