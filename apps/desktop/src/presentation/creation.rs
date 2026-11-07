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
    fn is_request(&self, account: AccountScope, request: RequestId) -> bool {
        self.account == account && self.request == request
    }
    pub(super) fn ready(&self, panel: &CreatePanel) -> bool {
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
            self.switcher_request_admitted();
        }
    }
    pub(super) fn open_creation(&mut self, cx: &mut Context<Self>) {
        if self.room_ui.create_account != self.snapshot.active_account {
            self.room_ui.create_account = self.snapshot.active_account;
            self.room_ui.create_people_request = None;
            self.room_ui.create_public = false;
            self.room_ui.create_preset = None;
            self.conversation_name.update(cx, |i, _| i.reset());
            self.room_ui.search.update(cx, |i, _| i.reset());
            self.room_ui.description.update(cx, |i, _| i.reset());
        }
        self.submit_creation(CreateAction::Refresh);
    }
    pub(super) fn submit_creation(&mut self, action: CreateAction) {
        let Some(account) = self.snapshot.active_account else {
            return;
        };
        if matches!(action, CreateAction::Direct(_) | CreateAction::AddInvite(_))
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
        let failure = match panel.state {
            RemoteState::Failed(request, reason)
                if self
                    .room_ui
                    .create_people_request
                    .as_ref()
                    .is_some_and(|people| people.is_request(panel.account, request)) =>
            {
                Some((request, reason))
            }
            _ => None,
        };
        if let Some(result) = panel.result
            && result.completion == Completion::ServerAccepted
        {
            let account = panel.account;
            if let Some(context) = result.conversation
                && Some(account) == self.snapshot.active_account
                && matches!(self.overlay, Some(Overlay::Create | Overlay::Search))
            {
                self.overlay = None;
                self.conversation_name.update(cx, |i, _| i.reset());
                self.room_ui.search.update(cx, |i, _| i.reset());
                self.room_ui.description.update(cx, |i, _| i.reset());
                self.room_ui.create_preset = None;
                self.room_ui.create_public = false;
                if result.incomplete_invitations {
                    self.feedback = Some(super::feedback::Feedback::warning("Das Gespräch wurde erstellt, aber einige Personen konnten nicht eingeladen werden.")
                        .in_conversation(context));
                }
            }
            self.submit(UiIntent::AcknowledgeCreation {
                account,
                request: result.request,
            });
        }
        if let Some((request, reason)) = failure {
            self.switcher_directory_failed(request, reason);
        }
        self.schedule_switcher_directory(cx);
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
            use talk_core::ui::presentation::FeatureSupport;
            body = body
                .child(div().flex().gap_2()
                    .child(self.button("create-private", "Privates Gespräch", if !self.room_ui.create_public { ButtonStyle::Primary } else { ButtonStyle::Secondary },
                        Availability::when(panel.is_some_and(|p| p.private_allowed), "Die Servervorgaben erlauben diese Sichtbarkeit nicht."), LocalAction::CreatePublic(false), cx))
                    .child(self.button("create-public", "Offenes Gespräch", if self.room_ui.create_public { ButtonStyle::Primary } else { ButtonStyle::Secondary },
                        Availability::when(panel.is_some_and(|p| p.public == FeatureSupport::Supported && p.public_allowed), "Offene Gespräche sind auf diesem Server nicht verfügbar."), LocalAction::CreatePublic(true), cx)))
                .child(self.hint(self.l(if self.room_ui.create_public { "Auf dem Server auffindbar; Personen mit Link können je nach Servereinstellung beitreten." } else { "Nur eingeladene Personen können dieses Gespräch finden." })))
                .child(self.title(self.l("Name des Gesprächs")))
                .child(
                    self.input_surface(&self.conversation_name, cx)
                        .min_h(px(48.))
                        .child(self.conversation_name.clone()),
                );
            if let Some(panel) = panel.filter(|p| p.extended == FeatureSupport::Supported) {
                body = body
                    .child(self.title(self.l("Beschreibung (optional)")))
                    .child(
                        self.input_surface(&self.room_ui.description, cx)
                            .min_h(px(48.))
                            .child(self.room_ui.description.clone()),
                    );
                if !panel.presets.as_slice().is_empty() {
                    let mut presets = div().flex().flex_wrap().gap_2().child(self.button(
                        "create-preset-default",
                        "Standard",
                        if self.room_ui.create_preset.is_none() {
                            ButtonStyle::Primary
                        } else {
                            ButtonStyle::Secondary
                        },
                        availability,
                        LocalAction::CreatePreset(None),
                        cx,
                    ));
                    for (index, preset) in panel.presets.as_slice().iter().enumerate() {
                        presets = presets.child(self.button(
                            ("create-preset", index),
                            preset.name.as_str().to_owned(),
                            if self.room_ui.create_preset.as_ref() == Some(&preset.id) {
                                ButtonStyle::Primary
                            } else {
                                ButtonStyle::Secondary
                            },
                            availability,
                            LocalAction::CreatePreset(Some(preset.id.clone())),
                            cx,
                        ));
                    }
                    body = body.child(self.title(self.l("Vorlage"))).child(presets);
                    if let Some(preset) = panel
                        .presets
                        .as_slice()
                        .iter()
                        .find(|p| self.room_ui.create_preset.as_ref() == Some(&p.id))
                    {
                        body = body.child(self.hint(preset.description.as_str()));
                    }
                }
                body = body.child(self.title(self.l("Personen einladen (optional)")));
                if !panel.invites.as_slice().is_empty() {
                    let mut invited = div().flex().flex_wrap().gap_2();
                    for person in panel.invites.as_slice() {
                        invited = invited.child(self.button(
                            ("create-invite-remove", person.id.local as usize),
                            format!("{} ×", person.name.as_str()),
                            ButtonStyle::Secondary,
                            availability,
                            LocalAction::RemoveCreationInvite(person.id),
                            cx,
                        ));
                    }
                    body = body.child(invited);
                }
                body = body.child(
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
                            "create-invite-search",
                            "Suchen",
                            ButtonStyle::Secondary,
                            availability,
                            LocalAction::FindPeople,
                            cx,
                        )),
                );
                if self
                    .room_ui
                    .create_people_request
                    .as_ref()
                    .is_some_and(|r| r.loading(panel))
                    && panel.people.as_slice().is_empty()
                {
                    body = body.child(self.collection_skeleton(Skeleton::Rows));
                }
                for person in panel.people.as_slice().iter().take(8) {
                    let selected = panel.invites.as_slice().iter().any(|p| p.id == person.id);
                    let can_add = !selected
                        && panel.invites.as_slice().len() < 20
                        && self
                            .room_ui
                            .create_people_request
                            .as_ref()
                            .is_some_and(|r| r.ready(panel));
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .py_1()
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
                                ("create-invite-add", person.id.local as usize),
                                if selected {
                                    "Hinzugefügt"
                                } else {
                                    "Hinzufügen"
                                },
                                ButtonStyle::Secondary,
                                Availability::when(
                                    availability.is_enabled() && can_add,
                                    "Suche erneut nach dieser Person.",
                                ),
                                LocalAction::AddCreationInvite(person.id),
                                cx,
                            )),
                    );
                }
                if panel.people_loaded && panel.people.as_slice().is_empty() {
                    body = body.child(self.hint(self.l("Keine passenden Personen gefunden.")));
                }
            } else {
                body = body.child(self.hint(self.l("Beschreibung und Einladungen sind beim Erstellen auf diesem Server nicht verfügbar.")));
            }
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
                && if self.room_ui.create_public {
                    p.public == talk_core::ui::presentation::FeatureSupport::Supported
                        && p.public_allowed
                } else {
                    p.private_allowed
                }
                && self
                    .room_ui
                    .create_preset
                    .as_ref()
                    .is_none_or(|id| p.presets.as_slice().iter().any(|p| &p.id == id))
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
            incomplete_invitations: false,
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
            incomplete_invitations: false,
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
