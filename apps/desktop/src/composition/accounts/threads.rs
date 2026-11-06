//! Account-scoped, bounded subscribed-thread observation.
use super::*;
use talk_core::ui::presentation::ThreadView;

impl Accounts {
    pub(super) fn refresh_threads(&mut self, scope: AccountScope) -> Result<(), SubmitError> {
        self.current(scope)?;
        if self.snapshot.active_account != Some(scope) {
            return Err(SubmitError::StaleScope);
        }
        if self
            .pending
            .iter()
            .any(|pending| matches!(pending.purpose, Purpose::Threads(s) if s == scope))
        {
            return Err(SubmitError::Busy);
        }
        self.request(
            scope,
            ProtocolRequest::SubscribedThreads { offset: 0 },
            Purpose::Threads(scope),
        )?;
        self.snapshot.threads.account = Some(scope);
        self.snapshot.threads.content = ContentState::Loading;
        self.touch();
        Ok(())
    }

    pub(super) fn complete_threads(
        &mut self,
        scope: AccountScope,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.snapshot.active_account != Some(scope) || self.current(scope).is_err() {
            return;
        }
        match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::SubscribedThreads(threads)))
                if threads.as_slice().iter().all(|thread| {
                    thread.root.conversation.account == scope.account && thread.root.sequence > 0
                }) =>
            {
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
                            .any(|room| room.context == context)
                            .then(|| ThreadView {
                                context,
                                root: u128::from(thread.root.sequence),
                                title: thread.title.clone(),
                                replies: thread.replies,
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
            Err(error) => self.snapshot.threads.content = content_error(error),
            _ => self.snapshot.threads.content = ContentState::Failed,
        }
        self.touch();
    }
}
