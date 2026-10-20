//! Compact navigation over bounded, typed conversation projections.
use super::controls::{Availability, scoped_id};
use super::{ButtonStyle, Desktop, LocalAction, Overlay, SidebarDestination};
use gpui::{Context, Div, Focusable, FontWeight, div, prelude::*, px};
use talk_core::preferences::navigation::{
    AccountNavigation, NavigationEdit, NavigationSection, RemoteNavigationSupport,
};
use talk_core::ui::presentation::{
    ContentState, ConversationCategory, ConversationKind, ConversationView, FeatureSupport,
    TagEdit, TagKind,
};

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
            .flex_none()
            .w(px(260.))
            .when(!narrow, |d| d.h_full())
            .flex()
            .flex_col()
            .px(px(12.))
            .pt(px(14.))
            .pb(px(10.));
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
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .rounded(px(7.))
                    .bg(t.subtle)
                    .relative()
                    .child(self.field_ring(self.sidebar_filter.focus_handle(cx), 7.))
                    .min_h(self.sp(28.).max(px(34.)))
                    .mb_2()
                    .child(self.scaled_icon("search", 15., t.muted))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .child(self.sidebar_filter.clone()),
                    )
                    .when(!self.sidebar_filter.read(cx).content.is_empty(), |d| {
                        d.child(self.icon_button(
                            "clear-filter",
                            ("x", "Filter leeren"),
                            ButtonStyle::Quiet,
                            Availability::available(),
                            LocalAction::ClearFilter,
                            cx,
                        ))
                    }),
            )
            .child(self.sidebar_destination(
                "nav-inbox",
                "Posteingang",
                "inbox",
                SidebarDestination::Inbox,
                cx,
            ))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(
                        self.sidebar_destination(
                            "nav-active",
                            "Gespräche",
                            "chat",
                            SidebarDestination::Conversations,
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(self.icon_button(
                        "reload-conversations",
                        ("refresh", "Gespräche neu laden"),
                        ButtonStyle::Quiet,
                        self.snapshot.active_account.is_some().into(),
                        self.snapshot.active_account.map_or(
                            LocalAction::Destination(SidebarDestination::Conversations),
                            |scope| {
                                LocalAction::Intent(
                                    talk_core::ui::presentation::UiIntent::ReloadConversations(
                                        scope,
                                    ),
                                )
                            },
                        ),
                        cx,
                    ))
                    .child(self.icon_button(
                        "new-conversation",
                        ("plus", "Neues Gespräch"),
                        ButtonStyle::Quiet,
                        Availability::available().beside_control(),
                        LocalAction::Open(Overlay::Create),
                        cx,
                    )),
            )
            .child(self.sidebar_destination(
                "nav-archive",
                "Archiv",
                "file",
                SidebarDestination::Archive,
                cx,
            ));

        let query = self.sidebar_filter.read(cx).content.to_lowercase();
        let archived = self.destination == SidebarDestination::Archive;
        let mut list = div().flex_none().flex().flex_col().pt_2();
        let mut visible = 0;
        let mut grouped = Vec::new();
        if self.snapshot.tag_state == ContentState::Ready {
            let mut tags: Vec<_> = self.snapshot.tags.as_slice().iter().collect();
            tags.sort_by_key(|t| t.order);
            for tag in tags {
                if tag.kind == TagKind::Unknown {
                    continue;
                }
                let rows: Vec<_> = self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .filter(|r| {
                        r.archived == Some(archived)
                            && r.title.as_str().to_lowercase().contains(&query)
                            && r.organization.tags == FeatureSupport::Supported
                            && !grouped.contains(&r.context)
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
                list =
                    list.child(
                        self.button(
                            gpui::SharedString::from(format!("sidebar-tag-{}", tag.id.local)),
                            "",
                            ButtonStyle::Quiet,
                            self.snapshot.conversations.as_slice().iter().any(|r| {
                                r.organization.tag_change == talk_core::ui::Decision::Allowed
                            }),
                            LocalAction::EditTag(TagEdit::Collapse {
                                tag: tag.id,
                                collapsed: !tag.collapsed,
                            }),
                            cx,
                        )
                        .justify_start()
                        .gap_2()
                        .mt_2()
                        .min_h(px(28.))
                        .py_1()
                        .px_2()
                        .text_size(self.sp(12.))
                        .child(self.scaled_icon(
                            if tag.collapsed && query.is_empty() {
                                "chevron-left"
                            } else {
                                "chevron-down"
                            },
                            12.,
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
                let mut rows: Vec<_> = self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .filter(|r| {
                        r.archived == Some(archived)
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
                    .min_h(px(28.))
                    .py_1()
                    .px_2()
                    .text_size(self.sp(12.))
                    .child(self.scaled_icon(
                        if group.collapsed && query.is_empty() {
                            "chevron-left"
                        } else {
                            "chevron-down"
                        },
                        12.,
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
            let mut rows: Vec<_> = self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .filter(|r| {
                    r.archived == Some(archived)
                        && !grouped.contains(&r.context)
                        && Section::for_room(r) == section
                        && self.placement(r).is_none_or(|p| p.group.is_none())
                        && r.title.as_str().to_lowercase().contains(&query)
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            // Stable sort changes only favorite priority inside this section.
            rows.sort_by_key(|r| {
                (
                    self.favorite_value(r) != Some(true),
                    self.placement(r).map_or(0, |p| p.order),
                )
            });
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
                .min_h(px(28.))
                .py_1()
                .px_2()
                .text_size(self.sp(12.))
                .text_color(t.sidebar_secondary)
                .child(self.scaled_icon(
                    if collapsed {
                        "chevron-left"
                    } else {
                        "chevron-down"
                    },
                    12.,
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
        if visible == 0 {
            list = list.child(div().px_2().py_3().child(self.hint(self.l(if archived {
                "Keine archivierten Gespräche geladen."
            } else {
                "Keine Gespräche in diesem Filter."
            }))));
        }
        if self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .any(|r| r.archived.is_none())
        {
            list = list.child(self.hint(self.l("Archivstatus noch nicht bekannt.")));
        }
        if !self.snapshot.conversation_list.complete {
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
        sidebar.child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .pt_2()
                .mt_2()
                .border_t_1()
                .border_color(t.border)
                .child(self.icon_button(
                    "discover",
                    ("compass", "Offene Gespräche entdecken"),
                    ButtonStyle::Quiet,
                    Availability::available(),
                    LocalAction::Open(Overlay::Discover),
                    cx,
                ))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .text_size(self.sp(12.))
                        .text_ellipsis()
                        .child(account.map_or_else(
                            || self.l("Noch kein Konto verbunden.").to_owned(),
                            |a| self.account_name(a),
                        )),
                )
                .child(self.icon_button(
                    "settings",
                    ("settings", "Einstellungen"),
                    ButtonStyle::Quiet,
                    Availability::available(),
                    LocalAction::Open(Overlay::Settings),
                    cx,
                )),
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
    ) -> Div {
        let t = self.theme;
        let context = row.context;
        let selected = Some(context) == self.snapshot.selected
            && self.destination != SidebarDestination::Inbox;
        let id = scoped_id(namespace, context, 0, 0);
        let row_focus = self.control_focus(id.clone(), cx);
        let control = self
            .button(
                id,
                "",
                ButtonStyle::Quiet,
                true,
                LocalAction::Navigate(context),
                cx,
            )
            .flex_none()
            .w_full()
            .justify_start()
            .gap_2()
            .px_2()
            .py_1()
            .min_h(self.sp(24.).max(px(if self.compact { 32. } else { 38. })))
            .when(selected, |d| d.bg(t.selected).text_color(t.selected_text))
            .child(if row.kind == ConversationKind::Direct {
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
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(self.sp(13.))
                    .font_weight(if row.unread > 0 {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_ellipsis()
                    .child(row.title.as_str().to_owned()),
            )
            .when(
                row.kind == ConversationKind::Direct && row.organization.classified == Some(true),
                |d| d.child(self.scaled_icon("lock", 12., t.muted)),
            )
            .when(self.favorite_value(row) == Some(true), |d| {
                d.child(self.status_icon(
                    scoped_id(namespace, context, 0, 1),
                    ("star", "Favorit"),
                    (13., t.accent),
                    row_focus,
                    cx,
                ))
            })
            .when(row.unread > 0, |d| {
                d.child(
                    if matches!(
                        row.kind,
                        ConversationKind::Direct | ConversationKind::NoteToSelf
                    ) {
                        div()
                            .flex_none()
                            .min_w(px(18.))
                            .h(px(18.))
                            .px_1()
                            .rounded_full()
                            .bg(t.accent)
                            .text_color(t.on_accent)
                            .text_size(px(11.))
                            .text_center()
                            .child(if row.unread > 99 {
                                "99+".to_owned()
                            } else {
                                row.unread.to_string()
                            })
                    } else {
                        div().flex_none().size(px(6.)).rounded_full().bg(t.accent)
                    },
                )
            });
        div().flex_none().child(control)
    }

    pub(super) fn inbox(&self, cx: &Context<Self>) -> Div {
        let mut page = div()
            .size_full()
            .rounded(px(12.))
            .bg(self.theme.surface)
            .flex()
            .flex_col()
            .p_5()
            .gap_3()
            .child(self.title(self.l("Posteingang")))
            .child(self.hint(self.l("Ungelesene Gespräche in diesem Konto.")));
        let mut rows = div().flex().flex_col().gap_1();
        let mut count = 0;
        for row in self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .filter(|r| r.unread > 0 && r.archived == Some(false))
        {
            rows = rows.child(self.compact_room(row, "inbox-room", cx));
            count += 1;
        }
        if count == 0 {
            rows = rows.child(self.hint(self.l("Keine ungelesenen Gespräche geladen.")));
        }
        if !self.snapshot.conversation_list.complete {
            rows = rows
                .child(self.hint(self.l("Die Gesprächsliste ist noch nicht vollständig geladen.")));
        }
        page = page.child(
            self.scroll_area(super::navigation::Region::Conversation)
                .flex_1()
                .min_h_0()
                .child(rows),
        );
        page
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
