//! Drives one authorized call while the desktop device owner supplies native frames.
use crate::{
    media::MediaService,
    nextcloud::{NetworkRuntime, NextcloudClient, test_support},
};
use std::{path::Path, sync::Arc, time::Duration};
use talk_core::{
    calls::{Command, DeviceState, Entry, MediaWishes, Phase},
    ids::{AccountId, AccountScope, CallScope, ClockEpoch, HostId, RuntimeGeneration},
    ports::{NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest},
};

type Result<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Microphone,
    Camera,
}

pub fn run(
    fixture: &Path,
    scenario: Scenario,
    media: Arc<MediaService>,
    pause_microphone: impl FnOnce(CallScope) -> Result<()>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|_| "runtime startup")?;
    let outcome = runtime.block_on(exercise(fixture, scenario, media, pause_microphone));
    runtime.shutdown_timeout(Duration::from_secs(2));
    outcome
}

struct Sender {
    client: NextcloudClient,
    network: NetworkRuntime,
    scope: AccountScope,
}

impl Sender {
    async fn request(&mut self, request: ProtocolRequest) -> Result<ProtocolReply> {
        let mut deadline = self.network.now();
        deadline.millis += 5000;
        let context = OperationContext {
            scope: self.scope,
            deadline,
            cancellation: self.client.cancellation(),
        };
        self.client
            .request(context, request)
            .await
            .map_err(|_| "Talk request")
    }

    // The receiver acknowledges each distinct phase in the fresh private fixture
    // directory. Continue polling the call so publication and signaling advance.
    async fn confirm(
        &mut self,
        call: CallScope,
        dir: &Path,
        name: &'static str,
        microphone: DeviceState,
        camera: DeviceState,
    ) -> Result<()> {
        let ready = dir.join(format!("{name}.ready"));
        let ack = dir.join(format!("{name}.ok"));
        let abort = dir.join("abort");
        let mut announced = false;
        tokio::time::timeout(Duration::from_secs(40), async {
            loop {
                if abort.is_file() {
                    return Err("receiver aborted");
                }
                let ProtocolReply::Meeting(state) = self
                    .request(ProtocolRequest::Meeting(Command::Observe(call)))
                    .await?
                else {
                    return Err("meeting reply");
                };
                if state.call != call || state.phase == Phase::Ended {
                    return Err("call generation ended");
                }
                if state.phase == Phase::Active
                    && state.microphone == microphone
                    && state.camera == camera
                {
                    if !announced {
                        std::fs::write(&ready, []).map_err(|_| "phase marker")?;
                        announced = true;
                    }
                    if ack.is_file() {
                        return Ok(());
                    }
                } else if announced {
                    return Err("device state lost before receiver confirmation");
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .map_err(|_| name)?
    }

    async fn exchange(
        &mut self,
        call: CallScope,
        dir: &Path,
        scenario: Scenario,
        pause_microphone: impl FnOnce(CallScope) -> Result<()>,
    ) -> Result<()> {
        use DeviceState::{Active, Off, Unavailable};
        self.request(ProtocolRequest::Meeting(Command::Start {
            call,
            entry: Entry::PreparedConference,
            wishes: MediaWishes::default(),
        }))
        .await?;
        self.confirm(call, dir, "receive-only", Off, Off).await?;
        let mut wishes = MediaWishes {
            microphone: true,
            ..Default::default()
        };
        self.request(ProtocolRequest::Meeting(Command::Change { call, wishes }))
            .await?;
        if scenario == Scenario::Camera {
            self.confirm(call, dir, "missing-mic", Unavailable, Off)
                .await?;
            wishes.camera = true;
            self.request(ProtocolRequest::Meeting(Command::Change { call, wishes }))
                .await?;
            self.confirm(call, dir, "camera-live", Unavailable, Active)
                .await
        } else {
            self.confirm(call, dir, "mic-live", Active, Off).await?;
            pause_microphone(call)?;
            self.confirm(call, dir, "mic-stalled", Unavailable, Off)
                .await?;
            self.request(ProtocolRequest::Meeting(Command::Change { call, wishes }))
                .await?;
            self.confirm(call, dir, "mic-retried", Active, Off).await
        }
    }
}

async fn exercise(
    fixture: &Path,
    scenario: Scenario,
    media: Arc<MediaService>,
    pause_microphone: impl FnOnce(CallScope) -> Result<()>,
) -> Result<()> {
    let scope = AccountScope {
        account: AccountId::from_local(1),
        runtime: RuntimeGeneration::from_local(1),
    };
    let network = NetworkRuntime::new(ClockEpoch::from_local(1));
    let mut client =
        test_support::from_private_fixture(fixture, scope, HostId::from_local(1), network.clone())
            .map_err(|_| "private fixture")?;
    let dir = fixture.parent().ok_or("fixture directory")?;
    client.fixture_media(media.clone());
    let mut sender = Sender {
        client,
        network,
        scope,
    };
    sender.request(ProtocolRequest::InspectAccount).await?;
    let room = sender.client.fixture_conversation().ok_or("fixture room")?;
    let ProtocolReply::Conversation(room) = sender
        .request(ProtocolRequest::ReconcileConversation(room))
        .await?
    else {
        return Err("room reply");
    };
    let call = room.call_scope.ok_or("room binding")?;
    let result = sender.exchange(call, dir, scenario, pause_microphone).await;
    let stop = sender
        .request(ProtocolRequest::Meeting(Command::Stop(call)))
        .await;
    sender.client.cancellation().cancel();
    result?;
    stop?;
    if media.capture().is_some() {
        return Err("capture still granted");
    }
    Ok(())
}
