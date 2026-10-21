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
        .request(
            context(&r),
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
            },
        )
        .await
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
        let result = attachment_result(&server, true).await.unwrap();
        assert!(
            matches!(result, ProtocolReply::Content { result: R::Image(image), .. }
            if image.as_slice() == b"raster" && image.document_pages().is_none())
        );
        let seen = server.seen.lock().unwrap();
        let request = String::from_utf8_lossy(&seen[3]);
        assert!(request.starts_with("GET /cloud/index.php/core/preview?fileId=42&x=1600&y=1200&a=1&mode=fill&forceIcon=0&mimeFallback=0 "));
        assert!(!request.contains("unrelated.invalid"));
        assert!(!request.contains("format=json"));
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
