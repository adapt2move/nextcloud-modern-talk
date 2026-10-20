//! Current-account creation: directory selection or an explicit named group.
use super::{ButtonStyle, Desktop, LocalAction, Overlay, controls::Availability};
use gpui::{Context, Div, Focusable, div, prelude::*, px};
use talk_core::ui::{
    Decision,
    conversation::{CreateAction, RemoteState},
    presentation::{Completion, RequestId, UiIntent},
};
impl Desktop {
    pub(super) fn find_search_people(&mut self, cx: &Context<Self>) {
        let Some(account) = self.snapshot.active_account else {
            return;
        };
        let query = self.search.read(cx).content.trim();
        let Ok(bounded) = talk_core::ui::Text::new(query) else {
            return;
        };
        if query.is_empty()
            || self
                .snapshot
                .create_panel
                .as_ref()
                .is_some_and(|p| p.state.pending())
        {
            return;
        }
        let requested = query.to_lowercase();
        let Some(request) = self.room_ui.request.checked_add(1) else {
            return;
        };
        self.room_ui.request = request;
        if self.submit(UiIntent::Create {
            account,
            request: RequestId(request),
            action: CreateAction::FindPeople(bounded),
        }) {
            self.directory_query = Some((account, requested));
        }
    }
    pub(super) fn open_creation(&mut self, cx: &mut Context<Self>) {
        if self.room_ui.create_account != self.snapshot.active_account {
            self.room_ui.create_account = self.snapshot.active_account;
            self.conversation_name.update(cx, |i, _| i.reset());
            self.room_ui.search.update(cx, |i, _| i.reset());
        }
        self.submit_creation(CreateAction::Refresh);
    }
    pub(super) fn submit_creation(&mut self, action: CreateAction) {
        let Some(account) = self.snapshot.active_account else {
            return;
        };
        let Some(request) = self.room_ui.request.checked_add(1) else {
            return;
        };
        self.room_ui.request = request;
        self.submit(UiIntent::Create {
            account,
            request: RequestId(request),
            action,
        });
    }
    pub(super) fn refresh_creation(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = &self.snapshot.create_panel else {
            return;
        };
        if let Some(result) = panel.result
            && result.completion == Completion::ServerAccepted
        {
            let account = panel.account;
            if Some(account) == self.snapshot.active_account
                && matches!(self.overlay, Some(Overlay::Create | Overlay::Search))
            {
                self.overlay = None;
                self.conversation_name.update(cx, |i, _| i.reset());
                self.room_ui.search.update(cx, |i, _| i.reset());
            }
            self.submit(UiIntent::AcknowledgeCreation {
                account,
                request: result.request,
            });
        }
    }
    pub(super) fn creation_body(&self, cx: &Context<Self>) -> Div {
        let direct = self.room_ui.create_direct;
        let panel = self
            .snapshot
            .create_panel
            .as_ref()
            .filter(|p| Some(p.account) == self.snapshot.active_account);
        let pending = panel.is_none_or(|p| p.state.pending() || p.uncertain);
        let mut body = div().flex().flex_col().gap_4().child(
            div()
                .flex()
                .gap_2()
                .child(self.button_with_icon(
                    "create-direct-mode",
                    ("Mit einer Person sprechen", "user"),
                    if direct {
                        ButtonStyle::Primary
                    } else {
                        ButtonStyle::Secondary
                    },
                    true,
                    LocalAction::CreateMode(true),
                    cx,
                ))
                .child(self.button_with_icon(
                    "create-group-mode",
                    ("Gruppengespräch", "users"),
                    if direct {
                        ButtonStyle::Secondary
                    } else {
                        ButtonStyle::Primary
                    },
                    true,
                    LocalAction::CreateMode(false),
                    cx,
                )),
        );
        if direct {
            body = body
                .child(
                    self.hint(
                        self.l("Suche im freigegebenen Verzeichnis und wähle eine Person aus."),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(self.room_ui.search.clone())
                        .child(self.button(
                            "create-find-people",
                            "Suchen",
                            ButtonStyle::Secondary,
                            !pending,
                            LocalAction::FindPeople,
                            cx,
                        )),
                );
            if let Some(panel) = panel {
                for person in panel.people.as_slice() {
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .py_2()
                            .child(self.content_avatar(
                                talk_core::ui::content::AssetTarget::PersonAvatar(person.id),
                                "?",
                                32.,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .child(person.name.as_str().to_owned())
                                    .child(self.hint(person.label.as_str())),
                            )
                            .child(self.button(
                                ("create-person", person.id.local as usize),
                                "Gespräch öffnen",
                                ButtonStyle::Secondary,
                                !pending,
                                LocalAction::CreateDirect(person.id),
                                cx,
                            )),
                    );
                }
                if panel.people_loaded && panel.people.as_slice().is_empty() {
                    body = body.child(self.hint(self.l("Keine passenden Personen gefunden.")));
                }
            }
        } else {
            body = body
                .child(self.hint(
                    self.l("Erstelle ein Gruppengespräch. Danach kannst du Personen einladen."),
                ))
                .child(self.title(self.l("Name des Gesprächs")))
                .child(
                    div()
                        .relative()
                        .min_h(px(48.))
                        .px_3()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(self.theme.input_border)
                        .child(self.field_ring(self.conversation_name.focus_handle(cx), 8.))
                        .child(self.conversation_name.clone()),
                );
            if panel.is_none_or(|p| p.group != Decision::Allowed) {
                body = body.child(self.hint(self.l(
                    if panel.is_some_and(|p| p.group == Decision::Denied) {
                        "Du darfst auf diesem Server keine Gruppengespräche erstellen."
                    } else {
                        "Die Berechtigung zum Erstellen wird geprüft."
                    },
                )));
            }
        }
        if let Some(panel) = panel {
            let status = match panel.state {
                RemoteState::Loading(_) => Some("Verfügbarkeit wird geprüft …"),
                RemoteState::Saving(_) => Some("Gespräch wird erstellt …"),
                RemoteState::Failed(_, _) => Some(
                    "Das Gespräch konnte nicht geöffnet werden. Bitte neu laden und erneut versuchen.",
                ),
                RemoteState::Uncertain(_) => Some(
                    "Ergebnis unklar. Prüfe deine Gesprächsliste, bevor du erneut ein Gespräch erstellst.",
                ),
                RemoteState::Idle => None,
            };
            if let Some(status) = status {
                body = body.child(self.hint(self.l(status)));
            }
        }
        body
    }
    pub(super) fn creation_footer(&self, cx: &Context<Self>) -> Div {
        let panel = self.snapshot.create_panel.as_ref();
        let enabled = panel.is_some_and(|p| {
            Some(p.account) == self.snapshot.active_account
                && p.group == Decision::Allowed
                && !p.state.pending()
                && !p.uncertain
        }) && !self.conversation_name.read(cx).content.trim().is_empty();
        div()
            .flex_none()
            .px_6()
            .py_4()
            .border_t_1()
            .border_color(self.theme.border)
            .when(!self.room_ui.create_direct, |d| {
                d.child(
                    self.button_with_icon(
                        "create-submit",
                        ("Gespräch erstellen", "plus"),
                        ButtonStyle::Primary,
                        Availability::when(
                            enabled,
                            "Gib einen Namen ein und warte auf die bestätigte Berechtigung.",
                        ),
                        LocalAction::CreateConversation,
                        cx,
                    )
                    .w_full(),
                )
            })
            .child(
                self.hint(self.l("Weitere Vorlagen sind in diesem Client noch nicht verfügbar.")),
            )
    }
}
