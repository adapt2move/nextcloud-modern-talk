//! Compact navigation over bounded, typed conversation projections.
use super::collection::CollectionState;
use super::controls::{Availability, scoped_id};
use super::{ButtonStyle, Desktop, LocalAction, Overlay, SidebarDestination};
use gpui::{Context, Div, FontWeight, MouseButton, div, prelude::*, px};
use talk_core::preferences::navigation::{
    AccountNavigation, NavigationEdit, NavigationSection, RemoteNavigationSupport, SidebarFilter,
};
use talk_core::ui::Decision;
use talk_core::ui::presentation::{
    ContentState, ConversationCategory, ConversationKind, ConversationView, FeatureSupport,
    TagEdit, TagKind,
};

const SECTION_ICON_SIZE: f32 = 14.;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Personal,
    Channels,
    Conferences,
    Huddles,
    Other,
}
impl Section {
    const ALL: [Self; 5] = [
        Self::Personal,
        Self::Channels,
        Self::Conferences,
        Self::Huddles,
        Self::Other,
    ];
    fn native(self) -> NavigationSection {
        match self {
            Self::Personal => NavigationSection::Personal,
            Self::Channels => NavigationSection::Channels,
            Self::Conferences => NavigationSection::Conferences,
            Self::Huddles => NavigationSection::Huddles,
            Self::Other => NavigationSection::Other,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Personal => "Direkte Gespräche",
            Self::Channels => "Kanäle",
            Self::Conferences => "Konferenzen",
            Self::Huddles => "Huddles",
            Self::Other => "Weitere Gespräche",
        }
    }
    fn for_room(row: &ConversationView) -> Self {
        Self::classify(row.kind, row.organization.category)
    }
    fn classify(kind: ConversationKind, category: ConversationCategory) -> Self {
        if matches!(
            kind,
            ConversationKind::Direct | ConversationKind::NoteToSelf
        ) {
            return Self::Personal;
        }
        match category {
            ConversationCategory::Channel => Self::Channels,
            ConversationCategory::Conference(_) => Self::Conferences,
            ConversationCategory::Huddle => Self::Huddles,
            ConversationCategory::Unknown => Self::Other,
        }
    }
}

impl Desktop {
    pub(super) fn active_account_connected(&self) -> bool {
        self.snapshot.accounts.as_slice().iter().any(|a| {
            Some(a.scope) == self.snapshot.active_account
                && matches!(
                    a.connection,
                    talk_core::ui::presentation::AccountConnectionState::Connected
                        | talk_core::ui::presentation::AccountConnectionState::Guest
                )
        })
    }

    pub(super) fn admitted_account_count(&self) -> usize {
        self.snapshot
            .accounts
            .as_slice()
            .iter()
            .filter(|a| {
                a.connection == talk_core::ui::presentation::AccountConnectionState::Connected
            })
            .count()
    }
    pub(super) fn account_layout(&self) -> Option<&AccountNavigation> {
        let key = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|a| Some(a.scope) == self.snapshot.active_account)?
            .navigation_key
            .as_ref()?;
        self.preferences.active.navigation.account(key)
    }
    fn placement(
        &self,
        row: &ConversationView,
    ) -> Option<&talk_core::preferences::navigation::NavigationPlacement> {
        if row.organization.tags != talk_core::ui::presentation::FeatureSupport::Unsupported {
            return None;
        }
        let key = row.navigation_key.as_ref()?;
        self.account_layout()?
            .placements()
            .iter()
            .find(|p| &p.room == key)
    }
    fn section_collapsed(&self, section: Section) -> bool {
        self.account_layout()
            .map_or(self.collapsed_sections[section as usize], |layout| {
                layout.collapsed(section.native())
            })
    }
    pub(super) fn collapse_sidebar_section(&mut self, section: Section, cx: &mut Context<Self>) {
        let collapsed = !self.section_collapsed(section);
        if self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .any(|a| Some(a.scope) == self.snapshot.active_account && a.navigation_key.is_some())
        {
            // Section collapse has no server feature. This is not tag/favorite fallback.
            self.edit_navigation(
                RemoteNavigationSupport::Unsupported,
                NavigationEdit::SetSectionCollapsed {
                    section: section.native(),
                    collapsed,
                },
                cx,
            );
        } else {
            self.collapsed_sections[section as usize] = collapsed;
        }
    }
    pub(super) fn edit_navigation(
        &mut self,
        support: RemoteNavigationSupport,
        edit: NavigationEdit,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|a| Some(a.scope) == self.snapshot.active_account)
            .and_then(|a| a.navigation_key.clone())
        else {
            return;
        };
        let mut values = self.preferences.active.clone();
        match values.navigation.apply(key, support, edit) {
            Ok(()) => self.commit_preferences(values, cx),
            Err(error) => {
                self.feedback = Some(super::feedback::Feedback::error(match error {
                    talk_core::preferences::navigation::NavigationError::Capacity => {
                        "Die lokale Gruppenablage ist voll. Entferne zuerst eine Gruppe oder Zuordnung."
                    }
                    talk_core::preferences::navigation::NavigationError::InvalidGroup => {
                        "Bitte gib einen gültigen Gruppennamen ein."
                    }
                    _ => "Die Gruppierung ist für dieses Konto derzeit nicht verfügbar.",
                }))
            }
        }
    }
    pub(super) fn favorite_value(&self, row: &ConversationView) -> Option<bool> {
        use talk_core::ui::presentation::FeatureSupport;
        match row.organization.favorites {
            FeatureSupport::Unsupported if row.navigation_key.is_some() => Some(
                self.account_layout()
                    .and_then(|a| {
                        a.placements()
                            .iter()
                            .find(|p| Some(&p.room) == row.navigation_key.as_ref())
                    })
                    .and_then(|p| p.favorite)
                    .unwrap_or(false),
            ),
            _ => row.favorite,
        }
    }
    pub(super) fn can_favorite(&self, row: &ConversationView) -> bool {
        use talk_core::ui::{
            Decision,
            presentation::{ContentState, FeatureSupport},
        };
        self.snapshot.conversation_list.content == ContentState::Ready
            && match row.organization.favorites {
                FeatureSupport::Supported => {
                    row.organization.favorite_change == Decision::Allowed && row.favorite.is_some()
                }
                FeatureSupport::Unsupported => {
                    row.navigation_key.is_some()
                        && self
                            .snapshot
                            .accounts
                            .as_slice()
                            .iter()
                            .any(|a| a.scope == row.context.account && a.navigation_key.is_some())
                }
                FeatureSupport::Unknown => false,
            }
    }
    pub(super) fn sidebar(&self, narrow: bool, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let account = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|a| Some(a.scope) == self.snapshot.active_account);
        let mut sidebar = div()
            .relative()
            .flex_none()
            .w(px(260.))
            .when(!narrow, |d| d.h_full())
            .flex()
            .flex_col()
            .px(px(12.))
            .pt(px(14.))
            .pb(px(10.))
            .border_r_1()
            .border_color(t.border)
            .children(
                (!t.reduced_transparency).then(|| super::controls::glass(t.sidebar, t.dark, 0.)),
            )
            .children(
                self.materials
                    .layer(super::controls::Texture::Grain, t.reduced_transparency),
            )
            .child(super::controls::inner_highlight(t, 0.));
        if self
            .preferences
            .active
            .quick_switcher
            .effective(self.admitted_account_count())
            && let Some(account) = account
        {
            sidebar = sidebar.child(
                self.button_with_leading_icon(
                    "accounts",
                    ("Konten", "user"),
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Open(Overlay::Accounts),
                    cx,
                )
                .min_h(px(30.))
                .mb_2()
                .child(
                    div()
                        .min_w_0()
                        .text_ellipsis()
                        .child(self.account_name(account)),
                ),
            );
        }
        sidebar = sidebar
            .child(
                self.button_with_leading_icon(
                    "global-search",
                    ("Suchen", "search"),
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Search,
                    cx,
                )
                .w_full()
                .justify_start()
                .bg(t.subtle)
                .mb_2()
                .child(div().flex_1())
                .child(
                    self.style_tonal_chip(self.hint(if cfg!(target_os = "macos") {
                        "⌘ K"
                    } else {
                        self.l("Strg K")
                    }))
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.border),
                ),
            )
            .child(self.sidebar_destination(
                "nav-inbox",
                "Posteingang",
                "inbox",
                SidebarDestination::Inbox,
                cx,
            ))
            .when(
                self.snapshot.threads.support == FeatureSupport::Supported,
                |d| {
                    d.child(self.sidebar_destination(
                        "nav-threads",
                        "Threads",
                        "chat",
                        SidebarDestination::Threads,
                        cx,
                    ))
                },
            )
            .when(
                self.snapshot
                    .account_reminders
                    .as_ref()
                    .is_some_and(|view| {
                        Some(view.scope) == self.snapshot.active_account
                            && view.support == FeatureSupport::Supported
                    }),
                |sidebar| {
                    sidebar.child(
                        self.button_with_leading_icon(
                            "nav-reminders",
                            ("Erinnerungen", "clock"),
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::Open(Overlay::Reminders),
                            cx,
                        )
                        .w_full()
                        .justify_start(),
                    )
                },
            )
            .child(self.sidebar_destination(
                "nav-archive",
                "Archiv",
                "file",
                SidebarDestination::Archive,
                cx,
            ))
            .when_some(
                self.snapshot
                    .federation_invitations
                    .as_ref()
                    .filter(|view| {
                        Some(view.scope) == self.snapshot.active_account
                            && view.support == FeatureSupport::Supported
                            && !view.invitations.as_slice().is_empty()
                    }),
                |sidebar, view| {
                    sidebar.child(
                        self.button_with_leading_icon(
                            "nav-federation",
                            ("Einladungen", "users"),
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::Open(Overlay::Federation),
                            cx,
                        )
                        .w_full()
                        .justify_start()
                        .child(div().flex_1())
                        .child(self.count_badge(
                            if view.invitations.as_slice().len() > 99 {
                                "99+".to_owned()
                            } else {
                                view.invitations.as_slice().len().to_string()
                            },
                        )),
                    )
                },
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .rounded(px(8.))
                    .when(self.destination == SidebarDestination::Conversations, |d| {
                        d.bg(t.selected)
                    })
                    .child(
                        self.sidebar_destination(
                            "nav-active",
                            "Gespräche",
                            "chat",
                            SidebarDestination::Conversations,
                            cx,
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    .child(div().w(px(1.)).h(px(14.)).bg(t.border).mx_1())
                    .child(
                        self.icon_button(
                            "new-conversation",
                            ("plus", "Neues Gespräch"),
                            ButtonStyle::Quiet,
                            Availability::available().beside_control(),
                            LocalAction::Open(Overlay::Create),
                            cx,
                        )
                        .mr_1(),
                    ),
            );

        let discovery_support = self.snapshot.discover_panel.as_ref().map_or(
            talk_core::ui::presentation::FeatureSupport::Unknown,
            |panel| panel.support,
        );
        let discovery_supported =
            discovery_support == talk_core::ui::presentation::FeatureSupport::Supported;
        sidebar = sidebar.child(
            self.button_with_leading_icon(
                "open-discovery",
                ("Offene Gespräche", "compass"),
                ButtonStyle::Quiet,
                Availability::when(
                    self.active_account_connected() && discovery_supported,
                    if !self.active_account_connected() {
                        "Das Verzeichnis benötigt eine Verbindung."
                    } else if discovery_support
                        == talk_core::ui::presentation::FeatureSupport::Unknown
                    {
                        "Das Verzeichnis wird geprüft."
                    } else {
                        "Das Gesprächsverzeichnis ist auf diesem Server nicht verfügbar."
                    },
                ),
                LocalAction::Open(Overlay::Discover),
                cx,
            )
            .w_full(),
        );

        let filter = self
            .account_layout()
            .map_or(SidebarFilter::All, AccountNavigation::filter);
        let can_filter = account.is_some_and(|a| a.navigation_key.is_some());
        let mentions_known = self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .any(|r| r.unread_mention.is_some());
        sidebar = sidebar.child(
            self.segmented().w_full().mt_2().children(
                [
                    (SidebarFilter::All, "Alle"),
                    (SidebarFilter::Unread, "Ungelesen"),
                    (SidebarFilter::Mentions, "Erwähnungen"),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (value, label))| {
                    self.chip_with_availability(
                        ("sidebar-filter", index),
                        label,
                        (value == filter, index, 3),
                        Availability::when(
                            can_filter
                                && (value != SidebarFilter::Mentions
                                    || mentions_known
                                    || filter == value),
                            if !can_filter {
                                "Filter sind für dieses Konto nicht verfügbar."
                            } else {
                                "Erwähnungen sind noch nicht verfügbar."
                            },
                        ),
                        LocalAction::SidebarFilter(value),
                        cx,
                    )
                    .flex_1()
                    .px_1()
                    .min_w_0()
                    .text_size(self.sp(12.))
                }),
            ),
        );
        let matches_filter = |r: &ConversationView| match filter {
            SidebarFilter::All => true,
            SidebarFilter::Unread => r.unread > 0,
            SidebarFilter::Mentions => r.unread_mention == Some(true),
        };
        let query = self.sidebar_filter.read(cx).content.to_lowercase();
        let archived = self.destination == SidebarDestination::Archive;
        let state = CollectionState::new(
            self.snapshot.conversation_list.content,
            !self.snapshot.conversations.as_slice().is_empty(),
        );
        let rooms = if state.shows_content() {
            self.snapshot.conversations.as_slice()
        } else {
            &[]
        };
        let mut list = div().flex_none().flex().flex_col().pt_1();
        if let Some(scope) = self.snapshot.active_account {
            list = list.child(self.collection_feedback(
                state,
                "retry-conversations",
                LocalAction::Intent(talk_core::ui::presentation::UiIntent::ReloadConversations(
                    scope,
                )),
                cx,
            ));
        }
        let mut visible = 0;
        let mut grouped = Vec::new();
        // Favorites have one explicit group, independent of room category and
        // server tags. Every row is rendered once so focus identity is stable.
        let favorites: Vec<_> = rooms
            .iter()
            .filter(|r| {
                r.archived == Some(archived)
                    && matches_filter(r)
                    && self.favorite_value(r) == Some(true)
                    && r.title.as_str().to_lowercase().contains(&query)
            })
            .collect();
        if !favorites.is_empty() {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .min_h(px(32.))
                    .mt_2()
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(self.sp(12.))
                    .text_color(t.sidebar_secondary)
                    .child(self.scaled_icon("star-filled", SECTION_ICON_SIZE, t.accent))
                    .child(self.l("Favoriten")),
            );
            visible += favorites.len();
            for row in favorites {
                grouped.push(row.context);
                list = list.child(self.compact_room(row, "room", cx));
            }
        }
        if self.snapshot.tag_state == ContentState::Ready {
            let mut tags: Vec<_> = self.snapshot.tags.as_slice().iter().collect();
            tags.sort_by_key(|t| t.order);
            for tag in tags {
                if tag.kind == TagKind::Unknown {
                    continue;
                }
                let rows: Vec<_> = rooms
                    .iter()
                    .filter(|r| {
                        r.archived == Some(archived)
                            && matches_filter(r)
                            && !grouped.contains(&r.context)
                            && r.title.as_str().to_lowercase().contains(&query)
                            && r.organization.tags == FeatureSupport::Supported
                            && r.tags
                                .as_ref()
                                .is_some_and(|ids| ids.as_slice().contains(&tag.id))
                    })
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                visible += rows.len();
                grouped.extend(rows.iter().map(|r| r.context));
                list = list.child(
                    self.button(
                        gpui::SharedString::from(format!("sidebar-tag-{}", tag.id.local)),
                        "",
                        ButtonStyle::Quiet,
                        rooms
                            .iter()
                            .any(|r| r.organization.tag_change == talk_core::ui::Decision::Allowed),
                        LocalAction::EditTag(TagEdit::Collapse {
                            tag: tag.id,
                            collapsed: !tag.collapsed,
                        }),
                        cx,
                    )
                    .justify_start()
                    .gap_2()
                    .mt_2()
                    .min_h(px(32.))
                    .py_1()
                    .px_2()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.sidebar_secondary)
                    .text_size(self.sp(12.))
                    .child(self.scaled_icon(
                        if tag.collapsed && query.is_empty() {
                            "chevron-left"
                        } else {
                            "chevron-down"
                        },
                        SECTION_ICON_SIZE,
                        t.muted,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .child(tag.name.as_str().to_owned()),
                    ),
                );
                if !tag.collapsed || !query.is_empty() {
                    for row in rows {
                        list = list.child(self.compact_room(row, "room", cx));
                    }
                }
            }
        }
        if let Some(layout) = self.account_layout() {
            let mut groups: Vec<_> = layout.groups().iter().collect();
            groups.sort_by_key(|g| g.order);
            for group in groups {
                let mut rows: Vec<_> = rooms
                    .iter()
                    .filter(|r| {
                        r.archived == Some(archived)
                            && matches_filter(r)
                            && !grouped.contains(&r.context)
                            && r.title.as_str().to_lowercase().contains(&query)
                            && self.placement(r).is_some_and(|p| p.group == Some(group.id))
                    })
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                rows.sort_by_key(|r| self.placement(r).map_or(0, |p| p.order));
                visible += rows.len();
                let mut changed = group.clone();
                changed.collapsed = !changed.collapsed;
                let scope = rows[0].context.account;
                list = list.child(
                    self.button(
                        gpui::SharedString::from(format!("sidebar-group-{}", group.id.get())),
                        "",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::EditGroup(scope, NavigationEdit::UpsertGroup(changed)),
                        cx,
                    )
                    .justify_start()
                    .gap_2()
                    .mt_2()
                    .min_h(px(32.))
                    .py_1()
                    .px_2()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.sidebar_secondary)
                    .text_size(self.sp(12.))
                    .child(self.scaled_icon(
                        if group.collapsed && query.is_empty() {
                            "chevron-left"
                        } else {
                            "chevron-down"
                        },
                        SECTION_ICON_SIZE,
                        t.muted,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .child(group.name.as_str().to_owned()),
                    ),
                );
                if !group.collapsed || !query.is_empty() {
                    for row in rows {
                        list = list.child(self.compact_room(row, "room", cx));
                    }
                }
            }
        }
        for section in Section::ALL {
            let mut rows: Vec<_> = rooms
                .iter()
                .filter(|r| {
                    r.archived == Some(archived)
                        && matches_filter(r)
                        && !grouped.contains(&r.context)
                        && Section::for_room(r) == section
                        && self.placement(r).is_none_or(|p| p.group.is_none())
                        && r.title.as_str().to_lowercase().contains(&query)
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            rows.sort_by_key(|r| self.placement(r).map_or(0, |p| p.order));
            visible += rows.len();
            let collapsed = self.section_collapsed(section) && query.is_empty();
            list = list.child(
                self.button(
                    ("sidebar-section", section as usize),
                    "",
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::CollapseSection(section),
                    cx,
                )
                .justify_start()
                .gap_2()
                .mt_2()
                .min_h(px(32.))
                .py_1()
                .px_2()
                .font_weight(FontWeight::MEDIUM)
                .text_size(self.sp(12.))
                .text_color(t.sidebar_secondary)
                .child(self.scaled_icon(
                    if collapsed {
                        "chevron-left"
                    } else {
                        "chevron-down"
                    },
                    SECTION_ICON_SIZE,
                    t.muted,
                ))
                .child(self.l(section.key())),
            );
            if !collapsed {
                for row in rows {
                    list = list.child(self.compact_room(row, "room", cx));
                }
            }
        }
        if filter == SidebarFilter::Mentions && rooms.iter().any(|r| r.unread_mention.is_none()) {
            list = list.child(self.hint(self.l("Einige Erwähnungen sind noch nicht bekannt.")));
        }
        if visible == 0 && self.snapshot.conversation_list.content == ContentState::Ready {
            list = list.child(div().px_2().py_3().child(self.hint(self.l(if archived {
                "Keine archivierten Gespräche geladen."
            } else {
                "Keine Gespräche in diesem Filter."
            }))));
        }
        if rooms.iter().any(|r| r.archived.is_none()) {
            list = list.child(self.hint(self.l("Archivstatus noch nicht bekannt.")));
        }
        if self.snapshot.conversation_list.content == ContentState::Ready
            && !self.snapshot.conversation_list.complete
            && self.active_account_connected()
        {
            list = list
                .child(self.hint(self.l("Die Gesprächsliste ist noch nicht vollständig geladen.")));
        }
        sidebar = if narrow {
            sidebar.child(list)
        } else {
            sidebar.child(
                self.scroll_area(super::navigation::Region::Rooms)
                    .flex_1()
                    .min_h_0()
                    .child(list),
            )
        };
        let profile_id: gpui::ElementId = "own-status-menu".into();
        let profile = self
            .control(
                profile_id.clone(),
                ButtonStyle::Quiet,
                Availability::when(account.is_some(), "Noch kein Konto verbunden.").hinted(),
                LocalAction::ToggleOwnStatus,
                cx,
            )
            .flex_1()
            .min_w_0()
            .justify_start()
            .gap_2()
            .p_0()
            .when_some(account, |d, a| {
                let avatar = self.content_avatar(
                    talk_core::ui::content::AssetTarget::SelfAvatar,
                    a.initials.as_str(),
                    32.,
                );
                d.child(self.avatar_with_presence(avatar, self.own_presence(), 32., t.sidebar))
            })
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_start()
                    .child(
                        div()
                            .w_full()
                            .text_size(self.sp(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_ellipsis()
                            .child(account.map_or_else(
                                || self.l("Noch kein Konto verbunden.").to_owned(),
                                |a| {
                                    self.snapshot
                                        .content_panel
                                        .self_name
                                        .as_ref()
                                        .filter(|name| {
                                            self.snapshot.content_panel.account == Some(a.scope)
                                                && !name.as_str().trim().is_empty()
                                        })
                                        .map_or(a.self_name.as_str(), |name| name.as_str())
                                        .to_owned()
                                },
                            )),
                    )
                    .when_some(
                        account.and_then(|a| {
                            self.snapshot
                                .content_panel
                                .organization
                                .as_ref()
                                .filter(|name| {
                                    self.snapshot.content_panel.account == Some(a.scope)
                                        && !name.as_str().trim().is_empty()
                                })
                        }),
                        |d, organization| {
                            d.child(
                                div()
                                    .w_full()
                                    .text_size(self.sp(12.))
                                    .text_color(t.sidebar_secondary)
                                    .text_ellipsis()
                                    .child(organization.as_str().to_owned()),
                            )
                        },
                    ),
            )
            .children(
                account.map(|a| self.value_tip(profile_id, a.server_label.as_str().to_owned(), cx)),
            );
        let sidebar = sidebar.when(
            self.docked_call()
                && !matches!(self.overlay, Some(Overlay::Settings | Overlay::Accounts)),
            |sidebar| sidebar.child(self.call_dock(cx)),
        );
        sidebar.child(
            div()
                .relative()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .pt_2()
                .mt_2()
                .border_t_1()
                .border_color(t.border)
                .child(profile)
                .child(self.icon_button(
                    "settings",
                    ("settings", "Einstellungen"),
                    ButtonStyle::Quiet,
                    Availability::available(),
                    LocalAction::Open(Overlay::Settings),
                    cx,
                ))
                .when(self.own_status_open, |d| d.child(self.own_status_menu(cx))),
        )
    }

    fn sidebar_destination(
        &self,
        id: &'static str,
        label: &'static str,
        icon: &'static str,
        destination: SidebarDestination,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let selected = self.destination == destination;
        self.button_with_leading_icon(
            id,
            (label, icon),
            ButtonStyle::Quiet,
            true,
            LocalAction::Destination(destination),
            cx,
        )
        .flex_none()
        .w_full()
        .justify_start()
        .min_h(px(32.))
        .py_1()
        .px_2()
        .text_size(self.sp(13.))
        .when(selected, |d| {
            d.bg(self.theme.selected)
                .text_color(self.theme.selected_text)
        })
    }

    fn compact_room(
        &self,
        row: &ConversationView,
        namespace: &'static str,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let t = self.theme;
        let context = row.context;
        let selected = Some(context) == self.snapshot.selected
            && self.destination == SidebarDestination::Conversations;
        let id = scoped_id(namespace, context, 0, 0);
        let row_focus = self.control_focus(id.clone(), cx);
        let avatar = if let Some(avatar) = self.projected_avatar(
            talk_core::ui::content::AssetTarget::ConversationAvatar(row.context),
            22.,
        ) {
            avatar
        } else if row.kind == ConversationKind::Direct {
            self.person_avatar(row.initials.as_str(), 22.)
        } else {
            div()
                .flex_none()
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .child(self.scaled_icon(
                    if row.organization.classified == Some(true) {
                        "lock"
                    } else {
                        match (row.kind, row.organization.category) {
                            (ConversationKind::NoteToSelf, _) => "file",
                            (_, ConversationCategory::Channel) => "megaphone",
                            (_, ConversationCategory::Conference(_)) => "video",
                            (_, ConversationCategory::Huddle) => "headphones",
                            _ => "chat",
                        }
                    },
                    16.,
                    t.sidebar_secondary,
                ))
        };
        let avatar = if row.kind == ConversationKind::Direct {
            self.avatar_with_presence(
                avatar,
                row.presence,
                22.,
                if selected { t.selected } else { t.sidebar },
            )
        } else {
            avatar
        };
        let can_mark_unread =
            self.message_feature_enabled(talk_core::ui::messaging::MessageFeature::MarkUnread);
        let menu_open = self.room_menu == Some((context, namespace));
        let control = self
            .style_list_row(
                self.button(
                    id,
                    "",
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Navigate(context),
                    cx,
                ),
                super::controls::RowSize::Compact,
            )
            .flex_none()
            .w_full()
            .justify_start()
            .gap_2()
            .px_2()
            .py_1()
            .min_h(self.sp(20.) + px(12.))
            .when(selected, |d| d.bg(t.selected).text_color(t.selected_text))
            .child(avatar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(self.sp(13.))
                    .font_weight(if row.unread > 0 {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_ellipsis()
                    .child(row.title.as_str().to_owned()),
            )
            .when(selected && !self.typing_people(context).is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(self.sp(13.))
                        .text_color(t.muted)
                        .child("…"),
                )
            })
            .when(
                row.kind == ConversationKind::Direct && row.organization.classified == Some(true),
                |d| d.child(self.scaled_icon("lock", 14., t.muted)),
            )
            .when(row.call_active == Some(true), |d| {
                let mut badge = d.child(self.status_icon(
                    scoped_id(namespace, context, 0, 2),
                    ("headphones", "Anruf läuft"),
                    (14., t.success),
                    row_focus.clone(),
                    cx,
                ));
                // Only the currently owned call has authoritative, scoped
                // participant identities; other active rooms retain the badge.
                if let Some(call) = self.snapshot.call.as_ref().filter(|call| {
                    !self.docked_call()
                        && call.scope.account == context.account
                        && call.scope.conversation == context.conversation
                        && matches!(call.stage, talk_core::ui::presentation::CallStage::Active)
                }) {
                    let mut avatars = div()
                        .relative()
                        .group_hover("sidebar-room", |style| style.right(px(6.)))
                        .flex()
                        .items_center()
                        .gap_1();
                    for person in call.participants.as_slice().iter().take(2) {
                        avatars = avatars.child(self.person_avatar(person.initials.as_str(), 18.));
                    }
                    badge = badge.child(avatars);
                }
                badge
            })
            .when(row.notification_muted == Some(true), |d| {
                d.child(self.status_icon(
                    scoped_id(namespace, context, 0, 4),
                    ("bell-off", "Nachrichten stummgeschaltet"),
                    (14., t.muted),
                    row_focus.clone(),
                    cx,
                ))
            })
            .when(self.favorite_value(row) == Some(true), |d| {
                d.child(self.status_icon(
                    scoped_id(namespace, context, 0, 1),
                    ("star-filled", "Favorit"),
                    (14., t.accent),
                    row_focus,
                    cx,
                ))
            })
            .when(row.unread > 0, |d| {
                let badge = if row.unread_direct_mention == Some(true)
                    || matches!(
                        row.kind,
                        ConversationKind::Direct | ConversationKind::NoteToSelf
                    ) {
                    self.count_badge(format!(
                        "{}{}",
                        if row.unread_direct_mention == Some(true) {
                            "@ "
                        } else {
                            ""
                        },
                        if row.unread > 99 {
                            "99+".to_owned()
                        } else {
                            row.unread.to_string()
                        }
                    ))
                } else {
                    div().flex_none().size(px(6.)).rounded_full().bg(t.accent)
                };
                d.child(self.unread_badge(scoped_id(namespace, context, 0, 8), context, badge))
            });
        // One small indent is enough to show group membership. The list shares
        // the top-level gutter; stacked container/row insets waste that space.
        let entry = super::state::call_entry(row, self.snapshot.content);
        let can_join = row.call_active == Some(true) && entry.is_some();
        let joining = self
            .snapshot
            .discover_panel
            .as_ref()
            .is_some_and(|panel| panel.joining == Some(context) && panel.state.pending());
        div()
            .id(scoped_id(namespace, context, 1, 0))
            .flex_none()
            .flex()
            .flex_col()
            .when(namespace == "room", |d| d.pl(px(4.)))
            .when(menu_open, |d| {
                d.on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                    if this.room_menu == Some((context, namespace)) {
                        this.room_menu = None;
                        cx.notify();
                    }
                }))
            })
            .child(
                div()
                    .relative()
                    .group("sidebar-room")
                    .flex()
                    .items_center()
                    .when(!joining, |d| {
                        d.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, window, cx| {
                                this.handle(
                                    LocalAction::ToggleRoomMenu(context, namespace),
                                    window,
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        )
                    })
                    .child(control.flex_1().min_w_0())
                    .when_some(row.call_scope.filter(|_| can_join), |d, scope| {
                        let needs_navigation =
                            matches!(entry, Some(super::state::CallEntry::Conference(_)))
                                && self.snapshot.selected != Some(context);
                        d.child(
                            self.button(
                                scoped_id(namespace, context, 0, 3),
                                if needs_navigation {
                                    "Öffnen"
                                } else {
                                    "Beitreten"
                                },
                                ButtonStyle::Quiet,
                                true,
                                LocalAction::JoinRoomCall(scope),
                                cx,
                            )
                            .absolute()
                            .right(px(30.))
                            .top(px(0.))
                            .text_size(self.sp(12.))
                            .px_1()
                            .bg(if selected { t.selected } else { t.sidebar })
                            .opacity(0.)
                            .group_hover("sidebar-room", |s| s.opacity(1.))
                            .focus(|s| s.opacity(1.)),
                        )
                    })
                    .when(!joining, |row| {
                        row.child(
                            self.icon_button(
                                scoped_id(namespace, context, 1, 1),
                                ("more", "Weitere Optionen"),
                                ButtonStyle::Quiet,
                                Availability::available(),
                                LocalAction::ToggleRoomMenu(context, namespace),
                                cx,
                            )
                            .size(px(28.))
                            .absolute()
                            .right_0()
                            .bg(if selected { t.selected } else { t.sidebar })
                            .when(!menu_open && !self.keyboard_navigation, |d| d.opacity(0.))
                            .group_hover("sidebar-room", |d| d.opacity(1.)),
                        )
                    }),
            )
            .when(menu_open && !joining, |container| {
                container.child(
                    self.context_menu_surface()
                        .ml_2()
                        .child(
                            self.menu_item(
                                scoped_id("sidebar-room-notifications", context, 0, 0),
                                ("Benachrichtigungen", "bell"),
                                Availability::available(),
                                LocalAction::SidebarRoomNotifications(context),
                                cx,
                            )
                            .w_full(),
                        )
                        .child(
                            self.menu_item(
                                scoped_id("sidebar-room-archive", context, 0, 0),
                                (
                                    if row.archived == Some(true) {
                                        "Aus Archiv holen"
                                    } else {
                                        "Archivieren"
                                    },
                                    "inbox",
                                ),
                                Availability::when(
                                    row.archive == Decision::Allowed && row.archived.is_some(),
                                    "Der Archivstatus kann derzeit nicht geändert werden.",
                                ),
                                LocalAction::Archive(context, row.archived != Some(true)),
                                cx,
                            )
                            .w_full(),
                        )
                        .child(
                            self.menu_item(
                                scoped_id(namespace, context, 1, 2),
                                ("Als ungelesen markieren", "inbox"),
                                Availability::when(
                                    can_mark_unread,
                                    "Auf diesem Server nicht verfügbar.",
                                ),
                                LocalAction::MarkRoomUnread(context),
                                cx,
                            )
                            .justify_start()
                            .w_full(),
                        ),
                )
            })
    }

    pub(super) fn inbox(&self, cx: &Context<Self>) -> Div {
        let mut page = div()
            .flex()
            .flex_col()
            .p_5()
            .gap_3()
            .child(self.title(self.l("Posteingang")))
            .child(self.hint(self.l("Ungelesene Gespräche in diesem Konto.")));
        let state = CollectionState::new(
            self.snapshot.conversation_list.content,
            !self.snapshot.conversations.as_slice().is_empty(),
        );
        let mut rows = div().flex().flex_col().gap_1();
        let mut count = 0;
        for row in self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .filter(|r| state.shows_content() && r.unread > 0 && r.archived == Some(false))
        {
            rows = rows.child(self.compact_room(row, "inbox-room", cx));
            count += 1;
        }
        if count == 0 && self.snapshot.conversation_list.content == ContentState::Ready {
            rows = rows.child(self.hint(self.l("Keine ungelesenen Gespräche geladen.")));
        }
        if self.snapshot.conversation_list.content == ContentState::Ready
            && !self.snapshot.conversation_list.complete
            && self.active_account_connected()
        {
            rows = rows
                .child(self.hint(self.l("Die Gesprächsliste ist noch nicht vollständig geladen.")));
        }
        if let Some(scope) = self.snapshot.active_account {
            page = page.child(self.collection_feedback(
                state,
                "retry-inbox",
                LocalAction::Intent(talk_core::ui::presentation::UiIntent::ReloadConversations(
                    scope,
                )),
                cx,
            ));
        }
        page = page.child(
            self.scroll_area(super::navigation::Region::Conversation)
                .flex_1()
                .min_h_0()
                .child(rows),
        );
        self.content_card(page)
    }

    pub(super) fn threads_page(&self, cx: &Context<Self>) -> Div {
        let mut rows = div().flex().flex_col().gap_1();
        for thread in self.snapshot.threads.entries.as_slice() {
            let Some(room) = self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|room| room.context == thread.context)
            else {
                continue;
            };
            let unread = thread_unread(
                room.last_read_message,
                room.unread,
                thread.root,
                thread.replies,
                thread.last_message,
            ) == Some(true);
            rows = rows.child(
                self.control(
                    scoped_id("subscribed-thread", thread.context, thread.root, 0),
                    ButtonStyle::Quiet,
                    Availability::available(),
                    LocalAction::OpenSubscribedThread(thread.context, thread.root),
                    cx,
                )
                .w_full()
                .justify_start()
                .p_3()
                .gap_3()
                .child(div().flex_none().size(px(8.)).rounded_full().bg(if unread {
                    self.theme.accent
                } else {
                    gpui::transparent_black()
                }))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_start()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(if unread {
                                    FontWeight::BOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .child(thread.title.as_str().to_owned()),
                        )
                        .child(self.hint(&format!(
                            "{} · {}",
                            room.title.as_str(),
                            self.count(super::localization::Count::Replies, thread.replies.into())
                        ))),
                )
                .when(unread, |row| row.child(self.hint(self.l("Ungelesen")))),
            );
        }
        if self.snapshot.threads.content == ContentState::Ready
            && self.snapshot.threads.entries.as_slice().is_empty()
        {
            let label = if self.snapshot.threads.offset > 0 || self.snapshot.threads.has_next {
                "Keine Threads auf dieser Seite."
            } else {
                "Keine abonnierten Threads."
            };
            rows = rows.child(self.hint(self.l(label)));
        }
        if matches!(
            self.snapshot.threads.content,
            ContentState::Failed | ContentState::Denied
        ) {
            rows = rows.child(self.hint(self.l("Threads konnten nicht geladen werden.")));
        }
        let mut heading = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().child(self.title(self.l("Threads"))));
        if let Some(account) = self.snapshot.active_account {
            heading = heading.child(self.icon_button(
                "refresh-threads",
                ("refresh", "Threads aktualisieren"),
                ButtonStyle::Quiet,
                Availability::when(self.snapshot.threads.content != ContentState::Loading, ""),
                LocalAction::Intent(talk_core::ui::presentation::UiIntent::RefreshThreads(
                    account,
                )),
                cx,
            ));
        }
        let mut page = div().flex().flex_col().p_5().gap_3().child(heading);
        if let Some(account) = self.snapshot.active_account {
            let list = &self.snapshot.threads;
            if list.offset > 0 || list.has_next {
                let ready = list.content != ContentState::Loading;
                page = page.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(self.button(
                            "threads-previous",
                            "Vorherige Threads",
                            ButtonStyle::Quiet,
                            ready && list.offset >= 50,
                            LocalAction::Intent(
                                talk_core::ui::presentation::UiIntent::PageThreads {
                                    account,
                                    offset: list.offset.saturating_sub(50),
                                },
                            ),
                            cx,
                        ))
                        .child(self.button(
                            "threads-next",
                            "Weitere Threads",
                            ButtonStyle::Quiet,
                            ready && list.has_next && list.offset.checked_add(50).is_some(),
                            LocalAction::Intent(
                                talk_core::ui::presentation::UiIntent::PageThreads {
                                    account,
                                    offset: list.offset.saturating_add(50),
                                },
                            ),
                            cx,
                        )),
                );
            }
        }
        self.content_card(
            page.child(
                self.scroll_area(super::navigation::Region::Conversation)
                    .flex_1()
                    .min_h_0()
                    .child(rows),
            ),
        )
    }
}

fn thread_unread(
    marker: Option<u128>,
    unread: u32,
    root: u128,
    replies: u32,
    last_message: Option<u128>,
) -> Option<bool> {
    let marker = marker?;
    let latest = last_message.or_else(|| (replies == 0).then_some(root))?;
    Some(unread > 0 && latest > marker)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unread_thread_requires_a_known_room_marker_and_recent_message() {
        assert_eq!(thread_unread(Some(40), 1, 5, 2, Some(41)), Some(true));
        assert_eq!(thread_unread(Some(41), 1, 5, 2, Some(41)), Some(false));
        assert_eq!(thread_unread(None, 1, 5, 2, Some(41)), None);
        assert_eq!(thread_unread(Some(40), 1, 5, 2, None), None);
        assert_eq!(thread_unread(Some(40), 1, 41, 0, None), Some(true));
    }
    #[test]
    fn direct_conversations_stay_first_and_titles_cannot_grant_a_category() {
        assert_eq!(
            Section::classify(ConversationKind::Group, ConversationCategory::Unknown),
            Section::Other
        );
        assert_eq!(
            Section::classify(ConversationKind::Direct, ConversationCategory::Channel),
            Section::Personal
        );
        assert_eq!(
            Section::classify(ConversationKind::NoteToSelf, ConversationCategory::Huddle),
            Section::Personal
        );
        assert_eq!(
            Section::classify(ConversationKind::Group, ConversationCategory::Channel),
            Section::Channels
        );
    }
}
