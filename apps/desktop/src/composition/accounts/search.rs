//! Latest account query and at most two result pages; no cursor reaches the view.
use super::*;
use talk_core::ui::{
    conversation::RemoteState,
    search::{Action, Hit, Panel},
};
impl Accounts {
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
