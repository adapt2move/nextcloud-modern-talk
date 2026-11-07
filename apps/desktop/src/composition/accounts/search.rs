//! Latest account query and at most two result pages; no cursor reaches the view.
use super::*;
use talk_core::ui::{
    conversation::RemoteState,
    search::{Action, Hit, Panel},
};
pub(super) struct Retry {
    request: RequestId,
    account: AccountScope,
    append: bool,
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
impl Accounts {
    pub(super) fn tick_account_search(&mut self) {
        let now = Instant::now();
        let Some(retry) = self.account_search_retry.as_ref() else {
            return;
        };
        if retry.due.is_none_or(|due| now < due) {
            return;
        }
        let (request, account, append, deadline) =
            (retry.request, retry.account, retry.append, retry.deadline);
        if self.snapshot.active_account != Some(account)
            || self
                .snapshot
                .search_panel
                .as_ref()
                .is_none_or(|panel| panel.request != request || panel.account != account)
        {
            self.account_search_retry = None;
            return;
        }
        let Some(query) = self.account_search.clone() else {
            self.account_search_retry = None;
            return;
        };
        match self.request(
            account,
            ProtocolRequest::SearchMessages(query),
            Purpose::Search(request, account, append),
        ) {
            Ok(()) => {
                if let Some(retry) = &mut self.account_search_retry {
                    retry.due = None;
                }
            }
            Err(SubmitError::Busy) if now < deadline => {
                if let Some(retry) = &mut self.account_search_retry {
                    retry.due = Some(now + Duration::from_secs(1));
                }
            }
            Err(_) => {
                self.account_search_retry = None;
                if let Some(panel) = &mut self.snapshot.search_panel {
                    panel.state = RemoteState::Failed(request, UnavailableReason::Busy);
                    panel.more = append;
                }
                self.touch();
            }
        }
    }

    pub(super) fn submit_account_search(
        &mut self,
        account: AccountScope,
        request: RequestId,
        action: Action,
    ) -> Result<(), SubmitError> {
        self.current(account)?;
        if self.snapshot.active_account != Some(account) {
            return Err(SubmitError::StaleScope);
        }
        if self
            .pending
            .iter()
            .any(|p| matches!(p.purpose, Purpose::Search(_, scope, _) if scope == account))
        {
            return Err(SubmitError::Busy);
        }
        let (query, term, filters, append) = match action {
            Action::Search { term, filters } => {
                if term.as_str().trim().is_empty() {
                    return Err(SubmitError::Unavailable(UnavailableReason::Unconfigured));
                }
                if filters.from_self && filters.person.is_some() {
                    return Err(SubmitError::StaleScope);
                }
                if filters.conversation.is_some_and(|c| {
                    c.account != account
                        || !self
                            .snapshot
                            .conversations
                            .as_slice()
                            .iter()
                            .any(|r| r.context == c)
                }) || filters.person.is_some_and(|p| p.account != account)
                {
                    return Err(SubmitError::StaleScope);
                }
                let query = protocol::search::Query {
                    term: term.clone(),
                    conversation: filters.conversation.map(|c| c.conversation),
                    person: filters.person.map(|p| protocol::conversation::RecipientId {
                        scope: account,
                        local: p.local,
                    }),
                    from_self: filters.from_self,
                    since: filters.since.clone(),
                    until: filters.until.clone(),
                    cursor: None,
                };
                (query, term, filters, false)
            }
            Action::More => {
                let panel = self
                    .snapshot
                    .search_panel
                    .as_ref()
                    .filter(|p| p.account == account && p.more && !p.state.pending())
                    .ok_or(SubmitError::StaleScope)?;
                (
                    self.account_search
                        .clone()
                        .filter(|q| q.cursor.is_some())
                        .ok_or(SubmitError::StaleScope)?,
                    panel.term.clone(),
                    panel.filters.clone(),
                    true,
                )
            }
        };
        self.request(
            account,
            ProtocolRequest::SearchMessages(query.clone()),
            Purpose::Search(request, account, append),
        )?;
        let old = self
            .snapshot
            .search_panel
            .take()
            .filter(|p| append && p.account == account);
        self.snapshot.search_panel = Some(Panel {
            account,
            request,
            term,
            filters,
            state: RemoteState::Loading(request),
            hits: old.as_ref().map_or_else(Page::empty, |p| p.hits.clone()),
            loaded: old.is_some(),
            more: false,
        });
        self.account_search = Some(query);
        self.account_search_retry = Some(Retry {
            request,
            account,
            append,
            attempts: 0,
            due: None,
            deadline: Instant::now() + Duration::from_secs(30),
        });
        self.touch();
        Ok(())
    }
    pub(super) fn complete_account_search(
        &mut self,
        request: RequestId,
        account: AccountScope,
        append: bool,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(account).is_err() || self.snapshot.active_account != Some(account) {
            return;
        }
        let Some(panel) = self.snapshot.search_panel.as_ref().filter(|p| {
            p.account == account && p.request == request && p.state == RemoteState::Loading(request)
        }) else {
            return;
        };
        let old = panel.hits.clone();
        if let Err(error) = result.as_ref()
            && transient(*error)
            && let Some(retry) = &mut self.account_search_retry
            && retry.request == request
            && retry.account == account
            && retry.attempts < 2
            && Instant::now() < retry.deadline
        {
            retry.attempts += 1;
            retry.due = Some(Instant::now() + Duration::from_secs(1 << retry.attempts));
            return;
        }
        self.account_search_retry = None;
        match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::SearchMessages(result))) => {
                let mut hits = if append { old } else { Page::empty() };
                for hit in result.hits.as_slice() {
                    let context = ConversationRef {
                        account,
                        conversation: hit.message.conversation,
                    };
                    if !self
                        .background
                        .account(account)
                        .is_some_and(|owner| owner.metadata.readable(context))
                        || !self
                            .snapshot
                            .conversations
                            .as_slice()
                            .iter()
                            .any(|r| r.context == context)
                        || hits.as_slice().iter().any(|h: &Hit| {
                            h.context == context && h.message == u128::from(hit.message.sequence)
                        })
                    {
                        continue;
                    }
                    if hits
                        .try_push(Hit {
                            context,
                            message: u128::from(hit.message.sequence),
                            title: hit.title.clone(),
                            excerpt: hit.excerpt.clone(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                let has_room = hits.as_slice().len() < 100;
                if let Some(query) = &mut self.account_search {
                    query.cursor = result
                        .cursor
                        .filter(|next| query.cursor.as_ref() != Some(next));
                }
                if let Some(panel) = &mut self.snapshot.search_panel {
                    panel.more = has_room
                        && self
                            .account_search
                            .as_ref()
                            .is_some_and(|q| q.cursor.is_some());
                    panel.hits = hits;
                    panel.loaded = true;
                    panel.state = RemoteState::Idle;
                }
            }
            failure => {
                let completion = match failure {
                    Err(error) => completion_error(error),
                    _ => Completion::Failed(UnavailableReason::PolicyUnknown),
                };
                if let Some(panel) = &mut self.snapshot.search_panel {
                    panel.more = append
                        && self
                            .account_search
                            .as_ref()
                            .is_some_and(|query| query.cursor.is_some());
                    panel.state = match completion {
                        Completion::Failed(reason) => RemoteState::Failed(request, reason),
                        _ => RemoteState::Failed(request, UnavailableReason::PolicyUnknown),
                    };
                }
            }
        }
        self.touch();
    }
}
