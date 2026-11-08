//! Compact navigation over bounded, typed conversation projections.
use super::collection::CollectionState;
use super::controls::{Availability, BannerRole, scoped_id};
use super::{ButtonStyle, Desktop, LocalAction, Overlay, SidebarDestination};
use gpui::{Context, Div, FontWeight, MouseButton, div, prelude::*, px};
use talk_core::preferences::navigation::{
    AccountNavigation, NavigationEdit, NavigationSection, RemoteNavigationSupport, SidebarFilter,
};
use talk_core::ui::Decision;
use talk_core::ui::presentation::UiIntent;
use talk_core::ui::presentation::{
    ContentState, ConversationCategory, ConversationKind, ConversationRef, ConversationView,
    FeatureSupport, TagEdit, TagKind,
};

const SECTION_ICON_SIZE: f32 = 14.;

/// Filters inside the single Activity destination.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Activity {
    #[default]
    Unread,
    Mentions,
    Threads,
    Reminders,
}

/// The two list-header menus. Opening one closes the other.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Menu {
    /// Start something: new conversation, browse open conversations.
    New,
    /// Change the list: visibility filter and the archive.
    List,
}

/// Initial navigation state for explicit UI-development scenarios only.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Fixture {
    #[default]
    None,
    Activity(Activity),
    Archive,
    Menu(Menu),
}
impl Fixture {
    pub fn for_scenario(scenario: Option<&str>) -> Self {
        match scenario {
            Some("sidebar-activity") => Self::Activity(Activity::Unread),
            Some("sidebar-activity-mentions") => Self::Activity(Activity::Mentions),
            Some("thread-list") => Self::Activity(Activity::Threads),
            Some("account-reminders" | "account-reminders-partial") => {
                Self::Activity(Activity::Reminders)
            }
            Some("sidebar-archive") => Self::Archive,
            Some("sidebar-new-menu") => Self::Menu(Menu::New),
            Some("sidebar-list-menu") => Self::Menu(Menu::List),
            _ => Self::None,
        }
    }
    pub(super) fn destination(self) -> SidebarDestination {
        match self {
            Self::Activity(_) => SidebarDestination::Activity,
            Self::Archive => SidebarDestination::Archive,
            _ => SidebarDestination::Conversations,
        }
    }
    pub(super) fn activity(self) -> Activity {
        match self {
            Self::Activity(activity) => activity,
            _ => Activity::default(),
        }
    }
    pub(super) fn menu(self) -> Option<Menu> {
        match self {
            Self::Menu(menu) => Some(menu),
            _ => None,
        }
    }
}

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
                    row.organization.favorite_change != Decision::Denied && row.favorite.is_some()
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
                FeatureSupport::Unknown => row.navigation_key.is_some() && row.favorite.is_some(),
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
        let filter = self
            .account_layout()
            .map_or(SidebarFilter::All, AccountNavigation::filter);
        let can_filter = account.is_some_and(|a| a.navigation_key.is_some());
        let archived = self.destination == SidebarDestination::Archive;
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
                .mb_1()
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
            .child(self.activity_entry(cx))
            .child(self.list_header(archived, filter, can_filter, cx))
            .when(filter != SidebarFilter::All && !archived, |d| {
                d.child(self.filter_chip(filter, cx))
            });

        let matches_filter = |r: &ConversationView| match filter {
            SidebarFilter::All => true,
            SidebarFilter::Unread => r.unread > 0,
            SidebarFilter::Mentions => r.unread_mention == Some(true),
        };
        let query = self.sidebar_filter.read(cx).content.to_lowercase();
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
                    .min_h(px(28.))
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
                list = list.child(self.sidebar_room(row, cx));
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
                    self.group_header(
                        gpui::SharedString::from(format!("sidebar-tag-{}", tag.id.local)),
                        tag.name.as_str().to_owned(),
                        (
                            tag.collapsed && query.is_empty(),
                            rows.iter().filter(|r| r.unread > 0).count(),
                        ),
                        rooms
                            .iter()
                            .any(|r| r.organization.tag_change == talk_core::ui::Decision::Allowed),
                        LocalAction::EditTag(TagEdit::Collapse {
                            tag: tag.id,
                            collapsed: !tag.collapsed,
                        }),
                        cx,
                    ),
                );
                if !tag.collapsed || !query.is_empty() {
                    for row in rows {
                        list = list.child(self.sidebar_room(row, cx));
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
                list = list.child(self.group_header(
                    gpui::SharedString::from(format!("sidebar-group-{}", group.id.get())),
                    group.name.as_str().to_owned(),
                    (
                        group.collapsed && query.is_empty(),
                        rows.iter().filter(|r| r.unread > 0).count(),
                    ),
                    true,
                    LocalAction::EditGroup(scope, NavigationEdit::UpsertGroup(changed)),
                    cx,
                ));
                if !group.collapsed || !query.is_empty() {
                    for row in rows {
                        list = list.child(self.sidebar_room(row, cx));
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
            list = list.child(self.group_header(
                ("sidebar-section", section as usize),
                self.l(section.key()).to_owned(),
                (collapsed, rows.iter().filter(|r| r.unread > 0).count()),
                true,
                LocalAction::CollapseSection(section),
                cx,
            ));
            if !collapsed {
                for row in rows {
                    list = list.child(self.sidebar_room(row, cx));
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

    /// Rooms that currently ask for attention: a direct message, a note to
    /// self or an @-mention with unread messages. This is the same rule that
    /// puts a number on a conversation row, so the two never disagree.
    fn needs_attention(row: &ConversationView) -> bool {
        attention(
            row.kind,
            row.unread,
            row.archived,
            row.unread_direct_mention,
            row.notification_muted,
        )
    }

    fn pending_invitations(&self) -> usize {
        self.snapshot
            .federation_invitations
            .as_ref()
            .filter(|view| {
                Some(view.scope) == self.snapshot.active_account
                    && view.support == FeatureSupport::Supported
            })
            .map_or(0, |view| view.invitations.as_slice().len())
    }

    pub(super) fn activity_count(&self) -> usize {
        let state = CollectionState::new(
            self.snapshot.conversation_list.content,
            !self.snapshot.conversations.as_slice().is_empty(),
        );
        let rooms = if state.shows_content() {
            self.snapshot
                .conversations
                .as_slice()
                .iter()
                .filter(|row| Self::needs_attention(row))
                .count()
        } else {
            0
        };
        rooms + self.pending_invitations()
    }

    /// Entering a filter asks its owner for fresh data, as the former
    /// reminders dialog did. Unread and mentions come from the room list.
    pub(super) fn refresh_activity(&mut self, filter: Activity) {
        let Some(scope) = self.snapshot.active_account else {
            return;
        };
        match filter {
            Activity::Reminders => {
                self.submit(UiIntent::RefreshAccountReminders(scope));
            }
            Activity::Threads | Activity::Unread | Activity::Mentions => {}
        }
        if self.pending_invitations() > 0 || self.snapshot.federation_invitations.is_some() {
            self.submit(UiIntent::RefreshFederationInvitations(scope));
        }
    }

    /// The single top-level destination below search.
    fn activity_entry(&self, cx: &Context<Self>) -> gpui::Stateful<Div> {
        let count = self.activity_count();
        self.sidebar_destination(
            "nav-activity",
            "Aktivität",
            "inbox",
            SidebarDestination::Activity,
            cx,
        )
        .when(count > 0, |d| {
            d.child(div().flex_1())
                .child(self.count_badge(if count > 99 {
                    "99+".to_owned()
                } else {
                    count.to_string()
                }))
        })
    }

    /// "Gespräche" title with the two list-level menus. Everything that is not
    /// a conversation lives behind them; the archive replaces the title with a
    /// way back.
    fn list_header(
        &self,
        archived: bool,
        filter: SidebarFilter,
        can_filter: bool,
        cx: &Context<Self>,
    ) -> Div {
        let t = self.theme;
        let row = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .mt_3()
            .min_h(px(30.))
            .pl_2();
        if archived {
            return row.child(
                self.button_with_leading_icon(
                    "archive-back",
                    ("Archiv", "arrow-left"),
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Destination(SidebarDestination::Conversations),
                    cx,
                )
                .min_h(px(28.))
                .py(px(2.))
                .px_1()
                .gap_1()
                .font_weight(FontWeight::MEDIUM)
                .text_size(self.sp(12.))
                .text_color(t.sidebar_secondary),
            );
        }
        let discovery_support = self
            .snapshot
            .discover_panel
            .as_ref()
            .map_or(FeatureSupport::Unknown, |panel| panel.support);
        let discovery_supported = discovery_support == FeatureSupport::Supported;
        let mentions_known = self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .any(|r| r.unread_mention.is_some());
        let trigger = |menu: Menu, id: &'static str, icon: &'static str, label: &'static str| {
            div().relative().flex_none().child(
                self.icon_button(
                    id,
                    (icon, label),
                    ButtonStyle::Quiet,
                    Availability::available().below_control(),
                    LocalAction::ToggleSidebarMenu(menu),
                    cx,
                )
                .size(px(28.))
                .min_h(px(28.))
                .when(self.sidebar_menu == Some(menu), |d| d.bg(t.hover)),
            )
        };
        let menu_surface = |menu: Menu, body: Div| {
            gpui::deferred(
                gpui::anchored()
                    .anchor(gpui::Corner::TopRight)
                    .offset(gpui::point(px(28.), px(4.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        self.context_menu_surface()
                            .id(match menu {
                                Menu::New => "sidebar-new-menu",
                                Menu::List => "sidebar-list-menu",
                            })
                            .w(px(244.))
                            .occlude()
                            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                                if this.sidebar_menu == Some(menu) {
                                    this.sidebar_menu = None;
                                    cx.notify();
                                }
                            }))
                            .child(body),
                    ),
            )
        };
        let new_menu = div()
            .flex()
            .flex_col()
            .child(self.menu_item(
                "new-conversation",
                ("Neues Gespräch", "plus"),
                Availability::available(),
                LocalAction::Open(Overlay::Create),
                cx,
            ))
            .child(self.menu_item(
                "open-discovery",
                ("Offene Gespräche durchsuchen", "compass"),
                Availability::when(
                    self.active_account_connected() && discovery_supported,
                    if !self.active_account_connected() {
                        "Das Verzeichnis benötigt eine Verbindung."
                    } else if discovery_support == FeatureSupport::Unknown {
                        "Das Verzeichnis wird geprüft."
                    } else {
                        "Das Gesprächsverzeichnis ist auf diesem Server nicht verfügbar."
                    },
                ),
                LocalAction::Open(Overlay::Discover),
                cx,
            ));
        let option = |index: usize, value: SidebarFilter, label: &'static str| {
            let enabled = can_filter
                && (value != SidebarFilter::Mentions || mentions_known || filter == value);
            let selected = value == filter;
            self.style_list_row(
                self.button(
                    ("sidebar-filter", index),
                    label,
                    ButtonStyle::Quiet,
                    Availability::when(
                        enabled,
                        if !can_filter {
                            "Filter sind für dieses Konto nicht verfügbar."
                        } else {
                            "Erwähnungen sind noch nicht verfügbar."
                        },
                    ),
                    LocalAction::SidebarFilter(value),
                    cx,
                ),
                super::controls::RowSize::Regular,
            )
            .w_full()
            .justify_between()
            .when(selected, |d| {
                d.child(self.scaled_icon("check", 16., t.selected_text))
            })
        };
        let list_menu = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb(px(2.))
                    .text_size(self.sp(12.))
                    .text_color(t.muted)
                    .child(self.l("Anzeigen").to_owned()),
            )
            .child(option(0, SidebarFilter::All, "Alle Gespräche"))
            .child(option(1, SidebarFilter::Unread, "Nur Ungelesene"))
            .child(option(2, SidebarFilter::Mentions, "Nur Erwähnungen"))
            .child(div().h(px(1.)).my_1().bg(t.border))
            .child(self.menu_item(
                "nav-archive",
                ("Archiv", "file"),
                Availability::available(),
                LocalAction::Destination(SidebarDestination::Archive),
                cx,
            ));
        row.child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(self.sp(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.sidebar_secondary)
                .child(self.l("Gespräche").to_owned()),
        )
        .child(
            trigger(
                Menu::New,
                "sidebar-new",
                "plus",
                "Neues Gespräch oder Verzeichnis",
            )
            .when(self.sidebar_menu == Some(Menu::New), |d| {
                d.child(menu_surface(Menu::New, new_menu))
            }),
        )
        .child(
            trigger(Menu::List, "sidebar-options", "more", "Liste anpassen")
                .when(self.sidebar_menu == Some(Menu::List), |d| {
                    d.child(menu_surface(Menu::List, list_menu))
                }),
        )
    }

    /// A non-default list filter stays visible and one click from reset.
    fn filter_chip(&self, filter: SidebarFilter, cx: &Context<Self>) -> Div {
        let t = self.theme;
        div().flex_none().flex().px_2().pb_1().child(
            self.button_with_icon(
                "sidebar-filter-reset",
                (
                    match filter {
                        SidebarFilter::Mentions => "Nur Erwähnungen",
                        _ => "Nur Ungelesene",
                    },
                    "x",
                ),
                ButtonStyle::Quiet,
                true,
                LocalAction::SidebarFilter(SidebarFilter::All),
                cx,
            )
            .gap_1()
            .min_h(px(24.))
            .py_0()
            .px_2()
            .rounded_full()
            .bg(t.selected)
            .text_color(t.selected_text)
            .text_size(self.sp(12.)),
        )
    }

    /// Collapsible group heading. A collapsed group still says that something
    /// inside it is unread.
    fn group_header(
        &self,
        id: impl Into<gpui::ElementId>,
        label: impl Into<gpui::SharedString>,
        (collapsed, unread): (bool, usize),
        enabled: bool,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let t = self.theme;
        let chevron = self.scaled_icon("chevron-down", SECTION_ICON_SIZE, t.muted);
        self.button(id, "", ButtonStyle::Quiet, enabled, action, cx)
            .justify_start()
            .gap(px(6.))
            .mt_2()
            .min_h(px(28.))
            .py(px(2.))
            .px_2()
            .font_weight(FontWeight::MEDIUM)
            .text_size(self.sp(12.))
            .text_color(t.sidebar_secondary)
            .child(if collapsed {
                chevron.with_transformation(gpui::Transformation::rotate(gpui::radians(
                    -std::f32::consts::FRAC_PI_2,
                )))
            } else {
                chevron
            })
            .child(div().min_w_0().text_ellipsis().child(label.into()))
            .when(collapsed && unread > 0, |d| {
                d.child(div().flex_1())
                    .child(self.count_badge(if unread > 99 {
                        "99+".to_owned()
                    } else {
                        unread.to_string()
                    }))
            })
    }

    /// Activity: the one place for what needs attention. Filters live inside
    /// it; the conversation list stays quiet.
    pub(super) fn activity_page(&self, cx: &Context<Self>) -> Div {
        let state = CollectionState::new(
            self.snapshot.conversation_list.content,
            !self.snapshot.conversations.as_slice().is_empty(),
        );
        let reminders = self
            .snapshot
            .account_reminders
            .as_ref()
            .is_some_and(|view| {
                Some(view.scope) == self.snapshot.active_account
                    && view.support == FeatureSupport::Supported
            });
        let threads = self.snapshot.threads.support == FeatureSupport::Supported;
        let mentions_known = self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .any(|r| r.unread_mention.is_some());
        let filters: Vec<(Activity, &'static str, bool)> = [
            (Activity::Unread, "Ungelesen", true),
            (Activity::Mentions, "Erwähnungen", mentions_known),
            (Activity::Threads, "Threads", threads),
            (Activity::Reminders, "Erinnerungen", reminders),
        ]
        .into_iter()
        .filter(|(value, _, _)| match value {
            Activity::Threads => threads,
            Activity::Reminders => reminders,
            _ => true,
        })
        .collect();
        // A filter whose feature is unavailable cannot stay selected.
        let selected = if filters.iter().any(|(value, _, _)| *value == self.activity) {
            self.activity
        } else {
            Activity::Unread
        };
        let count = filters.len();
        let mut page = div().flex().flex_col().px_5().pt_4().pb_3().gap_3().child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x_4()
                .gap_y_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(120.))
                        .child(self.title(self.l("Aktivität"))),
                )
                .child(
                    self.segmented()
                        .children(filters.into_iter().enumerate().map(
                            |(index, (value, label, enabled))| {
                                self.chip_with_availability(
                                    ("activity-filter", index),
                                    label,
                                    (value == selected, index, count),
                                    Availability::when(
                                        enabled,
                                        "Erwähnungen sind noch nicht verfügbar.",
                                    ),
                                    LocalAction::ActivityFilter(value),
                                    cx,
                                )
                            },
                        )),
                ),
        );
        if let Some(scope) = self.snapshot.active_account {
            page = page.child(self.collection_feedback(
                state,
                "retry-activity",
                LocalAction::Intent(UiIntent::ReloadConversations(scope)),
                cx,
            ));
        }
        let invitations = self.pending_invitations();
        if invitations > 0 {
            page = page.child(
                self.notice(
                    BannerRole::Info,
                    self.l("Einladungen von anderen Servern"),
                    Some(&self.lf(
                        if invitations == 1 {
                            "{n} Einladung wartet auf deine Antwort."
                        } else {
                            "{n} Einladungen warten auf deine Antwort."
                        },
                        &[("n", &invitations.to_string())],
                    )),
                    Some("users"),
                )
                .child(self.notice_action(
                    "nav-federation",
                    "Ansehen",
                    LocalAction::Open(Overlay::Federation),
                    cx,
                )),
            );
        }
        let body = match selected {
            Activity::Unread | Activity::Mentions => self.activity_rooms(selected, state, cx),
            Activity::Threads => self.threads_body(cx),
            Activity::Reminders => self.account_reminders_body(cx),
        };
        let page = page.child(
            self.scroll_area(super::navigation::Region::Conversation)
                .flex_1()
                .min_h_0()
                .child(body),
        );
        self.content_card(page)
    }

    fn activity_rooms(&self, filter: Activity, state: CollectionState, cx: &Context<Self>) -> Div {
        let mut rows = div().flex().flex_col().gap_1();
        let mut count = 0;
        for row in self.snapshot.conversations.as_slice().iter().filter(|r| {
            state.shows_content()
                && r.archived == Some(false)
                && match filter {
                    Activity::Mentions => r.unread_mention == Some(true) && r.unread > 0,
                    _ => r.unread > 0,
                }
        }) {
            rows = rows.child(self.compact_room(row, "inbox-room", cx));
            count += 1;
        }
        if count == 0 && self.snapshot.conversation_list.content == ContentState::Ready {
            rows = rows.child(self.state_note(
                "check-circle",
                match filter {
                    Activity::Mentions => "Keine neuen Erwähnungen.",
                    _ => "Alles gelesen.",
                },
                match filter {
                    Activity::Mentions => "Wenn dich jemand erwähnt, erscheint es hier.",
                    _ => "Ungelesene Gespräche erscheinen hier.",
                },
            ));
        }
        if filter == Activity::Mentions
            && self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|r| r.unread_mention.is_none())
        {
            rows = rows.child(self.hint(self.l("Einige Erwähnungen sind noch nicht bekannt.")));
        }
        if self.snapshot.conversation_list.content == ContentState::Ready
            && !self.snapshot.conversation_list.complete
            && self.active_account_connected()
        {
            rows = rows
                .child(self.hint(self.l("Die Gesprächsliste ist noch nicht vollständig geladen.")));
        }
        rows
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

    /// Cull expensive row contents against the current scroll mask, without a
    /// retained geometry cache or an estimated scroll position. Offscreen rows
    /// keep their two real controls: native Tab can focus them and the existing
    /// reveal owner scrolls that same stable handle into view. Focused rows and
    /// expanded/variable-height rows always use their complete presentation.
    fn sidebar_room(&self, row: &ConversationView, cx: &Context<Self>) -> gpui::Stateful<Div> {
        let context = row.context;
        let host = div()
            .id(scoped_id("sidebar-row-host", context, 0, 0))
            .w_full()
            .flex_none();
        let joining = self
            .snapshot
            .discover_panel
            .as_ref()
            .is_some_and(|panel| panel.joining == Some(context) && panel.state.pending());
        if self.room_menu == Some((context, "room")) || row.call_active == Some(true) || joining {
            return host.child(self.compact_room(row, "room", cx));
        }
        // The ordinary single-line row uses the main button minimum; the
        // absolute 28-pixel More control does not determine its flow height.
        let height = self.sp(20.) + px(12.);
        host.child(
            gpui::canvas(
                cx.processor(
                    move |this, bounds: gpui::Bounds<gpui::Pixels>, window, cx| {
                        let row = this
                            .snapshot
                            .conversations
                            .as_slice()
                            .iter()
                            .find(|row| row.context == context)?;
                        let focused = [
                            scoped_id("room", context, 0, 0),
                            scoped_id("room", context, 1, 1),
                        ]
                        .into_iter()
                        .filter_map(|id| this.control_focus(id, cx))
                        .any(|focus| focus.contains_focused(window, cx));
                        let visible = bounds.intersects(&window.content_mask().bounds);
                        let mut element = if visible || focused {
                            this.compact_room(row, "room", cx)
                                .w_full()
                                .into_any_element()
                        } else {
                            this.sidebar_room_shell(context, height, cx)
                                .into_any_element()
                        };
                        element.layout_as_root(
                            gpui::size(
                                gpui::AvailableSpace::Definite(bounds.size.width),
                                gpui::AvailableSpace::Definite(bounds.size.height),
                            ),
                            window,
                            cx,
                        );
                        element.prepaint_at(bounds.origin, window, cx);
                        Some(element)
                    },
                ),
                |_, element: Option<gpui::AnyElement>, window, cx| {
                    if let Some(mut element) = element {
                        element.paint(window, cx);
                    }
                },
            )
            .w_full()
            .h(height),
        )
    }

    fn sidebar_room_shell(
        &self,
        context: ConversationRef,
        height: gpui::Pixels,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        // These controls are clipped out. Keep native focus and activation,
        // but omit decoration, hover surfaces and hint geometry. The next
        // focused prepaint uses compact_room, including its focus/reveal ring.
        let control = |id: gpui::ElementId, action: LocalAction| {
            div()
                .id(id.clone())
                .when_some(self.control_focus(id, cx), |d, focus| {
                    d.track_focus(&focus.tab_stop(!self.background_inert))
                })
                .when(!self.background_inert, |d| {
                    d.on_key_down(|event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                        }
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.handle(action.clone(), window, cx);
                    }))
                })
        };
        div()
            .id(scoped_id("room", context, 1, 0))
            .w_full()
            .h(height)
            .pl(px(4.))
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .child(
                        control(
                            scoped_id("room", context, 0, 0),
                            LocalAction::Navigate(context),
                        )
                        .flex_1()
                        .min_w_0()
                        .h(height),
                    )
                    .child(
                        control(
                            scoped_id("room", context, 1, 1),
                            LocalAction::ToggleRoomMenu(context, "room"),
                        )
                        .absolute()
                        .right_0()
                        .size(px(28.)),
                    ),
            )
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
            && self.destination != SidebarDestination::Activity;
        // The sidebar row is one dense line. In the Activity page the same row
        // gains a larger avatar and the last-message preview.
        let roomy = namespace != "room";
        let size = if roomy { 32. } else { 22. };
        let id = scoped_id(namespace, context, 0, 0);
        let row_focus = self.control_focus(id.clone(), cx);
        let avatar = if let Some(avatar) = self.projected_avatar(
            talk_core::ui::content::AssetTarget::ConversationAvatar(row.context),
            size,
        ) {
            avatar
        } else if row.kind == ConversationKind::Direct {
            self.person_avatar(row.initials.as_str(), size)
        } else {
            div()
                .flex_none()
                .size(px(size))
                .flex()
                .items_center()
                .justify_center()
                .when(roomy, |d| d.rounded(px(8.)).bg(t.selected))
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
                    if roomy { 18. } else { 16. },
                    if roomy {
                        t.selected_text
                    } else {
                        t.sidebar_secondary
                    },
                ))
        };
        let avatar = if row.kind == ConversationKind::Direct {
            self.avatar_with_presence(
                avatar,
                row.presence,
                size,
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
            .min_h(if roomy {
                self.sp(36.) + px(8.)
            } else {
                self.sp(20.) + px(12.)
            })
            .when(selected, |d| d.bg(t.selected).text_color(t.selected_text))
            .child(avatar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .child(
                        div()
                            .w_full()
                            .text_size(self.sp(13.))
                            .font_weight(if row.unread > 0 {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_ellipsis()
                            .child(row.title.as_str().to_owned()),
                    )
                    .when(roomy && !row.subtitle.as_str().is_empty(), |d| {
                        d.child(
                            div()
                                .w_full()
                                .text_size(self.sp(12.))
                                .text_color(t.muted)
                                .text_ellipsis()
                                .child(row.subtitle.as_str().to_owned()),
                        )
                    }),
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
                    row_focus.clone(),
                    cx,
                ))
            })
            .when_some(row.action_error, |d, reason| {
                d.child(self.status_icon(
                    scoped_id(
                        "sidebar-action-error",
                        context,
                        u128::from(namespace != "room"),
                        0,
                    ),
                    ("info", super::toast::copy(reason)),
                    (12., t.warning),
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
                                    row.archive != Decision::Denied && row.archived.is_some(),
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

    /// Subscribed threads, as the body of the Activity "Threads" filter.
    fn threads_body(&self, cx: &Context<Self>) -> Div {
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
        let mut controls = div().flex().flex_wrap().items_center().gap_2();
        if let Some(account) = self.snapshot.active_account {
            controls = controls.child(self.icon_button(
                "refresh-threads",
                ("refresh", "Threads aktualisieren"),
                ButtonStyle::Quiet,
                Availability::when(self.snapshot.threads.content != ContentState::Loading, ""),
                LocalAction::Intent(UiIntent::RefreshThreads(account)),
                cx,
            ));
            let list = &self.snapshot.threads;
            if list.offset > 0 || list.has_next {
                let ready = list.content != ContentState::Loading;
                controls = controls
                    .child(self.button(
                        "threads-previous",
                        "Vorherige Threads",
                        ButtonStyle::Quiet,
                        ready && list.offset >= 50,
                        LocalAction::Intent(UiIntent::PageThreads {
                            account,
                            offset: list.offset.saturating_sub(50),
                        }),
                        cx,
                    ))
                    .child(self.button(
                        "threads-next",
                        "Weitere Threads",
                        ButtonStyle::Quiet,
                        ready && list.has_next && list.offset.checked_add(50).is_some(),
                        LocalAction::Intent(UiIntent::PageThreads {
                            account,
                            offset: list.offset.saturating_add(50),
                        }),
                        cx,
                    ));
            }
        }
        div().flex().flex_col().gap_3().child(controls).child(rows)
    }
}

/// Unread messages that ask for the user: personal rooms and direct mentions.
fn attention(
    kind: ConversationKind,
    unread: u32,
    archived: Option<bool>,
    direct_mention: Option<bool>,
    muted: Option<bool>,
) -> bool {
    unread > 0
        && archived == Some(false)
        && muted != Some(true)
        && (direct_mention == Some(true)
            || matches!(
                kind,
                ConversationKind::Direct | ConversationKind::NoteToSelf
            ))
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
    fn navigation_fixtures_map_one_scenario_to_one_initial_state() {
        assert_eq!(Fixture::for_scenario(None), Fixture::None);
        assert_eq!(Fixture::for_scenario(Some("chat")), Fixture::None);
        for (scenario, destination, activity, menu) in [
            (
                "sidebar-activity",
                SidebarDestination::Activity,
                Activity::Unread,
                None,
            ),
            (
                "sidebar-activity-mentions",
                SidebarDestination::Activity,
                Activity::Mentions,
                None,
            ),
            (
                "thread-list",
                SidebarDestination::Activity,
                Activity::Threads,
                None,
            ),
            (
                "account-reminders",
                SidebarDestination::Activity,
                Activity::Reminders,
                None,
            ),
            (
                "sidebar-archive",
                SidebarDestination::Archive,
                Activity::Unread,
                None,
            ),
            (
                "sidebar-new-menu",
                SidebarDestination::Conversations,
                Activity::Unread,
                Some(Menu::New),
            ),
            (
                "sidebar-list-menu",
                SidebarDestination::Conversations,
                Activity::Unread,
                Some(Menu::List),
            ),
        ] {
            let fixture = Fixture::for_scenario(Some(scenario));
            assert_eq!(fixture.destination(), destination, "{scenario}");
            assert_eq!(fixture.activity(), activity, "{scenario}");
            assert_eq!(fixture.menu(), menu, "{scenario}");
        }
    }

    #[test]
    fn only_direct_mentions_and_personal_rooms_count_toward_activity() {
        let (direct, group, note) = (
            ConversationKind::Direct,
            ConversationKind::Group,
            ConversationKind::NoteToSelf,
        );
        let quiet = Some(false);
        assert!(!attention(group, 4, Some(false), Some(false), quiet));
        assert!(!attention(group, 4, Some(false), None, quiet));
        assert!(attention(group, 4, Some(false), Some(true), quiet));
        assert!(
            !attention(group, 4, Some(true), Some(true), quiet),
            "archived rooms stay quiet"
        );
        assert!(
            !attention(group, 4, None, Some(true), quiet),
            "unknown archive state is not attention"
        );
        assert!(!attention(group, 0, Some(false), Some(true), quiet));
        assert!(
            !attention(group, 4, Some(false), Some(true), Some(true)),
            "muted rooms stay quiet"
        );
        assert!(attention(direct, 1, Some(false), Some(false), None));
        assert!(!attention(direct, 1, Some(false), Some(false), Some(true)));
        assert!(attention(note, 2, Some(false), None, quiet));
    }

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
