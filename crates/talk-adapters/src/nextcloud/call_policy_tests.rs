//! Synthetic protocol payloads based on spreed main a009ace328ee and stable35
//! 8a724875e0ef: Capabilities.php, E2EE.isEnabled and recording consent constants.
//! They test admission, not interoperability with an untested server release.
use super::*;
use crate::nextcloud::call_policy::{Bundle, Sources};
use nextcloud_talk_api::{
    self as api,
    meeting::{Observation, ObservedReply},
    models::RoomToken,
    signaling::PeerSession,
};
use talk_core::{
    calls::{CallSession, Entry, Gate, MediaWishes, PolicyEvidence},
    ids::{CallGeneration, CallScope},
    policy::{ConsentMode, Observed},
};

fn current_caps(major: u16) -> Value {
    let mut value: Value =
        serde_json::from_str(include_str!("fixtures/spreed-call-capabilities.json")).unwrap();
    value["version"]["major"] = json!(major);
    value
}
async fn bundle(mut capabilities: Value, changes: Value) -> Result<Bundle, PolicyEvidence> {
    capabilities["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!("audio"));
    let mut room = room();
    room["canStartCall"] = json!(true);
    room["hasCall"] = json!(true);
    room["callRecording"] = json!(0);
    room["recordingConsent"] = json!(0);
    for (key, value) in changes.as_object().unwrap() {
        room[key] = value.clone();
    }
    let server = Server::start(vec![
        Reply::data(json!({"id":"fixture_user"})), Reply::data(capabilities), Reply::data(room),
        Reply::data(json!([{"actorType":"users","actorId":"fixture_user","sessionIds":["own-session"],"inCall":0,"displayName":"Fixture"},{"actorType":"users","actorId":"fixture_peer","sessionIds":["peer-session"],"inCall":7}])),
        Reply::data({
            let mut settings: Value = serde_json::from_str(include_str!("fixtures/spreed-signaling-settings.json")).unwrap();
            settings["future-settings"] = json!({"value":77}); settings
        }),
        Reply::data(json!([{"sessionId":"own-session"},{"sessionId":"peer-session"}])),
    ]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let id = bind(&mut client);
    let scope = CallScope {
        account: scope(),
        conversation: id,
        call: CallGeneration::from_local(8),
    };
    let mut sources = Sources::new(scope, PeerSession::parse("own-session").unwrap());
    let room = RoomToken::parse("testroom").unwrap();
    for (observation, request) in [
        (Observation::Account, api::Request::Account),
        (Observation::Capabilities, api::Request::Capabilities),
        (Observation::Room, api::Request::Room(room.clone())),
        (
            Observation::ConnectedParticipants,
            api::Request::Participants(room.clone()),
        ),
        (
            Observation::SignalingSettings,
            api::Request::SignalingSettings(room.clone()),
        ),
        (
            Observation::CallParticipants,
            api::Request::CallParticipants(room),
        ),
    ] {
        let started = client.transport.api_now();
        let reply = client
            .transport
            .api_request(&context(&runtime), request)
            .await
            .unwrap();
        sources.add(
            observation,
            ObservedReply {
                started,
                received: client.transport.api_now(),
                reply,
            },
        );
    }
    sources.finish(false)
}
fn advance(bundle: &Bundle) -> Result<talk_core::calls::Grant, Gate> {
    advance_with(bundle, super::super::meeting_runtime::SUPPORT)
}
fn advance_with(
    bundle: &Bundle,
    support: talk_core::calls::NativeSupport,
) -> Result<talk_core::calls::Grant, Gate> {
    let mut call = CallSession::new(bundle.inputs.scope, Entry::GroupAudio).unwrap();
    call.accept(bundle.inputs.scope, MediaWishes::default())
        .unwrap();
    let now = match bundle.inputs.account_valid {
        Observed::Known { lease, .. } => Moment {
            millis: lease.expires().millis - 1,
            ..lease.expires()
        },
        _ => unreachable!(),
    };
    call.advance(
        &bundle.inputs,
        bundle.exists,
        support,
        super::super::meeting_runtime::DEVICES,
        now,
    )
}
#[tokio::test]
async fn newer_server_and_optional_fields_do_not_deny_join() {
    let mut caps = current_caps(99);
    let c = &mut caps["capabilities"]["spreed"];
    c["features"]
        .as_array_mut()
        .unwrap()
        .push(json!("future-capability"));
    c["config"]["call"]["future-setting"] = json!({"unknown":"value"});
    c["config"]["call"]["end-to-end-encryption"] = json!(true); // No E2EE capability => off.
    c["config"]["call"]["enabled"] = json!(false); // Only start is prohibited.
    c["config"]["call"]["supported-reactions"] = json!([{"unknown":true}]);
    for key in ["recording", "grid-limit", "grid-limit-enforced"] {
        c["config"]["call"].as_object_mut().unwrap().remove(key);
    }
    let bundle = bundle(caps, json!({"permissions":511+1024,"displayName":null}))
        .await
        .unwrap();
    assert_eq!(advance(&bundle).unwrap().remote_video_limit(), 0);
    assert!(bundle.recording_control.is_none());
    assert!(bundle.reactions.as_slice().is_empty());
    assert_eq!(
        bundle.peers[0].name.as_str(),
        "",
        "display labels are not call authority"
    );
}
#[tokio::test]
async fn e2ee_support_and_requirement_match_official_feature_guard() {
    for (enabled, expected) in [
        (json!(false), false),
        (json!(true), true),
        (Value::Null, true),
        (json!("future"), true),
    ] {
        let mut caps = current_caps(36);
        caps["capabilities"]["spreed"]["features"]
            .as_array_mut()
            .unwrap()
            .push(json!("call-end-to-end-encryption"));
        caps["capabilities"]["spreed"]["config"]["call"]["end-to-end-encryption"] = enabled;
        let bundle = bundle(caps, json!({})).await.unwrap();
        // This client implements call E2EE, so a required call is admitted...
        assert!(advance(&bundle).is_ok());
        // ...and is blocked, never downgraded, by a build or platform without it.
        let without = talk_core::calls::NativeSupport {
            end_to_end_encryption: false,
            ..super::super::meeting_runtime::SUPPORT
        };
        assert_eq!(
            advance_with(&bundle, without).err(),
            expected.then_some(Gate::RequiredEncryption)
        );
    }
}
#[tokio::test]
async fn unknown_consent_requires_explicit_agreement_classified_never_assents() {
    for consent in [json!(3), json!("future"), Value::Null] {
        let mut caps = current_caps(36);
        caps["capabilities"]["spreed"]["features"]
            .as_array_mut()
            .unwrap()
            .push(json!("classified-conversations"));
        caps["capabilities"]["spreed"]["config"]["call"]["recording-consent"] = consent;
        let ordinary = bundle(caps.clone(), json!({})).await.unwrap();
        assert_eq!(advance(&ordinary).err(), Some(Gate::RecordingConsent));
        let Observed::Known { value, .. } = ordinary.inputs.host_configuration else {
            panic!()
        };
        assert_eq!(value.consent, ConsentMode::Required);
        assert!(advance(&bundle(caps, json!({"attributes":4})).await.unwrap()).is_ok());
    }
    let mut caps = current_caps(35);
    caps["capabilities"]["spreed"]["config"]["call"]["recording-consent"] = json!(2);
    assert_eq!(
        advance(&bundle(caps, json!({"recordingConsent":99})).await.unwrap()).err(),
        Some(Gate::RecordingConsent)
    );
}
#[tokio::test]
async fn missing_security_evidence_is_named_and_has_no_grant() {
    for (feature, evidence) in [
        ("conversation-v4", PolicyEvidence::ConversationApi),
        ("signaling-v3", PolicyEvidence::SignalingApi),
    ] {
        let mut caps = current_caps(36);
        caps["capabilities"]["spreed"]["features"]
            .as_array_mut()
            .unwrap()
            .retain(|value| value != feature);
        assert_eq!(bundle(caps, json!({})).await.err(), Some(evidence));
    }

    for (changes, evidence) in [
        (json!({"permissions":null}), PolicyEvidence::Permissions),
        (json!({"attributes":32}), PolicyEvidence::Attributes),
        (json!({"hasCall":null}), PolicyEvidence::CallExists),
    ] {
        assert_eq!(
            bundle(current_caps(35), changes).await.err(),
            Some(evidence)
        );
    }
}

#[tokio::test]
async fn signaling_fetch_failure_stays_pending_and_retry_can_reach_consent() {
    use talk_core::calls::Phase;
    let mut caps = current_caps(35);
    caps["capabilities"]["spreed"]["config"]["call"]["recording-consent"] = json!(1);
    let mut current_room = room();
    for (key, value) in [
        ("hasCall", json!(true)),
        ("canStartCall", json!(true)),
        ("callRecording", json!(0)),
        ("recordingConsent", json!(0)),
    ] {
        current_room[key] = value;
    }
    let mut replies = Vec::new();
    for fail in [true, false] {
        replies.extend([
            Reply::data(json!({"sessionId":"own-session"})),
            Reply::data(json!({"id":"fixture_user"})), Reply::data(caps.clone()),
            Reply::data(current_room.clone()),
            Reply::data(json!([{"actorType":"users","actorId":"fixture_user","sessionIds":["own-session"]}])),
        ]);
        if fail {
            replies.push(Reply::envelope(503, "failure", 503, json!({})));
        } else {
            replies.push(Reply::data(
                json!({"signalingMode":"internal","stunservers":[],"turnservers":[]}),
            ));
            replies.push(Reply::data(json!([])));
        }
    }
    let server = Server::start(replies).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    client.set_media_service(Some(crate::media::MediaService::new()));
    let conversation = bind(&mut client);
    let scope = client.call_scope_for(conversation).unwrap().unwrap();
    for command in [
        talk_core::calls::Command::Start {
            call: scope,
            entry: Entry::GroupAudio,
            wishes: MediaWishes::default(),
        },
        talk_core::calls::Command::Reconnect(scope),
    ] {
        let reply = client.meeting(&context(&runtime), command).await.unwrap();
        let ProtocolReply::Meeting(value) = reply else {
            panic!()
        };
        let expected = if server.count() == 6 {
            Gate::PolicyUnavailable(PolicyEvidence::SignalingSettings)
        } else {
            Gate::RecordingConsent
        };
        assert_eq!(value.phase, Phase::Waiting(expected));
        assert_eq!(value.microphone, talk_core::calls::DeviceState::Off);
    }
    let seen = server.seen.lock().unwrap();
    assert!(
        seen.iter()
            .all(|request| !request.starts_with(b"POST /ocs/v2.php/apps/spreed/api/v4/call/"))
    );
}
