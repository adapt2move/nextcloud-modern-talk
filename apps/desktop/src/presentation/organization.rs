//! Account-scoped layout edits are admitted only with durable keys and known fallback.
use super::{ButtonStyle, Desktop, LocalAction, controls::Availability};
use gpui::{Context, Div, Focusable, div, prelude::*, px};
use talk_core::preferences::navigation::{
    NavigationEdit, NavigationGroup, NavigationGroupId, RemoteNavigationSupport,
};
use talk_core::ui::{
    AccountScope, Decision, Page, Text,
    presentation::{
        ContentState, ConversationRef, FeatureSupport, RequestId, TagEdit, TagKind, TagRef,
        UiIntent,
    },
};

impl Desktop {
    fn local_groups_available(&self) -> bool {
        self.snapshot.conversations.as_slice().iter().any(|r| {
            r.organization.tags == FeatureSupport::Unsupported && r.navigation_key.is_some()
        }) && self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .any(|a| Some(a.scope) == self.snapshot.active_account && a.navigation_key.is_some())
    }
    pub(super) fn edit_group(
        &mut self,
        scope: AccountScope,
        edit: NavigationEdit,
        cx: &mut Context<Self>,
    ) {
        if Some(scope) == self.snapshot.active_account && self.local_groups_available() {
            self.edit_navigation(RemoteNavigationSupport::Unsupported, edit, cx);
        }
    }
    pub(super) fn create_group(&mut self, cx: &mut Context<Self>) {
        if self
            .selected()
            .is_some_and(|r| r.organization.tags == FeatureSupport::Supported)
        {
            if let Ok(name) = Text::new(self.group_name.read(cx).content.trim()) {
                self.edit_tag(TagEdit::Create(name));
            }
            return;
        }
        if !self.local_groups_available() {
            return;
        }
        let name = self.group_name.read(cx).content.trim();
        let Ok(name) = Text::<128>::new(name) else {
            return;
        };
        let Some(id) = self
            .account_layout()
            .map_or(Some(1), |l| {
                l.groups()
                    .iter()
                    .map(|g| g.id.get())
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
            })
            .and_then(|id| NavigationGroupId::new(id).ok())
        else {
            return;
        };
        let order = self.account_layout().map_or(0, |l| {
            l.groups()
                .iter()
                .map(|g| g.order)
                .max()
                .unwrap_or(0)
                .saturating_add(1)
        });
        self.edit_navigation(
            RemoteNavigationSupport::Unsupported,
            NavigationEdit::UpsertGroup(NavigationGroup {
                id,
                name,
                order,
                collapsed: false,
            }),
            cx,
        );
    }
    pub(super) fn place_in_group(
        &mut self,
        context: ConversationRef,
        group: Option<NavigationGroupId>,
        cx: &mut Context<Self>,
    ) {
        if Some(context) != self.snapshot.selected || !self.local_groups_available() {
            return;
        }
        let Some(room) = self.selected().and_then(|r| r.navigation_key.clone()) else {
            return;
        };
        let order = self.account_layout().map_or(0, |l| {
            l.placements()
                .iter()
                .filter(|p| p.group == group)
                .map(|p| p.order)
                .max()
                .unwrap_or(0)
                .saturating_add(1)
        });
        self.edit_navigation(
            RemoteNavigationSupport::Unsupported,
            NavigationEdit::PlaceRoom { room, group, order },
            cx,
        );
    }
    pub(super) fn organization_body(&self, cx: &Context<Self>) -> Div {
        if self
            .selected()
            .is_some_and(|r| r.organization.tags == FeatureSupport::Supported)
        {
            return self.server_groups_body(cx);
        }
        let available = self.local_groups_available();
        let gate = Availability::when(
            available,
            "Die Gruppierung ist für dieses Konto derzeit nicht verfügbar.",
        );
        let mut body = div().flex().flex_col().gap_3()
            .child(self.hint(self.l("Eigene Gruppen gelten für dieses Konto auf diesem Computer, wenn der Server keine Gesprächs-Tags anbietet.")))
            .child(div().relative().rounded(px(7.)).border_1().border_color(self.theme.input_border).p_3()
                .child(self.field_ring(self.group_name.focus_handle(cx), 7.)).child(self.group_name.clone()))
            .child(self.button("create-group", "Gruppe hinzufügen", ButtonStyle::Secondary, gate, LocalAction::CreateGroup, cx));
        if let Some(row) = self.selected() {
            body = body.child(self.button(
                "group-default",
                "Standardbereich verwenden",
                ButtonStyle::Quiet,
                gate,
                LocalAction::PlaceInGroup(row.context, None),
                cx,
            ));
            if let Some(layout) = self.account_layout() {
                let mut groups: Vec<_> = layout.groups().iter().collect();
                groups.sort_by_key(|g| g.order);
                for group in groups {
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                self.button(
                                    gpui::SharedString::from(format!(
                                        "group-place-{}",
                                        group.id.get()
                                    )),
                                    "",
                                    ButtonStyle::Secondary,
                                    gate,
                                    LocalAction::PlaceInGroup(row.context, Some(group.id)),
                                    cx,
                                )
                                .flex_1()
                                .min_w_0()
                                .child(div().text_ellipsis().child(group.name.as_str().to_owned())),
                            )
                            .child(self.icon_button(
                                gpui::SharedString::from(format!(
                                    "group-rename-{}",
                                    group.id.get()
                                )),
                                ("check", "Namen übernehmen"),
                                ButtonStyle::Quiet,
                                gate,
                                LocalAction::RenameLocalGroup(group.id),
                                cx,
                            ))
                            .child(self.icon_button(
                                gpui::SharedString::from(format!("group-up-{}", group.id.get())),
                                ("arrow-up", "Nach oben"),
                                ButtonStyle::Quiet,
                                gate,
                                LocalAction::MoveLocalGroup(group.id, true),
                                cx,
                            ))
                            .child(self.icon_button(
                                gpui::SharedString::from(format!("group-down-{}", group.id.get())),
                                ("chevron-down", "Nach unten"),
                                ButtonStyle::Quiet,
                                gate,
                                LocalAction::MoveLocalGroup(group.id, false),
                                cx,
                            ))
                            .child(self.icon_button(
                                gpui::SharedString::from(format!(
                                    "group-remove-{}",
                                    group.id.get()
                                )),
                                ("x", "Gruppe entfernen"),
                                ButtonStyle::Quiet,
                                gate,
                                LocalAction::EditGroup(
                                    row.context.account,
                                    NavigationEdit::RemoveGroup(group.id),
                                ),
                                cx,
                            )),
                    );
                }
            }
        }
        if !available {
            body = body.child(
                self.hint(self.l("Die Gruppierung ist für dieses Konto derzeit nicht verfügbar.")),
            );
        }
        body
    }
    pub(super) fn rename_local_group(&mut self, id: NavigationGroupId, cx: &mut Context<Self>) {
        let Some(mut group) = self
            .account_layout()
            .and_then(|l| l.groups().iter().find(|g| g.id == id))
            .cloned()
        else {
            return;
        };
        let Ok(name) = Text::new(self.group_name.read(cx).content.trim()) else {
            return;
        };
        group.name = name;
        if let Some(scope) = self.snapshot.active_account {
            self.edit_group(scope, NavigationEdit::UpsertGroup(group), cx);
        }
    }
    pub(super) fn move_local_group(
        &mut self,
        id: NavigationGroupId,
        up: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.local_groups_available() {
            return;
        }
        let Some(layout) = self.account_layout() else {
            return;
        };
        let mut groups = layout.groups().to_vec();
        groups.sort_by_key(|g| g.order);
        let Some(index) = groups.iter().position(|g| g.id == id) else {
            return;
        };
        let other = if up {
            index.checked_sub(1)
        } else {
            index.checked_add(1).filter(|i| *i < groups.len())
        };
        let Some(other) = other else {
            return;
        };
        groups.swap(index, other);
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
        for (order, mut group) in groups.into_iter().enumerate() {
            group.order = order as u16;
            if values
                .navigation
                .apply(
                    key.clone(),
                    RemoteNavigationSupport::Unsupported,
                    NavigationEdit::UpsertGroup(group),
                )
                .is_err()
            {
                return;
            }
        }
        self.commit_preferences(values, cx);
    }
    fn server_groups_available(&self) -> bool {
        self.snapshot.tag_state == ContentState::Ready
            && self.snapshot.conversations.as_slice().iter().any(|r| {
                r.organization.tags == FeatureSupport::Supported
                    && r.organization.tag_change == Decision::Allowed
            })
    }
    pub(super) fn edit_tag(&mut self, edit: TagEdit) {
        if !self.server_groups_available() {
            return;
        }
        let Some(account) = self.snapshot.active_account else {
            return;
        };
        let Some(request) = self.archive_request.checked_add(1) else {
            return;
        };
        self.archive_request = request;
        self.submit(UiIntent::EditTags {
            request: RequestId(request),
            account,
            edit,
        });
    }
    pub(super) fn rename_tag(&mut self, tag: TagRef, cx: &mut Context<Self>) {
        if let Ok(name) = Text::new(self.group_name.read(cx).content.trim()) {
            self.edit_tag(TagEdit::Rename { tag, name });
        }
    }
    pub(super) fn move_tag(&mut self, tag: TagRef, up: bool) {
        let mut tags: Vec<_> = self.snapshot.tags.as_slice().iter().collect();
        tags.sort_by_key(|t| t.order);
        let Some(index) = tags.iter().position(|t| t.id == tag) else {
            return;
        };
        let other = if up {
            index.checked_sub(1)
        } else {
            index.checked_add(1).filter(|i| *i < tags.len())
        };
        let Some(other) = other else {
            return;
        };
        tags.swap(index, other);
        let mut order = Page::empty();
        for tag in tags {
            if order.try_push(tag.id).is_err() {
                return;
            }
        }
        self.edit_tag(TagEdit::Reorder(order));
    }
    pub(super) fn toggle_tag(&mut self, tag: TagRef) {
        let Some(row) = self.selected() else {
            return;
        };
        let Some(current) = &row.tags else {
            return;
        };
        let remove = current.as_slice().contains(&tag);
        let mut tags = Page::empty();
        for id in current.as_slice() {
            // SetConversationTags accepts custom tags; built-ins remain server-owned.
            if *id != tag
                && self
                    .snapshot
                    .tags
                    .as_slice()
                    .iter()
                    .any(|t| t.id == *id && t.kind == TagKind::Custom)
                && tags.try_push(*id).is_err()
            {
                return;
            }
        }
        if !remove && tags.try_push(tag).is_err() {
            return;
        }
        self.edit_tag(TagEdit::Assign {
            context: row.context,
            tags,
        });
    }
    fn server_groups_body(&self, cx: &Context<Self>) -> Div {
        let gate = Availability::when(
            self.server_groups_available(),
            "Die Gruppierung ist für dieses Konto derzeit nicht verfügbar.",
        );
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                self.hint(self.l("Diese Gruppen werden auf deinem Nextcloud-Server gespeichert.")),
            )
            .child(
                div()
                    .relative()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(self.theme.input_border)
                    .p_3()
                    .child(self.field_ring(self.group_name.focus_handle(cx), 7.))
                    .child(self.group_name.clone()),
            )
            .child(self.button(
                "create-server-group",
                "Gruppe hinzufügen",
                ButtonStyle::Secondary,
                gate,
                LocalAction::CreateGroup,
                cx,
            ));
        let mut tags: Vec<_> = self.snapshot.tags.as_slice().iter().collect();
        tags.sort_by_key(|t| t.order);
        for tag in tags {
            let selected = self
                .selected()
                .and_then(|r| r.tags.as_ref())
                .is_some_and(|ids| ids.as_slice().contains(&tag.id));
            let custom = tag.kind == TagKind::Custom;
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        self.button(
                            gpui::SharedString::from(format!("tag-place-{}", tag.id.local)),
                            "",
                            ButtonStyle::Secondary,
                            Availability::when(
                                custom && self.server_groups_available(),
                                "Systemgruppen werden vom Server verwaltet.",
                            ),
                            LocalAction::ToggleTag(tag.id),
                            cx,
                        )
                        .flex_1()
                        .min_w_0()
                        .child(self.scaled_icon(
                            if selected { "check" } else { "plus" },
                            14.,
                            self.theme.muted,
                        ))
                        .child(div().text_ellipsis().child(tag.name.as_str().to_owned())),
                    )
                    .when(custom, |d| {
                        d.child(self.icon_button(
                            gpui::SharedString::from(format!("tag-rename-{}", tag.id.local)),
                            ("check", "Namen übernehmen"),
                            ButtonStyle::Quiet,
                            gate,
                            LocalAction::RenameTag(tag.id),
                            cx,
                        ))
                    })
                    .child(self.icon_button(
                        gpui::SharedString::from(format!("tag-up-{}", tag.id.local)),
                        ("arrow-up", "Nach oben"),
                        ButtonStyle::Quiet,
                        gate,
                        LocalAction::MoveTag(tag.id, true),
                        cx,
                    ))
                    .child(self.icon_button(
                        gpui::SharedString::from(format!("tag-down-{}", tag.id.local)),
                        ("chevron-down", "Nach unten"),
                        ButtonStyle::Quiet,
                        gate,
                        LocalAction::MoveTag(tag.id, false),
                        cx,
                    ))
                    .when(custom, |d| {
                        d.child(self.icon_button(
                            gpui::SharedString::from(format!("tag-delete-{}", tag.id.local)),
                            ("x", "Gruppe entfernen"),
                            ButtonStyle::Quiet,
                            gate,
                            LocalAction::EditTag(TagEdit::Delete(tag.id)),
                            cx,
                        ))
                    }),
            );
        }
        body
    }
}
