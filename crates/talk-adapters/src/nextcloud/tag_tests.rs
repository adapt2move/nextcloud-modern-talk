use super::*;
use talk_core::protocol::{ConversationTagId, TagKind};

fn tag_caps(major: u16) -> Value {
    let mut value = caps(major);
    value["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!("conversation-tags"));
    value
}
fn tag(id: &str, name: &str, order: u32, collapsed: bool, kind: &str) -> Value {
    json!({"id":id,"name":name,"sortOrder":order,"collapsed":collapsed,"type":kind})
}
fn catalog(custom: Option<Value>) -> Value {
    let mut values = vec![
        tag("1", "Favorites", 0, false, "favorites"),
        tag("2", "Other", 2, false, "other"),
    ];
    if let Some(custom) = custom {
        values.push(custom);
    }
    json!(values)
}
fn one<T: Clone, const N: usize>(value: T) -> Page<T, N> {
    let mut page = Page::empty();
    page.try_push(value).unwrap();
    page
}

#[tokio::test]
async fn tags_create_rename_collapse_reorder_assign_delete_return_server_truth() {
    for major in [34, 35] {
        let initial = tag("31", "My group", 1, false, "custom");
        let renamed = tag("31", "Work", 1, false, "custom");
        let collapsed = tag("31", "Work", 1, true, "custom");
        let reordered = tag("31", "Work", 0, true, "custom");
        let final_catalog = json!([
            reordered.clone(),
            tag("1", "Favorites", 1, false, "favorites"),
            tag("2", "Other", 2, false, "other")
        ]);
        let mut before_room = room();
        before_room["tagIds"] = json!([]);
        let mut after_room = room();
        after_room["tagIds"] = json!(["31"]);
        let replies = vec![
            // Create, explicit full catalog, rename with readback, collapse.
            Reply::data(tag_caps(major)),
            Reply::envelope(201, "ok", 201, initial.clone()),
            Reply::data(tag_caps(major)),
            Reply::data(catalog(Some(initial.clone()))),
            Reply::data(tag_caps(major)),
            Reply::data(catalog(Some(initial))),
            Reply::data(renamed.clone()),
            Reply::data(catalog(Some(renamed.clone()))),
            Reply::data(tag_caps(major)),
            Reply::data(catalog(Some(renamed))),
            Reply::data(collapsed.clone()),
            Reply::data(catalog(Some(collapsed.clone()))),
            // Reorder, assign a room, delete: each mutation reconciles server data.
            Reply::data(tag_caps(major)),
            Reply::data(catalog(Some(collapsed))),
            Reply::data(final_catalog.clone()),
            Reply::data(final_catalog.clone()),
            Reply::data(tag_caps(major)),
            Reply::data(final_catalog.clone()),
            Reply::data(before_room),
            Reply::data(after_room.clone()),
            Reply::data(after_room),
            Reply::data(tag_caps(major)),
            Reply::data(final_catalog),
            Reply::data(Value::Null),
            Reply::data(catalog(None)),
        ];
        let server = Server::start(replies).await;
        let runtime = runtime();
        let mut owner = client(&server.origin, runtime.clone());
        let ProtocolReply::TagCreated(created) = owner
            .request(
                context(&runtime),
                ProtocolRequest::CreateTag {
                    name: Text::new("My group").unwrap(),
                },
            )
            .await
            .unwrap()
        else {
            panic!("created tag");
        };
        assert_eq!(created.id.scope, scope());
        assert_eq!(created.kind, TagKind::Custom);
        let id = created.id;
        let ProtocolReply::Tags(read) = owner
            .request(context(&runtime), ProtocolRequest::ReadTags)
            .await
            .unwrap()
        else {
            panic!("catalog");
        };
        assert!(read.tags.as_slice().iter().any(|tag| tag.id == id));
        let ProtocolReply::Tags(renamed) = owner
            .request(
                context(&runtime),
                ProtocolRequest::RenameTag {
                    tag: id,
                    name: Text::new("Work").unwrap(),
                },
            )
            .await
            .unwrap()
        else {
            panic!("rename");
        };
        assert_eq!(
            renamed
                .tags
                .as_slice()
                .iter()
                .find(|tag| tag.id == id)
                .unwrap()
                .name
                .as_str(),
            "Work"
        );
        let ProtocolReply::Tags(collapsed) = owner
            .request(
                context(&runtime),
                ProtocolRequest::SetTagCollapsed {
                    tag: id,
                    collapsed: true,
                },
            )
            .await
            .unwrap()
        else {
            panic!("collapse");
        };
        assert!(
            collapsed
                .tags
                .as_slice()
                .iter()
                .find(|tag| tag.id == id)
                .unwrap()
                .collapsed
        );
        let mut order = one(id);
        for item in read.tags.as_slice().iter().filter(|tag| tag.id != id) {
            order.try_push(item.id).unwrap();
        }
        let ProtocolReply::Tags(reordered) = owner
            .request(
                context(&runtime),
                ProtocolRequest::ReorderTags { tags: order },
            )
            .await
            .unwrap()
        else {
            panic!("order");
        };
        assert_eq!(
            reordered
                .tags
                .as_slice()
                .iter()
                .find(|tag| tag.id == id)
                .unwrap()
                .order,
            0
        );
        let conversation = bind(&mut owner);
        let ProtocolReply::Conversation(room) = owner
            .request(
                context(&runtime),
                ProtocolRequest::SetConversationTags {
                    conversation,
                    tags: one(id),
                },
            )
            .await
            .unwrap()
        else {
            panic!("assign");
        };
        assert_eq!(room.tag_ids.unwrap().as_slice(), &[id]);
        let ProtocolReply::Tags(deleted) = owner
            .request(context(&runtime), ProtocolRequest::DeleteTag(id))
            .await
            .unwrap()
        else {
            panic!("delete");
        };
        assert!(!deleted.tags.as_slice().iter().any(|tag| tag.id == id));
        let requests = server.seen.lock().unwrap();
        let mutations: Vec<_> = requests
            .iter()
            .filter(|request| !request.starts_with(b"GET "))
            .collect();
        assert_eq!(mutations.len(), 6);
        let assignment = String::from_utf8_lossy(mutations[4]);
        assert!(assignment.starts_with("POST ") && assignment.contains("/room/testroom/tags"));
        assert!(assignment.contains("tagIds%5B%5D=31"));
    }
}

#[tokio::test]
async fn tags_builtin_and_foreign_runtime_handles_cannot_mutate() {
    let values = catalog(Some(tag("31", "Group", 1, false, "custom")));
    let server = Server::start(vec![
        Reply::data(tag_caps(35)),
        Reply::data(values.clone()),
        Reply::data(tag_caps(35)),
        Reply::data(values.clone()),
        Reply::data(tag_caps(35)),
        Reply::data(values),
    ])
    .await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let ProtocolReply::Tags(read) = owner
        .request(context(&runtime), ProtocolRequest::ReadTags)
        .await
        .unwrap()
    else {
        panic!("catalog");
    };
    let builtin = read
        .tags
        .as_slice()
        .iter()
        .find(|tag| tag.kind == TagKind::Favorites)
        .unwrap()
        .id;
    assert_eq!(
        owner
            .request(context(&runtime), ProtocolRequest::DeleteTag(builtin))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::PermissionDenied
    );
    let foreign = ConversationTagId {
        scope: AccountScope {
            runtime: RuntimeGeneration::from_local(99),
            ..scope()
        },
        ..builtin
    };
    assert_eq!(
        owner
            .request(
                context(&runtime),
                ProtocolRequest::SetTagCollapsed {
                    tag: foreign,
                    collapsed: true
                }
            )
            .await
            .unwrap_err()
            .kind,
        ErrorKind::WrongScope
    );
    assert!(
        server
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.starts_with(b"GET "))
    );
}

#[tokio::test]
async fn tags_uncertain_creation_never_replays_until_explicit_reconciliation() {
    let created = tag("31", "Group", 1, false, "custom");
    let server = Server::start(vec![
        Reply::data(tag_caps(34)),
        Reply::envelope(500, "failure", 500, Value::Null),
        Reply::data(tag_caps(34)),
        Reply::data(tag_caps(34)),
        Reply::data(catalog(Some(created))),
    ])
    .await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let create = ProtocolRequest::CreateTag {
        name: Text::new("Group").unwrap(),
    };
    for _ in 0..2 {
        assert_eq!(
            owner
                .request(context(&runtime), create.clone())
                .await
                .unwrap_err()
                .kind,
            ErrorKind::Uncertain
        );
    }
    let ProtocolReply::Tags(read) = owner
        .request(context(&runtime), ProtocolRequest::ReadTags)
        .await
        .unwrap()
    else {
        panic!("reconcile");
    };
    assert!(
        read.tags
            .as_slice()
            .iter()
            .any(|tag| tag.name.as_str() == "Group")
    );
    assert_eq!(
        server
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.starts_with(b"POST "))
            .count(),
        1
    );
}

#[tokio::test]
async fn tags_excessive_catalog_and_unknown_support_do_not_mutate() {
    let server = Server::start(vec![
        Reply::data(tag_caps(35)),
        Reply::data(json!(
            (1..=103)
                .map(|i| tag(&i.to_string(), "Group", i, false, "custom"))
                .collect::<Vec<_>>()
        )),
        Reply::data(caps(35)),
    ])
    .await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    assert_eq!(
        owner
            .request(context(&runtime), ProtocolRequest::ReadTags)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::TooLarge
    );
    assert!(owner.tags.is_empty());
    assert_eq!(
        owner
            .request(
                context(&runtime),
                ProtocolRequest::CreateTag {
                    name: Text::new("Group").unwrap()
                }
            )
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    assert!(
        server
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.starts_with(b"GET "))
    );
}
