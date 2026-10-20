//! Negative TLS verification against an isolated, explicitly untrusted local certificate.
#![cfg(feature = "backend-test")]
use std::path::Path;
use talk_adapters::nextcloud::{NetworkRuntime, test_support};
use talk_core::ids::{AccountId, AccountScope, ClockEpoch, HostId, RuntimeGeneration};
use talk_core::lifecycle::Cancellation;
use talk_core::policy::Moment;
use talk_core::ports::{ErrorKind, NextcloudPort, OperationContext, ProtocolRequest};
#[tokio::test]
#[ignore = "run_tls.py owns local TLS fixture and private intake"]
async fn untrusted_tls_certificate_cannot_receive_credentials() {
    let path = std::env::var_os("NMT_TLS_FIXTURE").expect("private fixture path");
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(91));
    let scope = AccountScope {
        account: AccountId::from_local(1),
        runtime: RuntimeGeneration::from_local(1),
    };
    let mut client = test_support::from_private_fixture(
        Path::new(&path),
        scope,
        HostId::from_local(1),
        runtime.clone(),
    )
    .expect("private intake");
    let now = runtime.now();
    let context = OperationContext {
        scope,
        deadline: Moment {
            epoch: now.epoch,
            millis: now.millis + 2900,
        },
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        client
            .request(context, ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Transport
    );
}
