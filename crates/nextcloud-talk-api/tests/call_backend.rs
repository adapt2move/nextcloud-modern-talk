//! Explicitly opted-in checks against the owned disposable Talk lab.
#![cfg(feature = "backend-test")]

use nextcloud_talk_api::{
    AppCredential, Client, NetworkRuntime, Reply, Request, ServerOrigin,
    meeting::{MeetingReader, MeetingWriter},
    models::{ParticipantFlags, RoomToken},
    scope::{
        BindingId, Cancellation, ClockId, LeaseId, Moment, OperationContext, OriginId, SessionScope,
    },
    signaling::{Event, PeerSession, activity::Activity},
};
use serde::Deserialize;
use std::path::PathBuf;
use zeroize::Zeroizing;

#[derive(Deserialize)]
struct Fixture {
    origin: String,
    room: String,
    alice_password: String,
    bob_password: String,
}

struct Actor {
    client: Client,
    runtime: NetworkRuntime,
    scope: SessionScope,
}
impl Actor {
    fn new(origin: &str, user: &str, password: String, id: u128) -> Self {
        let runtime = NetworkRuntime::new(ClockId::from_key(id));
        let scope = SessionScope {
            account: BindingId::from_key(id),
            runtime: LeaseId::from_key(id),
        };
        let client = Client::new(
            AppCredential::new(
                ServerOrigin::loopback(origin).expect("owned loopback origin"),
                Zeroizing::new(user.to_owned()),
                Zeroizing::new(password),
                Some(Zeroizing::new(user.to_owned())),
            ),
            runtime.clone(),
            scope,
            OriginId::from_key(id),
            Cancellation::default(),
        )
        .expect("synthetic client");
        Self {
            client,
            runtime,
            scope,
        }
    }
    fn context(&self) -> OperationContext {
        let now = self.runtime.now();
        OperationContext {
            scope: self.scope,
            deadline: Moment {
                epoch: now.epoch,
                millis: now.millis + 12000,
            },
            cancellation: Cancellation::default(),
        }
    }
    async fn request(&mut self, request: Request) -> Reply {
        let context = self.context();
        self.client
            .request(&context, request)
            .await
            .unwrap_or_else(|error| panic!("lab request: {:?}", error.kind))
    }
    async fn join(&mut self, room: &RoomToken) -> (MeetingReader, MeetingWriter) {
        self.request(Request::Account).await;
        self.request(Request::Rooms).await;
        self.request(Request::Room(room.clone())).await;
        self.request(Request::ActivateRoom(room.clone())).await;
        let Reply::CallJoined(call) = self
            .request(Request::JoinCall {
                room: room.clone(),
                flags: ParticipantFlags::new(false, false),
                silent: true,
                recording_consent: false,
            })
            .await
        else {
            panic!("call join reply")
        };
        let context = self.context();
        let (reader, writer, _stop) = self
            .client
            .open_meeting(&context, call)
            .expect("meeting lanes");
        (reader, writer)
    }
}

async fn activity(
    reader: &mut MeetingReader,
    actor: &Actor,
    source: &PeerSession,
    expected: &Activity,
) {
    for _ in 0..3 {
        let batch = reader
            .poll(&actor.context())
            .await
            .unwrap_or_else(|error| panic!("lab poll: {:?}", error.kind));
        if batch.events().iter().any(|event| {
            matches!(event,
            Event::Activity { from, activity } if from.matches(source) && activity == expected)
        }) {
            return;
        }
    }
    panic!("expected scoped activity was not delivered");
}

#[tokio::test]
#[ignore = "requires the private disposable r35 call fixture"]
async fn released_server_accepts_scoped_activity_and_moderation() {
    let path = PathBuf::from(std::env::var_os("NMT_CALL_FIXTURE").expect("private fixture"));
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(path).expect("fixture bytes"))
        .expect("fixture shape");
    let room = RoomToken::parse(&fixture.room).expect("synthetic room");
    let mut alice = Actor::new(&fixture.origin, "lab_alice", fixture.alice_password, 801);
    let (_alice_reader, mut alice_writer) = alice.join(&room).await;
    let alice_session = alice_writer.own_session().expect("owner session");
    let mut moderator = alice_writer
        .take_moderation_writer()
        .expect("moderation lane");
    let mut sender = alice_writer.take_activity_writer().expect("activity lane");
    let observation = alice_writer
        .observe(
            &alice.context(),
            nextcloud_talk_api::meeting::Observation::ConnectedParticipants,
        )
        .await
        .expect("participant observation");
    let Reply::Participants(people) = observation.reply else {
        panic!("participant reply")
    };
    let attendee = people
        .iter()
        .find(|person| person.actor_id.as_deref() == Some("lab_bob"))
        .and_then(|person| person.attendee_id)
        .expect("synthetic invitee");
    moderator
        .ring(&alice.context(), attendee)
        .await
        .unwrap_or_else(|error| panic!("ring: {:?}", error.kind));

    let mut bob = Actor::new(&fixture.origin, "lab_bob", fixture.bob_password, 802);
    let (mut bob_reader, mut bob_writer) = bob.join(&room).await;
    let bob_session = bob_writer.own_session().expect("member session");
    assert!(moderator.ring(&alice.context(), attendee).await.is_err());
    let hand = Activity::RaisedHand {
        state: true,
        timestamp: 42,
    };
    let lowered = Activity::RaisedHand {
        state: false,
        timestamp: 43,
    };
    let reaction = Activity::Reaction("👍".into());
    for item in [&hand, &lowered, &reaction] {
        sender
            .send(&alice.context(), std::slice::from_ref(&bob_session), item)
            .await
            .unwrap_or_else(|error| panic!("activity send: {:?}", error.kind));
        activity(&mut bob_reader, &bob, &alice_session, item).await;
    }
    moderator
        .mute_participant(&alice.context(), &bob_session)
        .await
        .unwrap_or_else(|error| panic!("moderator mute: {:?}", error.kind));
    let mut saw_mute = false;
    for _ in 0..3 {
        let batch = bob_reader
            .poll(&bob.context())
            .await
            .unwrap_or_else(|error| panic!("mute poll: {:?}", error.kind));
        if batch.events().iter().any(
            |event| matches!(event, Event::ForceMuteBy { from } if from.matches(&alice_session)),
        ) {
            saw_mute = true;
            break;
        }
    }
    assert!(saw_mute, "moderator control was not delivered");
    let mut member_moderation = bob_writer.take_moderation_writer().expect("member lane");
    assert!(
        member_moderation
            .mute_participant(&bob.context(), &alice_session)
            .await
            .is_err()
    );
    moderator
        .end_for_everyone(&alice.context())
        .await
        .unwrap_or_else(|error| panic!("end call: {:?}", error.kind));
}
