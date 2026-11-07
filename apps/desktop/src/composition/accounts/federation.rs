//! Bounded account-wide invitations and a single optimistic change per account.
use super::*;
use talk_core::ui::presentation::{FederationInvitationView, FederationInvitationsView};

struct Entry {
    scope: AccountScope,
    observed: Option<protocol::federation::Invitations>,
    due: Instant,
    failures: u8,
    version: u64,
    pending: Option<(OperationId, u128)>,
    // An ambiguous write is not offered again until a fresh observation has
    // had time to settle. No accept/decline is replayed automatically.
    uncertain: Option<(u128, Instant)>,
    failed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_invitation_is_hidden_until_reconciliation_releases_it() {
        let mut model = super::super::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
        let scope = model.snapshot.active_account.unwrap();
        let entry = model.federation.entry(scope).unwrap();
        entry.observed = Some(protocol::federation::Invitations {
            scope,
            supported: true,
            items: page([protocol::federation::Invitation {
                id: 9,
                room: text("Synthetic room"),
                inviter: text("Synthetic inviter"),
            }]),
        });
        entry.uncertain = Some((9, Instant::now() + Duration::from_secs(30)));
        model.project_federation();
        assert!(
            model
                .snapshot
                .federation_invitations
                .as_ref()
                .unwrap()
                .invitations
                .as_slice()
                .is_empty()
        );
        model.federation.entry(scope).unwrap().uncertain = None;
        model.project_federation();
        assert_eq!(
            model
                .snapshot
                .federation_invitations
                .as_ref()
                .unwrap()
                .invitations
                .as_slice()
                .len(),
            1
        );
    }
}
#[derive(Default)]
pub(super) struct Federation {
    entries: Vec<Entry>,
}
impl Federation {
    pub(super) fn retain_accounts(&mut self, scopes: &[AccountScope]) {
        self.entries.retain(|entry| scopes.contains(&entry.scope));
    }
    fn entry(&mut self, scope: AccountScope) -> Option<&mut Entry> {
        if !self.entries.iter().any(|entry| entry.scope == scope) {
            if self.entries.len() == 8 {
                return None;
            }
            self.entries.push(Entry {
                scope,
                observed: None,
                due: Instant::now(),
                failures: 0,
                version: 0,
                pending: None,
                uncertain: None,
                failed: false,
            });
        }
        self.entries.iter_mut().find(|entry| entry.scope == scope)
    }
}
impl Accounts {
    pub(super) fn refresh_federation(&mut self, scope: AccountScope) {
        if let Some(entry) = self.federation.entry(scope) {
            entry.due = Instant::now();
        }
    }
    pub(super) fn tick_federation(&mut self) {
        self.project_federation();
        let Some(scope) = self.snapshot.active_account else {
            return;
        };
        if self.snapshot.conversation_list.content != ContentState::Ready || self.pending.len() >= 12
            || self.pending.iter().any(|pending| matches!(pending.purpose, Purpose::FederationInvitations(s, _) if s == scope))
        { return; }
        let Some(entry) = self.federation.entry(scope) else {
            return;
        };
        let now = Instant::now();
        if now < entry.due || entry.pending.is_some() {
            return;
        }
        entry.due = now + Duration::from_secs(5);
        let version = entry.version;
        let _ = self.request(
            scope,
            ProtocolRequest::FederationInvitations,
            Purpose::FederationInvitations(scope, version),
        );
    }
    pub(super) fn complete_federation(
        &mut self,
        scope: AccountScope,
        version: u64,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(scope).is_err() {
            return;
        }
        let Some(entry) = self.federation.entry(scope) else {
            return;
        };
        if entry.version != version {
            entry.due = Instant::now();
            return;
        }
        match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::FederationInvitations(value)))
                if value.scope == scope =>
            {
                entry.failures = 0;
                let now = Instant::now();
                if let Some((id, deadline)) = entry.uncertain
                    && (!value.items.as_slice().iter().any(|item| item.id == id) || now >= deadline)
                {
                    entry.uncertain = None;
                }
                entry.due = if let Some((_, deadline)) = entry.uncertain {
                    (now + Duration::from_secs(5)).min(deadline)
                } else {
                    now + Duration::from_secs(if value.supported { 60 } else { 3600 })
                };
                entry.observed = Some(value);
            }
            _ => {
                entry.failures = entry.failures.saturating_add(1).min(6);
                entry.due = Instant::now() + Duration::from_secs((5u64 << entry.failures).min(300));
            }
        }
        self.project_federation();
    }
    pub(super) fn change_federation(
        &mut self,
        scope: AccountScope,
        invitation: u128,
        accept: bool,
    ) -> Result<(), SubmitError> {
        self.current(scope)?;
        if self.snapshot.active_account != Some(scope) {
            return Err(SubmitError::StaleScope);
        }
        let entry = self.federation.entry(scope).ok_or(SubmitError::Busy)?;
        if entry.pending.is_some() || entry.uncertain.is_some() {
            return Err(SubmitError::Busy);
        }
        if !entry.observed.as_ref().is_some_and(|value| {
            value.supported
                && value
                    .items
                    .as_slice()
                    .iter()
                    .any(|item| item.id == invitation)
        }) {
            return Err(SubmitError::StaleScope);
        }
        let operation = self
            .connection
            .as_ref()
            .map_err(|error| submit_error(*error))?
            .new_operation_id(scope)
            .map_err(submit_error)?;
        self.request(
            scope,
            ProtocolRequest::ChangeFederationInvitation {
                operation,
                invitation,
                accept,
            },
            Purpose::FederationChange(scope, operation, invitation),
        )?;
        if let Some(entry) = self.federation.entry(scope) {
            entry.version = entry.version.saturating_add(1);
            entry.pending = Some((operation, invitation));
            entry.failed = false;
        }
        self.project_federation();
        Ok(())
    }
    pub(super) fn complete_federation_change(
        &mut self,
        scope: AccountScope,
        operation: OperationId,
        invitation: u128,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(scope).is_err() {
            return;
        }
        let Some(entry) = self.federation.entry(scope) else {
            return;
        };
        if entry.pending != Some((operation, invitation)) {
            return;
        }
        entry.pending = None;
        entry.version = entry.version.saturating_add(1);
        entry.due = Instant::now();
        let confirmed = matches!(&result, Ok(RuntimeReply::Protocol(ProtocolReply::FederationInvitationChanged { operation: returned, invitation: id })) if *returned == operation && *id == invitation);
        if confirmed {
            if let Some(observed) = entry.observed.as_mut() {
                observed.items = page(
                    observed
                        .items
                        .as_slice()
                        .iter()
                        .filter(|item| item.id != invitation)
                        .cloned(),
                );
            }
        } else if matches!(
            result,
            Ok(_)
                | Err(IdentityError::Request(
                    ErrorKind::Transport
                        | ErrorKind::Deadline
                        | ErrorKind::RateLimited
                        | ErrorKind::ServerUnavailable
                        | ErrorKind::WrongScope
                        | ErrorKind::Cancelled
                        | ErrorKind::Uncertain
                ))
        ) {
            entry.uncertain = Some((invitation, Instant::now() + Duration::from_secs(30)));
        } else {
            entry.failed = true;
        }
        self.project_federation();
        if confirmed {
            self.invalidate_account(scope, protocol::AccountChange::Conversations);
        }
    }
    pub(super) fn project_federation(&mut self) {
        let value = self.snapshot.active_account.and_then(|scope| {
            let entry = self
                .federation
                .entries
                .iter()
                .find(|entry| entry.scope == scope)?;
            let observed = entry.observed.as_ref()?;
            Some(FederationInvitationsView {
                scope,
                support: if observed.supported {
                    FeatureSupport::Supported
                } else {
                    FeatureSupport::Unsupported
                },
                invitations: page(
                    observed
                        .items
                        .as_slice()
                        .iter()
                        .filter(|item| entry.pending.is_none_or(|(_, id)| id != item.id))
                        .filter(|item| entry.uncertain.is_none_or(|(id, _)| id != item.id))
                        .map(|item| FederationInvitationView {
                            id: item.id,
                            room: item.room.clone(),
                            inviter: item.inviter.clone(),
                        }),
                ),
                pending: entry.pending.is_some() || entry.uncertain.is_some(),
                failed: entry.failed,
            })
        });
        if self.snapshot.federation_invitations != value {
            self.snapshot.federation_invitations = value;
            self.touch();
        }
    }
}
