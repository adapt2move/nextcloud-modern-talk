//! Sidebar grouping by pointer: dragging a conversation onto a group, a heading or
//! another conversation, creating a group inline, and instant collapse feedback.
//! Server tags assign membership only; local groups and sections also keep an order.
use super::{Desktop, LocalAction, sidebar::Section};
use gpui::{
    Context, Focusable, Hsla, IntoElement, Render, SharedString, Stateful, Window, div, prelude::*,
    px,
};
use std::time::{Duration, Instant};
use talk_core::preferences::navigation::{
    NavigationEdit, NavigationGroupId, RemoteNavigationSupport,
};
use talk_core::ui::{
    Decision, Page,
    presentation::{ConversationRef, ConversationView, FeatureSupport, TagEdit, TagKind, TagRef},
};

/// A collapse request shows its result at once; the owner confirms it later.
const COLLAPSE_GRACE: Duration = Duration::from_secs(4);
const MAX_COLLAPSE_OVERRIDES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollapseKey {
    Tag(TagRef),
    Group(NavigationGroupId),
    Section(Section),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DropTarget {
    Favorites,
    Tag(TagRef),
    Group(NavigationGroupId),
    Section(Section),
}

#[derive(Clone)]
pub(super) struct DraggedRoom {
    context: ConversationRef,
    label: SharedString,
}

struct DragGhost {
    label: SharedString,
    background: Hsla,
    text: Hsla,
    border: Hsla,
}
impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(220.))
            .px_3()
            .py(px(6.))
            .rounded_lg()
            .border_1()
            .border_color(self.border)
            .bg(self.background)
            .text_color(self.text)
            .text_ellipsis()
            .child(self.label.clone())
    }
}

#[derive(Default)]
pub(super) struct CollapseOverrides(Vec<(CollapseKey, bool, Instant)>);
impl CollapseOverrides {
    pub fn get(&self, key: CollapseKey) -> Option<bool> {
        self.0
            .iter()
            .find(|(candidate, _, at)| *candidate == key && at.elapsed() < COLLAPSE_GRACE)
            .map(|(_, value, _)| *value)
    }
    fn set(&mut self, key: CollapseKey, value: bool) {
        self.0
            .retain(|(candidate, _, at)| *candidate != key && at.elapsed() < COLLAPSE_GRACE);
        if self.0.len() < MAX_COLLAPSE_OVERRIDES {
            self.0.push((key, value, Instant::now()));
        }
    }
}

impl Desktop {
    pub(super) fn effective_collapsed(&self, key: CollapseKey, actual: bool) -> bool {
        self.collapse_overrides.get(key).unwrap_or(actual)
    }

    pub(super) fn toggle_collapse(&mut self, key: CollapseKey, cx: &mut Context<Self>) {
        let actual = match key {
            CollapseKey::Tag(tag) => self
                .snapshot
                .tags
                .as_slice()
                .iter()
                .find(|t| t.id == tag)
                .map(|t| t.collapsed),
            CollapseKey::Group(id) => self
                .account_layout()
                .and_then(|l| l.groups().iter().find(|g| g.id == id))
                .map(|g| g.collapsed),
            CollapseKey::Section(section) => Some(self.section_collapsed(section)),
        };
        let Some(actual) = actual else {
            return;
        };
        let collapsed = !self.effective_collapsed(key, actual);
        self.collapse_overrides.set(key, collapsed);
        match key {
            CollapseKey::Tag(tag) => self.edit_tag(TagEdit::Collapse { tag, collapsed }),
            CollapseKey::Group(id) => {
                if let (Some(scope), Some(mut group)) = (
                    self.snapshot.active_account,
                    self.account_layout()
                        .and_then(|l| l.groups().iter().find(|g| g.id == id))
                        .cloned(),
                ) {
                    group.collapsed = collapsed;
                    self.edit_group(scope, NavigationEdit::UpsertGroup(group), cx);
                }
            }
            CollapseKey::Section(section) => self.collapse_sidebar_section(section, collapsed, cx),
        }
        cx.notify();
    }

    pub(super) fn open_new_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_menu = None;
        self.new_group_open = true;
        self.group_name.update(cx, |input, cx| {
            input.reset();
            cx.notify();
        });
        self.group_name.focus_handle(cx).focus(window);
        cx.notify();
    }

    pub(super) fn close_new_group(&mut self, cx: &mut Context<Self>) {
        if self.new_group_open {
            self.new_group_open = false;
            self.group_name.update(cx, |input, cx| {
                input.reset();
                cx.notify();
            });
            cx.notify();
        }
    }

    /// Groups exist on the server for rooms with tags and on this computer
    /// otherwise; either way a new group needs a usable account.
    pub(super) fn can_create_group(&self) -> bool {
        self.local_groups_available() || self.server_groups_in_use()
    }

    pub(super) fn submit_new_group(&mut self, cx: &mut Context<Self>) {
        if self.group_name.read(cx).composing()
            || self.group_name.read(cx).content.trim().is_empty()
        {
            return;
        }
        self.create_group(cx);
        self.close_new_group(cx);
    }

    pub(super) fn group_drop_accepted(&self, target: DropTarget) -> bool {
        match target {
            DropTarget::Favorites => true,
            DropTarget::Tag(_) => self.server_groups_in_use(),
            DropTarget::Group(_) => self.local_groups_available(),
            DropTarget::Section(_) => self.server_groups_in_use() || self.local_groups_available(),
        }
    }

    /// A conversation that can be dragged; it is also a drop target meaning
    /// "place the dragged conversation before this one".
    pub(super) fn draggable_room(
        &self,
        row: &ConversationView,
        target: DropTarget,
        cx: &Context<Self>,
    ) -> Stateful<gpui::Div> {
        let context = row.context;
        let dragged = DraggedRoom {
            context,
            label: SharedString::from(row.title.as_str().to_owned()),
        };
        let t = self.theme;
        let accepted = self.group_drop_accepted(target);
        let highlight = t.selected;
        self.sidebar_room(row, cx)
            .drag_threshold(px(6.))
            .on_drag(dragged, move |dragged, _, _, cx| {
                let label = dragged.label.clone();
                cx.new(|_| DragGhost {
                    label,
                    background: t.surface,
                    text: t.text,
                    border: t.border,
                })
            })
            .drag_over::<DraggedRoom>(
                move |style, _, _, _| {
                    if accepted { style.bg(highlight) } else { style }
                },
            )
            .on_drop(cx.listener(move |this, dragged: &DraggedRoom, window, cx| {
                this.drop_room(dragged.context, target, Some(context), window, cx);
            }))
    }

    /// Make a heading accept dragged conversations.
    pub(super) fn drop_onto_heading<T: StatefulInteractiveElement>(
        &self,
        heading: T,
        target: DropTarget,
        cx: &Context<Self>,
    ) -> T {
        let accepted = self.group_drop_accepted(target);
        let highlight = self.theme.selected;
        heading
            .drag_over::<DraggedRoom>(
                move |style, _, _, _| {
                    if accepted { style.bg(highlight) } else { style }
                },
            )
            .on_drop(cx.listener(move |this, dragged: &DraggedRoom, window, cx| {
                this.drop_room(dragged.context, target, None, window, cx);
            }))
    }

    fn server_groups_in_use(&self) -> bool {
        self.snapshot.tag_state == talk_core::ui::presentation::ContentState::Ready
            && self.snapshot.conversations.as_slice().iter().any(|r| {
                r.organization.tags == FeatureSupport::Supported
                    && r.organization.tag_change == Decision::Allowed
            })
    }

    fn drop_room(
        &mut self,
        dragged: ConversationRef,
        target: DropTarget,
        before: Option<ConversationRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if before == Some(dragged) {
            return;
        }
        let Some(row) = self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .find(|r| r.context == dragged)
            .cloned()
        else {
            return;
        };
        if target == DropTarget::Favorites {
            if self.favorite_value(&row) != Some(true) {
                self.handle(LocalAction::Favorite(dragged, true), window, cx);
            }
            return;
        }
        if row.organization.tags == FeatureSupport::Supported {
            self.drop_on_server_group(&row, target, window, cx);
        } else if row.organization.tags == FeatureSupport::Unsupported {
            self.drop_on_local_group(&row, target, before, cx);
        }
    }

    /// Server tags only record membership. The conversation joins the target as
    /// its single custom group; built-in groups remain the server's.
    fn drop_on_server_group(
        &mut self,
        row: &ConversationView,
        target: DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut tags = Page::empty();
        match target {
            DropTarget::Tag(tag) => {
                let custom = self
                    .snapshot
                    .tags
                    .as_slice()
                    .iter()
                    .any(|t| t.id == tag && t.kind == TagKind::Custom);
                if !custom || tags.try_push(tag).is_err() {
                    return;
                }
            }
            DropTarget::Section(_) => {}
            DropTarget::Favorites | DropTarget::Group(_) => return,
        }
        let held: Vec<_> = row
            .tags
            .as_ref()
            .map(|ids| ids.as_slice().to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|id| {
                self.snapshot
                    .tags
                    .as_slice()
                    .iter()
                    .any(|t| t.id == *id && t.kind == TagKind::Custom)
            })
            .collect();
        if held.as_slice() == tags.as_slice() {
            return;
        }
        if self.favorite_value(row) == Some(true) {
            self.handle(LocalAction::Favorite(row.context, false), window, cx);
        }
        self.edit_tag(TagEdit::Assign {
            context: row.context,
            tags,
        });
    }

    /// Local layout keeps group and order: the dragged conversation goes before
    /// `before`, or last, and its whole destination list is renumbered at once.
    fn drop_on_local_group(
        &mut self,
        row: &ConversationView,
        target: DropTarget,
        before: Option<ConversationRef>,
        cx: &mut Context<Self>,
    ) {
        let group = match target {
            DropTarget::Group(id) => Some(id),
            DropTarget::Section(section) if section == Section::for_room(row) => None,
            _ => return,
        };
        if !self.local_groups_available() {
            return;
        }
        let Some(account) = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|a| Some(a.scope) == self.snapshot.active_account)
            .and_then(|a| a.navigation_key.clone())
        else {
            return;
        };
        let Some(moved) = row.navigation_key.clone() else {
            return;
        };
        let section = Section::for_room(row);
        let mut bucket: Vec<_> = self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .filter(|r| {
                r.organization.tags == FeatureSupport::Unsupported
                    && r.navigation_key.is_some()
                    && r.archived == row.archived
                    && r.context != row.context
                    && match group {
                        Some(id) => self.placement(r).is_some_and(|p| p.group == Some(id)),
                        None => {
                            Section::for_room(r) == section
                                && self.placement(r).is_none_or(|p| p.group.is_none())
                        }
                    }
            })
            .collect();
        bucket.sort_by_key(|r| self.placement(r).map_or(0, |p| p.order));
        let at = before
            .and_then(|context| bucket.iter().position(|r| r.context == context))
            .unwrap_or(bucket.len());
        let mut keys: Vec<_> = bucket
            .iter()
            .filter_map(|r| r.navigation_key.clone())
            .collect();
        keys.insert(at, moved.clone());
        let was_favorite = self.favorite_value(row) == Some(true)
            && row.organization.favorites == FeatureSupport::Unsupported;
        let mut values = self.preferences.active.clone();
        let mut edits: Vec<_> = keys
            .into_iter()
            .enumerate()
            .map(|(order, room)| NavigationEdit::PlaceRoom {
                room,
                group,
                order: u16::try_from(order).unwrap_or(u16::MAX),
            })
            .collect();
        if was_favorite {
            edits.push(NavigationEdit::SetFavorite {
                room: moved,
                favorite: Some(false),
            });
        }
        for edit in edits {
            if let Err(error) =
                values
                    .navigation
                    .apply(account.clone(), RemoteNavigationSupport::Unsupported, edit)
            {
                self.report_navigation_error(error);
                return;
            }
        }
        self.commit_preferences(values, cx);
    }
}
