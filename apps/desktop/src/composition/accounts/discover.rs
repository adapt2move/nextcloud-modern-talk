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

impl Accounts {
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
            let _ = self.open(context);
        }
        self.touch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
