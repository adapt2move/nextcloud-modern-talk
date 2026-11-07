//! Account-scoped, bounded subscribed-thread observation.
use super::*;
use talk_core::ui::presentation::{ThreadList, ThreadView};

impl Accounts {
    pub(super) fn tick_thread_refresh(&mut self, now: Instant) {
        let Some((scope, offset, due)) = self.thread_refresh_due else {
            return;
        };
        if self.snapshot.active_account != Some(scope)
            || self.snapshot.threads.support == FeatureSupport::Unsupported
        {
            self.thread_refresh_due = None;
            return;
        }
        if now < due {
            return;
        }
        match self.load_threads(scope, offset) {
            Ok(()) => self.thread_refresh_due = None,
            Err(SubmitError::Busy) => {
                self.thread_refresh_due = Some((scope, offset, now + Duration::from_secs(2)));
            }
            Err(_) => {
                self.thread_refresh_due = Some((scope, offset, now + Duration::from_secs(30)));
            }
        }
    }

    pub(super) fn hint_thread_refresh(&mut self, scope: AccountScope, now: Instant) {
        if self.snapshot.active_account == Some(scope)
            && self.snapshot.threads.support == FeatureSupport::Supported
        {
            let soon = now + Duration::from_secs(2);
            self.thread_refresh_due = Some(match self.thread_refresh_due {
                Some((current, offset, due)) if current == scope && due <= soon => {
                    (scope, offset, due)
                }
                _ => (scope, self.snapshot.threads.offset, soon),
            });
        }
    }
    pub(super) fn refresh_threads(&mut self, scope: AccountScope) -> Result<(), SubmitError> {
        self.load_threads(scope, 0)
    }
    pub(super) fn page_threads(
        &mut self,
        scope: AccountScope,
        offset: u32,
    ) -> Result<(), SubmitError> {
        let current = self.snapshot.threads.offset;
        let previous = current.checked_sub(50) == Some(offset);
        let next = self.snapshot.threads.has_next && current.checked_add(50) == Some(offset);
        if !previous && !next {
            return Err(SubmitError::StaleScope);
        }
        self.load_threads(scope, offset)
    }
    fn load_threads(&mut self, scope: AccountScope, offset: u32) -> Result<(), SubmitError> {
        self.current(scope)?;
        if self.snapshot.active_account != Some(scope) {
            return Err(SubmitError::StaleScope);
        }
        if self
            .pending
            .iter()
            .any(|pending| matches!(pending.purpose, Purpose::Threads(s, _) if s == scope))
        {
            return Err(SubmitError::Busy);
        }
        self.request(
            scope,
            ProtocolRequest::SubscribedThreads { offset },
            Purpose::Threads(scope, offset),
        )?;
        self.snapshot.threads.account = Some(scope);
        self.snapshot.threads.content = ContentState::Loading;
        self.touch();
        Ok(())
    }

    pub(super) fn complete_threads(
        &mut self,
        scope: AccountScope,
        offset: u32,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.snapshot.active_account != Some(scope) || self.current(scope).is_err() {
            return;
        }
        let mut retry = false;
        match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::SubscribedThreads { threads, has_next }))
                if threads.as_slice().iter().all(|thread| {
                    thread.root.conversation.account == scope.account
                        && thread.root.sequence > 0
                        && thread.last_message.is_none_or(|last| {
                            last.conversation == thread.root.conversation
                                && last.sequence >= thread.root.sequence
                        })
                }) =>
            {
                self.snapshot.threads.offset = offset;
                self.snapshot.threads.has_next = has_next;
                self.snapshot.threads.entries =
                    page(threads.as_slice().iter().filter_map(|thread| {
                        let context = ConversationRef {
                            account: scope,
                            conversation: thread.root.conversation,
                        };
                        self.snapshot
                            .conversations
                            .as_slice()
                            .iter()
                            .find(|room| room.context == context)
                            .map(|_| ThreadView {
                                context,
                                root: u128::from(thread.root.sequence),
                                title: thread.title.clone(),
                                replies: thread.replies,
                                last_message: thread
                                    .last_message
                                    .map(|last| u128::from(last.sequence)),
                            })
                    }));
                self.snapshot.threads.support = FeatureSupport::Supported;
                self.snapshot.threads.content = ContentState::Ready;
            }
            Err(IdentityError::Request(ErrorKind::Unsupported)) => {
                self.snapshot.threads.entries = Page::empty();
                self.snapshot.threads.support = FeatureSupport::Unsupported;
                self.snapshot.threads.content = ContentState::Unconfigured;
            }
            Err(error) => retry = record_thread_failure(&mut self.snapshot.threads, error),
            _ => self.snapshot.threads.content = ContentState::Failed,
        }
        if retry || self.snapshot.threads.content == ContentState::Ready {
            let next = Instant::now() + Duration::from_secs(30);
            self.thread_refresh_due = Some(match self.thread_refresh_due {
                Some((current, _, due)) if current == scope => (scope, offset, due.min(next)),
                _ => (scope, offset, next),
            });
        } else {
            self.thread_refresh_due = None;
        }
        self.touch();
    }
}

/// Keep the last useful page visible while transient failures retry in the background.
fn record_thread_failure(list: &mut ThreadList, error: IdentityError) -> bool {
    if matches!(
        error,
        IdentityError::Request(
            ErrorKind::Transport
                | ErrorKind::Deadline
                | ErrorKind::ServerUnavailable
                | ErrorKind::RateLimited
                | ErrorKind::Busy
        )
    ) {
        list.content = if list.support == FeatureSupport::Supported {
            ContentState::Ready
        } else {
            ContentState::Loading
        };
        true
    } else {
        list.content = content_error(error);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_hints_coalesce_without_delaying_the_first_refresh() {
        let mut model = super::super::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("thread-list"));
        let scope = model.snapshot.active_account.unwrap();
        let now = Instant::now();
        model.hint_thread_refresh(scope, now);
        assert_eq!(
            model.thread_refresh_due,
            Some((scope, 0, now + Duration::from_secs(2)))
        );
        model.hint_thread_refresh(scope, now + Duration::from_secs(1));
        assert_eq!(
            model.thread_refresh_due,
            Some((scope, 0, now + Duration::from_secs(2)))
        );
        model.snapshot.threads.support = FeatureSupport::Unsupported;
        model.tick_thread_refresh(now + Duration::from_secs(3));
        assert!(model.thread_refresh_due.is_none());
    }

    #[test]
    fn transient_failures_keep_the_observed_page_and_retry_discovery() {
        let snapshot = crate::composition::fixtures::snapshot(Some("thread-list"));
        let mut list = snapshot.threads;
        let entries = list.entries.clone();
        for kind in [
            ErrorKind::Transport,
            ErrorKind::Deadline,
            ErrorKind::ServerUnavailable,
            ErrorKind::RateLimited,
            ErrorKind::Busy,
        ] {
            assert!(record_thread_failure(
                &mut list,
                IdentityError::Request(kind)
            ));
            assert_eq!(list.content, ContentState::Ready);
            assert_eq!(list.entries, entries);
        }
        list.support = FeatureSupport::Unknown;
        list.entries = Page::empty();
        assert!(record_thread_failure(
            &mut list,
            IdentityError::Request(ErrorKind::Transport)
        ));
        assert_eq!(list.content, ContentState::Loading);
        assert!(!record_thread_failure(
            &mut list,
            IdentityError::Request(ErrorKind::PermissionDenied)
        ));
        assert_eq!(list.content, ContentState::Denied);

        let mut model = super::super::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("thread-list"));
        let scope = model.snapshot.active_account.unwrap();
        model.snapshot.threads.support = FeatureSupport::Unknown;
        let now = Instant::now();
        model.thread_refresh_due = Some((scope, 50, now));
        model.tick_thread_refresh(now);
        assert_eq!(
            model.thread_refresh_due,
            Some((scope, 50, now + Duration::from_secs(30)))
        );
    }
}
