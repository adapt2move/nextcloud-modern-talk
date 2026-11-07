//! Opt-in released-server checks. Credentials exist only in the private disposable lab.
#![cfg(feature = "backend-test")]
use std::path::Path;
use talk_adapters::nextcloud::{NetworkRuntime, NextcloudClient, test_support};
use talk_core::bounded::{Page, Text};
use talk_core::ids::{
    AccountId, AccountScope, ClockEpoch, ConversationId, HostId, OperationId, RuntimeGeneration,
};
use talk_core::lifecycle::Cancellation;
use talk_core::policy::{Decision, Moment};
use talk_core::ports::{NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest};
use talk_core::protocol::conversation::{
    BreakoutAssignment, BreakoutMode, ConversationCommand as C, ConversationResult as CR,
    NotificationLevel, RecipientSource, Setting,
};
use talk_core::protocol::messaging::{
    ComposeOptions, MessagingCommand as M, MessagingFeature, MessagingResult as MR,
};
use talk_core::protocol::{FeatureSupport, MessageId, MessageKind, Role, RoomKind, SendState};
fn scope() -> AccountScope {
    AccountScope {
        account: AccountId::from_local(700),
        runtime: RuntimeGeneration::from_local(1),
    }
}
fn context(r: &NetworkRuntime) -> OperationContext {
    context_for(r, scope())
}
fn context_for(r: &NetworkRuntime, scope: AccountScope) -> OperationContext {
    let now = r.now();
    OperationContext {
        scope,
        cancellation: Cancellation::default(),
        deadline: Moment {
            epoch: now.epoch,
            millis: now.millis + 60000,
        },
    }
}
fn text<const N: usize>(s: &str) -> Text<N> {
    Text::new(s).unwrap()
}
fn canonical_accepted_id(result: MR, operation: OperationId) -> MessageId {
    let id = match result {
        MR::Message(message) => {
            assert!(
                message.own,
                "accepted message must be the current user's own send"
            );
            assert_eq!(message.message.own_operation, Some(operation));
            message.message.id
        }
        MR::Send(receipt) => {
            assert_eq!(receipt.operation, operation);
            let SendState::ServerAccepted(id) = receipt.state else {
                panic!("send is not confirmed accepted");
            };
            id
        }
        _ => panic!("send returned no correlated canonical acceptance"),
    };
    assert!(id.sequence > 0);
    id
}
async fn read_marker(
    client: &mut NextcloudClient,
    runtime: &NetworkRuntime,
    id: ConversationId,
) -> Option<u64> {
    let ProtocolReply::Conversations(list) = client
        .request(context(runtime), ProtocolRequest::ListConversations)
        .await
        .expect("conversation list")
    else {
        panic!("conversation list")
    };
    list.rooms
        .as_slice()
        .iter()
        .find(|room| room.id == id)
        .expect("test room")
        .last_read_message
}
async fn m(
    c: &mut NextcloudClient,
    r: &NetworkRuntime,
    id: ConversationId,
    n: u128,
    name: &str,
    command: M,
) -> MR {
    match c
        .request(
            context(r),
            ProtocolRequest::Messaging {
                conversation: id,
                operation: OperationId::from_local(n),
                command,
            },
        )
        .await
    {
        Ok(ProtocolReply::Messaging {
            conversation,
            operation,
            result,
        }) => {
            assert_eq!(conversation, id);
            assert_eq!(operation, OperationId::from_local(n));
            println!("alpha-backend {name}: pass");
            result
        }
        Ok(_) => panic!("{name}: wrong reply"),
        Err(e) => panic!("{name}: {:?}", e.kind),
    }
}
async fn settings(
    c: &mut NextcloudClient,
    r: &NetworkRuntime,
    id: ConversationId,
    n: u128,
    name: &str,
    command: C,
) -> CR {
    match c
        .request(
            context_for(
                r,
                AccountScope {
                    account: id.account,
                    runtime: RuntimeGeneration::from_local(1),
                },
            ),
            ProtocolRequest::ConversationSettings {
                conversation: id,
                operation: OperationId::from_local(n),
                command,
            },
        )
        .await
    {
        Ok(ProtocolReply::ConversationSettings { result, .. }) => {
            println!("alpha-backend {name}: pass");
            result
        }
        Ok(_) => panic!("{name}: wrong reply"),
        Err(e) => panic!("{name}: {:?}", e.kind),
    }
}
#[tokio::test]
#[ignore = "requires a private disposable NC34/Talk24 or NC35/Talk25 fixture"]
async fn released_server_messaging_and_room_controls() {
    let directory = std::env::var_os("NMT_FIXTURE_DIR").expect("private fixture directory");
    let root = Path::new(&directory);
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(700));
    let mut client = test_support::from_private_fixture(
        &root.join("alice.json"),
        scope(),
        HostId::from_local(700),
        runtime.clone(),
    )
    .expect("private synthetic fixture");
    client
        .request(context(&runtime), ProtocolRequest::InspectAccount)
        .await
        .unwrap();
    let id = client.fixture_conversation().unwrap();
    let bob_scope = AccountScope {
        account: AccountId::from_local(701),
        runtime: RuntimeGeneration::from_local(1),
    };
    let mut bob = test_support::from_private_fixture(
        &root.join("bob.json"),
        bob_scope,
        HostId::from_local(701),
        runtime.clone(),
    )
    .expect("second private synthetic fixture");
    bob.request(
        context_for(&runtime, bob_scope),
        ProtocolRequest::InspectAccount,
    )
    .await
    .unwrap();
    let bob_room = bob.fixture_conversation().unwrap();
    let ProtocolReply::Messaging {
        result: bob_message,
        ..
    } = bob
        .request(
            context_for(&runtime, bob_scope),
            ProtocolRequest::Messaging {
                conversation: bob_room,
                operation: OperationId::from_local(201),
                command: M::Send {
                    text: text("A group message from Bob"),
                    options: ComposeOptions::default(),
                },
            },
        )
        .await
        .expect("send the second user's source message")
    else {
        panic!("second user's message")
    };
    println!("alpha-backend private-reply-source: pass");
    let source = MessageId {
        conversation: id,
        sequence: canonical_accepted_id(bob_message, OperationId::from_local(201)).sequence,
    };
    let ProtocolReply::Conversation(direct) = client
        .request(
            context(&runtime),
            ProtocolRequest::CreatePrivateReply {
                operation: OperationId::from_local(202),
                source,
            },
        )
        .await
        .expect("resolve the observed author to a direct conversation")
    else {
        panic!("private reply direct conversation")
    };
    assert_eq!(direct.kind, RoomKind::Direct);
    let reply = m(
        &mut client,
        &runtime,
        direct.id,
        203,
        "private-reply-send",
        M::Send {
            text: text("A private response"),
            options: ComposeOptions {
                private_reply: Some(Box::new(source)),
                ..Default::default()
            },
        },
    )
    .await;
    if let MR::Message(message) = reply {
        assert!(
            message.parent.is_some(),
            "private reply keeps the copied parent"
        );
    } else {
        panic!("private reply returned no canonical message");
    }
    let ProtocolReply::OpenConversations(open) = client
        .request(
            context(&runtime),
            ProtocolRequest::ListOpenConversations {
                query: text("Disposable discovery"),
            },
        )
        .await
        .expect("open conversations")
    else {
        panic!("open conversations reply")
    };
    let listed = open
        .rooms
        .as_slice()
        .iter()
        .find(|room| room.name.as_str() == "Disposable discovery fixture")
        .expect("listed room");
    assert_eq!(
        listed.description.as_ref().map(Text::as_str),
        Some("Synthetic open conversation")
    );
    assert_eq!(listed.password_required, Some(false));
    let listed_id = listed.id;
    let ProtocolReply::Conversation(joined) = client
        .request(
            context(&runtime),
            ProtocolRequest::JoinOpenConversation(listed_id),
        )
        .await
        .expect("join listed room")
    else {
        panic!("join reply")
    };
    assert_eq!(joined.id, listed_id);
    let ProtocolReply::Conversations(rooms) = client
        .request(context(&runtime), ProtocolRequest::ListConversations)
        .await
        .expect("joined conversations")
    else {
        panic!("rooms")
    };
    assert!(
        rooms
            .rooms
            .as_slice()
            .iter()
            .any(|room| room.id == listed_id)
    );
    println!("alpha-backend listable room search and join: pass");
    let MR::Capabilities(message_caps) = m(
        &mut client,
        &runtime,
        id,
        999,
        "provider-capabilities",
        M::Capabilities,
    )
    .await
    else {
        panic!("messaging capabilities")
    };
    for feature in [MessagingFeature::Translation, MessagingFeature::Summary] {
        let availability = message_caps
            .actions
            .as_slice()
            .iter()
            .find(|(key, _)| *key == feature)
            .unwrap()
            .1;
        assert_ne!(
            availability.server,
            FeatureSupport::Supported,
            "the lab has no translation or summary provider"
        );
    }
    assert!(message_caps.summary_offer.is_none());
    println!("alpha-backend provider-absent-gates: pass");
    let ProtocolReply::FederationInvitations(invitations) = client
        .request(context(&runtime), ProtocolRequest::FederationInvitations)
        .await
        .unwrap()
    else {
        panic!("federation invitations")
    };
    assert!(invitations.items.as_slice().is_empty());
    println!(
        "alpha-backend federation-empty: pass (supported={})",
        invitations.supported
    );
    let CR::Details(details) =
        settings(&mut client, &runtime, id, 1, "settings-read", C::Read).await
    else {
        panic!("details")
    };
    assert_eq!(details.conversation.role, Role::Owner);
    if details.breakout_supported
        && details
            .controls
            .as_slice()
            .iter()
            .any(|(setting, availability)| {
                *setting == Setting::Breakouts
                    && availability.server == FeatureSupport::Supported
                    && availability.allowed == talk_core::policy::Decision::Allowed
            })
    {
        let CR::Members(members) = settings(
            &mut client,
            &runtime,
            id,
            1001,
            "breakout-members",
            C::Members,
        )
        .await
        else {
            panic!("breakout members")
        };
        let participant = members
            .as_slice()
            .iter()
            .find(|member| !member.is_self && member.role == Role::Member)
            .expect("synthetic member");
        let mut assignments = talk_core::bounded::Page::empty();
        assignments
            .try_push(BreakoutAssignment {
                member: participant.id,
                room: 0,
            })
            .unwrap();
        let configured = settings(
            &mut client,
            &runtime,
            id,
            1002,
            "breakout-configure",
            C::ConfigureBreakouts {
                amount: 2,
                mode: BreakoutMode::Manual,
                assignments: assignments.clone(),
            },
        )
        .await;
        assert!(matches!(configured, CR::BreakoutRooms(_)));
        settings(
            &mut client,
            &runtime,
            id,
            1003,
            "breakout-assign",
            C::AssignBreakouts(assignments),
        )
        .await;
        settings(
            &mut client,
            &runtime,
            id,
            1004,
            "breakout-start",
            C::StartBreakouts,
        )
        .await;
        // Reuse the participant's existing runtime registration. A second
        // client for the same live scope correctly fails with Conflict.
        bob.request(
            context_for(&runtime, bob_scope),
            ProtocolRequest::InspectAccount,
        )
        .await
        .unwrap();
        let bob_parent = bob.fixture_conversation().unwrap();
        let CR::BreakoutRooms(assigned) = settings(
            &mut bob,
            &runtime,
            bob_parent,
            2001,
            "breakout-assigned-room",
            C::BreakoutRooms,
        )
        .await
        else {
            panic!("assigned room")
        };
        assert_eq!(assigned.as_slice().len(), 1);
        let child = &assigned.as_slice()[0];
        assert_eq!(
            child.actions.read_messages,
            talk_core::policy::Decision::Allowed
        );
        settings(
            &mut bob,
            &runtime,
            child.id,
            2002,
            "breakout-request-help",
            C::RequestBreakoutAssistance,
        )
        .await;
        settings(
            &mut client,
            &runtime,
            id,
            1005,
            "breakout-broadcast",
            C::BroadcastBreakouts(text("Synthetic breakout notice")),
        )
        .await;
        settings(
            &mut client,
            &runtime,
            id,
            1006,
            "breakout-stop",
            C::StopBreakouts,
        )
        .await;
        settings(
            &mut client,
            &runtime,
            id,
            1007,
            "breakout-remove",
            C::RemoveBreakouts,
        )
        .await;
        settings(
            &mut client,
            &runtime,
            id,
            1008,
            "breakout-configure-free",
            C::ConfigureBreakouts {
                amount: 2,
                mode: BreakoutMode::Free,
                assignments: talk_core::bounded::Page::empty(),
            },
        )
        .await;
        settings(
            &mut client,
            &runtime,
            id,
            1009,
            "breakout-start-free",
            C::StartBreakouts,
        )
        .await;
        let CR::BreakoutRooms(choices) = settings(
            &mut bob,
            &runtime,
            bob_parent,
            2003,
            "breakout-free-choices",
            C::BreakoutRooms,
        )
        .await
        else {
            panic!("free choices")
        };
        assert_eq!(choices.as_slice().len(), 2);
        let chosen = choices.as_slice()[0].id;
        assert!(matches!(
            settings(
                &mut bob,
                &runtime,
                bob_parent,
                2004,
                "breakout-switch",
                C::SwitchBreakout(chosen)
            )
            .await,
            CR::BreakoutSwitched(_)
        ));
        settings(
            &mut client,
            &runtime,
            id,
            1010,
            "breakout-stop-free",
            C::StopBreakouts,
        )
        .await;
        settings(
            &mut client,
            &runtime,
            id,
            1011,
            "breakout-remove-free",
            C::RemoveBreakouts,
        )
        .await;
    } else {
        println!("alpha-backend breakouts: server unsupported (correctly gated)");
    }
    settings(
        &mut client,
        &runtime,
        id,
        2,
        "rename",
        C::SetName(text("Alpha backend room")),
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        3,
        "description",
        C::SetDescription(text("Unicode Grüße — backend")),
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        400,
        "mute-messages",
        C::SetNotifications(NotificationLevel::Never),
    )
    .await;
    let ProtocolReply::Conversations(rooms) = client
        .request(context(&runtime), ProtocolRequest::ListConversations)
        .await
        .unwrap()
    else {
        panic!("conversation list")
    };
    let listed = rooms
        .rooms
        .as_slice()
        .iter()
        .find(|room| room.id == id)
        .expect("fixture room is listed");
    assert_eq!(listed.notification_muted, Some(true));
    assert!(
        listed.unread_mention.is_some(),
        "mention flag must be observed on the pinned server"
    );
    settings(
        &mut client,
        &runtime,
        id,
        4,
        "notification-level",
        C::SetNotifications(NotificationLevel::Mention),
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        5,
        "call-notification",
        C::SetCallNotifications(false),
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        6,
        "important",
        C::SetImportant(true),
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        7,
        "sensitive",
        C::SetSensitive(false),
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        8,
        "read-only",
        C::SetReadOnly(true),
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        9,
        "read-write",
        C::SetReadOnly(false),
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        10,
        "emoji-avatar",
        C::SetEmojiAvatar {
            emoji: text("🦀"),
            color: Some(text("#336699")),
        },
    )
    .await;
    let avatar_file = tempfile::tempdir().unwrap();
    let avatar_path = avatar_file.path().join("synthetic-wide.png");
    image::RgbaImage::from_pixel(8, 4, image::Rgba([80, 140, 210, 255]))
        .save(&avatar_path)
        .unwrap();
    let uploaded = settings(
        &mut client,
        &runtime,
        id,
        1000,
        "image-avatar",
        C::SetImageAvatar {
            local_path: text(avatar_path.to_str().unwrap()),
        },
    )
    .await;
    assert!(matches!(uploaded, CR::Details(_)));
    settings(
        &mut client,
        &runtime,
        id,
        11,
        "avatar-remove",
        C::RemoveAvatar,
    )
    .await;
    if details
        .controls
        .as_slice()
        .iter()
        .any(|(setting, availability)| {
            *setting == Setting::Preserve && availability.server == FeatureSupport::Supported
        })
    {
        settings(
            &mut client,
            &runtime,
            id,
            12,
            "preserve",
            C::SetPreserved(true),
        )
        .await;
        settings(
            &mut client,
            &runtime,
            id,
            13,
            "unpreserve",
            C::SetPreserved(false),
        )
        .await;
    } else {
        let error = client
            .request(
                context(&runtime),
                ProtocolRequest::ConversationSettings {
                    conversation: id,
                    operation: OperationId::from_local(12),
                    command: C::SetPreserved(true),
                },
            )
            .await
            .err()
            .unwrap();
        assert_eq!(error.kind, talk_core::ports::ErrorKind::Unsupported);
        println!("alpha-backend preserve: server unsupported (correctly gated)");
    }
    let lobby_timer = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 7200;
    settings(
        &mut client,
        &runtime,
        id,
        14,
        "lobby-enable",
        C::SetLobby {
            enabled: true,
            timer: Some(lobby_timer),
        },
    )
    .await;
    let CR::Details(details) =
        settings(&mut client, &runtime, id, 140, "lobby-timer-read", C::Read).await
    else {
        panic!("lobby details")
    };
    assert_eq!(details.lobby_timer, Some(lobby_timer));
    settings(
        &mut client,
        &runtime,
        id,
        15,
        "lobby-disable",
        C::SetLobby {
            enabled: false,
            timer: None,
        },
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        16,
        "permissions",
        C::SetDefaultPermissions(511),
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        17,
        "public-password",
        C::SetGuestAccess {
            enabled: true,
            password: text("Synthetic alpha password 2026!"),
        },
    )
    .await;
    let link = settings(&mut client, &runtime, id, 18, "guest-link", C::GuestLink).await;
    assert!(matches!(link, CR::GuestLink(_)));
    settings(
        &mut client,
        &runtime,
        id,
        19,
        "private",
        C::SetGuestAccess {
            enabled: false,
            password: text(""),
        },
    )
    .await;
    let sent = m(
        &mut client,
        &runtime,
        id,
        101,
        "send",
        M::Send {
            text: text("Alpha message Grüße"),
            options: ComposeOptions::default(),
        },
    )
    .await;
    let first = canonical_accepted_id(sent, OperationId::from_local(101));
    assert_eq!(first.conversation, id);
    let sent = m(
        &mut client,
        &runtime,
        id,
        102,
        "reply",
        M::Send {
            text: text("Alpha reply"),
            options: ComposeOptions {
                reply_to: Some(first),
                ..ComposeOptions::default()
            },
        },
    )
    .await;
    let second = canonical_accepted_id(sent, OperationId::from_local(102));
    assert_eq!(second.conversation, id);
    let marker_before = read_marker(&mut client, &runtime, id).await;
    let MR::Messages {
        messages: recent, ..
    } = m(
        &mut client,
        &runtime,
        id,
        139,
        "unread-latest-control",
        M::History {
            before: None,
            thread: None,
        },
    )
    .await
    else {
        panic!("history control")
    };
    assert!(
        recent
            .as_slice()
            .iter()
            .any(|message| message.message.id == first),
        "history control omitted first fixture message"
    );
    let MR::Messages { messages, .. } = m(
        &mut client,
        &runtime,
        id,
        135,
        "unread-from-start",
        M::UnreadAfter(None),
    )
    .await
    else {
        panic!("unread page")
    };
    assert!(
        messages
            .as_slice()
            .iter()
            .any(|message| message.message.id.sequence <= first.sequence),
        "first unread page: count={}, min={:?}, max={:?}, fixture={}",
        messages.as_slice().len(),
        messages
            .as_slice()
            .iter()
            .map(|row| row.message.id.sequence)
            .min(),
        messages
            .as_slice()
            .iter()
            .map(|row| row.message.id.sequence)
            .max(),
        first.sequence,
    );
    let MR::Messages { messages, .. } = m(
        &mut client,
        &runtime,
        id,
        136,
        "unread-after-marker",
        M::UnreadAfter(Some(first)),
    )
    .await
    else {
        panic!("unread page")
    };
    assert!(
        messages
            .as_slice()
            .iter()
            .any(|message| message.message.id == second)
    );
    assert_eq!(read_marker(&mut client, &runtime, id).await, marker_before);
    m(
        &mut client,
        &runtime,
        id,
        103,
        "paged-history",
        M::History {
            before: Some(second),
            thread: None,
        },
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        104,
        "future-poll",
        M::Poll {
            after: first,
            thread: None,
        },
    )
    .await;
    let MR::Message(edited) = m(
        &mut client,
        &runtime,
        id,
        105,
        "edit",
        M::Edit {
            message: first,
            text: text("Alpha edited Grüße"),
        },
    )
    .await
    else {
        panic!("canonical edit")
    };
    assert_eq!(edited.message.id, first);
    assert_eq!(edited.message.text.as_str(), "Alpha edited Grüße");
    let MR::Messages { messages, .. } = m(
        &mut client,
        &runtime,
        id,
        305,
        "observe-edit",
        M::Poll {
            after: second,
            thread: None,
        },
    )
    .await
    else {
        panic!("future edit")
    };
    assert!(
        messages
            .as_slice()
            .iter()
            .any(|m| m.message.id == first && m.message.text.as_str() == "Alpha edited Grüße")
    );
    m(
        &mut client,
        &runtime,
        id,
        106,
        "reaction-add",
        M::React {
            message: first,
            emoji: text("👍"),
            add: true,
        },
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        107,
        "reaction-authors",
        M::ReactionAuthors {
            message: first,
            emoji: text("👍"),
        },
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        108,
        "reaction-remove",
        M::React {
            message: first,
            emoji: text("👍"),
            add: false,
        },
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        109,
        "mentions",
        M::Mentions(text("lab")),
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        110,
        "search",
        M::Search {
            query: text("Alpha edited"),
            cursor: None,
        },
    )
    .await;
    let query = talk_core::protocol::search::Query {
        term: text("Alpha edited"),
        conversation: Some(id),
        person: None,
        from_self: false,
        since: Some(text("2020-01-01T00:00:00Z")),
        until: Some(text("2099-12-31T23:59:59Z")),
        cursor: None,
    };
    let ProtocolReply::SearchMessages(found) = client
        .request(context(&runtime), ProtocolRequest::SearchMessages(query))
        .await
        .expect("account search")
    else {
        panic!("account search reply")
    };
    assert!(found.hits.as_slice().iter().any(|hit| hit.message == first));
    for (since, until) in [
        (Some("2090-01-01T00:00:00Z"), None),
        (None, Some("2020-01-01T00:00:00Z")),
    ] {
        let query = talk_core::protocol::search::Query {
            term: text("Alpha edited"),
            conversation: Some(id),
            person: None,
            from_self: false,
            since: since.map(text),
            until: until.map(text),
            cursor: None,
        };
        let ProtocolReply::SearchMessages(outside) = client
            .request(context(&runtime), ProtocolRequest::SearchMessages(query))
            .await
            .expect("date-filtered search")
        else {
            panic!("search")
        };
        assert!(
            !outside
                .hits
                .as_slice()
                .iter()
                .any(|hit| hit.message == first)
        );
    }
    let own_query = talk_core::protocol::search::Query {
        term: text("Alpha edited"),
        conversation: None,
        person: None,
        from_self: true,
        since: None,
        until: None,
        cursor: None,
    };
    let ProtocolReply::SearchMessages(own) = client
        .request(
            context(&runtime),
            ProtocolRequest::SearchMessages(own_query),
        )
        .await
        .expect("self-filtered search")
    else {
        panic!("search")
    };
    assert!(own.hits.as_slice().iter().any(|hit| hit.message == first));
    let ProtocolReply::People(people) = client
        .request(
            context(&runtime),
            ProtocolRequest::FindPeople {
                query: text("lab_bob"),
            },
        )
        .await
        .expect("directory for sender filter")
    else {
        panic!("people")
    };
    let bob_person = people.as_slice().first().expect("synthetic user").id;
    let person_query = talk_core::protocol::search::Query {
        term: text("Alpha edited"),
        conversation: Some(id),
        person: Some(bob_person),
        from_self: false,
        since: None,
        until: None,
        cursor: None,
    };
    let ProtocolReply::SearchMessages(filtered) = client
        .request(
            context(&runtime),
            ProtocolRequest::SearchMessages(person_query),
        )
        .await
        .expect("person-filtered search")
    else {
        panic!("search")
    };
    assert!(
        !filtered
            .hits
            .as_slice()
            .iter()
            .any(|hit| hit.message == first)
    );
    println!("alpha-backend account message search and date/room/person filters: pass");
    m(
        &mut client,
        &runtime,
        id,
        111,
        "pin",
        M::Pin {
            message: first,
            until: 0,
        },
    )
    .await;
    let MR::Messages { messages, .. } = m(&mut client, &runtime, id, 112, "pins", M::Pins).await
    else {
        panic!("pins")
    };
    assert!(messages.as_slice().iter().any(|m| m.message.id == first));
    m(
        &mut client,
        &runtime,
        id,
        113,
        "hide-pin",
        M::HidePin(first),
    )
    .await;
    m(&mut client, &runtime, id, 114, "unpin", M::Unpin(first)).await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    m(
        &mut client,
        &runtime,
        id,
        115,
        "reminder-create",
        M::Reminder {
            message: first,
            timestamp: Some(now + 3600),
        },
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        116,
        "reminder-read",
        M::ReadReminder(first),
    )
    .await;
    let MR::Reminder(Some(edited_reminder)) = m(
        &mut client,
        &runtime,
        id,
        1160,
        "reminder-edit",
        M::Reminder {
            message: first,
            timestamp: Some(now + 5400),
        },
    )
    .await
    else {
        panic!("updated reminder")
    };
    assert_eq!(edited_reminder.timestamp, now + 5400);
    m(
        &mut client,
        &runtime,
        id,
        117,
        "reminders-list",
        M::Reminders,
    )
    .await;
    let reminders = client
        .request(context(&runtime), ProtocolRequest::UpcomingReminders)
        .await
        .unwrap();
    assert!(
        matches!(reminders, ProtocolReply::UpcomingReminders(ref value)
        if value.supported && value.reminders.as_slice().iter().any(|reminder| reminder.message == first))
    );
    println!("alpha-backend account-reminders: pass");
    m(
        &mut client,
        &runtime,
        id,
        118,
        "reminder-delete",
        M::Reminder {
            message: first,
            timestamp: None,
        },
    )
    .await;
    let baseline = client
        .request(context(&runtime), ProtocolRequest::PollMessageNotifications)
        .await
        .expect("notification baseline");
    assert!(
        matches!(baseline, ProtocolReply::MessageNotifications(ref value)
        if value.source == talk_core::protocol::NotificationSourceState::Available)
    );
    m(
        &mut client,
        &runtime,
        id,
        1180,
        "reminder-due",
        M::Reminder {
            message: first,
            timestamp: Some(now.saturating_sub(5)),
        },
    )
    .await;
    // The private lab triggers Talk's registered background job after the
    // client's baseline, so the next observation must deliver a new event.
    std::fs::write(root.join("reminder.request"), first.sequence.to_string()).expect("lab signal");
    tokio::time::timeout(std::time::Duration::from_secs(45), async {
        while !root.join("reminder.done").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("lab reminder job");
    let mut delivered = false;
    for _ in 0..8 {
        let ProtocolReply::MessageNotifications(observed) = client
            .request(context(&runtime), ProtocolRequest::PollMessageNotifications)
            .await
            .expect("notification observation")
        else {
            panic!("notification observation")
        };
        println!(
            "alpha-backend reminder-poll: source={:?} changed={} notices={}",
            observed.source,
            observed.changed,
            observed.notices.as_slice().len()
        );
        delivered = observed.notices.as_slice().iter().any(|notice| {
            notice.reminder && notice.conversation == id && notice.message == first.sequence
        });
        if delivered || !observed.changed {
            break;
        }
    }
    assert!(
        delivered,
        "the new reminder must resolve to its original message"
    );
    println!("alpha-backend reminder-notification-target: pass");
    let MR::Schedule(schedule) = m(
        &mut client,
        &runtime,
        id,
        119,
        "schedule-create",
        M::Schedule {
            text: text("Scheduled alpha"),
            send_at: now + 7200,
            options: ComposeOptions::default(),
        },
    )
    .await
    else {
        panic!("schedule")
    };
    m(
        &mut client,
        &runtime,
        id,
        120,
        "schedule-list",
        M::Scheduled,
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        121,
        "schedule-edit",
        M::EditScheduled {
            id: schedule.id,
            text: text("Edited schedule"),
            send_at: now + 10800,
            silent: false,
        },
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        122,
        "schedule-cancel",
        M::CancelScheduled(schedule.id),
    )
    .await;
    let sent = m(
        &mut client,
        &runtime,
        id,
        123,
        "thread-create",
        M::Send {
            text: text("Thread root"),
            options: ComposeOptions {
                thread_title: Some(text("Alpha thread")),
                ..ComposeOptions::default()
            },
        },
    )
    .await;
    let thread = canonical_accepted_id(sent, OperationId::from_local(123));
    assert_eq!(thread.conversation, id);
    let ProtocolReply::Conversations(_) = client
        .request(context(&runtime), ProtocolRequest::ListConversations)
        .await
        .expect("bind account conversations before reading subscribed threads")
    else {
        panic!("conversation list")
    };
    let ProtocolReply::SubscribedThreads {
        threads: subscribed,
        ..
    } = client
        .request(
            context(&runtime),
            ProtocolRequest::SubscribedThreads { offset: 0 },
        )
        .await
        .expect("subscribed threads")
    else {
        panic!("subscribed thread list")
    };
    assert!(subscribed.as_slice().iter().any(|item| item.root == thread));
    println!("alpha-backend subscribed-threads: pass");
    m(&mut client, &runtime, id, 124, "thread-list", M::Threads).await;
    let MR::Thread(details) = m(
        &mut client,
        &runtime,
        id,
        125,
        "thread-read",
        M::Thread(thread),
    )
    .await
    else {
        panic!("thread detail")
    };
    assert_eq!(details.rename, Decision::Allowed);
    m(
        &mut client,
        &runtime,
        id,
        126,
        "thread-rename",
        M::RenameThread {
            root: thread,
            title: text("Renamed alpha thread"),
        },
    )
    .await;
    let thread_reply = m(
        &mut client,
        &runtime,
        id,
        127,
        "thread-reply",
        M::Send {
            text: text("Thread reply"),
            options: ComposeOptions {
                thread: Some(thread),
                ..ComposeOptions::default()
            },
        },
    )
    .await;
    let thread_reply = canonical_accepted_id(thread_reply, OperationId::from_local(127));
    let nested = m(
        &mut client,
        &runtime,
        id,
        133,
        "thread-quoted-reply",
        M::Send {
            text: text("Quoted thread reply"),
            options: ComposeOptions {
                reply_to: Some(thread_reply),
                thread: Some(thread),
                ..ComposeOptions::default()
            },
        },
    )
    .await;
    let nested = canonical_accepted_id(nested, OperationId::from_local(133));
    let MR::Messages { messages, .. } = m(
        &mut client,
        &runtime,
        id,
        134,
        "thread-quoted-history",
        M::History {
            before: None,
            thread: Some(thread),
        },
    )
    .await
    else {
        panic!("thread history")
    };
    assert!(messages.as_slice().iter().any(|message| {
        message.message.id == nested
            && message.thread == Some(thread)
            && message
                .parent
                .as_ref()
                .is_some_and(|parent| parent.id == thread_reply)
    }));
    let MR::Messages { messages, .. } = m(
        &mut client,
        &runtime,
        id,
        138,
        "thread-older-page",
        M::History {
            before: Some(nested),
            thread: Some(thread),
        },
    )
    .await
    else {
        panic!("older thread page")
    };
    assert!(
        messages
            .as_slice()
            .iter()
            .any(|message| message.message.id == thread_reply)
    );
    assert!(
        !messages
            .as_slice()
            .iter()
            .any(|message| message.message.id == nested)
    );
    let MR::Messages { messages, .. } = m(
        &mut client,
        &runtime,
        id,
        140,
        "thread-live-room-poll",
        M::Poll {
            after: thread,
            thread: None,
        },
    )
    .await
    else {
        panic!("thread live poll")
    };
    assert!(
        messages
            .as_slice()
            .iter()
            .any(|message| message.message.id == nested && message.thread == Some(thread))
    );
    let MR::Threads(threads) = m(
        &mut client,
        &runtime,
        id,
        141,
        "thread-live-count",
        M::Threads,
    )
    .await
    else {
        panic!("thread activity")
    };
    assert!(
        threads
            .as_slice()
            .iter()
            .any(|item| item.root == thread && item.replies >= 2)
    );
    m(
        &mut client,
        &runtime,
        id,
        128,
        "thread-notifications",
        M::ThreadNotifications {
            root: thread,
            level: 2,
        },
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        129,
        "mark-read",
        M::MarkRead(second),
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        132,
        "mark-unread-from",
        M::MarkUnreadFrom(first),
    )
    .await;
    m(
        &mut client,
        &runtime,
        id,
        137,
        "mark-unread-from-start",
        M::MarkUnreadFromStart,
    )
    .await;
    assert_eq!(read_marker(&mut client, &runtime, id).await, Some(0));
    m(&mut client, &runtime, id, 130, "mark-unread", M::MarkUnread).await;
    let MR::Message(deleted) = m(&mut client, &runtime, id, 131, "delete", M::Delete(second)).await
    else {
        panic!("deleted")
    };
    assert_eq!(deleted.message.id, second);
    assert_eq!(deleted.message.kind, MessageKind::Deleted);
    assert!(deleted.message.text.as_str().is_empty());
    let bob_thread = MessageId {
        conversation: bob_room,
        sequence: thread.sequence,
    };
    let ProtocolReply::Messaging {
        result: MR::Thread(other),
        ..
    } = bob
        .request(
            context_for(&runtime, bob_scope),
            ProtocolRequest::Messaging {
                conversation: bob_room,
                operation: OperationId::from_local(211),
                command: M::Thread(bob_thread),
            },
        )
        .await
        .expect("member observes another person's thread")
    else {
        panic!("member thread detail")
    };
    assert_eq!(other.rename, Decision::Denied);
    let ProtocolReply::Messaging { result: own, .. } = bob
        .request(
            context_for(&runtime, bob_scope),
            ProtocolRequest::Messaging {
                conversation: bob_room,
                operation: OperationId::from_local(212),
                command: M::Send {
                    text: text("A thread started by Bob"),
                    options: ComposeOptions {
                        thread_title: Some(text("Bob's topic")),
                        ..ComposeOptions::default()
                    },
                },
            },
        )
        .await
        .expect("member starts a thread")
    else {
        panic!("member thread creation")
    };
    let own = canonical_accepted_id(own, OperationId::from_local(212));
    let ProtocolReply::Messaging {
        result: MR::Thread(own_detail),
        ..
    } = bob
        .request(
            context_for(&runtime, bob_scope),
            ProtocolRequest::Messaging {
                conversation: bob_room,
                operation: OperationId::from_local(213),
                command: M::Thread(own),
            },
        )
        .await
        .expect("member observes their thread")
    else {
        panic!("own thread detail")
    };
    assert_eq!(own_detail.rename, Decision::Allowed);
    let MR::Thread(moderator_detail) = m(
        &mut client,
        &runtime,
        id,
        142,
        "thread-moderator-permission",
        M::Thread(MessageId {
            conversation: id,
            sequence: own.sequence,
        }),
    )
    .await
    else {
        panic!("moderator thread detail")
    };
    assert_eq!(moderator_detail.rename, Decision::Allowed);
    println!("alpha-backend thread-rename-authority: pass");
    let CR::Directory(recipients) = settings(
        &mut client,
        &runtime,
        id,
        201,
        "directory",
        C::Directory {
            search: text("lab_outsider"),
            source: RecipientSource::User,
        },
    )
    .await
    else {
        panic!("directory")
    };
    let recipient = recipients
        .as_slice()
        .first()
        .expect("synthetic outsider directory")
        .id;
    settings(
        &mut client,
        &runtime,
        id,
        202,
        "invite",
        C::Invite(recipient),
    )
    .await;
    let CR::Members(members) =
        settings(&mut client, &runtime, id, 203, "members", C::Members).await
    else {
        panic!("members")
    };
    let member = members
        .as_slice()
        .iter()
        .find(|m| m.name.as_str().to_lowercase().contains("outsider"))
        .expect("invited outsider")
        .id;
    settings(
        &mut client,
        &runtime,
        id,
        204,
        "promote",
        C::SetRole {
            member,
            role: Role::Moderator,
        },
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        205,
        "demote",
        C::SetRole {
            member,
            role: Role::Member,
        },
    )
    .await;
    settings(
        &mut client,
        &runtime,
        id,
        206,
        "remove-member",
        C::RemoveMember(member),
    )
    .await;
    settings(&mut client, &runtime, id, 207, "bots", C::Bots).await;
    settings(&mut client, &runtime, id, 208, "bans", C::Bans).await;
    let local = root.join("upload.txt");
    let content = vec![b'A'; 1024 * 1024];
    std::fs::write(&local, &content).unwrap();
    let MR::Uploaded { path, bytes } = m(
        &mut client,
        &runtime,
        id,
        301,
        "file-upload",
        M::UploadFile {
            local_path: text(local.to_str().unwrap()),
            name: text("alpha-file.txt"),
            progress: talk_core::protocol::content::TransferProgress::default(),
        },
    )
    .await
    else {
        panic!("upload")
    };
    assert_eq!(bytes, 1024 * 1024);
    m(
        &mut client,
        &runtime,
        id,
        302,
        "file-share",
        M::ShareFile {
            path,
            caption: text(""),
            options: Default::default(),
            voice_message: false,
        },
    )
    .await;
    let MR::Messages { messages, .. } = m(
        &mut client,
        &runtime,
        id,
        303,
        "shared-files",
        M::SharedFiles { before: None },
    )
    .await
    else {
        panic!("shared files")
    };
    let shared = messages
        .as_slice()
        .iter()
        .find(|m| m.rich.as_slice().iter().any(|o| o.kind.as_str() == "file"))
        .expect("shared file message");
    let object = shared
        .rich
        .as_slice()
        .iter()
        .find(|o| o.kind.as_str() == "file")
        .unwrap();
    let ProtocolReply::Content {
        result: talk_core::protocol::content::ContentResult::CloudLink(link),
        ..
    } = client
        .request(
            context(&runtime),
            ProtocolRequest::Content {
                operation: OperationId::from_local(305),
                command: talk_core::protocol::content::ContentCommand::OpenCloud {
                    message: shared.message.id,
                    placeholder: object.placeholder.clone(),
                },
            },
        )
        .await
        .expect("server file link")
    else {
        panic!("cloud link")
    };
    let ProtocolReply::Reference(reference) = client
        .request(
            context(&runtime),
            ProtocolRequest::ResolveReference {
                message: shared.message.id,
                url: talk_core::protocol::reference::ReferenceUrl::parse(link.as_str())
                    .expect("safe file link"),
            },
        )
        .await
        .expect("reference resolver")
    else {
        panic!("reference")
    };
    if let Some(reference) = reference {
        assert!(!reference.title.as_str().is_empty());
    }
    println!("alpha-backend server reference resolution: pass");
    let saved = root.join("download.txt");
    m(
        &mut client,
        &runtime,
        id,
        304,
        "file-download",
        M::DownloadFile {
            progress: talk_core::protocol::content::TransferProgress::default(),
            message: shared.message.id,
            placeholder: object.placeholder.clone(),
            destination: text(saved.to_str().unwrap()),
        },
    )
    .await;
    assert_eq!(std::fs::read(saved).unwrap(), content);
    let overwrite = client
        .request(
            context(&runtime),
            ProtocolRequest::Messaging {
                conversation: id,
                operation: OperationId::from_local(306),
                command: M::DownloadFile {
                    progress: talk_core::protocol::content::TransferProgress::default(),
                    message: shared.message.id,
                    placeholder: object.placeholder.clone(),
                    destination: text(root.join("download.txt").to_str().unwrap()),
                },
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(overwrite.kind, talk_core::ports::ErrorKind::Conflict);
    assert_eq!(std::fs::read(root.join("download.txt")).unwrap(), content);
    println!("alpha-backend download-no-overwrite: pass");
    let ProtocolReply::Conversation(destination) = client
        .request(
            context(&runtime),
            ProtocolRequest::CreateGroup {
                name: text("Alpha forward destination"),
            },
        )
        .await
        .unwrap()
    else {
        panic!("destination")
    };
    m(
        &mut client,
        &runtime,
        id,
        307,
        "explicit-forward",
        M::Forward {
            source: first,
            destination: destination.id,
        },
    )
    .await;
    settings(
        &mut client,
        &runtime,
        destination.id,
        308,
        "delete-destination",
        C::Delete,
    )
    .await;
    let ProtocolReply::CreationOptions(options) = client
        .request(context(&runtime), ProtocolRequest::InspectCreation)
        .await
        .expect("creation presets")
    else {
        panic!("creation options")
    };
    assert_eq!(options.extended, FeatureSupport::Supported);
    assert!(options.private_allowed);
    let preset = options
        .presets
        .as_slice()
        .iter()
        .find(|p| p.id.as_str() == "presentation")
        .expect("presentation preset")
        .id
        .clone();
    let ProtocolReply::People(found) = client
        .request(
            context(&runtime),
            ProtocolRequest::FindPeople {
                query: text("lab_bob"),
            },
        )
        .await
        .expect("creation directory")
    else {
        panic!("directory")
    };
    let bob = found
        .as_slice()
        .iter()
        .find(|p| p.name.as_str().contains("bob"))
        .expect("fixture member")
        .id;
    client
        .request(context(&runtime), ProtocolRequest::StageCreationPerson(bob))
        .await
        .expect("stage member");
    let mut participants = Page::empty();
    participants.try_push(bob).unwrap();
    let ProtocolReply::CreatedConversation {
        room: created,
        incomplete_invitations: false,
    } = client
        .request(
            context(&runtime),
            ProtocolRequest::CreateConversation(talk_core::protocol::creation::Room {
                name: text("Disposable preset fixture"),
                description: text("Synthetic creation description"),
                public: false,
                preset: Some(preset),
                participants,
            }),
        )
        .await
        .expect("preset conversation")
    else {
        panic!("created conversation")
    };
    let details = settings(
        &mut client,
        &runtime,
        created.id,
        311,
        "created-room-details",
        C::Read,
    )
    .await;
    let CR::Details(details) = details else {
        panic!("created room details")
    };
    assert_eq!(
        details.description.as_ref().map(Text::as_str),
        Some("Synthetic creation description")
    );
    let ProtocolReply::Participants(people) = client
        .request(context(&runtime), ProtocolRequest::Participants(created.id))
        .await
        .expect("created members")
    else {
        panic!("created participants")
    };
    assert!(
        people
            .people
            .as_slice()
            .iter()
            .any(|p| p.display_name.as_str().contains("bob"))
    );
    settings(
        &mut client,
        &runtime,
        created.id,
        312,
        "delete-created-preset-room",
        C::Delete,
    )
    .await;
    assert_eq!(options.public, FeatureSupport::Supported);
    assert!(options.public_allowed);
    let ProtocolReply::CreatedConversation {
        room: public,
        incomplete_invitations: false,
    } = client
        .request(
            context(&runtime),
            ProtocolRequest::CreateConversation(talk_core::protocol::creation::Room {
                name: text("Disposable public fixture"),
                description: text("Public fixture description"),
                public: true,
                preset: None,
                participants: Page::empty(),
            }),
        )
        .await
        .expect("public conversation")
    else {
        panic!("public room")
    };
    let CR::Details(details) = settings(
        &mut client,
        &runtime,
        public.id,
        313,
        "created-public-details",
        C::Read,
    )
    .await
    else {
        panic!("public details")
    };
    assert_eq!(
        details.listable,
        Some(talk_core::protocol::conversation::Discoverability::Users)
    );
    settings(
        &mut client,
        &runtime,
        public.id,
        314,
        "delete-created-public-room",
        C::Delete,
    )
    .await;
    println!("alpha-backend creation presets, inline invite and public room: pass");
    settings(
        &mut client,
        &runtime,
        id,
        309,
        "clear-history",
        C::ClearHistory,
    )
    .await;
    settings(&mut client, &runtime, id, 310, "delete-room", C::Delete).await;
    println!("alpha-backend full scoped fixture complete");
}
