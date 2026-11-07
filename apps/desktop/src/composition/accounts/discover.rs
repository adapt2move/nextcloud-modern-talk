//! Discovery results and self-join attempts belong to the active account.
use super::*;
use talk_core::ui::{
    conversation::RemoteState,
    discover::{Action, Panel, Room},
};

pub(super) struct Probe {
    account: AccountScope,
    generation: u64,
    due: Option<Instant>,
    attempts: u8,
}
pub(super) struct Retry {
    account: AccountScope,
    request: RequestId,
    query: Text<256>,
    attempts: u8,
    due: Option<Instant>,
    deadline: Instant,
}
fn transient(error: IdentityError) -> bool {
    matches!(
        error,
        IdentityError::Request(
            ErrorKind::Transport
                | ErrorKind::Deadline
                | ErrorKind::ServerUnavailable
                | ErrorKind::RateLimited
                | ErrorKind::Busy
        )
    )
}
impl Probe {
    pub(super) fn new(account: AccountScope, generation: u64) -> Self {
        Self {
            account,
            generation,
            due: Some(Instant::now()),
            attempts: 0,
        }
    }
}
pub(super) fn empty_panel(account: AccountScope) -> Panel {
    Panel {
        account,
        request: RequestId(0),
        query: text(""),
        support: FeatureSupport::Unknown,
        state: RemoteState::Idle,
        rooms: Page::empty(),
        loaded: false,
        joining: None,
        uncertain: false,
    }
}

/// Display-only transition. The pending row has no policy or membership authority.
pub(super) struct OptimisticJoin {
    pub context: ConversationRef,
    request: RequestId,
    previous: Option<ConversationRef>,
    row: ConversationView,
}

fn pending_row(room: &Room) -> ConversationView {
    ConversationView {
        context: room.context,
        navigation_key: None,
        title: room.name.clone(),
        initials: initials(room.name.as_str()),
        subtitle: text(""),
        updated_label: text(""),
        kind: ConversationKind::Unknown,
        unread: 0,
        unread_direct_mention: None,
        last_read_message: None,
        unread_mention: None,
        notification_muted: None,
        last_activity: None,
        favorite: None,
        organization: ConversationOrganization {
            category: ConversationCategory::Unknown,
            classified: None,
            favorites: FeatureSupport::Unknown,
            favorite_change: Decision::Unknown,
            tags: FeatureSupport::Unknown,
            tag_change: Decision::Unknown,
        },
        tags: None,
        archived: Some(false),
        archive: Decision::Unknown,
        presence: Presence::Unknown,
        status_icon: None,
        status_message: None,
        participants: room.participants.and_then(|n| u16::try_from(n).ok()),
        participant_names: Page::empty(),
        send: Decision::Unknown,
        attach: Decision::Unknown,
        draft_clipboard: Decision::Unknown,
        call: Decision::Unknown,
        call_scope: None,
        call_active: None,
    }
}

fn with_pending_row(
    rows: &Page<ConversationView, 100>,
    join: &OptimisticJoin,
) -> Page<ConversationView, 100> {
    let previous = join
        .previous
        .filter(|id| *id != join.context)
        .and_then(|id| rows.as_slice().iter().find(|row| row.context == id))
        .cloned();
    let remaining = 99usize.saturating_sub(usize::from(previous.is_some()));
    page(
        std::iter::once(join.row.clone()).chain(previous).chain(
            rows.as_slice()
                .iter()
                .filter(|row| row.context != join.context && Some(row.context) != join.previous)
                .take(remaining)
                .cloned(),
        ),
    )
}

impl Accounts {
    fn begin_optimistic_join(&mut self, request: RequestId, room: &Room) {
        let join = OptimisticJoin {
            context: room.context,
            request,
            previous: self.snapshot.selected,
            row: pending_row(room),
        };
        self.clear_content();
        self.snapshot.conversations = with_pending_row(&self.snapshot.conversations, &join);
        self.retain_visible_search_hits();
        self.snapshot.selected = Some(room.context);
        self.snapshot.content = ContentState::Loading;
        self.discovery_join = Some(join);
    }
    pub(super) fn restore_optimistic_join(&mut self, scope: AccountScope) {
        if let Some(join) = self
            .discovery_join
            .as_ref()
            .filter(|join| join.context.account == scope)
        {
            self.snapshot.conversations = with_pending_row(&self.snapshot.conversations, join);
            self.retain_visible_search_hits();
        }
    }
    fn retain_visible_search_hits(&mut self) {
        if let Some(panel) = &mut self.snapshot.search_panel {
            panel.hits = page(
                panel
                    .hits
                    .as_slice()
                    .iter()
                    .filter(|hit| {
                        self.snapshot
                            .conversations
                            .as_slice()
                            .iter()
                            .any(|row| row.context == hit.context)
                    })
                    .cloned(),
            );
        }
    }
    fn take_optimistic_join(
        &mut self,
        request: RequestId,
        context: ConversationRef,
    ) -> Option<OptimisticJoin> {
        if self
            .discovery_join
            .as_ref()
            .is_some_and(|join| join.request == request && join.context == context)
        {
            self.discovery_join.take()
        } else {
            None
        }
    }
    fn rollback_optimistic_join(&mut self, join: OptimisticJoin) {
        self.snapshot.conversations = page(
            self.snapshot
                .conversations
                .as_slice()
                .iter()
                .filter(|row| row.context != join.context)
                .cloned(),
        );
        if self.snapshot.selected == Some(join.context) {
            self.clear_content();
            if let Some(previous) = join.previous.filter(|previous| {
                self.snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .any(|row| row.context == *previous)
            }) {
                if self.open(previous).is_err() {
                    let _ = self.select_conversation(previous);
                }
            } else {
                self.snapshot.content = ContentState::Ready;
            }
        }
    }
    pub(super) fn tick_discovery_retry(&mut self) {
        let now = Instant::now();
        let Some(retry) = self.discovery_retry.as_ref() else {
            return;
        };
        if retry.due.is_none_or(|due| now < due) {
            return;
        }
        let (account, request, query, deadline) = (
            retry.account,
            retry.request,
            retry.query.clone(),
            retry.deadline,
        );
        if self.snapshot.active_account != Some(account)
            || self.snapshot.discover_panel.as_ref().is_none_or(|panel| {
                panel.account != account || panel.request != request || panel.joining.is_some()
            })
        {
            self.discovery_retry = None;
            return;
        }
        match self.request(
            account,
            ProtocolRequest::ListOpenConversations { query },
            Purpose::Discover(request, account, None),
        ) {
            Ok(()) => {
                if let Some(retry) = &mut self.discovery_retry {
                    retry.due = None;
                }
            }
            Err(SubmitError::Busy) if now < deadline => {
                if let Some(retry) = &mut self.discovery_retry {
                    retry.due = Some(now + Duration::from_secs(1));
                }
            }
            Err(_) => {
                self.discovery_retry = None;
                if let Some(panel) = &mut self.snapshot.discover_panel {
                    panel.state = RemoteState::Failed(request, UnavailableReason::Busy);
                }
                self.touch();
            }
        }
    }
    pub(super) fn tick_discovery_probe(&mut self) {
        let now = Instant::now();
        let Some(probe) = self.discover_probe.as_ref() else {
            return;
        };
        if probe.due.is_none_or(|due| now < due)
            || self.snapshot.active_account != Some(probe.account)
            || !self.snapshot.accounts.as_slice().iter().any(|account| {
                account.scope == probe.account
                    && account.connection == AccountConnectionState::Connected
            })
        {
            return;
        }
        let (account, generation) = (probe.account, probe.generation);
        match self.request(
            account,
            ProtocolRequest::InspectAccount,
            Purpose::DiscoverProbe(account, generation),
        ) {
            Ok(()) => {
                if let Some(probe) = &mut self.discover_probe {
                    probe.due = None;
                }
            }
            Err(_) => {
                if let Some(probe) = &mut self.discover_probe {
                    probe.due = Some(now + Duration::from_secs(2));
                }
            }
        }
    }
    pub(super) fn complete_discovery_probe(
        &mut self,
        account: AccountScope,
        generation: u64,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        let Some(probe) = self.discover_probe.as_mut().filter(|probe| {
            probe.account == account && probe.generation == generation && probe.due.is_none()
        }) else {
            return;
        };
        if self.snapshot.active_account != Some(account) {
            return;
        }
        if let Ok(RuntimeReply::Protocol(ProtocolReply::Account(observed))) = result
            && observed.scope == account
            && observed.listable_rooms != protocol::FeatureSupport::Unknown
        {
            if let Some(panel) = self
                .snapshot
                .discover_panel
                .as_mut()
                .filter(|panel| panel.account == account)
            {
                panel.support = support(observed.listable_rooms);
            }
            self.discover_probe = None;
            self.touch();
        } else {
            probe.attempts = probe.attempts.saturating_add(1);
            probe.due =
                Some(Instant::now() + Duration::from_secs((1u64 << probe.attempts.min(5)).min(30)));
        }
    }
    pub(super) fn submit_discovery(
        &mut self,
        account: AccountScope,
        request: RequestId,
        action: Action,
    ) -> Result<(), SubmitError> {
        self.current(account)?;
        if self.snapshot.active_account != Some(account) {
            return Err(SubmitError::StaleScope);
        }
        if self.snapshot.discover_panel.as_ref().is_none_or(|panel| {
            panel.account != account || panel.support != FeatureSupport::Supported
        }) {
            return Err(SubmitError::Unavailable(UnavailableReason::PolicyUnknown));
        }
        if self
            .pending
            .iter()
            .any(|p| matches!(p.purpose, Purpose::Discover(_, scope, _) if scope == account))
        {
            return Err(SubmitError::Busy);
        }
        let (command, query, joining) = match action {
            Action::List(query) => {
                if self
                    .snapshot
                    .discover_panel
                    .as_ref()
                    .is_some_and(|p| p.uncertain)
                {
                    return Err(SubmitError::Busy);
                }
                (
                    ProtocolRequest::ListOpenConversations {
                        query: query.clone(),
                    },
                    query,
                    None,
                )
            }
            Action::Join(context) => {
                let panel = self
                    .snapshot
                    .discover_panel
                    .as_ref()
                    .filter(|p| p.account == account && !p.uncertain)
                    .ok_or(SubmitError::StaleScope)?;
                if context.account != account
                    || !panel
                        .rooms
                        .as_slice()
                        .iter()
                        .any(|r| r.context == context && r.join == Decision::Allowed)
                {
                    return Err(SubmitError::StaleScope);
                }
                (
                    ProtocolRequest::JoinOpenConversation(context.conversation),
                    panel.query.clone(),
                    Some(context),
                )
            }
        };
        let transition = joining.and_then(|context| {
            self.snapshot
                .discover_panel
                .as_ref()
                .and_then(|panel| {
                    panel
                        .rooms
                        .as_slice()
                        .iter()
                        .find(|room| room.context == context)
                })
                .cloned()
        });
        self.request(
            account,
            command,
            Purpose::Discover(request, account, joining),
        )?;
        self.discovery_retry = if joining.is_none() {
            Some(Retry {
                account,
                request,
                query: query.clone(),
                attempts: 0,
                due: None,
                deadline: Instant::now() + Duration::from_secs(30),
            })
        } else {
            None
        };
        let panel = self
            .snapshot
            .discover_panel
            .get_or_insert_with(|| empty_panel(account));
        panel.request = request;
        panel.query = query;
        panel.joining = joining;
        panel.state = if joining.is_some() {
            RemoteState::Saving(request)
        } else {
            RemoteState::Loading(request)
        };
        if let Some(room) = transition {
            self.begin_optimistic_join(request, &room);
        }
        self.touch();
        Ok(())
    }

    pub(super) fn complete_discovery(
        &mut self,
        request: RequestId,
        account: AccountScope,
        joining: Option<ConversationRef>,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(account).is_err() || self.snapshot.active_account != Some(account) {
            return;
        }
        if joining.is_none()
            && let Err(error) = result.as_ref()
            && transient(*error)
            && let Some(retry) = &mut self.discovery_retry
            && retry.account == account
            && retry.request == request
            && retry.attempts < 2
            && Instant::now() < retry.deadline
        {
            retry.attempts += 1;
            retry.due = Some(Instant::now() + Duration::from_secs(1 << retry.attempts));
            return;
        }
        if joining.is_none() {
            self.discovery_retry = None;
        }
        let Some(panel) = self.snapshot.discover_panel.as_mut().filter(|p| {
            p.account == account
                && p.request == request
                && p.joining == joining
                && p.state.pending()
        }) else {
            return;
        };
        let mut joined = None;
        let completion = match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::OpenConversations(found)))
                if joining.is_none() && found.scope == account =>
            {
                if found
                    .rooms
                    .as_slice()
                    .iter()
                    .any(|room| room.id.account != account.account)
                {
                    Completion::Failed(UnavailableReason::PolicyStale)
                } else {
                    panel.rooms = page(found.rooms.as_slice().iter().map(|room| Room {
                        context: ConversationRef {
                            account,
                            conversation: room.id,
                        },
                        name: room.name.clone(),
                        description: room.description.clone(),
                        participants: room.participants,
                        join: match room.password_required {
                            Some(false) => Decision::Allowed,
                            Some(true) => Decision::Denied,
                            None => Decision::Unknown,
                        },
                    }));
                    panel.loaded = true;
                    panel.support = FeatureSupport::Supported;
                    Completion::ServerAccepted
                }
            }
            Ok(RuntimeReply::Protocol(ProtocolReply::Conversation(room)))
                if joining.is_some_and(|context| context.conversation == room.id) =>
            {
                joined = Some(room);
                Completion::ServerAccepted
            }
            Err(IdentityError::Request(ErrorKind::Unsupported)) if joining.is_none() => {
                panel.support = FeatureSupport::Unsupported;
                panel.rooms = Page::empty();
                Completion::Failed(UnavailableReason::Unsupported)
            }
            Err(error) => completion_error(error),
            _ => Completion::Failed(UnavailableReason::PolicyStale),
        };
        panel.state = match completion {
            Completion::Queued | Completion::ServerAccepted => RemoteState::Idle,
            Completion::Failed(reason) => RemoteState::Failed(request, reason),
            Completion::Uncertain => RemoteState::Uncertain(request),
        };
        panel.uncertain = joining.is_some() && completion == Completion::Uncertain;
        if panel.uncertain {
            self.load_rooms = Some(account);
        }
        let transition = joining.and_then(|context| self.take_optimistic_join(request, context));
        if let Some(room) = joined {
            let row = conversation(&room, account);
            let context = row.context;
            self.snapshot.conversations = page(
                std::iter::once(row).chain(
                    self.snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .filter(|r| r.context != context)
                        .take(99)
                        .cloned(),
                ),
            );
            self.observe_created_room(account, room);
            self.load_rooms = Some(account);
            if transition.is_some() && self.snapshot.selected == Some(context) {
                self.snapshot.selected = None;
                let _ = self.open(context);
            }
        } else if let Some(transition) = transition {
            self.rollback_optimistic_join(transition);
            self.load_rooms = Some(account);
        }
        self.touch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optimistic_join_has_no_authority_and_does_not_override_later_navigation() {
        let mut model = super::super::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
        let old = model.snapshot.selected.unwrap();
        let room = Room {
            context: crate::composition::fixtures::context(7),
            name: text("Synthetic room"),
            description: None,
            participants: None,
            join: Decision::Allowed,
        };
        model.begin_optimistic_join(RequestId(1), &room);
        assert_eq!(model.snapshot.selected, Some(room.context));
        assert_eq!(model.snapshot.content, ContentState::Loading);
        let row = model
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .find(|r| r.context == room.context)
            .unwrap();
        assert_eq!(
            (row.send, row.attach, row.call),
            (Decision::Unknown, Decision::Unknown, Decision::Unknown)
        );
        assert!(row.call_scope.is_none());
        model.snapshot.conversations = page(
            model
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .filter(|r| r.context != room.context)
                .cloned(),
        );
        model.restore_optimistic_join(old.account);
        assert!(
            model
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|r| r.context == room.context)
        );
        model.snapshot.selected = Some(old);
        let join = model
            .take_optimistic_join(RequestId(1), room.context)
            .unwrap();
        model.rollback_optimistic_join(join);
        assert_eq!(
            model.snapshot.selected,
            Some(old),
            "a later navigation remains selected"
        );
        assert!(
            !model
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|r| r.context == room.context)
        );
    }

    #[test]
    fn failed_optimistic_join_restores_the_originating_room() {
        let mut model = super::super::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
        let old = model.snapshot.selected.unwrap();
        let room = Room {
            context: crate::composition::fixtures::context(7),
            name: text("Synthetic room"),
            description: None,
            participants: None,
            join: Decision::Allowed,
        };
        model.begin_optimistic_join(RequestId(2), &room);
        assert!(
            model
                .take_optimistic_join(RequestId(1), room.context)
                .is_none()
        );
        let join = model
            .take_optimistic_join(RequestId(2), room.context)
            .unwrap();
        model.rollback_optimistic_join(join);
        assert_eq!(model.snapshot.selected, Some(old));
    }

    #[test]
    fn optimistic_join_keeps_the_selected_room_and_bounds_related_search_hits() {
        let mut model = super::super::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("account-search"));
        let template = model.snapshot.conversations.as_slice()[0].clone();
        model.snapshot.conversations = page((1..=100).map(|id| {
            let mut row = template.clone();
            row.context = crate::composition::fixtures::context(id);
            row
        }));
        model.snapshot.selected = Some(crate::composition::fixtures::context(100));
        let panel = model.snapshot.search_panel.as_mut().unwrap();
        let mut hit = panel.hits.as_slice()[0].clone();
        hit.context = crate::composition::fixtures::context(99);
        panel.hits = page([hit]);
        let room = Room {
            context: crate::composition::fixtures::context(101),
            name: text("Synthetic room"),
            description: None,
            participants: None,
            join: Decision::Allowed,
        };
        model.begin_optimistic_join(RequestId(3), &room);
        assert_eq!(model.snapshot.conversations.as_slice().len(), 100);
        assert!(
            model
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|r| r.context == crate::composition::fixtures::context(100))
        );
        assert!(
            model
                .snapshot
                .search_panel
                .as_ref()
                .unwrap()
                .hits
                .as_slice()
                .is_empty()
        );
    }
    #[test]
    fn old_capability_probe_cannot_reenable_a_reselected_account() {
        let mut model = super::super::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
        let account = model.snapshot.active_account.unwrap();
        model.snapshot.discover_panel = Some(empty_panel(account));
        let mut probe = Probe::new(account, 2);
        probe.due = None;
        model.discover_probe = Some(probe);
        let result = || {
            Ok(RuntimeReply::Protocol(ProtocolReply::Account(
                protocol::AccountProbe {
                    scope: account,
                    display_name: text("Synthetic"),
                    family: protocol::ServerFamily::Nextcloud35,
                    talk_available: true,
                    group_creation: Decision::Allowed,
                    listable_rooms: protocol::FeatureSupport::Supported,
                    message_limit: Some(32000),
                    reference_correlation: true,
                    channel_semantics: true,
                },
            )))
        };
        model.complete_discovery_probe(account, 1, result());
        assert_eq!(
            model.snapshot.discover_panel.as_ref().unwrap().support,
            FeatureSupport::Unknown
        );
        model.complete_discovery_probe(account, 2, result());
        assert_eq!(
            model.snapshot.discover_panel.as_ref().unwrap().support,
            FeatureSupport::Supported
        );
    }
}
