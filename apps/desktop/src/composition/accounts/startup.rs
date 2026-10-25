//! One startup restoration plan: at most eight saved accounts, one admission at
//! a time, expiring after 60 seconds. Failures require deliberate user retry.
//! The worker still reads the exact eligible provider and validates identity;
//! `connected_once` is history, never permission to activate a credential.
use super::*;

#[derive(Default)]
pub(super) struct Restoration {
    planned: bool,
    queue: Vec<(AccountId, talk_core::identity::QualifiedItem)>,
    deadline: Option<Instant>,
}

fn eligible(account: &talk_adapters::identity::ConnectionAccount) -> bool {
    account.state == SavedAccountState::Dormant
        && account.summary.scope.is_none()
        && matches!(account.summary.storage, StoredState::Eligible(_))
        && !account.summary.suppression_unconfirmed
}

impl Restoration {
    fn next(
        &mut self,
        snapshot: &ConnectionSnapshot,
        now: Instant,
    ) -> Option<(AccountId, talk_core::identity::QualifiedItem)> {
        if snapshot.state != ConnectionState::Ready {
            return None;
        }
        if !self.planned {
            self.planned = true;
            self.queue = snapshot
                .accounts
                .as_slice()
                .iter()
                .filter(|account| eligible(account))
                .take(8)
                .filter_map(|account| match account.summary.storage {
                    StoredState::Eligible(item) => Some((account.summary.account, item)),
                    _ => None,
                })
                .collect();
            self.deadline = Some(now + Duration::from_secs(60));
        }
        self.queue.retain(|(id, item)| {
            snapshot.accounts.as_slice().iter().any(|account| {
                account.summary.account == *id
                    && eligible(account)
                    && account.summary.storage == StoredState::Eligible(*item)
            })
        });
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.queue.clear();
        }
        if snapshot
            .accounts
            .as_slice()
            .iter()
            .any(|account| account.state == SavedAccountState::Restoring)
        {
            return None;
        }
        self.queue.first().copied()
    }

    fn settled(&mut self, account: AccountId) {
        self.queue.retain(|(id, _)| *id != account);
    }
}

impl Accounts {
    pub(super) fn restore_on_startup(&mut self, snapshot: &ConnectionSnapshot) {
        let Some((account, item)) = self.startup.next(snapshot, Instant::now()) else {
            return;
        };
        // A just-admitted restore may not have entered the worker snapshot yet.
        if self.pending.iter().any(|pending| {
            matches!(
                pending.purpose,
                Purpose::Restore(_) | Purpose::RestoreAutomatic(_)
            )
        }) {
            return;
        }
        match self.admit(
            RuntimeCommand::RestoreExact { account, item },
            Purpose::RestoreAutomatic(account),
        ) {
            Err(IdentityError::Request(ErrorKind::Busy)) => (),
            Ok(_) => self.startup.settled(account),
            Err(error) => {
                self.startup.settled(account);
                self.failures.retain(|(id, _)| *id != account);
                if self.failures.len() < 8 {
                    self.failures.push((account, login_error(error)));
                }
                self.touch();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_adapters::identity::{AccountPhase, ConnectionAccount};
    use talk_core::identity::{ProviderBinding, QualifiedItem};
    use talk_core::ids::{ProfileId, RuntimeGeneration, VaultGeneration};

    fn saved(id: u128) -> ConnectionAccount {
        let account = AccountId::from_local(id);
        ConnectionAccount {
            state: SavedAccountState::Dormant,
            summary: AccountSummary {
                account,
                navigation_key: talk_core::preferences::navigation::NavigationAccountKey::new(
                    &format!("a1:{id:032x}:00000000000000000000000000000001"),
                )
                .unwrap(),
                server_label: Some(text("cloud.example.test")),
                account_label: Some(text("Synthetic account")),
                connected_once: false,
                attempt: None,
                scope: None,
                phase: AccountPhase::Disconnected,
                display_name: text("Synthetic"),
                storage: StoredState::Eligible(QualifiedItem {
                    account,
                    generation: VaultGeneration::from_local(7),
                    provider: ProviderBinding {
                        backend: StorageBackend::MacKeychain,
                        profile: ProfileId::from_local(11),
                        store: 13,
                    },
                }),
                cleanup_pending: false,
                suppression_unconfirmed: false,
                connection_degraded: false,
                connection_error: None,
                recovery: None,
                revision: 1,
            },
        }
    }

    fn ready(accounts: impl IntoIterator<Item = ConnectionAccount>) -> ConnectionSnapshot {
        ConnectionSnapshot {
            state: ConnectionState::Ready,
            accounts: page(accounts),
            pending: Page::empty(),
            retained: 0,
        }
    }

    fn scope(id: u128) -> AccountScope {
        AccountScope {
            account: AccountId::from_local(id),
            runtime: RuntimeGeneration::from_local(31),
        }
    }

    fn candidate(id: u128) -> Option<(AccountId, QualifiedItem)> {
        let StoredState::Eligible(item) = saved(id).summary.storage else {
            unreachable!()
        };
        Some((AccountId::from_local(id), item))
    }

    #[test]
    fn first_ready_snapshot_freezes_the_candidates_and_never_replans() {
        let now = Instant::now();
        let mut plan = Restoration::default();
        let mut starting = ready([saved(1)]);
        starting.state = ConnectionState::Starting;
        assert_eq!(plan.next(&starting, now), None);
        assert_eq!(plan.next(&ready([saved(2), saved(3)]), now), candidate(2));
        plan.settled(AccountId::from_local(2));
        let later = ready([saved(2), saved(3), saved(4)]);
        assert_eq!(plan.next(&later, now), candidate(3));
        plan.settled(AccountId::from_local(3));
        assert_eq!(plan.next(&later, now), None);
    }

    #[test]
    fn an_empty_first_ready_snapshot_does_not_restore_accounts_added_later() {
        let now = Instant::now();
        let mut plan = Restoration::default();
        assert_eq!(plan.next(&ready([]), now), None);
        assert_eq!(plan.next(&ready([saved(1)]), now), None);
    }

    #[test]
    fn history_marker_is_not_credential_authority() {
        for connected_once in [false, true] {
            let mut candidate = saved(1);
            candidate.summary.connected_once = connected_once;
            assert!(
                eligible(&candidate),
                "eligible credentials do not require a history flag"
            );
            for storage in [
                StoredState::Unknown,
                StoredState::Suppressed,
                StoredState::Unresolved,
                StoredState::SessionOnly,
            ] {
                candidate.summary.storage = storage;
                assert!(
                    !eligible(&candidate),
                    "history never makes an ineligible record restorable"
                );
            }
        }
        for state in [
            SavedAccountState::Restoring,
            SavedAccountState::WaitingForBrowser,
            SavedAccountState::AwaitingStorageChoice,
            SavedAccountState::Active,
            SavedAccountState::ReauthenticationRequired,
            SavedAccountState::RecoveryRequired,
        ] {
            let mut candidate = saved(1);
            candidate.state = state;
            assert!(!eligible(&candidate));
        }
        let mut candidate = saved(1);
        candidate.summary.suppression_unconfirmed = true;
        assert!(!eligible(&candidate));
        candidate.summary.suppression_unconfirmed = false;
        candidate.summary.scope = Some(scope(1));
        assert!(!eligible(&candidate));
    }

    #[test]
    fn eight_accounts_are_admitted_in_order_and_settled_work_is_not_retried() {
        let now = Instant::now();
        let mut plan = Restoration::default();
        let snapshot = ready((1..=8).map(saved));
        for id in 1..=8 {
            assert_eq!(plan.next(&snapshot, now), candidate(id));
            // Admission or a terminal error consumes this candidate, even while
            // its old Dormant state is still present in the worker snapshot.
            plan.settled(AccountId::from_local(id));
        }
        assert_eq!(plan.next(&snapshot, now), None);
        assert_eq!(plan.next(&ready([saved(9)]), now), None);
    }

    #[test]
    fn busy_admission_may_retry_only_until_the_fixed_startup_deadline() {
        let now = Instant::now();
        let snapshot = ready([saved(1)]);
        let mut plan = Restoration::default();
        assert_eq!(plan.next(&snapshot, now), candidate(1));
        assert_eq!(
            plan.next(&snapshot, now + Duration::from_secs(30)),
            candidate(1)
        );
        assert_eq!(
            plan.next(&snapshot, now + Duration::from_millis(59_999)),
            candidate(1)
        );
        assert_eq!(plan.next(&snapshot, now + Duration::from_secs(60)), None);
        assert_eq!(plan.next(&snapshot, now + Duration::from_secs(61)), None);
    }

    #[test]
    fn a_restoring_account_blocks_the_next_admission_until_it_settles() {
        let now = Instant::now();
        let mut plan = Restoration::default();
        assert_eq!(plan.next(&ready([saved(1), saved(2)]), now), candidate(1));
        plan.settled(AccountId::from_local(1));
        let mut first = saved(1);
        first.state = SavedAccountState::Restoring;
        assert_eq!(plan.next(&ready([first.clone(), saved(2)]), now), None);
        first.state = SavedAccountState::Active;
        first.summary.scope = Some(scope(1));
        assert_eq!(plan.next(&ready([first, saved(2)]), now), candidate(2));
    }

    #[test]
    fn removal_logout_or_suppression_permanently_retires_a_waiting_candidate() {
        let now = Instant::now();
        for change in 0..5 {
            let mut plan = Restoration::default();
            let original = ready([saved(1), saved(2)]);
            assert_eq!(plan.next(&original, now), candidate(1));
            plan.settled(AccountId::from_local(1));
            let mut second = saved(2);
            match change {
                0 => second.summary.storage = StoredState::Suppressed,
                1 => second.summary.suppression_unconfirmed = true,
                2 => second.summary.scope = Some(scope(2)),
                3 => second.state = SavedAccountState::ReauthenticationRequired,
                _ => (),
            }
            let changed = if change == 4 {
                ready([])
            } else {
                ready([second])
            };
            assert_eq!(plan.next(&changed, now), None);
            assert_eq!(
                plan.next(&original, now),
                None,
                "a later eligible snapshot cannot re-add retired work"
            );
        }
    }

    #[test]
    fn replacement_credentials_do_not_inherit_an_older_startup_candidate() {
        let now = Instant::now();
        for change in 0..3 {
            let mut plan = Restoration::default();
            let original = ready([saved(1), saved(2)]);
            assert_eq!(plan.next(&original, now), candidate(1));
            plan.settled(AccountId::from_local(1));
            let mut replacement = saved(2);
            let StoredState::Eligible(mut item) = replacement.summary.storage else {
                unreachable!()
            };
            match change {
                0 => item.generation = VaultGeneration::from_local(8),
                1 => item.provider.profile = ProfileId::from_local(12),
                _ => item.provider.store = 14,
            }
            replacement.summary.storage = StoredState::Eligible(item);
            assert_eq!(plan.next(&ready([replacement]), now), None);
            assert_eq!(plan.next(&original, now), None);
        }
    }

    fn active_completion(id: u128, current: bool) -> ConnectionCompletion {
        let scope = scope(id);
        let mut summary = saved(id).summary;
        summary.scope = Some(scope);
        summary.phase = AccountPhase::Active;
        ConnectionCompletion {
            cursor: ConnectionCursor::default(),
            request: ConnectionRequestId(1),
            account: Some(scope.account),
            attempt: None,
            captured_scope: None,
            cancel_requested: false,
            scope_is_current: current,
            result: Ok(RuntimeReply::Active {
                summary,
                probe: protocol::AccountProbe {
                    scope,
                    display_name: text("Synthetic"),
                    family: protocol::ServerFamily::Nextcloud35,
                    talk_available: true,
                    group_creation: talk_core::policy::Decision::Unknown,
                    message_limit: None,
                    reference_correlation: true,
                    channel_semantics: false,
                },
            }),
        }
    }

    #[test]
    fn automatic_completion_preserves_the_account_the_user_selected() {
        let mut model = super::super::tests::model();
        model.snapshot.active_account = Some(scope(9));
        let done = active_completion(1, true);
        model.complete(Purpose::RestoreAutomatic(AccountId::from_local(1)), &done);
        assert_eq!(model.snapshot.active_account, Some(scope(9)));
    }

    #[test]
    fn pending_admission_blocks_another_restore_before_the_worker_snapshot_changes() {
        for purpose in [
            Purpose::Restore(AccountId::from_local(9)),
            Purpose::RestoreAutomatic(AccountId::from_local(9)),
        ] {
            let mut model = super::super::tests::model();
            model.pending.push(Pending {
                id: ConnectionRequestId(1),
                purpose,
            });
            let snapshot = ready([saved(1)]);
            model.restore_on_startup(&snapshot);
            assert_eq!(model.pending.len(), 1);
            assert!(model.failures.is_empty());
            assert_eq!(model.startup.next(&snapshot, Instant::now()), candidate(1));
        }
    }

    #[test]
    fn terminal_admission_failure_is_reported_once_and_requires_deliberate_retry() {
        let mut model = super::super::tests::model();
        let snapshot = ready([saved(1)]);
        model.restore_on_startup(&snapshot);
        assert_eq!(model.failures.len(), 1);
        assert_eq!(model.failures[0].0, AccountId::from_local(1));
        assert_eq!(model.startup.next(&snapshot, Instant::now()), None);
        model.restore_on_startup(&snapshot);
        assert_eq!(model.failures.len(), 1);
        assert!(model.pending.is_empty());
    }

    #[test]
    fn stale_automatic_activation_cannot_reselect_a_logged_out_account() {
        let mut model = super::super::tests::model();
        model.snapshot.active_account = None;
        model.snapshot.accounts = Page::empty();
        let done = active_completion(1, false);
        model.complete(Purpose::RestoreAutomatic(AccountId::from_local(1)), &done);
        assert!(model.snapshot.active_account.is_none());
        assert!(model.load_rooms.is_none());
    }
}
