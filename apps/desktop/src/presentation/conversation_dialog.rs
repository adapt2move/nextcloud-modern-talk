//! Native room dialogs keep their originating context and submit typed, correlated intents.
use super::{ButtonStyle, Desktop, LocalAction, Overlay, controls::Availability, input::TextInput};
use gpui::{Context, Div, Entity, Focusable, FontWeight, Window, div, prelude::*, px};
use talk_core::ui::conversation::{
    self, ConversationAction, DirectorySource, MemberRole, RemoteState, RoomSetting,
};
use talk_core::ui::presentation::{
    ConversationKind, ConversationRef, FeatureSupport, RequestId, UiIntent,
};
use talk_core::ui::{Decision, Text};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Tab {
    General,
    Notifications,
    Privacy,
    Permissions,
    Meeting,
}
impl Tab {
    const ALL: [Self; 5] = [
        Self::General,
        Self::Notifications,
        Self::Privacy,
        Self::Permissions,
        Self::Meeting,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::General => "Allgemein",
            Self::Notifications => "Benachrichtigungen",
            Self::Privacy => "Datenschutz",
            Self::Permissions => "Berechtigungen",
            Self::Meeting => "Meeting",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Self::General => "settings",
            Self::Notifications => "bell",
            Self::Privacy => "shield",
            Self::Permissions => "lock",
            Self::Meeting => "video",
        }
    }
}
#[derive(Clone)]
pub(crate) enum Action {
    Tab(Tab),
    People(bool),
    Command(ConversationAction),
    SaveName,
    SaveDescription,
    Search,
    CsvPreview,
    ShowCsv,
    Confirm(ConversationAction),
    SubmitConfirmation,
    CancelConfirmation,
    Avatar,
    Password,
    CopyGuestLink,
}
pub(super) struct RoomUi {
    pub create_direct: bool,
    pub create_account: Option<talk_core::ui::AccountScope>,
    pub context: Option<ConversationRef>,
    tab: Tab,
    bots: bool,
    loaded: bool,
    pub request: u64,
    confirmation: Option<ConversationAction>,
    feedback: Option<RequestId>,
    csv_open: bool,
    pub name: Entity<TextInput>,
    pub description: Entity<TextInput>,
    pub search: Entity<TextInput>,
    pub csv: Entity<TextInput>,
    pub avatar: Entity<TextInput>,
    pub password: Entity<TextInput>,
}
impl RoomUi {
    pub fn new(theme: super::theme::Theme, cx: &mut Context<Desktop>) -> Self {
        let name = cx.new(|cx| TextInput::new("Name des Gesprächs", 256, theme, cx));
        let description = cx.new(|cx| {
            let mut input = TextInput::new("Beschreibung", 4000, theme, cx);
            input.multiline = true;
            input
        });
        let search = cx.new(|cx| TextInput::new("Personen suchen …", 256, theme, cx));
        let csv = cx.new(|cx| {
            let mut input = TextInput::new("E-Mail-Adressen, eine pro Zeile", 65536, theme, cx);
            input.multiline = true;
            input
        });
        let avatar = cx.new(|cx| TextInput::new("Emoji", 64, theme, cx));
        let password = cx.new(|cx| TextInput::new("Neues Passwort", 512, theme, cx));
        Self {
            create_direct: true,
            create_account: None,
            context: None,
            tab: Tab::General,
            bots: false,
            loaded: false,
            request: 0,
            confirmation: None,
            feedback: None,
            csv_open: false,
            name,
            description,
            search,
            csv,
            avatar,
            password,
        }
    }
    pub fn inputs(&self) -> [Entity<TextInput>; 6] {
        [
            self.name.clone(),
            self.description.clone(),
            self.search.clone(),
            self.csv.clone(),
            self.avatar.clone(),
            self.password.clone(),
        ]
    }
}
pub(super) fn unavailable(value: conversation::Availability) -> &'static str {
    if !value.implemented {
        "In diesem Client noch nicht verfügbar."
    } else if value.server == FeatureSupport::Unsupported {
        "Der Server unterstützt diese Funktion nicht."
    } else {
        match value.allowed {
            Decision::Denied => "Du hast dafür keine Berechtigung.",
            Decision::Stale => "Berechtigungen müssen erneut geprüft werden.",
            _ => "Verfügbarkeit wird geprüft …",
        }
    }
}
impl Desktop {
    pub(super) fn open_room_dialog(&mut self, overlay: Overlay, cx: &mut Context<Self>) {
        let Some(context) = self.snapshot.selected else {
            return;
        };
        self.dismiss_room_feedback();
        self.room_ui.confirmation = None;
        if self.room_ui.context != Some(context) {
            self.room_ui.context = Some(context);
            self.room_ui.loaded = false;
            self.room_ui.confirmation = None;
            self.room_ui.csv_open = false;
            for input in self.room_ui.inputs() {
                input.update(cx, |i, _| i.reset());
            }
        }
        self.refresh_room_dialog(cx);
        let command = if overlay == Overlay::Participants {
            ConversationAction::Members
        } else {
            ConversationAction::Read
        };
        self.submit_room(command);
    }
    pub(super) fn refresh_room_dialog(&mut self, cx: &mut Context<Self>) {
        if self.room_ui.context != self.snapshot.selected {
            self.room_ui.context = None;
            self.room_ui.loaded = false;
            self.room_ui.confirmation = None;
            if matches!(
                self.overlay,
                Some(Overlay::RoomInfo | Overlay::Participants)
            ) {
                self.overlay = None;
            }
            for input in self.room_ui.inputs() {
                input.update(cx, |i, _| i.reset());
            }
            return;
        }
        if !self.room_ui.loaded
            && let Some(details) = self
                .snapshot
                .conversation_panel
                .as_ref()
                .and_then(|p| p.details.as_ref())
        {
            self.room_ui
                .name
                .update(cx, |i, _| i.set_context_value(details.name.as_str()));
            self.room_ui.description.update(cx, |i, _| {
                i.set_context_value(details.description.as_ref().map_or("", |v| v.as_str()))
            });
            self.room_ui.loaded = true;
        }
    }
    fn submit_room(&mut self, action: ConversationAction) {
        let Some(context) = self
            .room_ui
            .context
            .filter(|c| Some(*c) == self.snapshot.selected)
        else {
            return;
        };
        let Some(next) = self.room_ui.request.checked_add(1) else {
            return;
        };
        self.room_ui.request = next;
        self.room_ui.feedback = (!action.reads()).then_some(RequestId(next));
        self.submit(UiIntent::Conversation {
            request: RequestId(next),
            context,
            action,
        });
    }
    fn dismiss_room_feedback(&mut self) {
        self.room_ui.feedback = None;
        if let Some(panel) = &self.snapshot.conversation_panel
            && let Some(result) = panel.result
        {
            self.submit(UiIntent::AcknowledgeRoom {
                context: panel.context,
                request: result.request,
            });
        }
    }
    pub(super) fn room_action(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::Tab(tab) => {
                self.dismiss_room_feedback();
                self.room_ui.tab = tab;
                self.room_ui.confirmation = None;
                if tab == Tab::Permissions {
                    self.submit_room(ConversationAction::Bans);
                }
            }
            Action::People(bots) => {
                self.dismiss_room_feedback();
                self.room_ui.bots = bots;
                self.room_ui.confirmation = None;
                self.submit_room(if bots {
                    ConversationAction::Bots
                } else {
                    ConversationAction::Members
                });
            }
            Action::CopyGuestLink => {
                if let Some(panel) = &self.snapshot.conversation_panel
                    && let Some(details) = &panel.details
                    && !details.classified
                    && !details.sensitive
                    && let Some(link) = &panel.guest_link
                {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        link.as_str().to_owned(),
                    ));
                }
            }
            Action::Command(command) => {
                if command == ConversationAction::Read {
                    self.room_ui.loaded = false;
                }
                self.submit_room(command);
            }
            Action::SaveName => {
                if let Ok(value) = Text::new(self.room_ui.name.read(cx).content.as_str()) {
                    self.submit_room(ConversationAction::SetName(value));
                }
            }
            Action::SaveDescription => {
                if let Ok(value) = Text::new(self.room_ui.description.read(cx).content.as_str()) {
                    self.submit_room(ConversationAction::SetDescription(value));
                }
            }
            Action::Search => {
                if let Ok(search) = Text::new(self.room_ui.search.read(cx).content.as_str()) {
                    self.submit_room(ConversationAction::Directory {
                        search,
                        source: DirectorySource::User,
                    });
                }
            }
            Action::CsvPreview => {
                if let Ok(csv) = Text::new(self.room_ui.csv.read(cx).content.as_str()) {
                    self.submit_room(ConversationAction::PreviewCsv(csv));
                }
            }
            Action::ShowCsv => self.room_ui.csv_open = !self.room_ui.csv_open,
            Action::Confirm(command) => self.room_ui.confirmation = Some(command),
            Action::CancelConfirmation => self.room_ui.confirmation = None,
            Action::SubmitConfirmation => {
                if let Some(command) = self.room_ui.confirmation.take() {
                    self.submit_room(command);
                }
            }
            Action::Avatar => {
                if let Ok(emoji) = Text::new(self.room_ui.avatar.read(cx).content.as_str()) {
                    self.submit_room(ConversationAction::SetEmojiAvatar(emoji));
                }
            }
            Action::Password => {
                if let Ok(password) = Text::new(self.room_ui.password.read(cx).content.as_str()) {
                    self.submit_room(ConversationAction::SetPassword(password));
                    self.room_ui.password.update(cx, |i, _| i.reset());
                }
            }
        }
        cx.notify();
    }
    fn room_available(&self, setting: RoomSetting) -> Availability {
        let value = self
            .snapshot
            .conversation_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.room_ui.context)
            .and_then(|p| p.details.as_ref())
            .map_or(conversation::Availability::UNKNOWN, |d| {
                d.availability(setting)
            });
        if self
            .snapshot
            .conversation_panel
            .as_ref()
            .is_some_and(|p| p.state.pending())
        {
            Availability::when(false, "Änderung wird geprüft …")
        } else {
            Availability::when(value.enabled(), unavailable(value))
        }
    }
    fn room_heading(&self, label: &str) -> Div {
        div()
            .text_size(self.sp(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(self.l(label).to_owned())
    }
    fn room_row(&self, label: &str, detail: &str) -> Div {
        div()
            .min_w_0()
            .flex_1()
            .child(self.room_heading(label))
            .child(self.hint(self.l(detail)))
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "one settings row keeps its text, observed value and typed action together"
    )]
    fn room_toggle(
        &self,
        id: &'static str,
        label: &'static str,
        detail: &'static str,
        on: Option<bool>,
        setting: RoomSetting,
        command: ConversationAction,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_4()
            .py_3()
            .border_b_1()
            .border_color(self.theme.border)
            .child(self.room_row(label, detail))
            .child(self.switch(
                id,
                label,
                on,
                self.room_available(setting),
                LocalAction::Room(Action::Command(command)),
                cx,
            ))
    }
    fn room_button(
        &self,
        id: &'static str,
        label: &'static str,
        setting: RoomSetting,
        command: ConversationAction,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        self.button(
            id,
            label,
            ButtonStyle::Secondary,
            self.room_available(setting),
            LocalAction::Room(Action::Command(command)),
            cx,
        )
    }
    pub(super) fn conversation_dialog(&self, window: &Window, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let members = self.overlay == Some(Overlay::Participants);
        let narrow = window.viewport_size().width < px(820.) || self.scale >= 150;
        let title = if members {
            "Mitglieder"
        } else {
            "Gesprächseinstellungen"
        };
        let row = self.selected();
        let heading = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_6()
            .py_4()
            .border_b_1()
            .border_color(t.border)
            .when_some(row, |d, r| d.child(self.conversation_avatar(r)))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(self.sp(20.))
                            .font_weight(FontWeight::BOLD)
                            .child(self.l(title)),
                    )
                    .when_some(row, |d, r| d.child(self.hint(r.title.as_str()))),
            )
            .child(self.icon_button(
                "close-overlay",
                ("x", "Schließen"),
                ButtonStyle::Quiet,
                Availability::available(),
                LocalAction::Close,
                cx,
            ));
        let mut tabs = div()
            .flex_none()
            .flex()
            .gap_1()
            .p_3()
            .when(!narrow && !members, |d| {
                d.flex_col().w(px(210.)).border_r_1().border_color(t.border)
            })
            .when(narrow || members, |d| {
                d.flex_wrap().border_b_1().border_color(t.border)
            });
        if members {
            for (bots, label, icon) in [(false, "Personen", "users"), (true, "Bots", "grid")] {
                tabs = tabs.child(self.button_with_icon(
                    if bots {
                        "room-bots-tab"
                    } else {
                        "room-people-tab"
                    },
                    (label, icon),
                    if self.room_ui.bots == bots {
                        ButtonStyle::Primary
                    } else {
                        ButtonStyle::Quiet
                    },
                    Availability::available(),
                    LocalAction::Room(Action::People(bots)),
                    cx,
                ));
            }
        } else {
            for (i, tab) in Tab::ALL.into_iter().enumerate() {
                tabs = tabs.child(
                    self.button_with_icon(
                        ("room-tab", i),
                        (tab.label(), tab.icon()),
                        if self.room_ui.tab == tab {
                            ButtonStyle::Primary
                        } else {
                            ButtonStyle::Quiet
                        },
                        Availability::available(),
                        LocalAction::Room(Action::Tab(tab)),
                        cx,
                    )
                    .justify_start(),
                );
            }
        }
        let body = if self.room_ui.confirmation.is_some() {
            self.room_confirmation(cx)
        } else if members {
            self.members_page(cx)
        } else {
            self.room_settings_page(cx)
        };
        let status = self
            .snapshot
            .conversation_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.room_ui.context)
            .map(|p| p.state);
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(body)
            .when(
                self.snapshot.conversation_panel.as_ref().is_some_and(|p| {
                    p.result.as_ref().is_some_and(|r| {
                        self.room_ui.feedback == Some(r.request)
                            && r.completion
                                == talk_core::ui::presentation::Completion::ServerAccepted
                    }) && p.state == RemoteState::Idle
                }),
                |d| d.child(self.hint(self.l("Änderung vom Server bestätigt."))),
            )
            .when_some(status, |d, state| {
                let key = match state {
                    RemoteState::Idle => None,
                    RemoteState::Loading(_) => Some("Gesprächsdaten werden geladen …"),
                    RemoteState::Saving(_) => Some("Änderung wird geprüft …"),
                    RemoteState::Uncertain(_) => {
                        Some("Ergebnis unklar. Vor erneutem Ändern neu laden.")
                    }
                    RemoteState::Failed(_, reason) => Some(match reason {
                        talk_core::ui::UnavailableReason::PermissionDenied => {
                            "Du hast dafür keine Berechtigung."
                        }
                        talk_core::ui::UnavailableReason::Unsupported => {
                            "Der Server unterstützt diese Funktion nicht."
                        }
                        talk_core::ui::UnavailableReason::Unconfigured => {
                            "In diesem Client noch nicht verfügbar."
                        }
                        _ => {
                            "Die Anfrage ist fehlgeschlagen. Der bestätigte Stand bleibt erhalten."
                        }
                    }),
                };
                d.when_some(key, |d, key| {
                    d.child(
                        div()
                            .p_3()
                            .rounded_lg()
                            .bg(t.subtle)
                            .child(self.hint(self.l(key))),
                    )
                })
                .when(
                    matches!(state, RemoteState::Failed(..) | RemoteState::Uncertain(_)),
                    |d| {
                        d.child(self.button(
                            "room-reload",
                            "Erneut laden",
                            ButtonStyle::Secondary,
                            true,
                            LocalAction::Room(Action::Command(ConversationAction::Read)),
                            cx,
                        ))
                    },
                )
            });
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(t.scrim)
            .flex()
            .items_center()
            .justify_center()
            .p_5()
            .child(
                div()
                    .w_full()
                    .max_w(px(920.))
                    .h(px(
                        (f32::from(window.viewport_size().height) - 48.).min(700.)
                    ))
                    .max_h_full()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .bg(t.surface)
                    .rounded(px(14.))
                    .border_1()
                    .border_color(t.border)
                    .shadow_lg()
                    .track_focus(&self.modal_focus)
                    .tab_group()
                    .occlude()
                    .child(heading)
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .when(narrow || members, |d| d.flex_col())
                            .child(tabs)
                            .child(
                                self.scroll_area(super::navigation::Region::Overlay)
                                    .flex_1()
                                    .min_w_0()
                                    .min_h_0()
                                    .p_6()
                                    .child(body),
                            ),
                    ),
            )
    }
    fn room_settings_page(&self, cx: &Context<Self>) -> Div {
        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.room_heading(self.room_ui.tab.label()));
        let Some(panel) = self
            .snapshot
            .conversation_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.room_ui.context)
        else {
            return body.child(self.hint(self.l("Gesprächsdaten werden geladen …")));
        };
        let Some(d) = panel.details.as_ref() else {
            return body;
        };
        let Some(row) = self.selected() else {
            return body;
        };
        match self.room_ui.tab {
            Tab::General => {
                if matches!(
                    row.kind,
                    ConversationKind::Direct | ConversationKind::NoteToSelf
                ) {
                    body =
                        body.child(self.hint(self.l("Der Name stammt aus dem Profil der Person.")));
                } else {
                    body = body
                        .child(self.room_heading("Name"))
                        .child(self.room_input(&self.room_ui.name, cx))
                        .when(
                            self.room_ui.name.read(cx).content.as_str() != d.name.as_str(),
                            |b| {
                                b.child(self.button(
                                    "room-save-name",
                                    "Name speichern",
                                    ButtonStyle::Primary,
                                    self.room_available(RoomSetting::Metadata),
                                    LocalAction::Room(Action::SaveName),
                                    cx,
                                ))
                            },
                        )
                        .child(self.room_heading("Beschreibung"))
                        .child(self.room_input(&self.room_ui.description, cx))
                        .when(
                            self.room_ui.description.read(cx).content.as_str()
                                != d.description.as_ref().map_or("", |v| v.as_str()),
                            |b| {
                                b.child(self.button(
                                    "room-save-description",
                                    "Beschreibung speichern",
                                    ButtonStyle::Primary,
                                    self.room_available(RoomSetting::Metadata),
                                    LocalAction::Room(Action::SaveDescription),
                                    cx,
                                ))
                            },
                        )
                        .child(self.room_heading("Avatar"))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(self.room_input(&self.room_ui.avatar, cx))
                                .child(self.button(
                                    "room-avatar",
                                    "Emoji als Avatar setzen",
                                    ButtonStyle::Secondary,
                                    self.room_available(RoomSetting::Avatar),
                                    LocalAction::Room(Action::Avatar),
                                    cx,
                                )),
                        );
                }
                body = body
                    .child(self.room_toggle(
                        "room-read-only",
                        "Schreibgeschützt",
                        "Nachrichten bleiben lesbar. Schreiben folgt der Serverberechtigung.",
                        d.read_only,
                        RoomSetting::ReadOnly,
                        ConversationAction::SetReadOnly(d.read_only != Some(true)),
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(self.button(
                                "room-archive",
                                if row.archived == Some(true) {
                                    "Aus Archiv holen"
                                } else {
                                    "Archivieren"
                                },
                                ButtonStyle::Secondary,
                                Availability::when(
                                    row.archive == Decision::Allowed,
                                    "Du hast dafür keine Berechtigung.",
                                ),
                                LocalAction::Archive(row.context, row.archived != Some(true)),
                                cx,
                            ))
                            .child(self.button(
                                "room-unread",
                                "Als ungelesen markieren",
                                ButtonStyle::Quiet,
                                Availability::when(
                                    self.snapshot.content == super::ContentState::Ready,
                                    "Die Verbindung ist derzeit nicht bereit.",
                                ),
                                LocalAction::MessageIn(
                                    row.context,
                                    super::messaging::Action::Command(
                                        talk_core::ui::messaging::MessageAction::MarkUnread,
                                    ),
                                ),
                                cx,
                            ))
                            .child(self.button(
                                "room-local-groups",
                                "Zu Gruppe hinzufügen …",
                                ButtonStyle::Quiet,
                                true,
                                LocalAction::Open(Overlay::Organize),
                                cx,
                            )),
                    )
                    .child(self.hint(self.l("Archivieren betrifft nur deine Gesprächsliste.")))
                    .child(self.button(
                        "room-leave",
                        "Gespräch verlassen",
                        ButtonStyle::Secondary,
                        Availability::when(
                            d.leave == Decision::Allowed,
                            "Du hast dafür keine Berechtigung.",
                        ),
                        LocalAction::Room(Action::Confirm(ConversationAction::Leave)),
                        cx,
                    ))
                    .child(self.button(
                        "room-delete",
                        "Gespräch für alle löschen",
                        ButtonStyle::Danger,
                        self.room_available(RoomSetting::Delete),
                        LocalAction::Room(Action::Confirm(ConversationAction::Delete)),
                        cx,
                    ));
            }
            Tab::Notifications => {
                body=body.child(self.room_toggle("room-important","Wichtig","NextCloud-Ruhemodus kann übersteuert werden. Der Fokusmodus des Betriebssystems gilt weiter.",d.important,RoomSetting::Important,ConversationAction::SetImportant(d.important!=Some(true)),cx))
                    .child(self.room_heading("Nachrichten"));
                let levels = [
                    (conversation::NotificationLevel::Default, "Serverstandard"),
                    (conversation::NotificationLevel::Always, "Alle Nachrichten"),
                    (conversation::NotificationLevel::Mention, "Nur Erwähnungen"),
                    (conversation::NotificationLevel::Never, "Keine Nachrichten"),
                ];
                if let Some(level) = d.notifications {
                    let label = levels
                        .iter()
                        .find(|(value, _)| *value == level)
                        .map_or("Unbekannt", |(_, label)| *label);
                    body = body.child(
                        self.hint(&self.lf("Aktuell: {value}", &[("value", self.l(label))])),
                    );
                }
                if !d.availability(RoomSetting::Notifications).enabled() {
                    body = body.child(
                        self.hint(self.l(unavailable(d.availability(RoomSetting::Notifications)))),
                    );
                }
                for (i, (value, label)) in levels.into_iter().enumerate() {
                    body = body.child(self.button(
                        ("room-notifications", i),
                        label,
                        if d.notifications == Some(value) {
                            ButtonStyle::Primary
                        } else {
                            ButtonStyle::Secondary
                        },
                        self.room_available(RoomSetting::Notifications),
                        LocalAction::Room(Action::Command(ConversationAction::SetNotifications(
                            value,
                        ))),
                        cx,
                    ));
                }
                body = body
                    .child(self.room_toggle(
                        "room-calls",
                        "Anrufbenachrichtigungen",
                        "Anrufhinweise unabhängig von Nachrichten einstellen.",
                        d.call_notifications,
                        RoomSetting::Notifications,
                        ConversationAction::SetCallNotifications(
                            d.call_notifications != Some(true),
                        ),
                        cx,
                    ))
                    .child(
                        self.hint(
                            self.l("Banner und Töne stellst du in den App-Einstellungen ein."),
                        ),
                    );
            }
            Tab::Privacy => {
                body = body
                    .child(self.room_toggle(
                        "room-sensitive",
                        "Vertrauliche Vorschau",
                        "Inhalte aus Navigation, Suche und Benachrichtigungen ausblenden.",
                        Some(d.sensitive || d.classified),
                        RoomSetting::Sensitive,
                        ConversationAction::SetSensitive(!d.sensitive),
                        cx,
                    ))
                    .when(d.classified, |b| {
                        b.child(self.hint(self.l(
                            "Für klassifizierte Gespräche ist diese Einstellung verpflichtend.",
                        )))
                    })
                    .child(self.room_heading("Nachrichten automatisch löschen"));
                if let Some(seconds) = d.message_expiration {
                    let label = match seconds {
                        0 => self.l("Nie").to_owned(),
                        86400 => self.l("Nach einem Tag").to_owned(),
                        604800 => self.l("Nach einer Woche").to_owned(),
                        2592000 => self.l("Nach 30 Tagen").to_owned(),
                        _ => self.lf(
                            "Nach {seconds} Sekunden",
                            &[("seconds", &seconds.to_string())],
                        ),
                    };
                    body =
                        body.child(self.hint(&self.lf("Aktuell: {value}", &[("value", &label)])));
                }
                if !d.availability(RoomSetting::Expiration).enabled() {
                    body = body.child(
                        self.hint(self.l(unavailable(d.availability(RoomSetting::Expiration)))),
                    );
                }
                for (i, (seconds, label)) in [
                    (0, "Nie"),
                    (86400, "Nach einem Tag"),
                    (604800, "Nach einer Woche"),
                    (2592000, "Nach 30 Tagen"),
                ]
                .into_iter()
                .enumerate()
                {
                    body = body.child(self.button(
                        ("room-expiry", i),
                        label,
                        if d.message_expiration == Some(seconds) {
                            ButtonStyle::Primary
                        } else {
                            ButtonStyle::Secondary
                        },
                        self.room_available(RoomSetting::Expiration),
                        LocalAction::Room(Action::Command(ConversationAction::SetExpiration(
                            seconds,
                        ))),
                        cx,
                    ));
                }
                body =
                    body.child(self.hint(self.l(
                        "Aufbewahrung und gesicherte Gespräche können das Löschen verhindern.",
                    )))
                    .child(self.button(
                        "room-clear",
                        "Chatverlauf für alle leeren",
                        ButtonStyle::Danger,
                        self.room_available(RoomSetting::ClearHistory),
                        LocalAction::Room(Action::Confirm(ConversationAction::ClearHistory)),
                        cx,
                    ));
            }
            Tab::Permissions => {
                body=body.child(self.hint(self.l("Serverrechte für Beitritt, Start und Medien. Gerätefreigaben des Betriebssystems sind davon unabhängig.")))
                    .child(self.hint(self.l("Änderungen an Standardrechten setzen individuelle Ausnahmen zurück.")));
                let defaults =
                    permission_defaults(d.default_permissions, d.server_default_permissions);
                if defaults.is_none() {
                    body=body.child(self.hint(self.l("Die Standardrechte des Servers sind nicht bekannt. Neu laden, bevor du Rechte änderst.")));
                } else if d.default_permissions == Some(0) {
                    body = body
                        .child(self.hint(self.l("Aktuell gelten die Standardrechte des Servers.")));
                }
                for (i, (bit, label)) in [
                    (4, "Meeting beitreten"),
                    (2, "Meeting starten"),
                    (16, "Audio veröffentlichen"),
                    (32, "Video veröffentlichen"),
                    (64, "Bildschirm veröffentlichen"),
                ]
                .into_iter()
                .enumerate()
                {
                    let on = defaults.map(|p| p & bit != 0);
                    let changed = defaults.map(|p| (p ^ bit) | 1);
                    let available = if changed.is_none() {
                        Availability::when(
                            false,
                            "Die Standardrechte des Servers sind nicht bekannt. Neu laden, bevor du Rechte änderst.",
                        )
                    } else {
                        self.room_available(RoomSetting::Permissions)
                    };
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .py_2()
                            .child(self.room_row(label, "Gilt als Standard für Teilnehmende."))
                            .child(self.switch(
                                ("room-permission", i),
                                label,
                                on,
                                available,
                                LocalAction::Room(Action::Confirm(
                                    ConversationAction::SetDefaultPermissions(changed.unwrap_or(0)),
                                )),
                                cx,
                            )),
                    );
                }
                body = body
                    .child(self.room_heading("Gesperrte Personen"))
                    .child(self.room_button(
                        "room-load-bans",
                        "Neu laden",
                        RoomSetting::Bans,
                        ConversationAction::Bans,
                        cx,
                    ));
                for ban in panel.bans.as_slice() {
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .child(ban.name.as_str().to_owned())
                                    .child(self.hint(ban.note.as_str())),
                            )
                            .child(self.button(
                                ("room-unban", ban.id.local as usize),
                                "Sperre aufheben",
                                ButtonStyle::Secondary,
                                self.room_available(RoomSetting::Bans),
                                LocalAction::Room(Action::Confirm(ConversationAction::Unban(
                                    ban.id,
                                ))),
                                cx,
                            )),
                    );
                }
                if panel.bans_loaded && panel.bans.as_slice().is_empty() {
                    body = body.child(self.hint(self.l("Keine gesperrten Personen.")));
                }
            }
            Tab::Meeting => {
                body = body
                    .child(self.button(
                        "room-manage-roles",
                        "Mitglieder und Rollen",
                        ButtonStyle::Secondary,
                        self.room_available(RoomSetting::Members),
                        LocalAction::Open(Overlay::Participants),
                        cx,
                    ))
                    .child(self.room_toggle(
                        "room-lobby",
                        "Warteraum",
                        "Gäste warten auf die Freigabe durch die Moderation.",
                        d.lobby,
                        RoomSetting::Lobby,
                        ConversationAction::SetLobby(d.lobby != Some(true)),
                        cx,
                    ))
                    .child(self.room_toggle(
                        "room-guest-access",
                        "Gastzugang",
                        "Öffentlicher Zugang ist unabhängig von der Auffindbarkeit.",
                        d.guest_access,
                        RoomSetting::GuestAccess,
                        ConversationAction::SetGuestAccess(d.guest_access != Some(true)),
                        cx,
                    ))
                    .child(self.button(
                        "room-guest-link",
                        "Gästelink anzeigen",
                        ButtonStyle::Secondary,
                        Availability::when(
                            d.guest_access == Some(true),
                            "Gastzugang ist nicht aktiviert.",
                        ),
                        LocalAction::Room(Action::Command(ConversationAction::GuestLink)),
                        cx,
                    ))
                    .when_some(panel.guest_link.as_ref(), |body, link| {
                        body.child(self.hint(link.as_str())).child(self.button(
                            "copy-guest-link",
                            "Gästelink kopieren",
                            ButtonStyle::Quiet,
                            Availability::when(
                                !d.classified && !d.sensitive,
                                "Weitergeben ist durch die Gesprächsrichtlinie gesperrt.",
                            ),
                            LocalAction::Room(Action::CopyGuestLink),
                            cx,
                        ))
                    })
                    .child(self.room_heading("Passwort für Gäste"))
                    .child(self.hint(self.l(if d.has_password == Some(true) {
                        "Ein Passwort ist eingerichtet."
                    } else {
                        "Kein bestätigtes Gastpasswort."
                    })))
                    .child(self.room_input(&self.room_ui.password, cx))
                    .child(self.button(
                        "room-set-password",
                        "Passwort setzen",
                        ButtonStyle::Secondary,
                        self.room_available(RoomSetting::Password),
                        LocalAction::Room(Action::Password),
                        cx,
                    ))
                    .child(self.room_heading("Gruppenräume"))
                    .child(self.hint(self.l(
                        "Gruppenräume teilen das Meeting auf. Sie sind keine Verzeichnisgruppen.",
                    )))
                    .child(self.room_button(
                        "room-breakouts",
                        "Zwei Gruppenräume vorbereiten",
                        RoomSetting::Breakouts,
                        ConversationAction::ConfigureBreakouts {
                            amount: 2,
                            automatic: true,
                        },
                        cx,
                    ));
            }
        }
        body
    }
    fn room_input(&self, input: &Entity<TextInput>, cx: &Context<Self>) -> Div {
        let entity = input.clone();
        div()
            .flex_1()
            .min_w_0()
            .relative()
            .min_h(px(42.))
            .rounded(px(8.))
            .border_1()
            .border_color(self.theme.input_border)
            .bg(self.theme.subtle)
            .px_3()
            .py_2()
            .child(self.field_ring(input.focus_handle(cx), 8.))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |_, _, window, cx| {
                    entity.focus_handle(cx).focus(window);
                }),
            )
            .child(input.clone())
    }
    fn members_page(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_3();
        let Some(panel) = self
            .snapshot
            .conversation_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.room_ui.context)
        else {
            return body;
        };
        if self.room_ui.bots {
            body=body.child(self.hint(self.l("Verfügbare Bots für dieses Gespräch. Registrierung und Zugangsdaten verwaltet die Administration.")))
                .child(self.room_button("reload-room-bots","Bots laden",RoomSetting::Bots,ConversationAction::Bots,cx));
            for bot in panel.bots.as_slice() {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .py_3()
                        .child(
                            div()
                                .flex_1()
                                .child(bot.name.as_str().to_owned())
                                .child(self.hint(bot.description.as_str())),
                        )
                        .child(self.switch(
                            ("room-bot", bot.id.local as usize),
                            "Bot aktivieren",
                            bot.enabled,
                            self.room_available(RoomSetting::Bots),
                            LocalAction::Room(Action::Command(ConversationAction::SetBotEnabled {
                                bot: bot.id,
                                enabled: bot.enabled != Some(true),
                            })),
                            cx,
                        )),
                );
            }
            if panel.bots_loaded && panel.bots.as_slice().is_empty() {
                body = body.child(self.hint(self.l("Keine verfügbaren Bots.")));
            }
            return body;
        }
        body = body
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.room_input(&self.room_ui.search, cx))
                    .child(self.button(
                        "room-directory-search",
                        "Verzeichnis durchsuchen",
                        ButtonStyle::Secondary,
                        self.room_available(RoomSetting::Invite),
                        LocalAction::Room(Action::Search),
                        cx,
                    )),
            )
            .child(self.room_heading("Im Gespräch"));
        let query = self.room_ui.search.read(cx).content.to_lowercase();
        let mut matching = 0;
        for member in panel
            .members
            .as_slice()
            .iter()
            .filter(|m| query.is_empty() || m.name.as_str().to_lowercase().contains(&query))
        {
            matching += 1;
            let key = match member.role {
                MemberRole::Owner => "Eigentümer",
                MemberRole::Moderator | MemberRole::GuestModerator => "Moderation",
                MemberRole::Guest => "Gast",
                MemberRole::Following => "Folgt",
                MemberRole::Member => "Teilnehmend",
                MemberRole::Unknown => "Unbekannte Rolle",
            };
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_3()
                    .border_b_1()
                    .border_color(self.theme.border)
                    .child(self.avatar(&person_initials(member.name.as_str()), false))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(member.name.as_str().to_owned())
                            .child(self.hint(self.l(key))),
                    )
                    .when(!member.self_user && member.role != MemberRole::Owner, |d| {
                        d.child(self.button(
                            ("room-role", member.id.local as usize),
                            if member.role == MemberRole::Moderator {
                                "Mitglied"
                            } else {
                                "Moderation"
                            },
                            ButtonStyle::Quiet,
                            self.room_available(RoomSetting::Roles),
                            LocalAction::Room(Action::Confirm(ConversationAction::SetRole {
                                member: member.id,
                                role: if member.role == MemberRole::Moderator {
                                    MemberRole::Member
                                } else {
                                    MemberRole::Moderator
                                },
                            })),
                            cx,
                        ))
                        .child(self.icon_button(
                            ("room-remove", member.id.local as usize),
                            ("x", "Person entfernen"),
                            ButtonStyle::Quiet,
                            self.room_available(RoomSetting::Invite),
                            LocalAction::Room(Action::Confirm(ConversationAction::RemoveMember(
                                member.id,
                            ))),
                            cx,
                        ))
                    }),
            );
        }
        if panel.members_loaded && matching == 0 {
            body = body.child(self.hint(self.l("Keine passenden Personen gefunden.")));
        }
        if !panel.members_loaded {
            body = body.child(self.room_button(
                "room-load-members",
                "Mitglieder laden",
                RoomSetting::Members,
                ConversationAction::Members,
                cx,
            ));
        }
        body = body
            .child(self.room_heading("Verzeichnis · Einladungsvorschau"))
            .child(
                self.hint(self.l("Suchergebnisse werden erst nach deiner Bestätigung eingeladen.")),
            );
        for person in panel.directory.as_slice() {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_2()
                    .child(self.avatar(&person_initials(person.name.as_str()), false))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(person.name.as_str().to_owned())
                            .child(self.hint(person.label.as_str())),
                    )
                    .child(self.button(
                        ("room-invite", person.id.local as usize),
                        "Einladen",
                        ButtonStyle::Secondary,
                        self.room_available(RoomSetting::Invite),
                        LocalAction::Room(Action::Confirm(ConversationAction::Invite(person.id))),
                        cx,
                    )),
            );
        }
        if panel.directory_loaded && panel.directory.as_slice().is_empty() {
            body = body.child(self.hint(self.l("Keine passenden Personen gefunden.")));
        }
        body = body.child(div().flex().justify_start().child(self.button(
            "room-csv-toggle",
            "CSV-Import",
            ButtonStyle::Quiet,
            true,
            LocalAction::Room(Action::ShowCsv),
            cx,
        )));
        if self.room_ui.csv_open {
            body = body
                .child(self.hint(
                    self.l("Maximal 100 Adressen und 64 KiB. Einfügen erstellt nur eine Vorschau."),
                ))
                .child(self.room_input(&self.room_ui.csv, cx))
                .child(self.button(
                    "room-csv-preview",
                    "Importvorschau erstellen",
                    ButtonStyle::Secondary,
                    self.room_available(RoomSetting::Invite),
                    LocalAction::Room(Action::CsvPreview),
                    cx,
                ))
                .child(self.hint(&self.lf(
                    "{duplicates} doppelte und {invalid} ungültige Zeilen.",
                    &[
                        ("duplicates", &panel.csv_duplicates.to_string()),
                        ("invalid", &panel.csv_invalid.to_string()),
                    ],
                )));
        }
        body
    }
    fn room_confirmation(&self, cx: &Context<Self>) -> Div {
        let action = self.room_ui.confirmation.as_ref();
        let key = match action {
            Some(ConversationAction::Delete) => {
                "Dieses Gespräch und seine Nachrichten werden für alle gelöscht."
            }
            Some(ConversationAction::ClearHistory) => {
                "Alle Nachrichten dieses Gesprächs werden für alle gelöscht."
            }
            Some(ConversationAction::Leave) => {
                "Du beendest deine Mitgliedschaft in diesem Gespräch."
            }
            Some(ConversationAction::RemoveMember(_)) => {
                "Diese Person wird entfernt. Sie wird dadurch nicht gesperrt."
            }
            Some(ConversationAction::Invite(_)) => {
                "Die ausgewählte Person erhält Zugang zu diesem Gespräch."
            }
            Some(ConversationAction::SetDefaultPermissions(_)) => {
                "Neue Standardrechte setzen individuelle Ausnahmen zurück."
            }
            _ => "Die ausgewählte Änderung wird auf dem Server angewendet.",
        };
        let person = match action {
            Some(
                ConversationAction::Invite(id)
                | ConversationAction::RemoveMember(id)
                | ConversationAction::Unban(id),
            ) => Some(*id),
            Some(ConversationAction::SetRole { member, .. }) => Some(*member),
            _ => None,
        };
        let person_name = person.and_then(|id| {
            self.snapshot.conversation_panel.as_ref().and_then(|p| {
                p.directory
                    .as_slice()
                    .iter()
                    .find(|v| v.id == id)
                    .map(|v| v.name.as_str())
                    .or_else(|| {
                        p.members
                            .as_slice()
                            .iter()
                            .find(|v| v.id == id)
                            .map(|v| v.name.as_str())
                    })
                    .or_else(|| {
                        p.bans
                            .as_slice()
                            .iter()
                            .find(|v| v.id == id)
                            .map(|v| v.name.as_str())
                    })
            })
        });
        let destructive = matches!(
            action,
            Some(
                ConversationAction::Delete
                    | ConversationAction::ClearHistory
                    | ConversationAction::Leave
                    | ConversationAction::RemoveMember(_)
            )
        );
        let verb = match action {
            Some(ConversationAction::Delete) => "Löschen",
            Some(ConversationAction::ClearHistory) => "Chatverlauf leeren",
            Some(ConversationAction::Leave) => "Verlassen",
            Some(ConversationAction::RemoveMember(_)) => "Entfernen",
            Some(ConversationAction::Invite(_)) => "Einladen",
            Some(ConversationAction::SetRole {
                role: MemberRole::Moderator,
                ..
            }) => "Zur Moderation ernennen",
            Some(ConversationAction::SetRole { .. }) => "Als Mitglied setzen",
            Some(ConversationAction::Unban(_)) => "Sperre aufheben",
            Some(ConversationAction::SetDefaultPermissions(_)) => "Berechtigung ändern",
            _ => "Bestätigen",
        };
        let permission = if let Some(ConversationAction::SetDefaultPermissions(next)) = action {
            self.snapshot
                .conversation_panel
                .as_ref()
                .and_then(|p| p.details.as_ref())
                .and_then(|d| {
                    permission_defaults(d.default_permissions, d.server_default_permissions)
                })
                .and_then(|prior| {
                    let difference = (prior ^ next) & !1;
                    [
                        (4, "Meeting beitreten"),
                        (2, "Meeting starten"),
                        (16, "Audio veröffentlichen"),
                        (32, "Video veröffentlichen"),
                        (64, "Bildschirm veröffentlichen"),
                    ]
                    .into_iter()
                    .find(|(bit, _)| difference == *bit)
                    .map(|(bit, label)| {
                        self.lf(
                            if next & bit != 0 {
                                "„{right}“ erlauben?"
                            } else {
                                "„{right}“ entziehen?"
                            },
                            &[("right", self.l(label))],
                        )
                    })
                })
        } else {
            None
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.room_heading(verb))
            .when_some(self.selected(), |d, row| {
                d.child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .child(row.title.as_str().to_owned()),
                )
            })
            .when_some(person_name, |d, name| {
                d.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name.to_owned()),
                )
            })
            .when_some(permission, |d, label| {
                d.child(div().font_weight(FontWeight::SEMIBOLD).child(label))
            })
            .child(self.hint(self.l(key)))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(self.button(
                        "room-cancel-confirm",
                        "Abbrechen",
                        ButtonStyle::Secondary,
                        true,
                        LocalAction::Room(Action::CancelConfirmation),
                        cx,
                    ))
                    .child(self.button(
                        "room-confirm",
                        verb,
                        if destructive {
                            ButtonStyle::Danger
                        } else {
                            ButtonStyle::Primary
                        },
                        true,
                        LocalAction::Room(Action::SubmitConfirmation),
                        cx,
                    )),
            )
    }
}

// Talk's zero room mask inherits capability defaults, not the moderator's rights.
fn permission_defaults(room: Option<u16>, server: Option<u16>) -> Option<u16> {
    match room.filter(|mask| mask & !511 == 0)? {
        0 => server.filter(|mask| mask & !511 == 0),
        custom => Some(custom),
    }
}
fn person_initials(name: &str) -> String {
    name.split(|c: char| c.is_whitespace() || matches!(c, '_' | '-' | '.'))
        .filter(|p| !p.is_empty())
        .take(2)
        .filter_map(|p| p.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_permissions_preserve_unshown_rights_and_never_guess() {
        let inherited = permission_defaults(Some(0), Some(502)).unwrap();
        assert_eq!(inherited, 502);
        let changed = (inherited ^ 16) | 1;
        assert_eq!(
            changed & (256 | 128 | 4 | 2 | 32 | 64),
            inherited & (256 | 128 | 4 | 2 | 32 | 64)
        );
        assert_eq!(changed & 16, 0);
        assert_eq!(permission_defaults(Some(0), Some(132)), Some(132));
        assert_eq!(permission_defaults(Some(129), Some(502)), Some(129));
        assert_eq!(permission_defaults(Some(0), None), None);
        assert_eq!(permission_defaults(None, Some(502)), None);
        assert_eq!(permission_defaults(Some(0), Some(512)), None);
    }
}
