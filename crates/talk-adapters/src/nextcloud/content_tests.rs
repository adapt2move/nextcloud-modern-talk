use super::*;
use talk_core::protocol::content::{ContentCommand as C, ContentResult as R, PollCommand as P};
fn features() -> Value {
    let mut c = caps(35);
    c["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!("talk-polls"));
    c
}
fn poll(author: &str, status: u8) -> Value {
    json!({"id":9,"question":"Which?","options":["A","B"],"votes":{},"votedSelf":[],"status":status,"resultMode":0,"maxVotes":1,"numVoters":0,"actorType":"users","actorId":author})
}
#[tokio::test]
async fn poll_author_closes_but_ordinary_voter_cannot_dispatch_close() {
    for (author, allowed) in [("fixture_user", true), ("another_user", false)] {
        let mut replies = vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(poll(author, 0)),
        ];
        if allowed {
            replies.push(Reply::data(poll(author, 1)));
        }
        let server = Server::start(replies).await;
        let r = runtime();
        let mut c = client(&server.origin, r.clone());
        let id = bind(&mut c);
        let result = c
            .request(
                context(&r),
                ProtocolRequest::Content {
                    operation: OperationId::from_local(800),
                    command: C::Poll {
                        conversation: id,
                        command: P::Close(9),
                    },
                },
            )
            .await;
        if allowed {
            assert!(matches!(result,Ok(ProtocolReply::Content{result:R::Poll(p),..}) if p.closed));
            assert_eq!(server.count(), 4);
        } else {
            assert_eq!(result.err().unwrap().kind, ErrorKind::PermissionDenied);
            assert_eq!(server.count(), 3);
        }
    }
}
#[tokio::test]
async fn closed_poll_converges_without_replaying_delete() {
    let server = Server::start(vec![
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(poll("another", 1)),
    ])
    .await;
    let r = runtime();
    let mut c = client(&server.origin, r.clone());
    let id = bind(&mut c);
    let result = c
        .request(
            context(&r),
            ProtocolRequest::Content {
                operation: OperationId::from_local(800),
                command: C::Poll {
                    conversation: id,
                    command: P::Close(9),
                },
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(result,ProtocolReply::Content{result:R::Poll(p),..} if p.closed&&p.close.allowed==Decision::Denied)
    );
    assert_eq!(server.count(), 3);
}
#[tokio::test]
async fn unavailable_poll_capability_and_direct_create_cannot_dispatch_mutation() {
    for direct in [false, true] {
        let mut data = room();
        if direct {
            data["type"] = json!(1);
        }
        let server = Server::start(vec![
            Reply::data(if direct { features() } else { caps(35) }),
            Reply::data(data),
        ])
        .await;
        let r = runtime();
        let mut c = client(&server.origin, r.clone());
        let id = bind(&mut c);
        let result = c
            .request(
                context(&r),
                ProtocolRequest::Content {
                    operation: OperationId::from_local(800),
                    command: C::Poll {
                        conversation: id,
                        command: P::Capabilities,
                    },
                },
            )
            .await
            .unwrap();
        let ProtocolReply::Content {
            result: R::PollCapabilities(a),
            ..
        } = result
        else {
            panic!("capabilities")
        };
        assert!(
            a.server != talk_core::protocol::FeatureSupport::Supported
                || a.allowed != Decision::Allowed
        );
        assert_eq!(server.count(), 2);
    }
}
#[tokio::test]
async fn classified_preview_cannot_fetch_message_or_binary() {
    let mut data = room();
    data["objectType"] = json!("classified");
    data["attributes"] = json!(4);
    let server = Server::start(vec![Reply::data(features()), Reply::data(data)]).await;
    let r = runtime();
    let mut c = client(&server.origin, r.clone());
    let id = bind(&mut c);
    let result = c
        .request(
            context(&r),
            ProtocolRequest::Content {
                operation: OperationId::from_local(800),
                command: C::Asset(talk_core::protocol::content::AssetRequest::Attachment {
                    message: MessageId {
                        conversation: id,
                        sequence: 1,
                    },
                    placeholder: Text::new("file").unwrap(),
                    large: false,
                }),
            },
        )
        .await;
    assert_eq!(result.err().unwrap().kind, ErrorKind::PermissionDenied);
    assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn shared_documents_and_images_merge_with_one_bounded_pagination_cursor() {
    let mut caps = features();
    caps["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .extend([
            json!("rich-object-sharing"),
            json!("rich-object-list-media"),
        ]);
    let list = |range: std::ops::Range<u64>| {
        let mut rows = serde_json::Map::new();
        for id in range {
            let mut row = message();
            row["id"] = json!(id);
            rows.insert(id.to_string(), row);
        }
        Value::Object(rows)
    };
    let server = Server::start(vec![
        Reply::data(caps),
        Reply::data(room()),
        Reply::data(list(1..41)),
        Reply::data(list(41..81)),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let id = bind(&mut client);
    let result = client
        .request(
            context(&runtime),
            ProtocolRequest::Messaging {
                conversation: id,
                operation: OperationId::from_local(801),
                command: talk_core::protocol::messaging::MessagingCommand::SharedFiles {
                    before: None,
                },
            },
        )
        .await
        .unwrap();
    let ProtocolReply::Messaging {
        result:
            talk_core::protocol::messaging::MessagingResult::Messages {
                messages, cursor, ..
            },
        ..
    } = result
    else {
        panic!("shared file page")
    };
    assert_eq!(messages.as_slice().len(), 50);
    assert_eq!(messages.as_slice().first().unwrap().message.id.sequence, 31);
    assert_eq!(messages.as_slice().last().unwrap().message.id.sequence, 80);
    assert_eq!(cursor.unwrap().sequence, 31);
    let seen = server.seen.lock().unwrap();
    assert!(String::from_utf8_lossy(&seen[2]).contains("objectType=file"));
    assert!(String::from_utf8_lossy(&seen[3]).contains("objectType=media"));
}

fn attachment_message(mime: &str) -> Value {
    let mut value = message();
    value["messageType"] = json!("system");
    value["systemMessage"] = json!("file_shared");
    value["message"] = json!("{file}");
    value["messageParameters"] = json!({"file": {
        "type": "file", "id": "42", "name": "Prüfung #1.png",
        "mimetype": mime, "size": 100,
        "path": "Talk/Prüfung #1.png", "link": "https://unrelated.invalid/leak"
    }});
    value
}
fn binary(status: u16, mime: &str, bytes: &[u8]) -> Reply {
    let mut value = Reply::data(Value::Null);
    value.status = status;
    value.headers = format!("Content-Type: {mime}\r\n");
    value.body = bytes.to_vec();
    value
}
async fn attachment_result(server: &Server, large: bool) -> Result<ProtocolReply, PortError> {
    let r = runtime();
    let mut client = client(&server.origin, r.clone());
    let id = bind(&mut client);
    client
        .request(context(&r), attachment_request(id, large))
        .await
}
fn attachment_request(id: ConversationId, large: bool) -> ProtocolRequest {
    ProtocolRequest::Content {
        operation: OperationId::from_local(901),
        command: C::Asset(talk_core::protocol::content::AssetRequest::Attachment {
            message: MessageId {
                conversation: id,
                sequence: 7,
            },
            placeholder: Text::new("file").unwrap(),
            large,
        }),
    }
}

#[tokio::test]
async fn image_and_pdf_server_previews_use_verified_file_id_and_preserve_installation_base() {
    for mime in ["image/png", "application/pdf"] {
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([attachment_message(mime)])),
            binary(200, "image/png", b"raster"),
        ])
        .await;
        let result = attachment_result(&server, false).await.unwrap();
        assert!(
            matches!(result, ProtocolReply::Content { result: R::Image(image), .. }
            if image.as_slice() == b"raster" && image.document_pages().is_none())
        );
        let seen = server.seen.lock().unwrap();
        let request = String::from_utf8_lossy(&seen[3]);
        assert!(request.starts_with("GET /cloud/index.php/core/preview?fileId=42&x=400&y=300&a=1&mode=fill&forceIcon=0&mimeFallback=0 "));
        assert!(!request.contains("unrelated.invalid"));
        assert!(!request.contains("format=json"));
    }
}

#[tokio::test]
async fn enlarged_raster_reads_original_bytes_without_requesting_a_server_thumbnail() {
    for mime in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
        let original = b"exact original encoded bytes, not a resized preview";
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([attachment_message(mime)])),
            binary(200, mime, original),
        ])
        .await;
        let result = attachment_result(&server, true).await.unwrap();
        assert!(
            matches!(result, ProtocolReply::Content { result: R::Image(image), .. }
            if image.as_slice() == original && image.document_pages().is_none())
        );
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        let request = String::from_utf8_lossy(&seen[3]);
        assert!(request.starts_with(
            "GET /cloud/remote.php/dav/files/fixture_user/Talk/Pr%C3%BCfung%20%231.png "
        ));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: basic ")
        );
        assert!(!request.contains("core/preview"));
        assert!(!request.contains("unrelated.invalid"));
    }
}

#[tokio::test]
async fn enlarged_original_errors_never_fall_back_to_a_thumbnail() {
    for (status, mime, expected) in [
        (403, "image/png", ErrorKind::PermissionDenied),
        (302, "image/png", ErrorKind::RedirectRejected),
        (404, "image/png", ErrorKind::NotFound),
        (413, "image/png", ErrorKind::TooLarge),
        (200, "text/html", ErrorKind::Unsupported),
    ] {
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([attachment_message("image/png")])),
            binary(status, mime, b"not an original raster"),
        ])
        .await;
        assert_eq!(
            attachment_result(&server, true).await.unwrap_err().kind,
            expected
        );
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        assert!(String::from_utf8_lossy(&seen[3]).contains("/remote.php/dav/files/"));
    }
}

#[tokio::test]
async fn enlarged_original_keeps_fresh_room_and_message_access_checks() {
    let mut denied = room();
    denied["lobbyState"] = json!(1);
    denied["permissions"] = json!(0);
    let mut sensitive = room();
    sensitive["isSensitive"] = json!(true);
    let mut classified = room();
    classified["objectType"] = json!("classified");
    classified["attributes"] = json!(4);
    for room in [denied, sensitive, classified] {
        let server = Server::start(vec![Reply::data(features()), Reply::data(room)]).await;
        assert_eq!(
            attachment_result(&server, true).await.unwrap_err().kind,
            ErrorKind::PermissionDenied
        );
        assert_eq!(server.count(), 2, "no message or binary read after denial");
    }
    for field in ["messageType", "expirationTimestamp"] {
        let mut message = attachment_message("image/png");
        message[field] = if field == "messageType" {
            json!("comment_deleted")
        } else {
            json!(1)
        };
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([message])),
        ])
        .await;
        assert_eq!(
            attachment_result(&server, true).await.unwrap_err().kind,
            ErrorKind::NotFound
        );
        assert_eq!(server.count(), 3, "no binary read for a retired message");
    }
}

#[tokio::test]
async fn enlarged_original_rejects_wrong_host_and_account_scope() {
    for wrong_host in [false, true] {
        let server = Server::start(if wrong_host {
            vec![Reply::data(features())]
        } else {
            vec![]
        })
        .await;
        let r = runtime();
        let mut client = client(&server.origin, r.clone());
        let mut id = bind(&mut client);
        let mut context = context(&r);
        if wrong_host {
            id.host = HostId::from_local(9);
        } else {
            context.scope.runtime = RuntimeGeneration::from_local(9);
        }
        assert_eq!(
            client
                .request(context, attachment_request(id, true))
                .await
                .unwrap_err()
                .kind,
            ErrorKind::WrongScope
        );
        assert_eq!(server.count(), usize::from(wrong_host));
    }
}

#[tokio::test]
async fn cancelled_original_download_cannot_publish_image_bytes() {
    for cancel_account in [false, true] {
        let mut body = binary(200, "image/png", b"encoded original image");
        body.delay = Duration::from_secs(2);
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([attachment_message("image/png")])),
            body,
        ])
        .await;
        let r = runtime();
        let mut client = client(&server.origin, r.clone());
        let id = bind(&mut client);
        let context = context(&r);
        let cancel = if cancel_account {
            client.cancellation()
        } else {
            context.cancellation.clone()
        };
        let (result, ()) = tokio::join!(
            client.request(context, attachment_request(id, true)),
            async {
                tokio::time::timeout(Duration::from_secs(1), async {
                    while server.count() < 4 {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("original DAV read was admitted");
                cancel.cancel();
            }
        );
        assert_eq!(result.unwrap_err().kind, ErrorKind::Cancelled);
        assert_eq!(server.count(), 4);
    }
}

#[tokio::test]
async fn enlarged_original_requires_a_bounded_verified_path_before_binary_admission() {
    for (mime, field, value, expected) in [
        (
            "image/png",
            "size",
            json!(nextcloud_talk_api::content::MAX_IMAGE_BYTES + 1),
            ErrorKind::TooLarge,
        ),
        (
            "application/pdf",
            "size",
            json!(nextcloud_talk_api::content::MAX_IMAGE_BYTES + 1),
            ErrorKind::TooLarge,
        ),
        ("image/png", "path", Value::Null, ErrorKind::Unsupported),
        (
            "image/png",
            "path",
            json!("Talk/../private.png"),
            ErrorKind::Malformed,
        ),
    ] {
        let mut message = attachment_message(mime);
        message["messageParameters"]["file"][field] = value;
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([message])),
        ])
        .await;
        assert_eq!(
            attachment_result(&server, true).await.unwrap_err().kind,
            expected
        );
        assert_eq!(server.count(), 3);
    }
}

#[tokio::test]
async fn enlarged_original_enforces_streamed_limit_without_a_content_length() {
    let original = vec![0; nextcloud_talk_api::content::MAX_IMAGE_BYTES + 1];
    let mut body = binary(200, "image/png", &original);
    body.chunked = true;
    let mut message = attachment_message("image/png");
    message["messageParameters"]["file"]["size"] = Value::Null;
    let server = Server::start(vec![
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(json!([message])),
        body,
    ])
    .await;
    assert_eq!(
        attachment_result(&server, true).await.unwrap_err().kind,
        ErrorKind::TooLarge
    );
    assert_eq!(server.count(), 4);
}

#[tokio::test]
async fn enlarged_pdf_uses_original_validation_before_any_native_renderer() {
    let mut replies = vec![
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(json!([attachment_message("application/pdf")])),
    ];
    #[cfg(target_os = "macos")]
    replies.push(binary(200, "application/pdf", b"invalid PDF source"));
    #[cfg(not(target_os = "macos"))]
    let _ = &mut replies;
    let server = Server::start(replies).await;
    let error = attachment_result(&server, true).await.unwrap_err();
    #[cfg(target_os = "macos")]
    {
        assert_eq!(error.kind, ErrorKind::Malformed);
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        assert!(String::from_utf8_lossy(&seen[3]).contains("/remote.php/dav/files/"));
    }
    #[cfg(not(target_os = "macos"))]
    {
        assert_eq!(error.kind, ErrorKind::Unsupported);
        assert_eq!(server.count(), 3);
    }
}

#[tokio::test]
async fn video_attachments_use_only_a_static_server_preview_not_original_raster_input() {
    let server = Server::start(vec![
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(json!([attachment_message("video/mp4")])),
        binary(200, "image/png", b"static video thumbnail"),
    ])
    .await;
    let result = attachment_result(&server, true).await.unwrap();
    assert!(
        matches!(result, ProtocolReply::Content { result: R::Image(image), .. }
        if image.as_slice() == b"static video thumbnail")
    );
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    assert!(String::from_utf8_lossy(&seen[3]).contains("/core/preview?fileId=42&x=1600"));
}

fn video_request(id: ConversationId) -> ProtocolRequest {
    ProtocolRequest::Content {
        operation: OperationId::from_local(902),
        command: C::Video {
            message: MessageId {
                conversation: id,
                sequence: 7,
            },
            placeholder: Text::new("file").unwrap(),
        },
    }
}
async fn video_result(server: &Server) -> Result<ProtocolReply, PortError> {
    let r = runtime();
    let mut client = client(&server.origin, r.clone());
    let id = bind(&mut client);
    client.request(context(&r), video_request(id)).await
}

#[tokio::test]
async fn original_video_reads_exact_bytes_only_from_authenticated_account_dav() {
    for mime in ["video/mp4", "video/webm"] {
        let original = b"\0\0\0\x18ftyp synthetic original video container";
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([attachment_message(mime)])),
            binary(200, mime, original),
        ])
        .await;
        let result = video_result(&server).await.unwrap();
        assert!(
            matches!(result, ProtocolReply::Content { result: R::Video(bytes), .. }
            if bytes.as_slice() == original)
        );
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        let request = String::from_utf8_lossy(&seen[3]);
        assert!(request.starts_with(
            "GET /cloud/remote.php/dav/files/fixture_user/Talk/Pr%C3%BCfung%20%231.png "
        ));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: basic ")
        );
        assert!(!request.contains("core/preview"));
        assert!(!request.contains("unrelated.invalid"));
    }
}

#[tokio::test]
async fn original_video_preflights_size_supported_mime_and_verified_path() {
    for (mime, field, value, expected) in [
        (
            "video/mp4",
            "size",
            json!(nextcloud_talk_api::content::MAX_VIDEO_BYTES + 1),
            ErrorKind::TooLarge,
        ),
        ("video/webm", "path", Value::Null, ErrorKind::Unsupported),
        (
            "video/mp4",
            "path",
            json!("Talk/../private.mp4"),
            ErrorKind::Malformed,
        ),
        (
            "video/quicktime",
            "size",
            json!(100),
            ErrorKind::Unsupported,
        ),
        (
            "application/vnd.apple.mpegurl",
            "size",
            json!(100),
            ErrorKind::Unsupported,
        ),
        ("image/png", "size", json!(100), ErrorKind::Unsupported),
    ] {
        let mut message = attachment_message(mime);
        message["messageParameters"]["file"][field] = value;
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([message])),
        ])
        .await;
        assert_eq!(video_result(&server).await.unwrap_err().kind, expected);
        assert_eq!(server.count(), 3);
    }
}

#[tokio::test]
async fn original_video_errors_mime_mismatches_and_empty_bodies_never_trigger_a_fallback() {
    for (status, mime, bytes, expected) in [
        (
            403,
            "video/mp4",
            b"video".as_slice(),
            ErrorKind::PermissionDenied,
        ),
        (
            302,
            "video/mp4",
            b"video".as_slice(),
            ErrorKind::RedirectRejected,
        ),
        (404, "video/mp4", b"video".as_slice(), ErrorKind::NotFound),
        (413, "video/mp4", b"video".as_slice(), ErrorKind::TooLarge),
        (
            200,
            "text/html",
            b"video".as_slice(),
            ErrorKind::Unsupported,
        ),
        (
            200,
            "video/webm",
            b"video".as_slice(),
            ErrorKind::Unsupported,
        ),
        (200, "video/mp4", b"".as_slice(), ErrorKind::Malformed),
    ] {
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([attachment_message("video/mp4")])),
            binary(status, mime, bytes),
        ])
        .await;
        assert_eq!(video_result(&server).await.unwrap_err().kind, expected);
        assert_eq!(server.count(), 4);
    }
}

#[tokio::test]
async fn original_video_has_a_distinct_32_mib_limit_for_declared_and_streamed_inputs() {
    assert_eq!(
        nextcloud_talk_api::content::MAX_VIDEO_BYTES,
        talk_core::protocol::content::MAX_VIDEO_BYTES
    );
    let source = vec![b'v'; nextcloud_talk_api::content::MAX_IMAGE_BYTES + 1];
    let mut message = attachment_message("video/mp4");
    message["messageParameters"]["file"]["size"] = json!(source.len());
    let server = Server::start(vec![
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(json!([message])),
        binary(200, "video/mp4", &source),
    ])
    .await;
    assert!(
        matches!(video_result(&server).await.unwrap(), ProtocolReply::Content {
        result: R::Video(bytes), ..
    } if bytes.as_slice() == source)
    );
    assert_eq!(
        server.count(),
        4,
        "the image cap does not limit original video"
    );
    drop(server);
    drop(source);
    for chunked in [false, true] {
        let mut body = binary(
            200,
            "video/mp4",
            &vec![0; nextcloud_talk_api::content::MAX_VIDEO_BYTES + 1],
        );
        body.chunked = chunked;
        let mut message = attachment_message("video/mp4");
        message["messageParameters"]["file"]["size"] = Value::Null;
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([message])),
            body,
        ])
        .await;
        assert_eq!(
            video_result(&server).await.unwrap_err().kind,
            ErrorKind::TooLarge
        );
        assert_eq!(server.count(), 4);
    }
}

#[tokio::test]
async fn original_video_keeps_fresh_restriction_deleted_and_expiry_checks() {
    let mut denied = room();
    denied["lobbyState"] = json!(1);
    denied["permissions"] = json!(0);
    let mut sensitive = room();
    sensitive["isSensitive"] = json!(true);
    let mut classified = room();
    classified["objectType"] = json!("classified");
    classified["attributes"] = json!(4);
    for room in [denied, sensitive, classified] {
        let server = Server::start(vec![Reply::data(features()), Reply::data(room)]).await;
        assert_eq!(
            video_result(&server).await.unwrap_err().kind,
            ErrorKind::PermissionDenied
        );
        assert_eq!(server.count(), 2);
    }
    for field in ["messageType", "expirationTimestamp"] {
        let mut message = attachment_message("video/mp4");
        message[field] = if field == "messageType" {
            json!("comment_deleted")
        } else {
            json!(1)
        };
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([message])),
        ])
        .await;
        assert_eq!(
            video_result(&server).await.unwrap_err().kind,
            ErrorKind::NotFound
        );
        assert_eq!(server.count(), 3);
    }
}

#[tokio::test]
async fn original_video_keeps_scope_and_inflight_account_or_request_cancellation() {
    for cancel_account in [false, true] {
        let mut body = binary(200, "video/mp4", b"synthetic original video");
        body.delay = Duration::from_secs(2);
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([attachment_message("video/mp4")])),
            body,
        ])
        .await;
        let r = runtime();
        let mut client = client(&server.origin, r.clone());
        let id = bind(&mut client);
        let context = context(&r);
        let cancel = if cancel_account {
            client.cancellation()
        } else {
            context.cancellation.clone()
        };
        let (result, ()) = tokio::join!(client.request(context, video_request(id)), async {
            tokio::time::timeout(Duration::from_secs(1), async {
                while server.count() < 4 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("original video DAV read admitted");
            cancel.cancel();
        });
        assert_eq!(result.unwrap_err().kind, ErrorKind::Cancelled);
        assert_eq!(server.count(), 4);
    }
    for wrong_host in [false, true] {
        let server = Server::start(if wrong_host {
            vec![Reply::data(features())]
        } else {
            vec![]
        })
        .await;
        let r = runtime();
        let mut client = client(&server.origin, r.clone());
        let mut id = bind(&mut client);
        let mut context = context(&r);
        if wrong_host {
            id.host = HostId::from_local(9);
        } else {
            context.scope.runtime = RuntimeGeneration::from_local(9);
        }
        assert_eq!(
            client
                .request(context, video_request(id))
                .await
                .unwrap_err()
                .kind,
            ErrorKind::WrongScope
        );
        assert_eq!(server.count(), usize::from(wrong_host));
    }
}

#[tokio::test]
async fn absent_raster_preview_falls_back_once_to_bounded_account_dav_content() {
    let server = Server::start(vec![
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(json!([attachment_message("image/png")])),
        binary(404, "application/json", b"[]"),
        binary(200, "image/png", b"native-raster-input"),
    ])
    .await;
    let result = attachment_result(&server, false).await.unwrap();
    assert!(
        matches!(result, ProtocolReply::Content { result: R::Image(image), .. }
        if image.as_slice() == b"native-raster-input")
    );
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 5);
    let dav = String::from_utf8_lossy(&seen[4]);
    assert!(
        dav.starts_with(
            "GET /cloud/remote.php/dav/files/fixture_user/Talk/Pr%C3%BCfung%20%231.png "
        )
    );
    assert!(dav.to_ascii_lowercase().contains("authorization: basic "));
    assert!(
        !seen
            .iter()
            .any(|request| String::from_utf8_lossy(request).contains("unrelated.invalid"))
    );
}

#[tokio::test]
async fn denied_redirected_and_oversized_raster_previews_never_trigger_dav_fallback() {
    for (status, expected) in [
        (403, ErrorKind::PermissionDenied),
        (302, ErrorKind::RedirectRejected),
        (413, ErrorKind::TooLarge),
    ] {
        let server = Server::start(vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(json!([attachment_message("image/png")])),
            binary(status, "application/json", b"[]"),
        ])
        .await;
        assert_eq!(
            attachment_result(&server, false).await.unwrap_err().kind,
            expected
        );
        assert_eq!(server.count(), 4);
    }
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn unavailable_pdf_provider_is_explicit_without_downloading_or_opening_web_content() {
    let server = Server::start(vec![
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(json!([attachment_message("application/pdf")])),
        binary(404, "application/json", b"[]"),
    ])
    .await;
    assert_eq!(
        attachment_result(&server, false).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(server.count(), 4);
}

#[tokio::test]
async fn direct_raster_read_is_bounded_before_content_reaches_native_decoder() {
    let large = vec![0; nextcloud_talk_api::content::MAX_IMAGE_BYTES + 1];
    let server = Server::start(vec![
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(json!([attachment_message("image/png")])),
        binary(404, "application/json", b"[]"),
        binary(200, "image/png", &large),
    ])
    .await;
    assert_eq!(
        attachment_result(&server, false).await.unwrap_err().kind,
        ErrorKind::TooLarge
    );
    assert_eq!(server.count(), 5);
}

#[tokio::test]
async fn cancelled_download_preserves_no_destination_or_staging_file() {
    let mut caps = features();
    caps["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!("rich-object-sharing"));
    let server = Server::start(vec![
        Reply::data(caps),
        Reply::data(room()),
        Reply::data(json!([attachment_message("image/png")])),
        binary(200, "image/png", b"downloaded bytes"),
    ])
    .await;
    let r = runtime();
    let mut client = client(&server.origin, r.clone());
    let id = bind(&mut client);
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("cancelled.png");
    let progress = talk_core::protocol::content::TransferProgress::default();
    progress.cancel();
    let result = client
        .request(
            context(&r),
            ProtocolRequest::Messaging {
                conversation: id,
                operation: OperationId::from_local(903),
                command: talk_core::protocol::messaging::MessagingCommand::DownloadFile {
                    message: MessageId {
                        conversation: id,
                        sequence: 7,
                    },
                    placeholder: Text::new("file").unwrap(),
                    destination: Text::new(destination.to_str().unwrap()).unwrap(),
                    progress: progress.clone(),
                },
            },
        )
        .await;
    assert_eq!(result.unwrap_err().kind, ErrorKind::Cancelled);
    assert!(!progress.is_published());
    assert!(!destination.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn native_pdf_source_requires_authenticated_bounded_pdf_without_redirects() {
    for (status, mime, bytes, expected) in [
        (200, "application/pdf", b"%PDF-1.7\nfixture".to_vec(), None),
        (
            200,
            "text/html",
            b"%PDF-1.7\nfixture".to_vec(),
            Some(ErrorKind::Unsupported),
        ),
        (
            200,
            "application/pdf",
            b"not pdf".to_vec(),
            Some(ErrorKind::Malformed),
        ),
        (
            200,
            "application/pdf",
            vec![0; nextcloud_talk_api::content::MAX_IMAGE_BYTES + 1],
            Some(ErrorKind::TooLarge),
        ),
        (
            302,
            "application/pdf",
            vec![],
            Some(ErrorKind::RedirectRejected),
        ),
    ] {
        let server = Server::start(vec![binary(status, mime, &bytes)]).await;
        let r = runtime();
        let mut client = client(&server.origin, r.clone());
        let result = client
            .transport
            .preview_pdf(
                &context(&r),
                nextcloud_talk_api::files::FilePath::parse("/Talk/Prüfung #1.pdf").unwrap(),
            )
            .await;
        if let Some(error) = expected {
            assert_eq!(result.unwrap_err().kind, error);
        } else {
            assert_eq!(result.unwrap(), bytes);
        }
        let seen = server.seen.lock().unwrap();
        let request = String::from_utf8_lossy(&seen[0]);
        assert!(request.starts_with(
            "GET /cloud/remote.php/dav/files/fixture_user/Talk/Pr%C3%BCfung%20%231.pdf HTTP/1.1\r\n"
        ));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: basic ")
        );
        assert_eq!(seen.len(), 1);
    }
}

#[tokio::test]
async fn avatar_requests_reuse_encoded_bytes_and_cancelled_hits_never_escape() {
    use talk_core::protocol::content::AssetRequest;
    let mut binary = Reply::data(json!({}));
    binary.headers = "Content-Type: image/png\r\n".into();
    binary.body = b"\x89PNG\r\n\x1a\nsynthetic-profile".to_vec();
    let server = Server::start(vec![
        Reply::data(features()),
        binary,
        Reply::data(features()),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    for operation in 1..=2 {
        let result = client
            .request(
                context(&runtime),
                ProtocolRequest::Content {
                    operation: OperationId::from_local(operation),
                    command: C::Asset(AssetRequest::SelfAvatar),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            result,
            ProtocolReply::Content {
                result: R::Image(_),
                ..
            }
        ));
    }
    assert_eq!(
        server.count(),
        3,
        "two fresh capability reads and one image GET"
    );
    client.cancellation().cancel();
    let result = client
        .request(
            context(&runtime),
            ProtocolRequest::Content {
                operation: OperationId::from_local(3),
                command: C::Asset(AssetRequest::SelfAvatar),
            },
        )
        .await;
    assert_eq!(result.err().unwrap().kind, ErrorKind::Cancelled);
    assert_eq!(server.count(), 3);
}

#[tokio::test]
async fn distinct_message_targets_share_one_canonical_avatar_get_but_revalidate_access() {
    use talk_core::protocol::content::AssetRequest;
    let mut binary = Reply::data(json!({}));
    binary.headers = "Content-Type: image/png\r\n".into();
    binary.body = b"\x89PNG\r\n\x1a\nsynthetic-profile".to_vec();
    let mut second = message();
    second["id"] = json!(8);
    let mut denied = room();
    denied["lobbyState"] = json!(1);
    denied["permissions"] = json!(0);
    let server = Server::start(vec![
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(json!([message()])),
        binary,
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(json!([second])),
        Reply::data(features()),
        Reply::data(denied),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let conversation = bind(&mut client);
    for sequence in [7, 8] {
        let reply = client
            .request(
                context(&runtime),
                ProtocolRequest::Content {
                    operation: OperationId::from_local(sequence.into()),
                    command: C::Asset(AssetRequest::MessageAvatar(MessageId {
                        conversation,
                        sequence,
                    })),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            reply,
            ProtocolReply::Content {
                result: R::Image(_),
                ..
            }
        ));
    }
    let error = client
        .request(
            context(&runtime),
            ProtocolRequest::Content {
                operation: OperationId::from_local(9),
                command: C::Asset(AssetRequest::MessageAvatar(MessageId {
                    conversation,
                    sequence: 8,
                })),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::PermissionDenied);
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 9);
    assert_eq!(
        seen.iter()
            .filter(|request| String::from_utf8_lossy(request).contains("/avatar/"))
            .count(),
        1,
        "a different message ID must not download the same actor image again"
    );
    assert!(
        seen.iter().all(|request| request.starts_with(b"GET ")),
        "avatar loading is read-only"
    );
}

#[tokio::test]
async fn only_complete_fresh_room_lists_retire_stored_room_avatars() {
    use super::super::avatars::AvatarStore;
    use std::sync::{Arc, Mutex};
    struct Observed(Arc<Mutex<Vec<Vec<String>>>>);
    impl AvatarStore for Observed {
        fn get(&mut self, _: &str, _: u64) -> Result<Option<(Vec<u8>, u64)>, ()> {
            Ok(None)
        }
        fn put(&mut self, _: &str, _: &[u8], _: u64) -> Result<(), ()> {
            Ok(())
        }
        fn remove(&mut self, _: &str) -> Result<(), ()> {
            Ok(())
        }
        fn retain_rooms(&mut self, allowed: &[String]) -> Result<(), ()> {
            self.0.lock().unwrap().push(allowed.to_vec());
            Ok(())
        }
    }
    let mut classified = room();
    classified["token"] = json!("privateroom");
    classified["id"] = json!(8);
    classified["objectType"] = json!("classified");
    classified["attributes"] = json!(4);
    let server = Server::start(vec![
        Reply::data(features()),
        Reply::data(json!([room(), classified])),
        Reply::data(features()),
        Reply::envelope(503, "failure", 503, json!({})),
        Reply::data(features()),
        Reply::data(json!([])),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let observed = Arc::new(Mutex::new(Vec::new()));
    client.set_avatar_store(Box::new(Observed(observed.clone())));
    client
        .request(context(&runtime), ProtocolRequest::ListConversations)
        .await
        .unwrap();
    assert_eq!(
        *observed.lock().unwrap(),
        vec![vec![super::super::avatars::key("room", "testroom")]]
    );
    let failure = client
        .request(context(&runtime), ProtocolRequest::ListConversations)
        .await
        .unwrap_err();
    assert_eq!(failure.kind, ErrorKind::ServerUnavailable);
    assert_eq!(
        observed.lock().unwrap().len(),
        1,
        "failure has no removal authority"
    );
    client
        .request(context(&runtime), ProtocolRequest::ListConversations)
        .await
        .unwrap();
    assert_eq!(
        observed.lock().unwrap().as_slice().last(),
        Some(&Vec::new())
    );
}
