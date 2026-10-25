#[path = "connection_tests.rs"]
mod connection;
#[path = "review_tests.rs"]
mod review;
use super::{
    error::IdentityError,
    ledger::{Ledger, random},
    private_fs::PrivateDir,
    provider::{Providers, WriteResult},
};
use std::{path::PathBuf, sync::Arc};
use talk_core::{
    identity::{StorageBackend, StorageFailure},
    ids::LoginAttemptId,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("nmt-identity-test-{:032x}", random().unwrap()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o700).create(&p).unwrap();
        }
        #[cfg(windows)]
        {
            PrivateDir::open(&p, true).unwrap();
        }
        Self(p)
    }
    fn ledger(&self) -> Ledger {
        Ledger::open(Arc::new(PrivateDir::open(&self.0, true).unwrap())).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn providers(ledger: &Ledger) -> Providers {
    Providers::new(ledger.dir.clone(), ledger.lock.clone())
}

#[test]
fn private_file_roundtrip_restart_requires_exact_eligible_backend() {
    let root = Fixture::new();
    let mut ledger = root.ledger();
    let p = providers(&ledger);
    let account = ledger
        .reserve("https://example.invalid/cloud/".into())
        .unwrap();
    ledger.bind_user(account.id, "synthetic-user").unwrap();
    let op = ledger
        .stage(
            account.id,
            LoginAttemptId::from_local(random().unwrap()),
            StorageBackend::PrivateFile,
            "synthetic-user",
        )
        .unwrap();
    assert_eq!(ledger.account(account.id).unwrap().disposition, 1);
    let password = format!("synthetic-{}", random().unwrap());
    assert!(matches!(p.write(op.item, &password), WriteResult::Present));
    // Crate/provider completion alone has not changed ledger restoration.
    assert_eq!(ledger.account(account.id).unwrap().disposition, 1);
    ledger.settle(op, true).unwrap();
    let revision = ledger.account(account.id).unwrap().revision;
    ledger.activate(op, revision).unwrap();
    drop(p);
    drop(ledger);
    let ledger = root.ledger();
    let a = ledger.account(account.id).unwrap();
    assert_eq!(a.disposition, 2);
    let selected = ledger.operation(account.id, a.eligible.unwrap()).unwrap();
    assert_eq!(selected.item.provider.backend, StorageBackend::PrivateFile);
    assert!(providers(&ledger).read(selected.item).unwrap().as_str() == password);
    // The nonsecret SQLite file contains no app credential.
    let bytes = std::fs::read(root.0.join("restoration.sqlite")).unwrap();
    assert!(
        !bytes
            .windows(password.len())
            .any(|b| b == password.as_bytes())
    );
    let mut wrong = selected.item;
    wrong.provider.store ^= 1;
    assert!(providers(&ledger).read(wrong).is_err());
}

#[test]
fn crash_unknown_writer_cannot_be_settled_by_early_absence_or_retried() {
    let root = Fixture::new();
    let mut ledger = root.ledger();
    let a = ledger.reserve("https://example.invalid/".into()).unwrap();
    let op = ledger
        .stage(
            a.id,
            LoginAttemptId::from_local(random().unwrap()),
            StorageBackend::PrivateFile,
            "synthetic-user",
        )
        .unwrap();
    drop(ledger);
    let mut ledger = root.ledger();
    assert_eq!(ledger.account(a.id).unwrap().disposition, 1);
    assert_eq!(ledger.operation(a.id, op.id).unwrap().settlement, 0);
    assert!(matches!(
        providers(&ledger).read(op.item),
        Err(IdentityError::Storage {
            failure: StorageFailure::Missing,
            ..
        })
    ));
    assert!(
        ledger
            .stage(
                a.id,
                LoginAttemptId::from_local(random().unwrap()),
                StorageBackend::PrivateFile,
                "synthetic-user"
            )
            .is_err()
    );
    ledger.suppress(a.id, true, Some(op.id), false).unwrap();
    assert!(ledger.cleaned(op).is_err());
    assert_eq!(ledger.account(a.id).unwrap().disposition, 0);
}

#[test]
fn late_file_write_after_suppression_never_activates_or_deletes_retained_p() {
    let root = Fixture::new();
    let mut ledger = root.ledger();
    let a = ledger.reserve("https://example.invalid/".into()).unwrap();
    let previous = ledger
        .stage(
            a.id,
            LoginAttemptId::from_local(random().unwrap()),
            StorageBackend::PrivateFile,
            "synthetic-user",
        )
        .unwrap();
    let p = providers(&ledger);
    let password = format!("synthetic-{}", random().unwrap());
    assert!(matches!(
        p.write(previous.item, &password),
        WriteResult::Present
    ));
    ledger.settle(previous, true).unwrap();
    let rev = ledger.account(a.id).unwrap().revision;
    ledger.activate(previous, rev).unwrap();
    let attempted = ledger
        .stage(
            a.id,
            LoginAttemptId::from_local(random().unwrap()),
            StorageBackend::PrivateFile,
            "synthetic-user",
        )
        .unwrap();
    let staged_revision = ledger.account(a.id).unwrap().revision;
    ledger
        .suppress(a.id, true, Some(attempted.id), false)
        .unwrap();
    assert!(matches!(
        p.write(attempted.item, &password),
        WriteResult::Present
    ));
    ledger.settle(attempted, true).unwrap();
    assert!(ledger.activate(attempted, staged_revision).is_err());
    assert_eq!(ledger.account(a.id).unwrap().disposition, 0);
    p.delete(attempted.item).unwrap();
    ledger.cleaned(attempted).unwrap();
    assert!(p.read(previous.item).unwrap().as_str() == password);
    assert!(!ledger.operation(a.id, previous.id).unwrap().abandoned);
    assert!(ledger.cleaned(previous).is_err());
}

#[test]
fn explicit_retain_only_validates_exact_p_and_preserves_unknown_n() {
    let root = Fixture::new();
    let mut ledger = root.ledger();
    let a = ledger.reserve("https://example.invalid/".into()).unwrap();
    let previous = ledger
        .stage(
            a.id,
            LoginAttemptId::from_local(random().unwrap()),
            StorageBackend::PrivateFile,
            "synthetic-user",
        )
        .unwrap();
    ledger.settle(previous, true).unwrap();
    let rev = ledger.account(a.id).unwrap().revision;
    ledger.activate(previous, rev).unwrap();
    let attempted = ledger
        .stage(
            a.id,
            LoginAttemptId::from_local(random().unwrap()),
            StorageBackend::LinuxSecretService,
            "synthetic-user",
        )
        .unwrap();
    let previous = ledger.operation(a.id, previous.id).unwrap();
    ledger
        .retain(attempted, previous, ledger.account(a.id).unwrap().revision)
        .unwrap();
    assert_eq!(ledger.account(a.id).unwrap().eligible, Some(previous.id));
    assert_eq!(ledger.operation(a.id, attempted.id).unwrap().settlement, 0);
    let mut forged = previous;
    forged.item.provider.backend = StorageBackend::MacKeychain;
    // Owner validates item correlation, not just the operation identifier.
    assert!(
        ledger
            .retain(attempted, forged, ledger.account(a.id).unwrap().revision)
            .is_err()
    );
}

#[test]
fn private_root_lock_and_missing_or_corrupt_ledger_do_not_reconstruct_by_enumeration() {
    let root = Fixture::new();
    let ledger = root.ledger();
    assert!(Ledger::open(Arc::new(PrivateDir::open(&root.0, true).unwrap())).is_err());
    drop(ledger);
    std::fs::remove_file(root.0.join("restoration.sqlite")).unwrap();
    assert!(Ledger::open(Arc::new(PrivateDir::open(&root.0, true).unwrap())).is_err());
}

#[cfg(unix)]
#[test]
fn private_file_rejects_modes_symlinks_hardlinks_and_generation_collisions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = Fixture::new();
    let dir = PrivateDir::open(&root.0, true).unwrap();
    dir.create("first", b"synthetic").unwrap();
    assert!(dir.create("first", b"replacement").is_err());
    assert!(dir.read("first", 32).unwrap().as_slice() == b"synthetic");
    symlink(root.0.join("first"), root.0.join("link")).unwrap();
    assert!(dir.read("link", 32).is_err());
    std::fs::hard_link(root.0.join("first"), root.0.join("hard")).unwrap();
    assert!(dir.read("first", 32).is_err());
    std::fs::remove_file(root.0.join("hard")).unwrap();
    std::fs::set_permissions(root.0.join("first"), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(dir.read("first", 32).is_err());
    std::fs::set_permissions(&root.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(dir.create("second", b"synthetic").is_err());
}

use super::{
    runtime::{
        AccountPhase, IdentityRuntime, RuntimeCommand, RuntimeConfig, RuntimeReply, StoredState,
    },
    test_server::Server,
};
use std::sync::atomic::Ordering;
use talk_core::{
    bounded::Text,
    identity::StorageChoice,
    ports::{ErrorKind, ProtocolRequest},
};
async fn command(
    runtime: &IdentityRuntime,
    command: RuntimeCommand,
) -> super::error::Result<RuntimeReply> {
    runtime.try_submit(command)?.wait().await
}
async fn login(
    runtime: &IdentityRuntime,
    server: &Server,
    replace: Option<talk_core::ids::AccountId>,
) -> (talk_core::ids::AccountId, LoginAttemptId) {
    let reply = command(
        runtime,
        RuntimeCommand::Begin {
            server: Text::new(&server.origin).unwrap(),
            replace,
        },
    )
    .await
    .unwrap();
    let RuntimeReply::Begun { account, attempt } = reply else {
        panic!("begin reply");
    };
    server.authorized.store(true, Ordering::Release);
    let result = command(runtime, RuntimeCommand::Poll { account, attempt }).await;
    assert!(
        matches!(result, Ok(RuntimeReply::Authorized { .. })),
        "authorization result: {:?}, authenticated calls: {}",
        result.err(),
        server.authenticated.load(Ordering::Acquire)
    );
    (account, attempt)
}
async fn wait_phase(
    runtime: &IdentityRuntime,
    account: talk_core::ids::AccountId,
    phase: AccountPhase,
) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    loop {
        assert!(runtime.failure().is_none(), "runtime failed");
        if runtime
            .snapshot()
            .as_slice()
            .iter()
            .any(|a| a.account == account && a.phase == phase && a.scope.is_none())
        {
            return;
        }
        assert!(tokio::time::Instant::now() < until, "phase deadline");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
#[tokio::test]
async fn failed_initial_flow_retains_account_correlation_and_can_be_forgotten() {
    let fixture = Fixture::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    command(&runtime, RuntimeCommand::Accounts).await.unwrap();
    let mut pending = runtime
        .try_submit(RuntimeCommand::Begin {
            server: Text::new(&origin).unwrap(),
            replace: None,
        })
        .unwrap();
    let result = loop {
        if let Some(result) = pending.try_take() {
            break result;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert!(matches!(
        result,
        Err(IdentityError::Request(ErrorKind::Transport))
    ));
    let account = pending
        .account()
        .expect("reserved correlation survives failure");
    assert_eq!(runtime.snapshot().as_slice()[0].account, account);
    command(&runtime, RuntimeCommand::ForgetAccount { account })
        .await
        .unwrap();
    assert!(runtime.snapshot().as_slice().is_empty());
    assert!(runtime.join());
}

#[tokio::test]
async fn runtime_grant_file_restart_restore_and_confirmed_auth_loss() {
    for major in [34, 35] {
        let server = Server::start(major).await;
        let fixture = Fixture::new();
        let runtime =
            IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
        let (account, attempt) = login(&runtime, &server, None).await;
        let RuntimeReply::Active { summary, .. } = command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt,
                choice: StorageChoice::PrivateFile,
            },
        )
        .await
        .unwrap() else {
            panic!("activation");
        };
        let old_scope = summary.scope.unwrap();
        assert!(
            matches!(summary.storage,StoredState::Eligible(item) if item.provider.backend==StorageBackend::PrivateFile)
        );
        assert!(server.authenticated.load(Ordering::Acquire) >= 6);
        assert!(runtime.join());
        // A clean exit preserves eligibility; it is not a logout.
        let runtime =
            IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
        command(&runtime, RuntimeCommand::Accounts).await.unwrap();
        let RuntimeReply::Active { summary, .. } =
            command(&runtime, RuntimeCommand::Restore { account })
                .await
                .unwrap()
        else {
            panic!("restore");
        };
        let current = summary.scope.unwrap();
        assert_ne!(old_scope.runtime, current.runtime);
        assert!(matches!(
            command(
                &runtime,
                RuntimeCommand::Request {
                    scope: old_scope,
                    request: ProtocolRequest::ProbeAccount
                }
            )
            .await,
            Err(IdentityError::Request(ErrorKind::WrongScope))
        ));
        assert!(matches!(
            command(
                &runtime,
                RuntimeCommand::Request {
                    scope: current,
                    request: ProtocolRequest::ProbeAccount
                }
            )
            .await,
            Ok(RuntimeReply::Protocol(_))
        ));
        server.revoked.store(true, Ordering::Release);
        assert!(matches!(
            command(
                &runtime,
                RuntimeCommand::Request {
                    scope: current,
                    request: ProtocolRequest::ProbeAccount
                }
            )
            .await,
            Err(IdentityError::Request(ErrorKind::ReauthenticationRequired))
        ));
        wait_phase(&runtime, account, AccountPhase::ReauthenticationRequired).await;
        assert!(runtime.join());
        let ledger = fixture.ledger();
        assert_eq!(ledger.account(account).unwrap().disposition, 0);
        // Logout/auth loss suppression never blanket-deletes previously activated P.
        assert!(
            ledger
                .operations(account)
                .unwrap()
                .iter()
                .all(|o| !o.abandoned)
        );
    }
}
#[tokio::test]
async fn runtime_logout_is_priority_and_late_flow_cannot_activate() {
    let server = Server::start(35).await;
    server.delayed.store(true, Ordering::Release);
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
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
    let pending = runtime
        .try_submit(RuntimeCommand::Poll { account, attempt })
        .unwrap();
    while server.polls.load(Ordering::Acquire) == 0 {
        tokio::task::yield_now().await;
    }
    let mut queued = Vec::new();
    for _ in 0..6 {
        queued.push(
            runtime
                .try_submit(RuntimeCommand::Poll { account, attempt })
                .unwrap(),
        );
    }
    assert!(matches!(
        runtime.try_submit(RuntimeCommand::Poll { account, attempt }),
        Err(IdentityError::Request(ErrorKind::Busy))
    ));
    runtime.logout(account).unwrap();
    wait_phase(&runtime, account, AccountPhase::Disconnected).await;
    assert!(pending.wait().await.is_err());
    drop(queued);
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt,
                choice: StorageChoice::SessionOnly
            }
        )
        .await,
        Err(IdentityError::Request(
            ErrorKind::Cancelled | ErrorKind::NotFound
        ))
    ));
    assert!(runtime.join());
    assert_eq!(fixture.ledger().account(account).unwrap().disposition, 0);
}
#[tokio::test]
async fn cancellation_session_choice_and_account_isolation() {
    let server = Server::start(34).await;
    let other = Server::start(35).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let (a, attempt) = login(&runtime, &server, None).await;
    let RuntimeReply::Active { summary, .. } = command(
        &runtime,
        RuntimeCommand::Save {
            account: a,
            attempt,
            choice: StorageChoice::SessionOnly,
        },
    )
    .await
    .unwrap() else {
        panic!("session");
    };
    assert_eq!(summary.storage, StoredState::SessionOnly);
    let scope = summary.scope.unwrap();
    let (b, b_attempt) = login(&runtime, &other, None).await;
    runtime.cancel_attempt(b, b_attempt).unwrap();
    wait_phase(&runtime, b, AccountPhase::Disconnected).await;
    assert!(
        command(
            &runtime,
            RuntimeCommand::Save {
                account: b,
                attempt: b_attempt,
                choice: StorageChoice::PrivateFile
            }
        )
        .await
        .is_err()
    );
    assert!(
        command(
            &runtime,
            RuntimeCommand::Request {
                scope,
                request: ProtocolRequest::ProbeAccount
            }
        )
        .await
        .is_ok()
    );
    runtime.logout(a).unwrap();
    wait_phase(&runtime, a, AccountPhase::Disconnected).await;
    assert!(runtime.join());
    let ledger = fixture.ledger();
    assert_eq!(ledger.account(a).unwrap().disposition, 0);
    assert!(ledger.operations(a).unwrap().is_empty());
}

#[tokio::test]
async fn canonical_mismatch_refuses_replacement_and_active_probe_closes_account() {
    let server = Server::start(35).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let (account, attempt) = login(&runtime, &server, None).await;
    let RuntimeReply::Active { summary, .. } = command(
        &runtime,
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
    let scope = summary.scope.unwrap();
    let previous = summary.storage;
    let RuntimeReply::Begun { attempt, .. } = command(
        &runtime,
        RuntimeCommand::Begin {
            server: Text::new(&server.origin).unwrap(),
            replace: Some(account),
        },
    )
    .await
    .unwrap() else {
        panic!("begin");
    };
    server.wrong_user.store(true, Ordering::Release);
    assert!(matches!(
        command(&runtime, RuntimeCommand::Poll { account, attempt }).await,
        Err(IdentityError::Request(ErrorKind::WrongScope))
    ));
    assert_eq!(runtime.snapshot().as_slice()[0].storage, previous);
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::Request {
                scope,
                request: ProtocolRequest::ProbeAccount
            }
        )
        .await,
        Err(IdentityError::Request(ErrorKind::WrongScope))
    ));
    wait_phase(&runtime, account, AccountPhase::ReauthenticationRequired).await;
    assert!(runtime.join());
    let l = fixture.ledger();
    let a = l.account(account).unwrap();
    assert_eq!(a.disposition, 0);
    assert!(
        l.login_name(l.operations(account).unwrap()[0])
            .unwrap()
            .as_str()
            == "Synthetic.Login@Example.invalid"
    );
    assert!(a.user == "synthetic-user");
    assert!(l.operations(account).unwrap().iter().all(|o| !o.abandoned));
}

#[tokio::test]
async fn auth_health_probe_coalesces_rate_limits_and_closes_only_confirmed_account() {
    let server = Server::start(35).await;
    let other = Server::start(34).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let (account, attempt) = login(&runtime, &server, None).await;
    let RuntimeReply::Active { summary, .. } = command(
        &runtime,
        RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::SessionOnly,
        },
    )
    .await
    .unwrap() else {
        panic!("active");
    };
    let scope = summary.scope.unwrap();
    let token = runtime.account_cancellation(scope).unwrap();
    let (other_account, attempt) = login(&runtime, &other, None).await;
    let RuntimeReply::Active { summary, .. } = command(
        &runtime,
        RuntimeCommand::Save {
            account: other_account,
            attempt,
            choice: StorageChoice::SessionOnly,
        },
    )
    .await
    .unwrap() else {
        panic!("other active");
    };
    let other_scope = summary.scope.unwrap();
    let before = server.user_probes.load(Ordering::Acquire);
    server.rate_limited.store(true, Ordering::Release);
    for _ in 0..3 {
        assert!(matches!(
            command(
                &runtime,
                RuntimeCommand::Request {
                    scope,
                    request: ProtocolRequest::ProbeAccount
                }
            )
            .await,
            Err(IdentityError::Request(ErrorKind::RateLimited))
        ));
    }
    assert!(!token.is_cancelled());
    assert!(
        runtime
            .snapshot()
            .as_slice()
            .iter()
            .any(|a| a.account == account && a.connection_degraded && a.scope == Some(scope))
    );
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        server.user_probes.load(Ordering::Acquire),
        before + 3,
        "Retry-After must delay the coalesced probe"
    );
    assert!(
        command(
            &runtime,
            RuntimeCommand::Request {
                scope: other_scope,
                request: ProtocolRequest::ProbeAccount
            }
        )
        .await
        .is_ok()
    );
    server.revoked.store(true, Ordering::Release);
    server.rate_limited.store(false, Ordering::Release);
    wait_phase(&runtime, account, AccountPhase::ReauthenticationRequired).await;
    assert_eq!(
        server.user_probes.load(Ordering::Acquire),
        before + 4,
        "one scheduled account probe, no request replay"
    );
    assert!(token.is_cancelled());
    assert!(runtime.account_cancellation(other_scope).is_ok());
    assert!(
        command(
            &runtime,
            RuntimeCommand::Request {
                scope: other_scope,
                request: ProtocolRequest::ProbeAccount
            }
        )
        .await
        .is_ok()
    );
    assert!(runtime.join());
}

#[tokio::test]
async fn uncertain_mutation_is_not_replayed_by_recovery_probe_or_new_binding() {
    let server = Server::start(35).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let (account, attempt) = login(&runtime, &server, None).await;
    let RuntimeReply::Active { summary, .. } = command(
        &runtime,
        RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::SessionOnly,
        },
    )
    .await
    .unwrap() else {
        panic!("active");
    };
    let scope = summary.scope.unwrap();
    let before = server.user_probes.load(Ordering::Acquire);
    let create = ProtocolRequest::CreateGroup {
        name: Text::new("Synthetic").unwrap(),
    };
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::Request {
                scope,
                request: create.clone()
            }
        )
        .await,
        Err(IdentityError::Request(ErrorKind::Uncertain))
    ));
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while server.user_probes.load(Ordering::Acquire) == before {
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(server.mutations.load(Ordering::Acquire), 1);
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::Request {
                scope,
                request: create.clone()
            }
        )
        .await,
        Err(IdentityError::Request(ErrorKind::Uncertain))
    ));
    runtime.logout(account).unwrap();
    wait_phase(&runtime, account, AccountPhase::Disconnected).await;
    let (_, attempt) = login(&runtime, &server, Some(account)).await;
    let RuntimeReply::Active { summary, .. } = command(
        &runtime,
        RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::SessionOnly,
        },
    )
    .await
    .unwrap() else {
        panic!("replacement");
    };
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::Request {
                scope: summary.scope.unwrap(),
                request: create
            }
        )
        .await,
        Err(IdentityError::Request(ErrorKind::Uncertain))
    ));
    assert_eq!(server.mutations.load(Ordering::Acquire), 1);
    assert!(runtime.join());
}

#[tokio::test]
async fn cold_explicit_previous_resolution_can_retry_validation_without_changing_disposition() {
    let server = Server::start(35).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let (account, attempt) = login(&runtime, &server, None).await;
    let RuntimeReply::Active { summary, .. } = command(
        &runtime,
        RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::PrivateFile,
        },
    )
    .await
    .unwrap() else {
        panic!("P active");
    };
    let stored = summary.storage;
    assert!(runtime.join());
    // A committed N intent with a known non-write survives a restart; P remains
    // explicitly named but is ineligible until deliberate revalidation/retention.
    let mut ledger = fixture.ledger();
    let n = ledger
        .stage(
            account,
            LoginAttemptId::from_local(random().unwrap()),
            StorageBackend::PrivateFile,
            "Synthetic.Login@Example.invalid",
        )
        .unwrap();
    ledger
        .settle_result(
            n,
            false,
            Some(IdentityError::Storage {
                backend: StorageBackend::PrivateFile,
                failure: StorageFailure::AccessDenied,
            }),
        )
        .unwrap();
    drop(ledger);
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    command(&runtime, RuntimeCommand::Accounts).await.unwrap();
    let RuntimeReply::StorageStatus { items, .. } =
        command(&runtime, RuntimeCommand::StorageStatus { account })
            .await
            .unwrap()
    else {
        panic!("stored status");
    };
    assert_eq!(items.as_slice().len(), 2);
    let failed = items
        .as_slice()
        .iter()
        .find(|item| item.item == n.item)
        .unwrap();
    assert_eq!(failed.last_failure, Some(StorageFailure::AccessDenied));
    assert_eq!(
        failed.settlement,
        talk_core::identity::Settlement::SettledAbsent
    );
    assert!(!failed.eligible);
    assert!(failed.retained.is_some());
    let selection = runtime.snapshot().as_slice()[0].recovery.unwrap();
    assert_eq!(selection.attempted, n.item);
    assert!(selection.previous.is_some());
    assert!(matches!(
        command(&runtime, RuntimeCommand::Restore { account }).await,
        Err(IdentityError::RecoveryRequired)
    ));
    server.rate_limited.store(true, Ordering::Release);
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::ResolvePrevious { account, selection }
        )
        .await,
        Err(IdentityError::Request(ErrorKind::RateLimited))
    ));
    assert_eq!(
        runtime.snapshot().as_slice()[0].storage,
        StoredState::Unresolved
    );
    assert!(runtime.snapshot().as_slice()[0].scope.is_none());
    server.rate_limited.store(false, Ordering::Release);
    let RuntimeReply::State(summary) = command(
        &runtime,
        RuntimeCommand::ResolvePrevious { account, selection },
    )
    .await
    .unwrap() else {
        panic!("retained P");
    };
    assert_eq!(summary.storage, stored);
    assert!(summary.scope.is_none());
    let count = server.authenticated.load(Ordering::Acquire);
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::ResolvePrevious { account, selection }
        )
        .await,
        Err(IdentityError::Request(ErrorKind::WrongScope))
    ));
    assert_eq!(server.authenticated.load(Ordering::Acquire), count);
    assert!(matches!(
        command(&runtime, RuntimeCommand::Restore { account }).await,
        Ok(RuntimeReply::Active { .. })
    ));
    clean(&runtime, account).await;
    assert!(runtime.join());
}

#[tokio::test]
async fn idle_pending_grant_expiry_clears_admission_and_requires_a_fresh_attempt() {
    let server = Server::start(35).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let (account, expired) = login(&runtime, &server, None).await;
    runtime.expire_pending(account);
    wait_phase(&runtime, account, AccountPhase::Disconnected).await;
    assert!(
        command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt: expired,
                choice: StorageChoice::PrivateFile
            }
        )
        .await
        .is_err()
    );
    let (_, fresh) = login(&runtime, &server, Some(account)).await;
    assert_ne!(fresh, expired);
    assert!(
        command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt: expired,
                choice: StorageChoice::SessionOnly
            }
        )
        .await
        .is_err()
    );
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt: fresh,
                choice: StorageChoice::SessionOnly
            }
        )
        .await,
        Ok(RuntimeReply::Active { .. })
    ));
    assert!(runtime.join());
}

struct ReleaseGate(Arc<super::provider::faults::Gate>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        self.0.release();
    }
}
async fn entered(gate: &super::provider::faults::Gate) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while !gate.entered.load(Ordering::Acquire) {
        assert!(tokio::time::Instant::now() < until, "writer not dispatched");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}
async fn clean(runtime: &IdentityRuntime, account: talk_core::ids::AccountId) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Ok(RuntimeReply::State(summary)) =
            command(runtime, RuntimeCommand::Maintain { account }).await
            && !summary.cleanup_pending
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "settled cleanup deadline"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
#[tokio::test]
async fn dispatched_late_writer_cannot_reopen_logout_or_session_and_live_p_retention_does_not_wait()
{
    for choice in ["logout", "session", "retain"] {
        let server = Server::start(35).await;
        let fixture = Fixture::new();
        let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
        let faults = config.faults.clone();
        let runtime = IdentityRuntime::start(config).unwrap();
        let (account, attempt) = login(&runtime, &server, None).await;
        let RuntimeReply::Active { summary, .. } = command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt,
                choice: StorageChoice::PrivateFile,
            },
        )
        .await
        .unwrap() else {
            panic!("P activation");
        };
        let previous = if let StoredState::Eligible(item) = summary.storage {
            item
        } else {
            panic!("P persistence");
        };
        let old_scope = summary.scope.unwrap();
        let token = runtime.account_cancellation(old_scope).unwrap();
        let (account, attempt) = login(&runtime, &server, Some(account)).await;
        let gate = ReleaseGate(faults.arm());
        let save = runtime
            .try_submit(RuntimeCommand::Save {
                account,
                attempt,
                choice: StorageChoice::PrivateFile,
            })
            .unwrap();
        entered(&gate.0).await;
        if choice == "logout" {
            let start = std::time::Instant::now();
            runtime.logout(account).unwrap();
            assert!(token.is_cancelled());
            wait_phase(&runtime, account, AccountPhase::Disconnected).await;
            assert!(
                start.elapsed() < std::time::Duration::from_millis(250),
                "local closure/suppression waited on provider"
            );
        }
        assert!(save.wait().await.is_err());
        match choice {
            "session" => {
                let RuntimeReply::Active { summary, .. } = command(
                    &runtime,
                    RuntimeCommand::Save {
                        account,
                        attempt,
                        choice: StorageChoice::SessionOnly,
                    },
                )
                .await
                .unwrap() else {
                    panic!("session activation");
                };
                assert_eq!(summary.storage, StoredState::SessionOnly);
                assert!(summary.cleanup_pending);
                assert!(
                    command(
                        &runtime,
                        RuntimeCommand::RetainPrevious { account, attempt }
                    )
                    .await
                    .is_err()
                );
            }
            "retain" => {
                let RuntimeReply::State(summary) = command(
                    &runtime,
                    RuntimeCommand::RetainPrevious { account, attempt },
                )
                .await
                .unwrap() else {
                    panic!("retention");
                };
                assert_eq!(summary.storage, StoredState::Eligible(previous));
                assert!(summary.cleanup_pending);
                assert_eq!(summary.scope, Some(old_scope));
                assert!(!token.is_cancelled());
            }
            _ => {}
        }
        gate.0.release();
        clean(&runtime, account).await;
        assert!(runtime.join());
        let ledger = fixture.ledger();
        let a = ledger.account(account).unwrap();
        assert_eq!(a.disposition, if choice == "retain" { 2 } else { 0 });
        assert!(providers(&ledger).read(previous).is_ok());
        let operations = ledger.operations(account).unwrap();
        assert!(
            operations
                .iter()
                .filter(|o| o.item != previous)
                .all(|o| o.abandoned && o.cleanup && o.settlement == 2)
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn failed_durable_suppression_does_not_activate_a_session_promise() {
    use std::os::unix::fs::PermissionsExt;
    let server = Server::start(34).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let (account, attempt) = login(&runtime, &server, None).await;
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt,
                choice: StorageChoice::SessionOnly
            }
        )
        .await,
        Err(IdentityError::SuppressionUnconfirmed)
    ));
    let summary = runtime.snapshot().as_slice()[0].clone();
    assert!(summary.scope.is_none());
    assert!(summary.suppression_unconfirmed);
    assert_eq!(summary.storage, StoredState::Unknown);
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    runtime.cancel_attempt(account, attempt).unwrap();
    wait_phase(&runtime, account, AccountPhase::Disconnected).await;
    assert!(runtime.join());
    assert_eq!(fixture.ledger().account(account).unwrap().disposition, 0);
}

#[tokio::test]
async fn dropped_save_observation_closes_activation_but_owned_writer_settles_and_cleans() {
    let server = Server::start(34).await;
    let fixture = Fixture::new();
    let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
    let faults = config.faults.clone();
    let runtime = IdentityRuntime::start(config).unwrap();
    let (account, attempt) = login(&runtime, &server, None).await;
    let gate = ReleaseGate(faults.arm());
    let observation = runtime
        .try_submit(RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::PrivateFile,
        })
        .unwrap();
    entered(&gate.0).await;
    drop(observation);
    gate.0.release();
    clean(&runtime, account).await;
    assert!(runtime.snapshot().as_slice()[0].scope.is_none());
    assert!(runtime.join());
    let ledger = fixture.ledger();
    assert_eq!(ledger.account(account).unwrap().disposition, 0);
    assert!(
        ledger
            .operations(account)
            .unwrap()
            .iter()
            .all(|o| o.cleanup && o.abandoned && !o.activated)
    );
}

#[test]
fn completed_cleanup_records_reclaim_capacity_without_collecting_retained_or_unknown_items() {
    let fixture = Fixture::new();
    let mut ledger = fixture.ledger();
    let account = ledger.reserve("https://example.invalid/".into()).unwrap();
    let p = ledger
        .stage(
            account.id,
            LoginAttemptId::from_local(random().unwrap()),
            StorageBackend::PrivateFile,
            "same-alias",
        )
        .unwrap();
    ledger.settle(p, true).unwrap();
    ledger
        .activate(p, ledger.account(account.id).unwrap().revision)
        .unwrap();
    for _ in 0..40 {
        let n = ledger
            .stage(
                account.id,
                LoginAttemptId::from_local(random().unwrap()),
                StorageBackend::LinuxSecretService,
                "new-alias",
            )
            .unwrap();
        assert_ne!(n.item, p.item);
        assert_eq!(n.retained, Some(p.id));
        ledger.settle(n, false).unwrap();
        assert!(ledger.retire(p.item).is_err());
        ledger
            .retain(
                n,
                ledger.operation(account.id, p.id).unwrap(),
                ledger.account(account.id).unwrap().revision,
            )
            .unwrap();
        ledger.cleaned(n).unwrap();
        assert!(ledger.operations(account.id).unwrap().len() <= 3);
        assert_eq!(ledger.account(account.id).unwrap().eligible, Some(p.id));
    }
    assert!(ledger.forget(account.id).is_err());
    ledger.retire_all(account.id).unwrap();
    ledger.cleaned(p).unwrap();
    ledger.forget(account.id).unwrap();
    assert!(ledger.accounts().unwrap().is_empty());
}

#[tokio::test]
async fn later_explicit_save_uses_private_memory_and_removal_requires_exact_cleanup() {
    let server = Server::start(35).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let (account, attempt) = login(&runtime, &server, None).await;
    let RuntimeReply::Active { summary, .. } = command(
        &runtime,
        RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::SessionOnly,
        },
    )
    .await
    .unwrap() else {
        panic!("active");
    };
    let old_scope = summary.scope.unwrap();
    let closure = runtime.account_cancellation(old_scope).unwrap();
    let polls = server.polls.load(Ordering::Acquire);
    let RuntimeReply::Authorized { attempt, .. } =
        command(&runtime, RuntimeCommand::BeginSave { account })
            .await
            .unwrap()
    else {
        panic!("save authorization");
    };
    let RuntimeReply::Active { summary, .. } = command(
        &runtime,
        RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::PrivateFile,
        },
    )
    .await
    .unwrap() else {
        panic!("active file");
    };
    assert!(closure.is_cancelled());
    assert_eq!(server.polls.load(Ordering::Acquire), polls);
    assert!(matches!(summary.storage, StoredState::Eligible(_)));
    assert!(
        command(&runtime, RuntimeCommand::ForgetAccount { account })
            .await
            .is_err()
    );
    let closure = runtime
        .account_cancellation(summary.scope.unwrap())
        .unwrap();
    command(&runtime, RuntimeCommand::RetireAll { account })
        .await
        .unwrap();
    assert!(closure.is_cancelled());
    assert!(
        command(&runtime, RuntimeCommand::ForgetAccount { account })
            .await
            .is_err()
    );
    command(&runtime, RuntimeCommand::Maintain { account })
        .await
        .unwrap();
    assert!(
        matches!(command(&runtime,RuntimeCommand::ForgetAccount{account}).await,Ok(RuntimeReply::Forgotten(id)) if id==account)
    );
    assert!(runtime.snapshot().as_slice().is_empty());
    // Repeated cancelled additions do not consume permanent account/worker slots.
    for _ in 0..20 {
        let (id, attempt) = login(&runtime, &server, None).await;
        runtime.cancel_attempt(id, attempt).unwrap();
        wait_phase(&runtime, id, AccountPhase::Disconnected).await;
        assert!(matches!(
            command(&runtime, RuntimeCommand::ForgetAccount { account: id }).await,
            Ok(RuntimeReply::Forgotten(_))
        ));
    }
    assert!(runtime.join());
    assert!(fixture.ledger().accounts().unwrap().is_empty());
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "native_service.py owns a private Secret Service bus/collection and tears it down"]
fn real_secret_service_roundtrip_and_locked_store_are_distinct() {
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
            "private-native-alias",
        )
        .unwrap();
    let password = zeroize::Zeroizing::new(format!("synthetic-{}", random().unwrap()));
    let p = providers(&ledger);
    assert!(matches!(p.write(op.item, &password), WriteResult::Present));
    assert_eq!(ledger.account(account.id).unwrap().disposition, 1);
    ledger.settle(op, true).unwrap();
    ledger
        .activate(op, ledger.account(account.id).unwrap().revision)
        .unwrap();
    drop(p);
    drop(ledger);
    let mut ledger = fixture.ledger();
    let p = providers(&ledger);
    assert!(p.read(op.item).unwrap().as_str() == password.as_str());
    ledger.retire_all(account.id).unwrap();
    p.delete(op.item).unwrap();
    ledger.cleaned(op).unwrap();
    assert!(matches!(
        p.read(op.item),
        Err(IdentityError::Storage {
            failure: StorageFailure::Missing,
            ..
        })
    ));
    let native =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .unwrap();
    let collection = native.get_default_collection().unwrap();
    collection.lock().unwrap();
    assert!(collection.is_locked().unwrap());
    assert!(matches!(
        p.read(op.item),
        Err(IdentityError::Storage {
            failure: StorageFailure::Locked,
            ..
        })
    ));
    let next = ledger
        .stage(
            account.id,
            LoginAttemptId::from_local(random().unwrap()),
            StorageBackend::LinuxSecretService,
            "private-native-alias",
        )
        .unwrap();
    assert!(matches!(
        p.write(next.item, &password),
        WriteResult::NotWritten(IdentityError::Storage {
            failure: StorageFailure::Locked,
            ..
        })
    ));
    ledger.settle(next, false).unwrap();
    ledger.retire_all(account.id).unwrap();
    ledger.cleaned(next).unwrap();
    ledger.forget(account.id).unwrap();
}

#[tokio::test]
async fn ambiguous_account_rejection_preserves_eligibility_and_other_account_until_genuine_loss() {
    use serde_json::json;
    let server = Server::start(35).await;
    let other = Server::start(34).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let mut active = Vec::new();
    for source in [&server, &other] {
        let (account, attempt) = login(&runtime, source, None).await;
        let RuntimeReply::Active { summary, .. } = command(
            &runtime,
            RuntimeCommand::Save {
                account,
                attempt,
                choice: StorageChoice::PrivateFile,
            },
        )
        .await
        .unwrap() else {
            panic!("active file");
        };
        active.push(summary);
    }
    let original = &active[0];
    let scope = original.scope.unwrap();
    let closure = runtime.account_cancellation(scope).unwrap();
    for meta in [
        json!({"status":"ok","statuscode":200,"message":""}),
        json!({"statuscode":997,"message":""}),
        json!({"status":true,"statuscode":997,"message":""}),
        json!({"status":"failure","statuscode":"997","message":""}),
        json!({"status":"failure","statuscode":403,"message":""}),
    ] {
        *server.probe_override.lock().unwrap() =
            Some((401, json!({"ocs":{"meta":meta,"data":[]}})));
        assert!(matches!(
            command(
                &runtime,
                RuntimeCommand::Request {
                    scope,
                    request: ProtocolRequest::ProbeAccount
                }
            )
            .await,
            Err(IdentityError::Request(ErrorKind::Malformed))
        ));
        let page = runtime.snapshot();
        let current = page
            .as_slice()
            .iter()
            .find(|a| a.account == original.account)
            .unwrap();
        assert_eq!(current.storage, original.storage);
        assert_eq!(current.revision, original.revision);
        assert_eq!(current.scope, Some(scope));
        assert!(!closure.is_cancelled());
        command(
            &runtime,
            RuntimeCommand::Request {
                scope: active[1].scope.unwrap(),
                request: ProtocolRequest::ProbeAccount,
            },
        )
        .await
        .unwrap();
    }
    assert!(runtime.join());
    {
        let ledger = fixture.ledger();
        let record = ledger.account(original.account).unwrap();
        assert_eq!(record.revision, original.revision);
        assert!(record.eligible.is_some());
        assert!(
            ledger
                .operations(original.account)
                .unwrap()
                .iter()
                .all(|o| !o.abandoned)
        );
    }
    *server.probe_override.lock().unwrap() = None;
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    command(&runtime, RuntimeCommand::Accounts).await.unwrap();
    let mut restored = Vec::new();
    for account in active.iter().map(|a| a.account) {
        let RuntimeReply::Active { summary, .. } =
            command(&runtime, RuntimeCommand::Restore { account })
                .await
                .unwrap()
        else {
            panic!("restore");
        };
        restored.push(summary.scope.unwrap());
    }
    let closure = runtime.account_cancellation(restored[0]).unwrap();
    server.revoked.store(true, Ordering::Release);
    assert!(matches!(
        command(
            &runtime,
            RuntimeCommand::Request {
                scope: restored[0],
                request: ProtocolRequest::ProbeAccount
            }
        )
        .await,
        Err(IdentityError::Request(ErrorKind::ReauthenticationRequired))
    ));
    assert!(closure.is_cancelled());
    wait_phase(
        &runtime,
        original.account,
        AccountPhase::ReauthenticationRequired,
    )
    .await;
    command(
        &runtime,
        RuntimeCommand::Request {
            scope: restored[1],
            request: ProtocolRequest::ProbeAccount,
        },
    )
    .await
    .unwrap();
    assert!(
        command(
            &runtime,
            RuntimeCommand::Request {
                scope,
                request: ProtocolRequest::ProbeAccount
            }
        )
        .await
        .is_err()
    );
    assert!(runtime.join());
    let ledger = fixture.ledger();
    assert!(ledger.account(original.account).unwrap().eligible.is_none());
    assert!(
        ledger
            .account(active[1].account)
            .unwrap()
            .eligible
            .is_some()
    );
}

#[test]
fn connected_history_survives_restart_without_restoring_authority() {
    let fixture = Fixture::new();
    let account = {
        let mut ledger = fixture.ledger();
        let a = ledger.reserve("https://example.invalid/".into()).unwrap();
        assert!(!a.connected_once);
        ledger.remember_connection(a.id).unwrap();
        a.id
    };
    let mut ledger = fixture.ledger();
    let restored = ledger.account(account).unwrap();
    assert!(restored.connected_once);
    assert_eq!(restored.disposition, 0);
    assert!(restored.eligible.is_none());
    ledger.suppress(account, false, None, true).unwrap();
    assert!(ledger.account(account).unwrap().connected_once);
    ledger.forget(account).unwrap();
    assert!(ledger.account(account).is_err());
}

#[test]
fn avatar_disk_cache_is_account_scoped_durable_bounded_and_removed_on_logout() {
    use super::avatars::Store;
    use crate::nextcloud::avatars::{AvatarStore, TTL};
    use std::sync::Mutex;
    let fixture = Fixture::new();
    let mut ledger = fixture.ledger();
    let mut accounts = Vec::new();
    for user in ["synthetic-a", "synthetic-b"] {
        let a = ledger.reserve(format!("https://{user}.invalid/")).unwrap();
        let op = ledger
            .stage(
                a.id,
                LoginAttemptId::from_local(random().unwrap()),
                StorageBackend::PrivateFile,
                user,
            )
            .unwrap();
        ledger.settle(op, true).unwrap();
        let revision = ledger.account(a.id).unwrap().revision;
        ledger.activate(op, revision).unwrap();
        accounts.push(a.id);
    }
    let session = ledger
        .reserve("https://session.invalid/".into())
        .unwrap()
        .id;
    let ledger = Arc::new(Mutex::new(ledger));
    let mut a = Store::new(ledger.clone(), accounts[0]);
    let mut b = Store::new(ledger.clone(), accounts[1]);
    let mut temporary = Store::new(ledger.clone(), session);
    let key = format!("u-{:064x}", 159);
    assert!(temporary.put(&key, b"temporary", 100).is_err());
    a.put(&key, b"account-a", 100).unwrap();
    b.put(&key, b"account-b", 100).unwrap();
    assert_eq!(a.get(&key, 101).unwrap().unwrap().0, b"account-a");
    assert_eq!(b.get(&key, 101).unwrap().unwrap().0, b"account-b");
    for n in 0..150 {
        a.put(&format!("u-{n:064x}"), b"x", 101).unwrap();
    }
    assert!(
        a.get(&key, 102).unwrap().is_none(),
        "oldest is evicted at 128 entries"
    );
    a.put(&key, b"after-eviction", 102).unwrap();
    drop((a, b, temporary, ledger));
    let ledger = Arc::new(Mutex::new(fixture.ledger()));
    let mut a = Store::new(ledger.clone(), accounts[0]);
    let mut b = Store::new(ledger.clone(), accounts[1]);
    assert_eq!(a.get(&key, 103).unwrap().unwrap().0, b"after-eviction");
    let lost_room = format!("r-{:064x}", 1);
    let retained_room = format!("r-{:064x}", 2);
    a.put(&lost_room, b"removed-room", 103).unwrap();
    a.put(&retained_room, b"retained-room", 103).unwrap();
    a.retain_rooms(std::slice::from_ref(&retained_room))
        .unwrap();
    assert!(a.get(&lost_room, 104).unwrap().is_none());
    assert_eq!(
        a.get(&retained_room, 104).unwrap().unwrap().0,
        b"retained-room"
    );
    assert!(
        a.get(&key, 104).unwrap().is_some(),
        "room cleanup keeps person avatars"
    );
    assert!(a.get(&key, 102 + TTL).unwrap().is_none());
    assert!(
        b.get(&key, 99).unwrap().is_none(),
        "clock rollback expires rather than extends"
    );
    let full = vec![7; 1024 * 1024];
    for n in 200..220 {
        a.put(&format!("u-{n:064x}"), &full, 150).unwrap();
    }
    assert!(a.get(&format!("u-{:064x}", 200), 151).unwrap().is_none());
    assert_eq!(
        a.get(&format!("u-{:064x}", 219), 151)
            .unwrap()
            .unwrap()
            .0
            .len(),
        full.len()
    );
    assert!(
        fixture
            .0
            .join(format!("avatars-{:032x}.sqlite", accounts[0].as_local()))
            .metadata()
            .unwrap()
            .len()
            <= 16 * 1024 * 1024
    );
    a.put(&key, &vec![0; 1024 * 1024 + 1], 151).unwrap();
    assert!(
        a.get(&key, 152).unwrap().is_none(),
        "oversized images stay out of disk"
    );
    a.put(&key, b"before-logout", 200).unwrap();
    ledger
        .lock()
        .unwrap()
        .suppress(accounts[0], false, None, true)
        .unwrap();
    assert!(a.get(&key, 201).is_err());
    assert!(a.put(&key, b"late-write", 201).is_err());
    assert!(
        !fixture
            .0
            .join(format!("avatars-{:032x}.sqlite", accounts[0].as_local()))
            .exists()
    );
    assert!(
        fixture
            .0
            .join(format!("avatars-{:032x}.sqlite", accounts[1].as_local()))
            .exists()
    );
    assert!(
        !fixture
            .0
            .join(format!("avatars-{:032x}.sqlite", session.as_local()))
            .exists()
    );
}

#[test]
fn schema_six_upgrade_recovers_historical_connection_without_changing_eligibility() {
    let fixture = Fixture::new();
    let (account, operation, revision) = {
        let mut ledger = fixture.ledger();
        let account = ledger
            .reserve("https://example.invalid/".into())
            .unwrap()
            .id;
        let op = ledger
            .stage(
                account,
                LoginAttemptId::from_local(random().unwrap()),
                StorageBackend::PrivateFile,
                "synthetic",
            )
            .unwrap();
        ledger.settle(op, true).unwrap();
        ledger
            .activate(op, ledger.account(account).unwrap().revision)
            .unwrap();
        let revision = ledger.account(account).unwrap().revision;
        (account, op, revision)
    };
    let db = rusqlite::Connection::open(fixture.0.join("restoration.sqlite")).unwrap();
    db.execute_batch("BEGIN IMMEDIATE; ALTER TABLE accounts DROP COLUMN connected_once; UPDATE meta SET schema=6; COMMIT;").unwrap();
    drop(db);
    let ledger = fixture.ledger();
    let account = ledger.account(account).unwrap();
    assert!(account.connected_once);
    assert_eq!(account.revision, revision);
    assert_eq!(account.disposition, 2);
    assert_eq!(account.eligible, Some(operation.id));
    assert_eq!(
        ledger.operation(account.id, operation.id).unwrap().item,
        operation.item
    );
}
