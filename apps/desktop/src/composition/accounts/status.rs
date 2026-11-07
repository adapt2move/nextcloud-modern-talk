//! Ephemeral account-status intent, not a room/content permission. One per
//! account (eight), five minutes, with read-only uncertainty reconciliation.
use super::*;
use talk_core::preferences::account::{
    AccountPreferenceChange as Change, AccountPreferences, StatusMessageChange,
};
pub(super) struct Retry {
    pub scope: AccountScope,
    pub request: PreferenceRequest,
    change: Change,
    read: bool,
    due: Option<Instant>,
    expires: Instant,
    attempts: u8,
}
impl Retry {
    pub(super) fn new(scope: AccountScope, request: PreferenceRequest, change: Change) -> Self {
        Self {
            scope,
            request,
            change,
            read: false,
            due: None,
            expires: Instant::now() + Duration::from_secs(300),
            attempts: 0,
        }
    }
    pub(super) fn queued(&mut self) {
        self.due = Some(Instant::now() + Duration::from_millis(100));
    }
}
fn confirmed(change: &Change, values: &AccountPreferences) -> bool {
    match change {
        Change::UserPresence(value) => values.user_presence == Some(*value),
        Change::StatusMessage(StatusMessageChange::Clear) => values
            .status_message
            .as_ref()
            .is_none_or(|m| m.icon.is_none() && m.message.is_none()),
        Change::StatusMessage(StatusMessageChange::Custom { icon, message, .. }) => {
            values.status_message.as_ref().is_some_and(|m| {
                m.icon == *icon && m.message.as_ref().map_or("", |s| s.as_str()) == message.as_str()
            })
        }
        Change::StatusMessage(StatusMessageChange::Predefined { id, .. }) => values
            .predefined_statuses
            .as_ref()
            .and_then(|all| all.as_slice().iter().find(|p| &p.id == id))
            .is_some_and(|p| {
                values.status_message.as_ref().is_some_and(|m| {
                    m.icon.as_ref() == Some(&p.icon) && m.message.as_ref() == Some(&p.message)
                })
            }),
        _ => false,
    }
}
impl Accounts {
    pub(super) fn complete_status(
        &mut self,
        scope: AccountScope,
        request: PreferenceRequest,
        result: &Result<RuntimeReply, IdentityError>,
    ) -> bool {
        let Some(index) = self
            .status_retry
            .iter()
            .position(|r| r.scope == scope && r.request == request)
        else {
            return false;
        };
        let mut failure = None;
        let mut values = None;
        match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::Preferences(reply)))
                if reply.request == request && reply.values.scope == scope =>
            {
                if reply.values.user_status_enabled != Some(true) {
                    failure = Some(AccountPreferenceError::Unavailable);
                } else if reply.values.user_status_observed {
                    if confirmed(&self.status_retry[index].change, &reply.values) {
                        values = Some(reply.values.clone());
                    } else if self.status_retry[index].read {
                        failure = Some(AccountPreferenceError::Uncertain);
                    } else {
                        self.status_retry[index].read = true;
                    }
                } else {
                    self.status_retry[index].read = true;
                }
            }
            Err(IdentityError::Request(ErrorKind::Uncertain)) => {
                self.status_retry[index].read = true
            }
            Err(IdentityError::Request(
                ErrorKind::Transport
                | ErrorKind::Deadline
                | ErrorKind::ServerUnavailable
                | ErrorKind::RateLimited
                | ErrorKind::Busy,
            )) => (),
            Err(error) => failure = Some(preference_error(*error)),
            _ => failure = Some(AccountPreferenceError::Unavailable),
        }
        if values.is_some() || failure.is_some() {
            self.status_retry.remove(index);
            self.status_pending
                .retain(|(s, r, _)| *s != scope || *r != request);
            self.message_pending
                .retain(|(s, r)| *s != scope || *r != request);
            if let Some(cache) = self.preferences.iter_mut().find(|p| p.scope == scope) {
                if let Some(values) = values {
                    cache.values = Some(values);
                    cache.state = AccountPreferenceState::Ready;
                } else {
                    cache.state = AccountPreferenceState::Failed {
                        request,
                        error: failure.unwrap_or(AccountPreferenceError::Unavailable),
                    };
                }
            }
        } else {
            let retry = &mut self.status_retry[index];
            retry.attempts = retry.attempts.saturating_add(1).min(5);
            let jitter = (scope.account.as_local() as u64 ^ request.0) % 500;
            retry.due = Some(
                Instant::now()
                    + Duration::from_secs((1u64 << retry.attempts).min(30))
                    + Duration::from_millis(jitter),
            );
        }
        self.touch();
        true
    }
    pub(super) fn tick_status(&mut self) {
        let now = Instant::now();
        self.status_retry.retain(|r| {
            self.snapshot
                .accounts
                .as_slice()
                .iter()
                .any(|a| a.scope == r.scope)
        });
        let expired: Vec<_> = self
            .status_retry
            .iter()
            .filter(|r| now >= r.expires)
            .map(|r| (r.scope, r.request))
            .collect();
        for (scope, request) in expired {
            let read = self
                .status_retry
                .iter()
                .find(|r| r.scope == scope && r.request == request)
                .is_some_and(|r| r.read);
            self.status_retry
                .retain(|r| r.scope != scope || r.request != request);
            self.status_pending
                .retain(|(s, r, _)| *s != scope || *r != request);
            self.message_pending
                .retain(|(s, r)| *s != scope || *r != request);
            if let Some(cache) = self.preferences.iter_mut().find(|p| p.scope == scope) {
                cache.state = AccountPreferenceState::Failed {
                    request,
                    error: if read {
                        AccountPreferenceError::Uncertain
                    } else {
                        AccountPreferenceError::Network
                    },
                };
            }
            self.touch();
        }
        for index in 0..self.status_retry.len() {
            let retry = &self.status_retry[index];
            if retry.due.is_none_or(|due|now<due) || self.pending.iter().any(|p|matches!(p.purpose,Purpose::Preferences(s,r)if s==retry.scope && r==retry.request)) {continue;}
            let scope = retry.scope;
            let request = retry.request;
            let protocol = if retry.read {
                ProtocolRequest::ReadPreferences { request }
            } else {
                ProtocolRequest::SetPreference {
                    request,
                    change: retry.change.clone(),
                }
            };
            if self.current(scope).is_ok()
                && self
                    .admit(
                        RuntimeCommand::Request {
                            scope,
                            request: protocol,
                        },
                        Purpose::Preferences(scope, request),
                    )
                    .is_ok()
            {
                self.status_retry[index].due = None;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uncertain_status_write_waits_for_fresh_observation_without_reissuing_write() {
        let mut model = crate::composition::accounts::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
        let scope = model.snapshot.active_account.unwrap();
        model.status_retry.push(Retry::new(
            scope,
            PreferenceRequest(1),
            Change::UserPresence(talk_core::preferences::account::StatusChoice::Away),
        ));
        assert!(model.complete_status(
            scope,
            PreferenceRequest(1),
            &Err(ErrorKind::Uncertain.into())
        ));
        assert!(model.status_retry[0].read);
        assert!(model.status_retry[0].due.is_some());
        let other = AccountScope {
            runtime: talk_core::ids::RuntimeGeneration::from_local(99),
            ..scope
        };
        assert!(!model.complete_status(
            other,
            PreferenceRequest(1),
            &Err(ErrorKind::PermissionDenied.into())
        ));
        assert_eq!(model.status_retry.len(), 1);
    }
}
