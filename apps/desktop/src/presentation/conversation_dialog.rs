//! Native room dialogs keep their originating context and submit typed, correlated intents.
use super::{ButtonStyle, Desktop, LocalAction, Overlay, controls::Availability, input::TextInput};
use gpui::{Context, Div, Entity, FontWeight, Window, div, prelude::*, px};
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
        self.submit(UiIntent::Conversation {
            request: RequestId(next),
            context,
            action,
        });
    }
    fn dismiss_room_feedback(&mut self) {
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
            .text_size(self.sp(13.))
            .line_height(self.sp(18.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(self.l(label).to_owned())
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
        self.dialog_row(
            label,
            Some(detail),
            self.switch(
                id,
                label,
                on,
                self.room_available(setting),
                LocalAction::Room(Action::Command(command)),
                cx,
            ),
        )
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
    /// A labelled text field with its optional commit action on the same line.
    fn room_field(
        &self,
        label: &'static str,
        input: &Entity<TextInput>,
        action: Option<gpui::Stateful<Div>>,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(self.sp(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.l(label).to_owned()),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(self.room_input(input, cx))
                    .when_some(action, |d, action| d.child(action.flex_none())),
            )
    }
    /// Remote state belongs to the dialog, not to one tab: it is the footer.
    fn room_status(&self, cx: &Context<Self>) -> Option<Div> {
        let panel = self
            .snapshot
            .conversation_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.room_ui.context)?;
        let key = match panel.state {
            // The controls still follow confirmed/pending authority, but routine
            // saves do not add a status bar or shift the settings page.
            RemoteState::Idle | RemoteState::Saving(_) => return None,
            RemoteState::Loading(_) => "Gesprächsdaten werden geladen …",
            RemoteState::Uncertain(_) => "Ergebnis unklar. Vor erneutem Ändern neu laden.",
            RemoteState::Failed(_, reason) => match reason {
                talk_core::ui::UnavailableReason::PermissionDenied => {
                    "Du hast dafür keine Berechtigung."
                }
                talk_core::ui::UnavailableReason::Unsupported => {
                    "Der Server unterstützt diese Funktion nicht."
                }
                talk_core::ui::UnavailableReason::Unconfigured => {
                    "In diesem Client noch nicht verfügbar."
                }
                _ => "Die Anfrage ist fehlgeschlagen. Der bestätigte Stand bleibt erhalten.",
            },
        };
        let problem = matches!(
            panel.state,
            RemoteState::Failed(..) | RemoteState::Uncertain(_)
        );
        Some(
            div()
                .flex()
                .items_center()
                .gap_3()
                .min_h(px(36.))
                .child(
                    self.hint(self.l(key))
                        .flex_1()
                        .min_w_0()
                        .when(matches!(panel.state, RemoteState::Failed(..)), |d| {
                            d.text_color(self.theme.danger)
                        }),
                )
                .when(problem, |d| {
                    d.child(self.button(
                        "room-reload",
                        "Erneut laden",
                        ButtonStyle::Secondary,
                        true,
                        LocalAction::Room(Action::Command(ConversationAction::Read)),
                        cx,
                    ))
                }),
        )
    }
    pub(super) fn conversation_dialog(&self, window: &Window, cx: &Context<Self>) -> Div {
        let members = self.overlay == Some(Overlay::Participants);
        let narrow = self.dialog_narrow(window);
        let title = if members {
            "Mitglieder"
        } else {
            "Gesprächseinstellungen"
        };
        let row = self.selected();
        let mut items = Vec::with_capacity(Tab::ALL.len());
        if members {
            for (bots, label, icon) in [(false, "Personen", "users"), (true, "Bots", "grid")] {
                items.push(self.dialog_nav_item(
                    if bots {
                        "room-bots-tab"
                    } else {
                        "room-people-tab"
                    },
                    (label, icon),
                    self.room_ui.bots == bots,
                    LocalAction::Room(Action::People(bots)),
                    cx,
                ));
            }
        } else {
            for (i, tab) in Tab::ALL.into_iter().enumerate() {
                items.push(self.dialog_nav_item(
                    ("room-tab", i),
                    (tab.label(), tab.icon()),
                    self.room_ui.tab == tab,
                    LocalAction::Room(Action::Tab(tab)),
                    cx,
                ));
            }
        }
        // Five or fewer destinations stay visible in the narrow bar instead of
        // moving behind a menu, so no focus target disappears on selection.
        let navigation = if narrow {
            self.dialog_bar(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(items.into_iter().map(|item| item.w_auto().mb(px(0.)))),
            )
        } else {
            let header = match row {
                Some(r) => div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.conversation_avatar(r))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .text_size(self.sp(14.))
                                    .font_weight(FontWeight::MEDIUM)
                                    // Unlike nowrap truncation, this remeasures
                                    // text when GPUI resolves the flex width.
                                    .line_clamp(1)
                                    .text_ellipsis()
                                    .child(r.title.as_str().to_owned()),
                            )
                            .child(self.hint(self.l("Gespräch"))),
                    ),
                None => self.dialog_title(title),
            };
            self.dialog_rail(header, items)
        };
        let (key, page) = if self.room_ui.confirmation.is_some() {
            (20, self.room_confirmation(cx))
        } else if members {
            (
                10 + usize::from(self.room_ui.bots),
                self.members_page(narrow, cx),
            )
        } else {
            (
                self.room_ui.tab as usize,
                self.room_settings_page(narrow, cx),
            )
        };
        self.dialog_frame(
            super::settings::DialogFrame {
                id: "conversation-dialog",
                close: ("close-overlay", "Schließen"),
                region: super::navigation::Region::Overlay,
                navigation,
                page: self.reveal(("room-page", key), page),
                footer: if self.feedback_visible() {
                    Some(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(self.room_status(cx))
                            .child(self.feedback_view(cx)),
                    )
                } else {
                    self.room_status(cx)
                },
            },
            window,
            cx,
        )
    }
    /// Page title; in the narrow layout the rail is gone, so the conversation name moves here.
    fn room_page_header(&self, label: &'static str, narrow: bool) -> Div {
        self.dialog_page_header(label, None).when(narrow, |d| {
            d.when_some(self.selected(), |d, r| d.child(self.hint(r.title.as_str())))
        })
    }
    fn room_settings_page(&self, narrow: bool, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let mut page = self
            .dialog_page()
            .child(self.room_page_header(self.room_ui.tab.label(), narrow));
        let Some(panel) = self
            .snapshot
            .conversation_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.room_ui.context)
        else {
            return page.child(self.hint(self.l("Gesprächsdaten werden geladen …")));
        };
        let Some(d) = panel.details.as_ref() else {
            return page;
        };
        let Some(row) = self.selected() else {
            return page;
        };
        // Destructive server actions close every page that offers them, set apart.
        let danger = || {
            self.dialog_section(None)
                .pt_4()
                .border_t_1()
                .border_color(t.border)
        };
        match self.room_ui.tab {
            Tab::General => {
                let mut about = self.dialog_section(Some("Gespräch"));
                if matches!(
                    row.kind,
                    ConversationKind::Direct | ConversationKind::NoteToSelf
                ) {
                    about = about
                        .child(self.hint(self.l("Der Name stammt aus dem Profil der Person.")));
                } else {
                    let name_changed =
                        self.room_ui.name.read(cx).content.as_str() != d.name.as_str();
                    let description_changed = self.room_ui.description.read(cx).content.as_str()
                        != d.description.as_ref().map_or("", |v| v.as_str());
                    about = about
                        .gap_3()
                        .child(self.room_field(
                            "Name",
                            &self.room_ui.name,
                            name_changed.then(|| {
                                self.button(
                                    "room-save-name",
                                    "Name speichern",
                                    ButtonStyle::Primary,
                                    self.room_available(RoomSetting::Metadata),
                                    LocalAction::Room(Action::SaveName),
                                    cx,
                                )
                            }),
                            cx,
                        ))
                        .child(self.room_field(
                            "Beschreibung",
                            &self.room_ui.description,
                            description_changed.then(|| {
                                self.button(
                                    "room-save-description",
                                    "Beschreibung speichern",
                                    ButtonStyle::Primary,
                                    self.room_available(RoomSetting::Metadata),
                                    LocalAction::Room(Action::SaveDescription),
                                    cx,
                                )
                            }),
                            cx,
                        ))
                        .child(self.room_field(
                            "Avatar",
                            &self.room_ui.avatar,
                            Some(self.button(
                                "room-avatar",
                                "Emoji als Avatar setzen",
                                ButtonStyle::Secondary,
                                self.room_available(RoomSetting::Avatar),
                                LocalAction::Room(Action::Avatar),
                                cx,
                            )),
                            cx,
                        ));
                }
                page = page
                    .child(about.child(self.room_toggle(
                        "room-read-only",
                        "Schreibgeschützt",
                        "Nachrichten bleiben lesbar. Schreiben folgt der Serverberechtigung.",
                        d.read_only,
                        RoomSetting::ReadOnly,
                        ConversationAction::SetReadOnly(d.read_only != Some(true)),
                        cx,
                    )))
                    .child(
                        self.dialog_section(None)
                            .child(
                                self.dialog_actions()
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
                                        LocalAction::Archive(
                                            row.context,
                                            row.archived != Some(true),
                                        ),
                                        cx,
                                    ))
                                    .child(self.button(
                                        "room-unread",
                                        "Als ungelesen markieren",
                                        ButtonStyle::Secondary,
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
                                        ButtonStyle::Secondary,
                                        true,
                                        LocalAction::Open(Overlay::Organize),
                                        cx,
                                    ))
                                    .child(self.create_poll_button("room-create-poll", cx)),
                            )
                            .child(
                                self.hint(self.l("Archivieren betrifft nur deine Gesprächsliste.")),
                            ),
                    )
                    .child(
                        danger().child(
                            self.dialog_actions()
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
                                )),
                        ),
                    );
            }
            Tab::Notifications => {
                let mut messages = self.dialog_section(Some("Nachrichten"));
                if !d.availability(RoomSetting::Notifications).enabled() {
                    messages = messages.child(
                        self.hint(self.l(unavailable(d.availability(RoomSetting::Notifications)))),
                    );
                }
                messages = messages.child(
                    div().flex().child(
                        self.segmented().children(
                            [
                                (conversation::NotificationLevel::Default, "Serverstandard"),
                                (conversation::NotificationLevel::Always, "Alle Nachrichten"),
                                (conversation::NotificationLevel::Mention, "Nur Erwähnungen"),
                                (conversation::NotificationLevel::Never, "Keine Nachrichten"),
                            ]
                            .into_iter()
                            .enumerate()
                            .map(|(i, (value, label))| {
                                self.segment(
                                    ("room-notifications", i),
                                    label,
                                    d.notifications == Some(value),
                                    self.room_available(RoomSetting::Notifications),
                                    LocalAction::Room(Action::Command(
                                        ConversationAction::SetNotifications(value),
                                    )),
                                    cx,
                                )
                            }),
                        ),
                    ),
                );
                page = page.child(messages).child(
                    self.dialog_section(None)
                        .gap_0()
                        .child(self.room_toggle(
                            "room-important",
                            "Wichtig",
                            "NextCloud-Ruhemodus kann übersteuert werden. Der Fokusmodus des Betriebssystems gilt weiter.",
                            d.important,
                            RoomSetting::Important,
                            ConversationAction::SetImportant(d.important != Some(true)),
                            cx,
                        ))
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
                            )
                            .pt_2(),
                        ),
                );
            }
            Tab::Privacy => {
                let presets = [
                    (0, "Nie"),
                    (86400, "Nach einem Tag"),
                    (604800, "Nach einer Woche"),
                    (2592000, "Nach 30 Tagen"),
                ];
                let mut expiry = self.dialog_section(Some("Nachrichten automatisch löschen"));
                // A server value outside the presets is still shown, never guessed.
                if let Some(seconds) = d
                    .message_expiration
                    .filter(|s| presets.iter().all(|(preset, _)| preset != s))
                {
                    expiry = expiry.child(self.hint(&self.lf(
                        "Aktuell: {value}",
                        &[(
                            "value",
                            &self.lf(
                                "Nach {seconds} Sekunden",
                                &[("seconds", &seconds.to_string())],
                            ),
                        )],
                    )));
                }
                if !d.availability(RoomSetting::Expiration).enabled() {
                    expiry = expiry.child(
                        self.hint(self.l(unavailable(d.availability(RoomSetting::Expiration)))),
                    );
                }
                expiry = expiry
                    .child(
                        div().flex().child(
                            self.segmented()
                                .children(presets.into_iter().enumerate().map(
                                    |(i, (seconds, label))| {
                                        self.segment(
                                            ("room-expiry", i),
                                            label,
                                            d.message_expiration == Some(seconds),
                                            self.room_available(RoomSetting::Expiration),
                                            LocalAction::Room(Action::Command(
                                                ConversationAction::SetExpiration(seconds),
                                            )),
                                            cx,
                                        )
                                    },
                                )),
                        ),
                    )
                    .child(self.hint(self.l(
                        "Aufbewahrung und gesicherte Gespräche können das Löschen verhindern.",
                    )));
                page = page
                    .child(
                        self.dialog_section(Some("Sichtbarkeit"))
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
                            }),
                    )
                    .child(expiry)
                    .child(danger().child(self.dialog_actions().child(self.button(
                        "room-clear",
                        "Chatverlauf für alle leeren",
                        ButtonStyle::Danger,
                        self.room_available(RoomSetting::ClearHistory),
                        LocalAction::Room(Action::Confirm(ConversationAction::ClearHistory)),
                        cx,
                    ))));
            }
            Tab::Permissions => {
                let defaults =
                    permission_defaults(d.default_permissions, d.server_default_permissions);
                let mut rights = self
                    .dialog_section(Some("Standardrechte"))
                    .child(self.hint(self.l("Serverrechte für Beitritt, Start und Medien. Gerätefreigaben des Betriebssystems sind davon unabhängig.")))
                    .child(self.hint(self.l("Änderungen an Standardrechten setzen individuelle Ausnahmen zurück.")));
                if defaults.is_none() {
                    rights = rights.child(self.hint(self.l("Die Standardrechte des Servers sind nicht bekannt. Neu laden, bevor du Rechte änderst.")));
                } else if d.default_permissions == Some(0) {
                    rights = rights
                        .child(self.hint(self.l("Aktuell gelten die Standardrechte des Servers.")));
                }
                let mut rows = div().flex().flex_col();
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
                    rows = rows.child(self.dialog_row(
                        label,
                        None,
                        self.switch(
                            ("room-permission", i),
                            label,
                            on,
                            available,
                            LocalAction::Room(Action::Confirm(
                                ConversationAction::SetDefaultPermissions(changed.unwrap_or(0)),
                            )),
                            cx,
                        ),
                    ));
                }
                let mut bans = self.dialog_section(None).child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(self.room_heading("Gesperrte Personen"))
                        .child(self.room_button(
                            "room-load-bans",
                            "Neu laden",
                            RoomSetting::Bans,
                            ConversationAction::Bans,
                            cx,
                        )),
                );
                for ban in panel.bans.as_slice() {
                    bans = bans.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .py_2()
                            .border_b_1()
                            .border_color(t.border)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
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
                    bans = bans.child(self.hint(self.l("Keine gesperrten Personen.")));
                }
                page = page.child(rights.child(rows)).child(bans);
            }
            Tab::Meeting => {
                page = page
                    .child(
                        self.dialog_section(Some("Mitglieder"))
                            .child(self.room_toggle(
                                "room-lobby",
                                "Warteraum",
                                "Gäste warten auf die Freigabe durch die Moderation.",
                                d.lobby,
                                RoomSetting::Lobby,
                                ConversationAction::SetLobby(d.lobby != Some(true)),
                                cx,
                            ))
                            .child(self.dialog_actions().child(self.button_with_leading_icon(
                                "room-manage-roles",
                                ("Mitglieder und Rollen", "users"),
                                ButtonStyle::Secondary,
                                self.room_available(RoomSetting::Members),
                                LocalAction::Open(Overlay::Participants),
                                cx,
                            ))),
                    )
                    .child(
                        self.dialog_section(Some("Gäste"))
                            .child(self.room_toggle(
                                "room-guest-access",
                                "Gastzugang",
                                "Öffentlicher Zugang ist unabhängig von der Auffindbarkeit.",
                                d.guest_access,
                                RoomSetting::GuestAccess,
                                ConversationAction::SetGuestAccess(d.guest_access != Some(true)),
                                cx,
                            ))
                            .child(
                                self.dialog_actions()
                                    .child(self.button_with_leading_icon(
                                        "room-guest-link",
                                        ("Gästelink anzeigen", "link"),
                                        ButtonStyle::Secondary,
                                        Availability::when(
                                            d.guest_access == Some(true),
                                            "Gastzugang ist nicht aktiviert.",
                                        ),
                                        LocalAction::Room(Action::Command(
                                            ConversationAction::GuestLink,
                                        )),
                                        cx,
                                    ))
                                    .when(panel.guest_link.is_some(), |d2| {
                                        d2.child(self.button_with_leading_icon(
                                            "copy-guest-link",
                                            ("Gästelink kopieren", "copy"),
                                            ButtonStyle::Quiet,
                                            Availability::when(
                                                !d.classified && !d.sensitive,
                                                "Weitergeben ist durch die Gesprächsrichtlinie gesperrt.",
                                            ),
                                            LocalAction::Room(Action::CopyGuestLink),
                                            cx,
                                        ))
                                    }),
                            )
                            .when_some(panel.guest_link.as_ref(), |b, link| {
                                b.child(self.hint(link.as_str()))
                            })
                            .child(
                                self.room_field(
                                    "Passwort für Gäste",
                                    &self.room_ui.password,
                                    Some(self.button(
                                        "room-set-password",
                                        "Passwort setzen",
                                        ButtonStyle::Secondary,
                                        self.room_available(RoomSetting::Password),
                                        LocalAction::Room(Action::Password),
                                        cx,
                                    )),
                                    cx,
                                )
                                .pt_2(),
                            )
                            .child(self.hint(self.l(if d.has_password == Some(true) {
                                "Ein Passwort ist eingerichtet."
                            } else {
                                "Kein bestätigtes Gastpasswort."
                            }))),
                    )
                    .child(
                        self.dialog_section(Some("Gruppenräume"))
                            .child(self.hint(self.l(
                                "Gruppenräume teilen das Meeting auf. Sie sind keine Verzeichnisgruppen.",
                            )))
                            .child(self.dialog_actions().child(self.room_button(
                                "room-breakouts",
                                "Zwei Gruppenräume vorbereiten",
                                RoomSetting::Breakouts,
                                ConversationAction::ConfigureBreakouts {
                                    amount: 2,
                                    automatic: true,
                                },
                                cx,
                            ))),
                    );
            }
        }
        page
    }
    fn room_input(&self, input: &Entity<TextInput>, cx: &Context<Self>) -> Div {
        self.input_surface(input, cx).flex_1().child(input.clone())
    }

    fn members_page(&self, narrow: bool, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_3().child(self.room_page_header(
            if self.room_ui.bots {
                "Bots"
            } else {
                "Personen"
            },
            narrow,
        ));
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
                .child(self.dialog_actions().child(self.room_button("reload-room-bots","Bots laden",RoomSetting::Bots,ConversationAction::Bots,cx)));
            for bot in panel.bots.as_slice() {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .py_2()
                        .border_b_1()
                        .border_color(self.theme.border)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
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
            .child(self.room_heading("Im Gespräch").pt_3());
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
                    .py_2()
                    .border_b_1()
                    .border_color(self.theme.border)
                    .child(self.content_avatar(
                        talk_core::ui::content::AssetTarget::ParticipantAvatar {
                            context: panel.context,
                            participant: member.id.local,
                        },
                        &person_initials(member.name.as_str()),
                        36.,
                    ))
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
            body = body.child(self.dialog_actions().child(self.room_button(
                "room-load-members",
                "Mitglieder laden",
                RoomSetting::Members,
                ConversationAction::Members,
                cx,
            )));
        }
        body = body
            .child(self.room_heading("Verzeichnis · Einladungsvorschau").pt_3())
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
                .child(self.dialog_actions().child(self.button(
                    "room-csv-preview",
                    "Importvorschau erstellen",
                    ButtonStyle::Secondary,
                    self.room_available(RoomSetting::Invite),
                    LocalAction::Room(Action::CsvPreview),
                    cx,
                )))
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
            .gap_3()
            .child(self.dialog_page_header(verb, None))
            .when_some(self.selected(), |d, row| {
                d.child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .child(row.title.as_str().to_owned()),
                )
            })
            .when_some(person_name, |d, name| {
                d.child(div().font_weight(FontWeight::MEDIUM).child(name.to_owned()))
            })
            .when_some(permission, |d, label| {
                d.child(div().font_weight(FontWeight::MEDIUM).child(label))
            })
            .child(self.hint(self.l(key)))
            .child(
                self.dialog_actions()
                    .pt_2()
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
