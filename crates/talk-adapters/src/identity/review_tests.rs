//! Independent R1 findings: deterministic author regressions, not review approval.
use super::super::runtime::AccountSummary;
use super::*;
use serde_json::json;

async fn active(runtime: &IdentityRuntime, server: &Server) -> AccountSummary {
    let (account, attempt) = login(runtime, server, None).await;
    let RuntimeReply::Active { summary, .. } = command(
        runtime,
        RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::PrivateFile,
        },
    )
    .await
    .unwrap() else {
        panic!("active");
    };
    summary
}
fn summary(runtime: &IdentityRuntime, account: talk_core::ids::AccountId) -> AccountSummary {
    runtime
        .snapshot()
        .as_slice()
        .iter()
        .find(|s| s.account == account)
        .unwrap()
        .clone()
}

#[cfg(unix)]
async fn failed_suppression(loss: bool) {
    use std::os::unix::fs::PermissionsExt;
    let server = Server::start(35).await;
    let other = Server::start(34).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let a = active(&runtime, &server).await;
    let b = active(&runtime, &other).await;
    let closed = runtime.account_cancellation(a.scope.unwrap()).unwrap();
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    if loss {
        server.revoked.store(true, Ordering::Release);
        assert!(
            command(
                &runtime,
                RuntimeCommand::Request {
                    scope: a.scope.unwrap(),
                    request: ProtocolRequest::ProbeAccount,
                }
            )
            .await
            .is_err()
        );
    } else {
        runtime.logout(a.account).unwrap();
    }
    wait_phase(
        &runtime,
        a.account,
        if loss {
            AccountPhase::ReauthenticationRequired
        } else {
            AccountPhase::Disconnected
        },
    )
    .await;
    let initial = summary(&runtime, a.account);
    let _ = command(&runtime, RuntimeCommand::Maintain { account: a.account }).await;
    let maintained = summary(&runtime, a.account);
    let RuntimeReply::StorageStatus {
        revision, items, ..
    } = command(
        &runtime,
        RuntimeCommand::StorageStatus { account: a.account },
    )
    .await
    .unwrap()
    else {
        panic!("status");
    };
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    server.revoked.store(false, Ordering::Release);
    let restored_without_ack =
        command(&runtime, RuntimeCommand::Restore { account: a.account }).await;
    // Keep all observations before assertions so one failure does not mask the trace.
    assert!(closed.is_cancelled());
    assert!(runtime.account_cancellation(b.scope.unwrap()).is_ok());
    assert!(
        initial.suppression_unconfirmed && maintained.suppression_unconfirmed,
        "failed restrictive obligation disappeared after maintenance"
    );
    assert_eq!(
        (initial.revision, maintained.revision, revision),
        (a.revision, a.revision, a.revision),
        "failed commit fabricated a revision"
    );
    assert_eq!(maintained.storage, StoredState::Unknown);
    assert!(items.as_slice().iter().all(|item| !item.eligible));
    assert!(
        restored_without_ack.is_err(),
        "live rejected P readmitted before suppression acknowledgement"
    );
    let RuntimeReply::State(committed) =
        command(&runtime, RuntimeCommand::Maintain { account: a.account })
            .await
            .unwrap()
    else {
        panic!("retry");
    };
    assert!(!committed.suppression_unconfirmed);
    assert_eq!(committed.storage, StoredState::Suppressed);
    assert_eq!(committed.revision, a.revision + 1);
    assert!(
        command(&runtime, RuntimeCommand::Restore { account: a.account })
            .await
            .is_err()
    );
    assert!(runtime.join());
    let ledger = fixture.ledger();
    assert_eq!(ledger.account(a.account).unwrap().disposition, 0);
    assert!(ledger.account(b.account).unwrap().eligible.is_some());
    drop(ledger);
    let restarted =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    command(&restarted, RuntimeCommand::Accounts).await.unwrap();
    assert!(
        command(&restarted, RuntimeCommand::Restore { account: a.account })
            .await
            .is_err()
    );
    assert!(
        command(&restarted, RuntimeCommand::Restore { account: b.account })
            .await
            .is_ok()
    );
    assert!(restarted.join());
}
#[cfg(unix)]
#[tokio::test]
async fn r1_failed_logout_retains_restrictive_owner() {
    failed_suppression(false).await;
}
#[cfg(unix)]
#[tokio::test]
async fn r1_failed_confirmed_loss_retains_restrictive_owner() {
    failed_suppression(true).await;
}

#[tokio::test]
async fn r1_malformed_success_probe_preserves_account_and_ledger() {
    let server = Server::start(35).await;
    let other = Server::start(34).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let a = active(&runtime, &server).await;
    let b = active(&runtime, &other).await;
    let token = runtime.account_cancellation(a.scope.unwrap()).unwrap();
    for data in [
        json!({}),
        json!({"id":null}),
        json!({"id":42}),
        json!({"id":""}),
        json!({"id":"x".repeat(257)}),
        json!({"id":"bad\nidentifier"}),
    ] {
        *server.probe_override.lock().unwrap() = Some((
            200,
            json!({"ocs":{"meta":{"status":"ok","statuscode":200},"data":data}}),
        ));
        for request in [
            ProtocolRequest::ProbeAccount,
            ProtocolRequest::InspectAccount,
        ] {
            assert!(
                matches!(
                    command(
                        &runtime,
                        RuntimeCommand::Request {
                            scope: a.scope.unwrap(),
                            request
                        }
                    )
                    .await,
                    Err(IdentityError::Request(ErrorKind::Malformed))
                ),
                "invalid canonical data became identity loss"
            );
            let current = summary(&runtime, a.account);
            assert_eq!(
                (current.revision, current.storage, current.scope),
                (a.revision, a.storage, a.scope)
            );
            assert!(!token.is_cancelled());
            assert!(current.connection_degraded);
        }
    }
    assert!(runtime.account_cancellation(b.scope.unwrap()).is_ok());
    assert!(runtime.join());
    let ledger = fixture.ledger();
    assert_eq!(ledger.account(a.account).unwrap().revision, a.revision);
    assert!(ledger.account(a.account).unwrap().eligible.is_some());
}

#[tokio::test]
async fn r1_scheduled_malformed_success_is_nonrevoking() {
    let server = Server::start(34).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let a = active(&runtime, &server).await;
    let token = runtime.account_cancellation(a.scope.unwrap()).unwrap();
    server.rate_limited.store(true, Ordering::Release);
    let _ = command(
        &runtime,
        RuntimeCommand::Request {
            scope: a.scope.unwrap(),
            request: ProtocolRequest::ProbeAccount,
        },
    )
    .await;
    *server.probe_override.lock().unwrap() = Some((
        200,
        json!({"ocs":{"meta":{"status":"ok","statuscode":200},"data":{}}}),
    ));
    let count = server.user_probes.load(Ordering::Acquire);
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while server.user_probes.load(Ordering::Acquire) == count {
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    command(
        &runtime,
        RuntimeCommand::StorageStatus { account: a.account },
    )
    .await
    .unwrap();
    assert!(
        !token.is_cancelled(),
        "scheduled malformed probe revoked the account"
    );
    let current = summary(&runtime, a.account);
    assert_eq!(
        (current.revision, current.storage, current.scope),
        (a.revision, a.storage, a.scope)
    );
    assert!(current.connection_degraded);
    assert!(runtime.join());
}

struct ReplyRelease(Arc<super::super::provider::faults::ReplyGate>);
impl Drop for ReplyRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}
async fn reply_entered(gate: &super::super::provider::faults::ReplyGate) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    while !gate.entered.load(Ordering::Acquire) {
        assert!(
            tokio::time::Instant::now() < until,
            "after-send barrier not reached"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}
#[tokio::test]
async fn r1_unread_authorized_poll_cannot_save_after_enqueue() {
    for replace in [false, true] {
        let server = Server::start(35).await;
        let other = Server::start(34).await;
        let fixture = Fixture::new();
        let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
        let faults = config.faults.clone();
        let runtime = IdentityRuntime::start(config).unwrap();
        let b = active(&runtime, &other).await;
        let previous = if replace {
            Some(active(&runtime, &server).await)
        } else {
            None
        };
        let RuntimeReply::Begun { account, attempt } = command(
            &runtime,
            RuntimeCommand::Begin {
                server: Text::new(&server.origin).unwrap(),
                replace: previous.as_ref().map(|p| p.account),
            },
        )
        .await
        .unwrap() else {
            panic!("begin");
        };
        server.authorized.store(true, Ordering::Release);
        let gate = ReplyRelease(faults.arm_reply());
        let poll = runtime
            .try_submit(RuntimeCommand::Poll { account, attempt })
            .unwrap();
        reply_entered(&gate.0).await;
        drop(poll); // Exactly after successful send, not snapshot-based scheduling.
        gate.0.release();
        let result = command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt,
                choice: StorageChoice::PrivateFile,
            },
        )
        .await;
        assert!(
            result.is_err(),
            "unread Poll retained save authority after enqueue"
        );
        assert!(runtime.account_cancellation(b.scope.unwrap()).is_ok());
        if let Some(p) = previous.as_ref() {
            assert!(runtime.account_cancellation(p.scope.unwrap()).is_ok());
        }
        let RuntimeReply::StorageStatus { items, .. } =
            command(&runtime, RuntimeCommand::StorageStatus { account })
                .await
                .unwrap()
        else {
            panic!("status");
        };
        assert_eq!(
            items.as_slice().len(),
            usize::from(replace),
            "abandoned grant reached provider"
        );
        let (_, fresh) = login(&runtime, &server, Some(account)).await;
        assert!(
            command(
                &runtime,
                RuntimeCommand::Save {
                    account,
                    attempt: fresh,
                    choice: StorageChoice::SessionOnly
                }
            )
            .await
            .is_ok(),
            "consumed fresh handoff lost authority"
        );
        assert!(runtime.join());
    }
}

#[tokio::test]
async fn r1_poll_drop_before_dispatch_and_during_validation_grants_nothing() {
    for during_validation in [false, true] {
        for replace in [false, true] {
            let server = Server::start(34).await;
            let fixture = Fixture::new();
            let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
            let faults = config.faults.clone();
            let runtime = IdentityRuntime::start(config).unwrap();
            let previous = if replace {
                Some(active(&runtime, &server).await)
            } else {
                None
            };
            server.authorized.store(false, Ordering::Release);
            let RuntimeReply::Begun { account, attempt } = command(
                &runtime,
                RuntimeCommand::Begin {
                    server: Text::new(&server.origin).unwrap(),
                    replace: previous.as_ref().map(|p| p.account),
                },
            )
            .await
            .unwrap() else {
                panic!("begin");
            };
            if during_validation {
                server.authorized.store(true, Ordering::Release);
                server.validation_pause.store(true, Ordering::Release);
                let poll = runtime
                    .try_submit(RuntimeCommand::Poll { account, attempt })
                    .unwrap();
                let until = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
                while !server.validation_entered.load(Ordering::Acquire) {
                    assert!(tokio::time::Instant::now() < until);
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                drop(poll);
                server.validation_pause.store(false, Ordering::Release);
                server.validation_release.notify_one();
            } else {
                let gate = ReplyRelease(faults.arm_reply());
                let first = runtime
                    .try_submit(RuntimeCommand::Poll { account, attempt })
                    .unwrap();
                reply_entered(&gate.0).await;
                assert!(matches!(
                    first.wait().await,
                    Ok(RuntimeReply::Pending { .. })
                ));
                let queued = runtime
                    .try_submit(RuntimeCommand::Poll { account, attempt })
                    .unwrap();
                drop(queued); // The preceding reply barrier still owns this worker.
                gate.0.release();
            }
            server.authorized.store(true, Ordering::Release);
            assert!(
                matches!(
                    command(&runtime, RuntimeCommand::Poll { account, attempt }).await,
                    Err(IdentityError::Request(
                        ErrorKind::Cancelled | ErrorKind::WrongScope
                    ))
                ),
                "a cancelled Poll attempt must not poll again, even before dispatch"
            );
            assert!(
                command(
                    &runtime,
                    RuntimeCommand::Save {
                        account,
                        attempt,
                        choice: StorageChoice::PrivateFile
                    }
                )
                .await
                .is_err()
            );
            let RuntimeReply::StorageStatus { items, .. } =
                command(&runtime, RuntimeCommand::StorageStatus { account })
                    .await
                    .unwrap()
            else {
                panic!("status");
            };
            assert_eq!(items.as_slice().len(), usize::from(replace));
            if let Some(p) = previous {
                assert!(runtime.account_cancellation(p.scope.unwrap()).is_ok());
                assert_eq!(summary(&runtime, account).storage, p.storage);
            }
            assert!(runtime.join());
        }
    }
}

#[tokio::test]
async fn r1_old_unread_poll_drop_cannot_revoke_new_attempt() {
    let server = Server::start(35).await;
    let fixture = Fixture::new();
    let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
    let faults = config.faults.clone();
    let runtime = IdentityRuntime::start(config).unwrap();
    let RuntimeReply::Begun { account, attempt } = command(
        &runtime,
        RuntimeCommand::Begin {
            server: Text::new(&server.origin).unwrap(),
            replace: None,
        },
    )
    .await
    .unwrap() else {
        panic!("begin");
    };
    server.authorized.store(true, Ordering::Release);
    let gate = ReplyRelease(faults.arm_reply());
    let old = runtime
        .try_submit(RuntimeCommand::Poll { account, attempt })
        .unwrap();
    reply_entered(&gate.0).await;
    runtime.cancel_attempt(account, attempt).unwrap();
    wait_phase(&runtime, account, AccountPhase::Disconnected).await;
    gate.0.release();
    let RuntimeReply::Begun { attempt: fresh, .. } = command(
        &runtime,
        RuntimeCommand::Begin {
            server: Text::new(&server.origin).unwrap(),
            replace: Some(account),
        },
    )
    .await
    .unwrap() else {
        panic!("new begin");
    };
    drop(old);
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::Poll {
                account,
                attempt: fresh
            }
        )
        .await,
        Ok(RuntimeReply::Authorized { .. })
    ));
    assert!(
        command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt: fresh,
                choice: StorageChoice::PrivateFile
            }
        )
        .await
        .is_ok()
    );
    assert!(runtime.join());
}

#[tokio::test]
async fn r1_recovery_selection_rejects_changed_settlement_at_same_revision() {
    for present in [true, false] {
        let server = Server::start(35).await;
        let fixture = Fixture::new();
        let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
        let faults = config.faults.clone();
        let runtime = IdentityRuntime::start(config).unwrap();
        let a = active(&runtime, &server).await;
        let (_, attempt) = login(&runtime, &server, Some(a.account)).await;
        let gate = ReleaseGate(faults.arm());
        let save = runtime
            .try_submit(RuntimeCommand::Save {
                account: a.account,
                attempt,
                choice: StorageChoice::PrivateFile,
            })
            .unwrap();
        entered(&gate.0).await;
        assert!(save.wait().await.is_err());
        let old = summary(&runtime, a.account).recovery.unwrap();
        assert_eq!(old.settlement, talk_core::identity::Settlement::Unknown);
        faults.not_written.store(!present, Ordering::Release);
        let expected = if present {
            talk_core::identity::Settlement::SettledPresent
        } else {
            talk_core::identity::Settlement::SettledAbsent
        };
        gate.0.release();
        let until = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let RuntimeReply::StorageStatus {
                items, revision, ..
            } = command(
                &runtime,
                RuntimeCommand::StorageStatus { account: a.account },
            )
            .await
            .unwrap()
            else {
                panic!("status");
            };
            assert_eq!(revision, old.revision);
            if items
                .as_slice()
                .iter()
                .any(|i| i.item == old.attempted && i.settlement == expected)
            {
                break;
            }
            assert!(tokio::time::Instant::now() < until);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            matches!(
                command(
                    &runtime,
                    RuntimeCommand::ResolvePrevious {
                        account: a.account,
                        selection: old
                    }
                )
                .await,
                Err(IdentityError::Request(ErrorKind::WrongScope))
            ),
            "stale selection ignored changed settlement"
        );
        let RuntimeReply::State(current) =
            command(&runtime, RuntimeCommand::Maintain { account: a.account })
                .await
                .unwrap()
        else {
            panic!("state");
        };
        let fresh = current.recovery.unwrap();
        assert_eq!(fresh.revision, old.revision);
        assert_eq!(fresh.settlement, expected);
        assert!(
            command(
                &runtime,
                RuntimeCommand::ResolvePrevious {
                    account: a.account,
                    selection: fresh
                }
            )
            .await
            .is_ok()
        );
        assert!(runtime.join());
    }
}

#[tokio::test]
async fn r1_settlement_changing_during_retention_validation_rejects_commit() {
    let server = Server::start(35).await;
    let fixture = Fixture::new();
    let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
    let faults = config.faults.clone();
    let runtime = IdentityRuntime::start(config).unwrap();
    let a = active(&runtime, &server).await;
    let (_, attempt) = login(&runtime, &server, Some(a.account)).await;
    let gate = ReleaseGate(faults.arm());
    let save = runtime
        .try_submit(RuntimeCommand::Save {
            account: a.account,
            attempt,
            choice: StorageChoice::PrivateFile,
        })
        .unwrap();
    entered(&gate.0).await;
    assert!(save.wait().await.is_err());
    let selection = summary(&runtime, a.account).recovery.unwrap();
    let settled = faults.settled.load(Ordering::Acquire);
    server.validation_pause.store(true, Ordering::Release);
    let retention = runtime
        .try_submit(RuntimeCommand::ResolvePrevious {
            account: a.account,
            selection,
        })
        .unwrap();
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while !server.validation_entered.load(Ordering::Acquire) {
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    gate.0.release();
    while faults.settled.load(Ordering::Acquire) == settled {
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    server.validation_pause.store(false, Ordering::Release);
    server.validation_release.notify_one();
    assert!(matches!(
        retention.wait().await,
        Err(IdentityError::Request(ErrorKind::WrongScope))
    ));
    assert!(runtime.account_cancellation(a.scope.unwrap()).is_ok());
    let RuntimeReply::State(current) =
        command(&runtime, RuntimeCommand::Maintain { account: a.account })
            .await
            .unwrap()
    else {
        panic!("state");
    };
    assert_eq!(current.revision, selection.revision);
    assert_eq!(current.storage, StoredState::Unresolved);
    assert!(
        command(
            &runtime,
            RuntimeCommand::ResolvePrevious {
                account: a.account,
                selection: current.recovery.unwrap()
            }
        )
        .await
        .is_ok()
    );
    assert!(runtime.join());
}

#[cfg(target_os = "linux")]
async fn native_unlock(sequence: u8) {
    let root =
        PathBuf::from(std::env::var_os("NMT_NATIVE_RACE_CONTROL").expect("private controller"));
    std::fs::write(root.join("unlock-request"), sequence.to_string()).unwrap();
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::fs::read_to_string(root.join("unlock-done"))
        .ok()
        .as_deref()
        != Some(sequence.to_string().as_str())
    {
        assert!(
            tokio::time::Instant::now() < until,
            "private OS-side unlock deadline"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
#[cfg(target_os = "linux")]
fn native_phase(phase: &str) {
    let root =
        PathBuf::from(std::env::var_os("NMT_NATIVE_RACE_CONTROL").expect("private controller"));
    std::fs::write(root.join("phase"), phase).unwrap();
}
#[cfg(target_os = "linux")]
async fn native_lock() {
    tokio::task::spawn_blocking(|| {
        let service =
            secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
                .unwrap();
        let collection = service.get_default_collection().unwrap();
        collection.lock().unwrap();
        assert!(collection.is_locked().unwrap());
    })
    .await
    .unwrap();
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "native_lock_race.py owns isolated GNOME and bounded Unlock/Prompt count monitor"]
async fn r1_native_lock_race_restore_and_exact_maintenance() {
    assert_eq!(
        std::env::var("NMT_PRIVATE_SECRET_SERVICE_TEST").as_deref(),
        Ok("1")
    );
    let server = Server::start(35).await;
    let other = Server::start(34).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let mut accounts = Vec::new();
    for source in [&server, &other] {
        let (account, attempt) = login(&runtime, source, None).await;
        let RuntimeReply::Active { summary, .. } = command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt,
                choice: StorageChoice::Native,
            },
        )
        .await
        .unwrap() else {
            panic!("native save");
        };
        accounts.push(summary);
    }
    assert!(runtime.join());
    let a = &accounts[0];
    let b = &accounts[1];
    let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
    let faults = config.faults.clone();
    let runtime = IdentityRuntime::start(config).unwrap();
    command(&runtime, RuntimeCommand::Accounts).await.unwrap();
    command(&runtime, RuntimeCommand::Restore { account: b.account })
        .await
        .unwrap();
    native_phase("restore-race");
    let gate = ReleaseGate(faults.arm_preflight());
    let restore = runtime
        .try_submit(RuntimeCommand::Restore { account: a.account })
        .unwrap();
    entered(&gate.0).await;
    native_lock().await;
    gate.0.release();
    let first = restore.wait().await;
    native_phase("restore-locked-retry");
    let second = command(&runtime, RuntimeCommand::Restore { account: a.account }).await;
    let closed = summary(&runtime, a.account);
    assert_eq!((closed.storage, closed.revision), (a.storage, a.revision));
    assert!(closed.scope.is_none());
    native_phase("os-unlock-1");
    native_unlock(1).await;
    let RuntimeReply::Active {
        summary: restored, ..
    } = command(&runtime, RuntimeCommand::Restore { account: a.account })
        .await
        .unwrap()
    else {
        panic!("restore after explicit unlock");
    };
    assert_eq!(restored.storage, a.storage);
    command(&runtime, RuntimeCommand::RetireAll { account: a.account })
        .await
        .unwrap();
    native_phase("maintain-race");
    let gate = ReleaseGate(faults.arm_preflight());
    let maintain = runtime
        .try_submit(RuntimeCommand::Maintain { account: a.account })
        .unwrap();
    entered(&gate.0).await;
    native_lock().await;
    gate.0.release();
    let third = maintain.wait().await;
    native_phase("maintain-locked-retry");
    let fourth = command(&runtime, RuntimeCommand::Maintain { account: a.account }).await;
    let RuntimeReply::StorageStatus { items, .. } = command(
        &runtime,
        RuntimeCommand::StorageStatus { account: a.account },
    )
    .await
    .unwrap() else {
        panic!("cleanup status");
    };
    assert!(
        items
            .as_slice()
            .iter()
            .all(|i| i.retired && i.cleanup_pending)
    );
    let kept = summary(&runtime, b.account);
    assert_eq!(kept.storage, b.storage);
    assert!(runtime.account_cancellation(kept.scope.unwrap()).is_ok());
    native_phase("os-unlock-2");
    native_unlock(2).await;
    clean(&runtime, a.account).await;
    command(
        &runtime,
        RuntimeCommand::ForgetAccount { account: a.account },
    )
    .await
    .unwrap();
    command(&runtime, RuntimeCommand::RetireAll { account: b.account })
        .await
        .unwrap();
    clean(&runtime, b.account).await;
    command(
        &runtime,
        RuntimeCommand::ForgetAccount { account: b.account },
    )
    .await
    .unwrap();
    assert!(runtime.join());
    native_phase("complete");
    for (stage, result) in [
        ("restore-race", first),
        ("restore-locked", second),
        ("maintain-race", third),
        ("maintain-locked", fourth),
    ] {
        // Public error variants only, no provider strings/addresses/item values.
        assert!(
            matches!(
                result,
                Err(IdentityError::Storage {
                    failure: StorageFailure::Locked,
                    ..
                })
            ),
            "noninteractive {stage} must report Locked: {:?}",
            result.err()
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "native_service.py owns a private service; legacy record interoperability"]
fn r1_legacy_native_p_remains_exactly_readable_without_migration() {
    assert_eq!(
        std::env::var("NMT_PRIVATE_SECRET_SERVICE_TEST").as_deref(),
        Ok("1")
    );
    let fixture = Fixture::new();
    let mut ledger = fixture.ledger();
    let account = ledger.reserve("https://example.invalid/".into()).unwrap();
    let op = ledger
        .stage(
            account.id,
            LoginAttemptId::from_local(random().unwrap()),
            StorageBackend::LinuxSecretService,
            "synthetic-alias",
        )
        .unwrap();
    let password = zeroize::Zeroizing::new(format!("synthetic-{}", random().unwrap()));
    super::super::provider::faults::legacy_create(op.item, &password).unwrap();
    ledger.settle(op, true).unwrap();
    ledger
        .activate(op, ledger.account(account.id).unwrap().revision)
        .unwrap();
    let revision = ledger.account(account.id).unwrap().revision;
    let providers = providers(&ledger);
    assert!(providers.read(op.item).unwrap().as_str() == password.as_str());
    assert_eq!(ledger.account(account.id).unwrap().revision, revision);
    assert_eq!(ledger.operations(account.id).unwrap().len(), 1);
    ledger.retire_all(account.id).unwrap();
    providers.delete(op.item).unwrap();
    ledger.cleaned(op).unwrap();
    ledger.forget(account.id).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn r1_external_priority_returns_before_delayed_suppression_commit() {
    use std::os::unix::fs::PermissionsExt;
    let server = Server::start(35).await;
    let other = Server::start(34).await;
    let fixture = Fixture::new();
    let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
    let faults = config.faults.clone();
    let runtime = IdentityRuntime::start(config).unwrap();
    let p = active(&runtime, &server).await;
    let b = active(&runtime, &other).await;
    let a_closed = runtime.account_cancellation(p.scope.unwrap()).unwrap();
    let b_closed = runtime.account_cancellation(b.scope.unwrap()).unwrap();
    let (account, attempt) = login(&runtime, &server, Some(p.account)).await;
    let writer = ReleaseGate(faults.arm());
    let save = runtime
        .try_submit(RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::PrivateFile,
        })
        .unwrap();
    entered(&writer.0).await;
    server.validation_pause.store(true, Ordering::Release);
    writer.0.release();
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    while !server.validation_entered.load(Ordering::Acquire) {
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let disk = ReleaseGate(faults.arm_suppression());
    drop(save);
    entered(&disk.0).await; // Restrictive owner installed; fallible commit has not begun.
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (complete, mut receive) = tokio::sync::oneshot::channel();
    let composition = std::thread::spawn(move || {
        let cancel = runtime.cancel_attempt(account, attempt);
        let logout = runtime.logout(account);
        let status = runtime.try_submit(RuntimeCommand::StorageStatus { account });
        let maintenance = runtime.try_submit(RuntimeCommand::Maintain { account });
        let b_work = runtime.try_submit(RuntimeCommand::StorageStatus { account: b.account });
        let _ = complete.send((runtime, cancel, logout, status, maintenance, b_work));
    });
    // The barrier, not elapsed disk latency, forces the ordering. Timeout only
    // bounds a deadlocked before-case so its original failure can be preserved.
    let early = tokio::time::timeout(std::time::Duration::from_secs(3), &mut receive).await;
    let returned_before_disk_release = early.is_ok();
    let local_closed_before_disk_release = a_closed.is_cancelled();
    let b_unchanged_before_disk_release = !b_closed.is_cancelled();
    disk.0.release();
    server.validation_pause.store(false, Ordering::Release);
    server.validation_release.notify_one();
    let (runtime, cancel, logout, status, maintenance, b_work) = match early {
        Ok(value) => value.unwrap(),
        Err(_) => receive.await.unwrap(),
    };
    composition.join().unwrap();
    assert!(matches!(
        cancel,
        Err(IdentityError::Request(ErrorKind::WrongScope))
    ));
    logout.unwrap();
    // These observations were admitted on the external composition thread while
    // disk was held. Their background completion is deliberately awaited later.
    status.unwrap().wait().await.unwrap();
    assert!(maintenance.unwrap().wait().await.is_err());
    b_work.unwrap().wait().await.unwrap();
    wait_phase(&runtime, account, AccountPhase::Disconnected).await;
    let rejected = summary(&runtime, account);
    assert!(rejected.suppression_unconfirmed);
    assert_eq!(rejected.revision, p.revision + 1); // Only N intent committed.
    assert!(
        command(&runtime, RuntimeCommand::Maintain { account })
            .await
            .is_err()
    );
    assert!(summary(&runtime, account).suppression_unconfirmed);
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        command(&runtime, RuntimeCommand::Restore { account })
            .await
            .is_err()
    );
    clean(&runtime, account).await;
    let suppressed = summary(&runtime, account);
    assert_eq!(suppressed.storage, StoredState::Suppressed);
    assert!(!suppressed.suppression_unconfirmed);
    assert_eq!(suppressed.revision, p.revision + 2);
    assert!(runtime.account_cancellation(b.scope.unwrap()).is_ok());
    assert!(runtime.join());
    let ledger = fixture.ledger();
    let ops = ledger.operations(account).unwrap();
    assert_eq!(ops.len(), 2);
    let p_item = match p.storage {
        StoredState::Eligible(item) => item,
        _ => panic!("eligible P"),
    };
    assert!(
        ops.iter()
            .any(|o| o.item == p_item && o.activated && !o.cleanup && !o.abandoned)
    );
    assert!(
        ops.iter()
            .any(|o| o.item != p_item && !o.activated && o.abandoned && o.cleanup)
    );
    assert_eq!(ledger.account(account).unwrap().disposition, 0);
    assert!(ledger.account(b.account).unwrap().eligible.is_some());
    assert!(
        returned_before_disk_release,
        "external priority/try_submit caller waited behind disk barrier"
    );
    assert!(
        local_closed_before_disk_release,
        "local closure waited behind disk barrier"
    );
    assert!(b_unchanged_before_disk_release);
}

// R2 schedules use the after-reply/validation barriers plus root FIFO fences.
// Snapshot phase alone is never used to establish retirement or forwarding.
async fn r2_first_save_validation(server: &Server) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while !server.validation_entered.load(Ordering::Acquire) {
        assert!(
            tokio::time::Instant::now() < until,
            "first Save validation not entered"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}
async fn r2_unread_during_save(replace: bool) {
    let server = Server::start(35).await;
    let other = Server::start(34).await;
    let fixture = Fixture::new();
    let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
    let faults = config.faults.clone();
    let runtime = IdentityRuntime::start(config).unwrap();
    let b = active(&runtime, &other).await;
    let previous = if replace {
        Some(active(&runtime, &server).await)
    } else {
        None
    };
    let RuntimeReply::Begun { account, attempt } = command(
        &runtime,
        RuntimeCommand::Begin {
            server: Text::new(&server.origin).unwrap(),
            replace: previous.as_ref().map(|p| p.account),
        },
    )
    .await
    .unwrap() else {
        panic!("begin");
    };
    server.authorized.store(true, Ordering::Release);
    let gate = ReplyRelease(faults.arm_reply());
    let poll = runtime
        .try_submit(RuntimeCommand::Poll { account, attempt })
        .unwrap();
    reply_entered(&gate.0).await;
    let before = summary(&runtime, account);
    let writes_before = faults.settled.load(Ordering::Acquire);
    server.validation_pause.store(true, Ordering::Release);
    gate.0.release();
    let save = runtime
        .try_submit(RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::PrivateFile,
        })
        .unwrap();
    r2_first_save_validation(&server).await;
    drop(poll);
    command(&runtime, RuntimeCommand::Accounts).await.unwrap(); // priority ran before this FIFO reply
    assert!(
        matches!(
            runtime.cancel_attempt(account, attempt),
            Err(IdentityError::Request(ErrorKind::WrongScope))
        ),
        "retired Pending must be absent before Save resumes"
    );
    server.validation_pause.store(false, Ordering::Release);
    server.validation_release.notify_one();
    let saved = save.wait().await;
    command(&runtime, RuntimeCommand::Maintain { account })
        .await
        .unwrap();
    let RuntimeReply::StorageStatus {
        revision, items, ..
    } = command(&runtime, RuntimeCommand::StorageStatus { account })
        .await
        .unwrap()
    else {
        panic!("status");
    };
    let after = summary(&runtime, account);
    let b_alive = runtime.account_cancellation(b.scope.unwrap()).is_ok();
    let p_alive = previous
        .as_ref()
        .is_none_or(|p| runtime.account_cancellation(p.scope.unwrap()).is_ok());
    let writes = faults.settled.load(Ordering::Acquire) - writes_before;
    assert!(runtime.join());
    let ledger = fixture.ledger();
    let ops = ledger.operations(account).unwrap();
    eprintln!(
        "R2-01 replace={replace} writes={writes} records={} revision_delta={} settled_unactivated_unabandoned={} cleanup_pending={}",
        ops.len(),
        revision - before.revision,
        ops.iter()
            .filter(|o| o.settlement == 1 && !o.activated && !o.abandoned)
            .count(),
        after.cleanup_pending
    );
    assert!(saved.is_err(), "cancelled Save must not activate");
    assert!(b_alive && p_alive, "independent live P/B changed");
    assert_eq!(
        writes, 0,
        "cancelled pre-intent attempt dispatched a provider writer"
    );
    assert_eq!(
        revision, before.revision,
        "pre-intent cancellation changed durable revision"
    );
    assert_eq!(items.as_slice().len(), usize::from(replace));
    assert_eq!(after.storage, before.storage);
    assert!(!after.cleanup_pending);
    if let Some(p) = previous {
        assert_eq!(p.storage, after.storage);
    }
    assert!(ledger.account(b.account).unwrap().eligible.is_some());
}
#[tokio::test]
async fn r2_unread_poll_during_save_new_account() {
    r2_unread_during_save(false).await;
}
#[tokio::test]
async fn r2_unread_poll_during_save_replacement_p() {
    r2_unread_during_save(true).await;
}

async fn r2_forwarded_stale(kind: u8, waiting: bool, cancelled: bool) {
    let server = Server::start(35).await;
    let other = Server::start(34).await;
    let fixture = Fixture::new();
    let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
    let faults = config.faults.clone();
    let runtime = IdentityRuntime::start(config).unwrap();
    let p = active(&runtime, &server).await;
    let b = active(&runtime, &other).await;
    let (account, old) = login(&runtime, &server, Some(p.account)).await;
    runtime.cancel_attempt(account, old).unwrap();
    command(&runtime, RuntimeCommand::Accounts).await.unwrap();
    server.authorized.store(!waiting, Ordering::Release);
    let RuntimeReply::Begun { attempt: fresh, .. } = command(
        &runtime,
        RuntimeCommand::Begin {
            server: Text::new(&server.origin).unwrap(),
            replace: Some(account),
        },
    )
    .await
    .unwrap() else {
        panic!("fresh begin");
    };
    let gate = ReplyRelease(faults.arm_reply());
    let fresh_poll = runtime
        .try_submit(RuntimeCommand::Poll {
            account,
            attempt: fresh,
        })
        .unwrap();
    reply_entered(&gate.0).await;
    let reply = fresh_poll.wait().await.unwrap();
    assert!(if waiting {
        matches!(reply, RuntimeReply::Pending { .. })
    } else {
        matches!(reply, RuntimeReply::Authorized { .. })
    });
    let stale = runtime
        .try_submit(match kind {
            0 => RuntimeCommand::Poll {
                account,
                attempt: old,
            },
            1 => RuntimeCommand::Save {
                account,
                attempt: old,
                choice: StorageChoice::PrivateFile,
            },
            2 => RuntimeCommand::Begin {
                server: Text::new(&server.origin).unwrap(),
                replace: Some(account),
            },
            3 => RuntimeCommand::BeginSave { account },
            _ => unreachable!(),
        })
        .unwrap();
    command(&runtime, RuntimeCommand::Accounts).await.unwrap(); // stale was forwarded, worker still gated
    let retained = if cancelled {
        drop(stale);
        None
    } else {
        Some(stale)
    };
    gate.0.release();
    if let Some(stale) = retained {
        assert!(stale.wait().await.is_err());
    }
    // This command is behind the stale job in the account FIFO.
    command(&runtime, RuntimeCommand::StorageStatus { account })
        .await
        .unwrap();
    let p_alive = runtime.account_cancellation(p.scope.unwrap()).is_ok();
    let b_alive = runtime.account_cancellation(b.scope.unwrap()).is_ok();
    let authorized = if waiting {
        server.authorized.store(true, Ordering::Release);
        // Only the protocol's documented two-second poll rate; not ownership proof.
        tokio::time::sleep(std::time::Duration::from_millis(2050)).await;
        matches!(
            command(
                &runtime,
                RuntimeCommand::Poll {
                    account,
                    attempt: fresh
                }
            )
            .await,
            Ok(RuntimeReply::Authorized { .. })
        )
    } else {
        true
    };
    let saved = command(
        &runtime,
        RuntimeCommand::Save {
            account,
            attempt: fresh,
            choice: StorageChoice::PrivateFile,
        },
    )
    .await;
    eprintln!(
        "R2-02 stale_kind={kind} waiting={waiting} cancelled={cancelled} authorized={authorized} save_error={:?}",
        saved.as_ref().err()
    );
    let result_ok = matches!(saved, Ok(RuntimeReply::Active { .. }));
    assert!(runtime.join());
    assert!(p_alive && b_alive);
    assert!(
        authorized && result_ok,
        "stale queued command erased the newer flow/grant"
    );
    let ledger = fixture.ledger();
    assert!(ledger.account(b.account).unwrap().eligible.is_some());
    let ops = ledger.operations(account).unwrap();
    assert_eq!(ops.len(), 2);
    assert!(
        ops.iter()
            .all(|o| o.activated && !o.abandoned && !o.cleanup)
    );
}
#[tokio::test]
async fn r2_forwarded_cancelled_poll_preserves_new_authorized() {
    r2_forwarded_stale(0, false, true).await;
}
#[tokio::test]
async fn r2_forwarded_cancelled_save_preserves_new_authorized() {
    r2_forwarded_stale(1, false, true).await;
}
#[tokio::test]
async fn r2_forwarded_cancelled_poll_preserves_new_waiting() {
    r2_forwarded_stale(0, true, true).await;
}
#[tokio::test]
async fn r2_forwarded_cancelled_save_preserves_new_waiting() {
    r2_forwarded_stale(1, true, true).await;
}
#[tokio::test]
async fn r2_forwarded_noncancelled_stale_controls() {
    for kind in [0, 1] {
        for waiting in [false, true] {
            r2_forwarded_stale(kind, waiting, false).await;
        }
    }
}

#[tokio::test]
async fn r2_consumed_handoff_save_and_priority_cancellation_controls() {
    for replace in [false, true] {
        for close in [0, 1, 2] {
            // consumed success, explicit attempt cancel, account logout
            let server = Server::start(35).await;
            let other = Server::start(34).await;
            let fixture = Fixture::new();
            let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
            let faults = config.faults.clone();
            let runtime = IdentityRuntime::start(config).unwrap();
            let b = active(&runtime, &other).await;
            let p = if replace {
                Some(active(&runtime, &server).await)
            } else {
                None
            };
            let (account, attempt) = login(&runtime, &server, p.as_ref().map(|p| p.account)).await;
            let before = summary(&runtime, account);
            let writes_before = faults.settled.load(Ordering::Acquire);
            server.validation_pause.store(true, Ordering::Release);
            let save = runtime
                .try_submit(RuntimeCommand::Save {
                    account,
                    attempt,
                    choice: StorageChoice::PrivateFile,
                })
                .unwrap();
            r2_first_save_validation(&server).await;
            if close == 1 {
                runtime.cancel_attempt(account, attempt).unwrap();
            }
            if close == 2 {
                runtime.logout(account).unwrap();
            }
            command(&runtime, RuntimeCommand::Accounts).await.unwrap();
            server.validation_pause.store(false, Ordering::Release);
            server.validation_release.notify_one();
            let result = save.wait().await;
            let RuntimeReply::StorageStatus {
                revision, items, ..
            } = command(&runtime, RuntimeCommand::StorageStatus { account })
                .await
                .unwrap()
            else {
                panic!("status");
            };
            assert!(runtime.account_cancellation(b.scope.unwrap()).is_ok());
            assert_eq!(
                matches!(result, Ok(RuntimeReply::Active { .. })),
                close == 0
            );
            assert_eq!(
                faults.settled.load(Ordering::Acquire) - writes_before,
                usize::from(close == 0)
            );
            assert_eq!(
                items.as_slice().len(),
                usize::from(replace) + usize::from(close == 0)
            );
            if close != 0 {
                assert_eq!(revision, before.revision + u64::from(close == 2));
                if let Some(p) = p {
                    assert_eq!(
                        runtime.account_cancellation(p.scope.unwrap()).is_ok(),
                        close != 2
                    );
                }
            }
            assert!(runtime.join());
        }
    }
}

#[tokio::test]
async fn r2_unread_handoff_after_writer_dispatch_keeps_exact_cleanup_owner() {
    for replace in [false, true] {
        let server = Server::start(35).await;
        let other = Server::start(34).await;
        let fixture = Fixture::new();
        let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
        let faults = config.faults.clone();
        let runtime = IdentityRuntime::start(config).unwrap();
        let b = active(&runtime, &other).await;
        let p = if replace {
            Some(active(&runtime, &server).await)
        } else {
            None
        };
        let RuntimeReply::Begun { account, attempt } = command(
            &runtime,
            RuntimeCommand::Begin {
                server: Text::new(&server.origin).unwrap(),
                replace: p.as_ref().map(|p| p.account),
            },
        )
        .await
        .unwrap() else {
            panic!("begin");
        };
        server.authorized.store(true, Ordering::Release);
        let replied = ReplyRelease(faults.arm_reply());
        let poll = runtime
            .try_submit(RuntimeCommand::Poll { account, attempt })
            .unwrap();
        reply_entered(&replied.0).await;
        let before = summary(&runtime, account);
        let writer = ReleaseGate(faults.arm());
        replied.0.release();
        let save = runtime
            .try_submit(RuntimeCommand::Save {
                account,
                attempt,
                choice: StorageChoice::PrivateFile,
            })
            .unwrap();
        entered(&writer.0).await;
        drop(poll);
        command(&runtime, RuntimeCommand::Accounts).await.unwrap();
        assert!(matches!(
            runtime.cancel_attempt(account, attempt),
            Err(IdentityError::Request(ErrorKind::WrongScope))
        ));
        assert!(runtime.account_cancellation(b.scope.unwrap()).is_ok());
        if let Some(p) = &p {
            assert!(runtime.account_cancellation(p.scope.unwrap()).is_ok());
        }
        writer.0.release();
        assert!(save.wait().await.is_err());
        let RuntimeReply::StorageStatus { items, .. } =
            command(&runtime, RuntimeCommand::StorageStatus { account })
                .await
                .unwrap()
        else {
            panic!("settled status");
        };
        let written = items
            .as_slice()
            .iter()
            .find(|item| !item.was_activated)
            .unwrap();
        assert_eq!(
            written.settlement,
            talk_core::identity::Settlement::SettledPresent
        );
        assert!(written.cleanup_pending && !written.eligible);
        clean(&runtime, account).await;
        let current = summary(&runtime, account);
        assert_eq!(current.revision, before.revision + 2); // intent plus suppression, not provider completion
        assert_eq!(current.storage, StoredState::Suppressed);
        assert!(!current.cleanup_pending);
        assert!(runtime.join());
        let ledger = fixture.ledger();
        let ops = ledger.operations(account).unwrap();
        assert_eq!(ops.len(), usize::from(replace) + 1);
        let n = ops.iter().find(|o| o.attempt == attempt).unwrap();
        // cleaned() records verified absence, not historical pre-deletion presence.
        assert!(n.settlement == 2 && n.abandoned && n.cleanup && !n.activated);
        assert!(
            !fixture
                .0
                .join(super::super::provider::filename(n.item))
                .exists()
        );
        if let Some(p) = p {
            let StoredState::Eligible(item) = p.storage else {
                panic!("P");
            };
            assert!(
                ops.iter()
                    .any(|o| o.item == item && o.activated && !o.abandoned && !o.cleanup)
            );
        }
        assert!(ledger.account(b.account).unwrap().eligible.is_some());
    }
}

#[tokio::test]
async fn r2_cancelled_unexecuted_creators_do_not_own_current_attempt() {
    for kind in [2, 3] {
        r2_forwarded_stale(kind, false, true).await;
    }
}
