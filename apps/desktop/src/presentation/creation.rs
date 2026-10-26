//! Current-account creation: directory selection or an explicit named group.
use super::{
    ButtonStyle, Desktop, LocalAction, Overlay,
    controls::{Availability, Skeleton},
};
use gpui::{Context, Div, div, prelude::*, px};
use talk_core::ui::{
    AccountScope, Decision,
    conversation::{CreateAction, CreatePanel, RemoteState},
    presentation::{Completion, RequestId, UiIntent},
};

pub(super) struct PeopleRequest {
    account: AccountScope,
    request: RequestId,
    ready: bool,
}
impl PeopleRequest {
    fn new(account: AccountScope, request: RequestId) -> Self {
        Self {
            account,
            request,
            ready: false,
        }
    }
    fn observe(&mut self, panel: &CreatePanel) {
        if panel.account == self.account
            && let Some(result) = panel.result
            && result.request == self.request
        {
            self.ready = result.completion == Completion::ServerAccepted;
        }
    }
    fn loading(&self, panel: &CreatePanel) -> bool {
        panel.account == self.account && panel.state == RemoteState::Loading(self.request)
    }
    fn ready(&self, panel: &CreatePanel) -> bool {
        panel.account == self.account && self.ready && panel.people_loaded && !panel.state.pending()
    }
}

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
        self.room_ui.create_people_request = Some(PeopleRequest::new(account, RequestId(request)));
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
            self.room_ui.create_people_request = None;
            self.conversation_name.update(cx, |i, _| i.reset());
            self.room_ui.search.update(cx, |i, _| i.reset());
        }
        self.submit_creation(CreateAction::Refresh);
    }
    pub(super) fn submit_creation(&mut self, action: CreateAction) {
        let Some(account) = self.snapshot.active_account else {
            return;
        };
        if matches!(action, CreateAction::Direct(_))
            && !self.snapshot.create_panel.as_ref().is_some_and(|panel| {
                panel.account == account
                    && self
                        .room_ui
                        .create_people_request
                        .as_ref()
                        .is_some_and(|request| request.ready(panel))
            })
        {
            return;
        }
        let Some(request) = self.room_ui.request.checked_add(1) else {
            return;
        };
        self.room_ui.request = request;
        let people = matches!(action, CreateAction::FindPeople(_));
        if people {
            // A new attempt invalidates prior authority even if admission fails.
            // Confirmed rows remain visible in the account owner's snapshot.
            self.room_ui.create_people_request =
                Some(PeopleRequest::new(account, RequestId(request)));
        }
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
        if let Some(request) = &mut self.room_ui.create_people_request {
            request.observe(panel);
        }
        if let Some(result) = panel.result
            && result.completion == Completion::ServerAccepted
        {
            let account = panel.account;
            if result.conversation.is_some()
                && Some(account) == self.snapshot.active_account
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
        let availability = match panel {
            Some(p) if p.uncertain || matches!(p.state, RemoteState::Uncertain(_)) => {
                Availability::when(
                    false,
                    "Ergebnis unklar. Prüfe deine Gesprächsliste, bevor du erneut ein Gespräch erstellst.",
                )
            }
            Some(p) if matches!(p.state, RemoteState::Loading(_) | RemoteState::Saving(_)) => {
                Availability::pending()
            }
            Some(_) => Availability::available(),
            None => Availability::when(
                false,
                "Die Verfügbarkeit ist nicht bekannt. Bitte neu laden.",
            ),
        };
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
                        .child(
                            self.input_surface(&self.room_ui.search, cx)
                                .flex_1()
                                .min_w_0()
                                .child(self.room_ui.search.clone()),
                        )
                        .child(self.button(
                            "create-find-people",
                            "Suchen",
                            ButtonStyle::Secondary,
                            availability,
                            LocalAction::FindPeople,
                            cx,
                        )),
                );
            if let Some(panel) = panel {
                if !panel.people_loaded
                    && panel.people.as_slice().is_empty()
                    && self
                        .room_ui
                        .create_people_request
                        .as_ref()
                        .is_some_and(|request| request.loading(panel))
                {
                    body = body.child(self.collection_skeleton(Skeleton::Rows));
                }
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
                                if !availability.is_enabled() {
                                    availability
                                } else if self.room_ui.create_people_request.as_ref().is_some_and(|request| request.ready(panel)) {
                                    Availability::available()
                                } else {
                                    Availability::when(false, "Die Vorschau ist nicht bestätigt. Bitte erneut suchen.")
                                },
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
                    self.input_surface(&self.conversation_name, cx)
                        .min_h(px(48.))
                        .child(self.conversation_name.clone()),
                );
            if panel.is_none_or(|p| {
                matches!(p.group, Decision::Denied | Decision::Stale)
                    || (p.group != Decision::Allowed
                        && !matches!(p.state, RemoteState::Loading(_) | RemoteState::Saving(_)))
            }) {
                body = body.child(self.hint(self.l(
                    if panel.is_some_and(|p| p.group == Decision::Denied) {
                        "Du darfst auf diesem Server keine Gruppengespräche erstellen."
                    } else if panel.is_some_and(|p| p.group == Decision::Stale) {
                        "Berechtigungen müssen erneut geprüft werden."
                    } else {
                        "Die Berechtigung zum Erstellen ist nicht bekannt. Bitte neu laden."
                    },
                )));
            }
        }
        if let Some(panel) = panel {
            let status = match panel.state {
                RemoteState::Failed(_, _) => Some(
                    "Das Gespräch konnte nicht geöffnet werden. Bitte neu laden und erneut versuchen.",
                ),
                RemoteState::Uncertain(_) => Some(
                    "Ergebnis unklar. Prüfe deine Gesprächsliste, bevor du erneut ein Gespräch erstellst.",
                ),
                RemoteState::Idle | RemoteState::Loading(_) | RemoteState::Saving(_) => None,
            };
            if let Some(status) = status {
                body = body.child(self.hint(self.l(status)));
            }
        }
        if panel.is_none_or(|p| {
            matches!(p.state, RemoteState::Failed(..))
                || (!p.state.pending() && !direct && p.group != Decision::Allowed)
        }) {
            body = body.child(self.button(
                "creation-reload",
                "Erneut laden",
                ButtonStyle::Secondary,
                true,
                LocalAction::Open(Overlay::Create),
                cx,
            ));
        }
        body
    }
    pub(super) fn creation_footer(&self, cx: &Context<Self>) -> Div {
        let panel = self
            .snapshot
            .create_panel
            .as_ref()
            .filter(|p| Some(p.account) == self.snapshot.active_account);
        let enabled = panel.is_some_and(|p| {
            Some(p.account) == self.snapshot.active_account
                && p.group == Decision::Allowed
                && !p.state.pending()
                && !p.uncertain
        }) && !self.conversation_name.read(cx).content.trim().is_empty();
        let availability = match panel {
            Some(p) if p.uncertain || matches!(p.state, RemoteState::Uncertain(_)) => {
                Availability::when(
                    false,
                    "Ergebnis unklar. Prüfe deine Gesprächsliste, bevor du erneut ein Gespräch erstellst.",
                )
            }
            Some(p) if p.group == Decision::Denied => Availability::when(
                false,
                "Du darfst auf diesem Server keine Gruppengespräche erstellen.",
            ),
            Some(p) if p.group == Decision::Stale => {
                Availability::when(false, "Berechtigungen müssen erneut geprüft werden.")
            }
            Some(p) if matches!(p.state, RemoteState::Loading(_) | RemoteState::Saving(_)) => {
                Availability::pending()
            }
            Some(p) if p.group == Decision::Allowed => {
                Availability::when(enabled, "Gib einen Namen für das Gespräch ein.")
            }
            _ => Availability::when(
                false,
                "Die Berechtigung zum Erstellen ist nicht bekannt. Bitte neu laden.",
            ),
        };
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
                        availability,
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

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, RuntimeGeneration, UnavailableReason};

    #[test]
    fn directory_skeleton_requires_its_admitted_search_not_a_permission_probe() {
        let account = AccountScope {
            account: AccountId::from_local(1),
            runtime: RuntimeGeneration::from_local(1),
        };
        let request = RequestId(7);
        let mut panel = CreatePanel::new(account);
        let requested = PeopleRequest::new(account, request);
        panel.state = RemoteState::Loading(request);
        assert!(requested.loading(&panel));
        assert!(!PeopleRequest::new(account, RequestId(8)).loading(&panel));
        for state in [
            RemoteState::Idle,
            RemoteState::Loading(RequestId(8)),
            RemoteState::Saving(request),
            RemoteState::Failed(request, UnavailableReason::PermissionDenied),
            RemoteState::Uncertain(request),
        ] {
            panel.state = state;
            assert!(!requested.loading(&panel));
        }
        panel.state = RemoteState::Loading(request);
        panel.account.runtime = RuntimeGeneration::from_local(2);
        assert!(!requested.loading(&panel));
    }

    #[test]
    fn retained_people_require_the_latest_search_success_in_the_same_account_runtime() {
        use talk_core::ui::conversation::CreationResult;
        let account = AccountScope {
            account: AccountId::from_local(1),
            runtime: RuntimeGeneration::from_local(1),
        };
        let request = RequestId(7);
        let mut panel = CreatePanel::new(account);
        let mut pending = PeopleRequest::new(account, request);
        assert!(!pending.ready(&panel));
        panel.result = Some(CreationResult {
            request,
            completion: Completion::ServerAccepted,
            conversation: None,
        });
        panel.people_loaded = true;
        pending.observe(&panel);
        assert!(pending.ready(&panel));
        panel.result = None;
        assert!(
            pending.ready(&panel),
            "acknowledgement preserves confirmed search authority"
        );
        pending = PeopleRequest::new(account, RequestId(8));
        assert!(
            !pending.ready(&panel),
            "a rejected new attempt cannot reuse old authority"
        );
        panel.result = Some(CreationResult {
            request,
            completion: Completion::ServerAccepted,
            conversation: None,
        });
        pending.observe(&panel);
        assert!(
            !pending.ready(&panel),
            "old success cannot authorize the new search"
        );
        panel.result.as_mut().unwrap().request = RequestId(8);
        for completion in [
            Completion::Failed(UnavailableReason::PermissionDenied),
            Completion::Uncertain,
        ] {
            panel.result.as_mut().unwrap().completion = completion;
            pending.observe(&panel);
            assert!(!pending.ready(&panel));
        }
        panel.result.as_mut().unwrap().completion = Completion::ServerAccepted;
        panel.account.runtime = RuntimeGeneration::from_local(2);
        pending.observe(&panel);
        assert!(!pending.ready(&panel));
        panel.account = account;
        pending.observe(&panel);
        assert!(pending.ready(&panel));
        panel.state = RemoteState::Loading(RequestId(9));
        assert!(!pending.ready(&panel));
    }
}
