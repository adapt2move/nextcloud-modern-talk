//! Labelled, disconnected raster and poll projection for native visual checks.
use super::*;
use talk_core::ui::content::*;
use talk_core::ui::conversation::Availability;
use talk_core::ui::messaging::RichObject;
use talk_core::ui::presentation::FeatureSupport;

pub(super) fn configure(snapshot: &mut PresentationSnapshot) {
    let context = snapshot.selected.unwrap();
    let ready = Availability {
        implemented: true,
        server: FeatureSupport::Supported,
        allowed: Decision::Allowed,
    };
    snapshot.conversations = page(snapshot.conversations.as_slice().iter().cloned().map(
        |mut room| {
            if room.context == context {
                room.favorite = Some(true);
            }
            room
        },
    ));
    let mut photo = message(
        101,
        "Mara Fischer",
        "MF",
        "09:41",
        "Die neue Meetingecke — ein fiktiver Entwurf für unsere Abstimmung.",
        false,
    );
    photo.metadata.rich = page([RichObject {
        id: Some(text("101")),
        mime_type: Some(text("image/png")),
        size: Some(190313),
        placeholder: text("photo"),
        kind: text("file"),
        name: text("Meetingecke.png"),
        link: None,
    }]);
    let mut document = message(
        102,
        "Mara Fischer",
        "MF",
        "09:43",
        "Das Raumkonzept zum gemeinsamen Durchsehen.",
        false,
    );
    document.metadata.rich = page([RichObject {
        id: Some(text("102")),
        mime_type: Some(text("application/pdf")),
        size: Some(284000),
        placeholder: text("document"),
        kind: text("file"),
        name: text("Raumkonzept.pdf"),
        link: None,
    }]);
    let mut poll = message(
        103,
        "Alex Sommer",
        "AS",
        "09:44",
        "Fiktive Umfrage für die Oberflächenprüfung",
        true,
    );
    poll.metadata.rich = page([RichObject {
        id: Some(text("9001")),
        mime_type: None,
        size: None,
        placeholder: text("poll"),
        kind: text("talk-poll"),
        name: text("Welcher Entwurf gefällt euch?"),
        link: None,
    }]);
    snapshot.messages = page([photo.clone(), document.clone(), poll]);
    if let Some(panel) = &mut snapshot.message_panel {
        panel.shared_files = page([photo, document]);
    }
    let mut assets = vec![
        AssetView {
            target: AssetTarget::SelfAvatar,
            state: AssetState::Ready { key: 1 },
            emoji: None,
            color: None,
            document_pages: None,
        },
        AssetView {
            target: AssetTarget::ConversationAvatar(context),
            state: AssetState::Ready { key: 0 },
            emoji: Some(text("🎨")),
            color: Some(text("#55736A")),
            document_pages: None,
        },
    ];
    for (id, placeholder, key) in [(101, "photo", 2), (102, "document", 3)] {
        let attachment = AttachmentRef {
            context,
            message: id,
            placeholder: text(placeholder),
        };
        for target in [
            AssetTarget::Thumbnail(attachment.clone()),
            AssetTarget::Image(attachment),
        ] {
            assets.push(AssetView {
                target,
                state: AssetState::Ready { key },
                emoji: None,
                color: None,
                document_pages: None,
            });
        }
    }
    snapshot.content_panel = ContentPanel {
        account: Some(context.account),
        context: Some(context),
        self_name: Some(text("Alex Sommer")),
        organization: Some(text("Nord Studio · Beispieldaten")),
        assets: page(assets),
        poll_create: ready,
        polls: page([PollView {
            id: 9001,
            question: text("Fiktive Umfrage: Welcher Entwurf gefällt euch?"),
            options: page([
                PollOption {
                    id: 0,
                    label: text("Die helle Meetingecke"),
                    votes: Some(4),
                    own: true,
                },
                PollOption {
                    id: 1,
                    label: text("Das ruhige Studio"),
                    votes: Some(2),
                    own: false,
                },
            ]),
            closed: false,
            hidden_results: false,
            max_votes: 1,
            voters: Some(6),
            details: Page::empty(),
            details_complete: false,
            vote: ready,
            close: ready,
        }]),
        ..Default::default()
    };
    if let Some(panel) = snapshot.message_panel.as_mut() {
        panel.media_caption = true;
    }
}

pub(super) fn attachments(snapshot: &mut PresentationSnapshot) {
    use talk_core::ui::messaging::{UploadState, UploadView};
    let mut file = snapshot.messages.as_slice()[1].clone();
    file.local_id = 104;
    file.body = text("");
    file.metadata.rich = page([
        RichObject {
            placeholder: text("data"),
            kind: text("file"),
            name: text("Projekt-Konfiguration.json"),
            mime_type: Some(text("application/json")),
            size: Some(2048),
            id: Some(text("104")),
            link: None,
        },
        RichObject {
            placeholder: text("archive"),
            kind: text("file"),
            name: text("Projektunterlagen.zip"),
            mime_type: Some(text("application/zip")),
            size: Some(3180000),
            id: Some(text("105")),
            link: None,
        },
    ]);
    snapshot.messages = page(
        snapshot
            .messages
            .as_slice()
            .iter()
            .filter(|message| message.local_id != 103)
            .cloned()
            .chain([file]),
    );
    if let Some(panel) = snapshot.message_panel.as_mut() {
        panel.uploads = page([
            UploadView {
                id: 1,
                name: text("Konfiguration.json"),
                bytes: Some(2048),
                state: UploadState::Selected,
                submitted: false,
                transferred: 0,
                uploaded: false,
                server_message: None,
            },
            UploadView {
                id: 2,
                name: text("Meetingecke.png"),
                bytes: Some(190313),
                state: UploadState::Selected,
                submitted: false,
                transferred: 0,
                uploaded: false,
                server_message: None,
            },
        ]);
    }
}

pub(super) fn transfer(snapshot: &mut PresentationSnapshot) {
    let context = snapshot.selected.unwrap();
    snapshot.content_panel.transfer = Some(DownloadView {
        request: RequestId(9002),
        attachment: AttachmentRef {
            context,
            message: 102,
            placeholder: text("document"),
        },
        name: text("Fiktive Übertragungsansicht.pdf"),
        received: 3 * 1024 * 1024,
        total: Some(8 * 1024 * 1024),
        state: DownloadState::Transferring,
    });
}

pub(super) fn action(
    snapshot: &mut PresentationSnapshot,
    action: ContentAction,
) -> Result<(), SubmitError> {
    match action {
        ContentAction::ListFiles(path) => {
            cloud_files(snapshot, path.as_str());
            Ok(())
        }
        ContentAction::ListFilesPage { path, .. } => {
            cloud_files(snapshot, path.as_str());
            Ok(())
        }
        ContentAction::RecentFiles
        | ContentAction::FavoriteFiles
        | ContentAction::SearchFiles(_) => {
            let section = match action {
                ContentAction::RecentFiles => talk_core::ui::content::FileSection::Recent,
                ContentAction::FavoriteFiles => talk_core::ui::content::FileSection::Favorites,
                _ => talk_core::ui::content::FileSection::Search,
            };
            cloud_files(snapshot, "/");
            if let Some(browser) = &mut snapshot.content_panel.file_browser {
                browser.section = section;
            }
            Ok(())
        }
        ContentAction::CloseFileBrowser => {
            snapshot.content_panel.file_browser = None;
            Ok(())
        }
        ContentAction::SortFiles(sort) => {
            if let Some(browser) = &mut snapshot.content_panel.file_browser {
                browser.sort = sort;
            }
            Ok(())
        }
        ContentAction::StageCloudFile { context, path } => {
            let entry = snapshot
                .content_panel
                .file_browser
                .as_ref()
                .filter(|browser| browser.account == context.account)
                .and_then(|browser| {
                    browser
                        .entries
                        .as_slice()
                        .iter()
                        .find(|item| item.path == path && !item.directory)
                })
                .ok_or(SubmitError::StaleScope)?;
            let size = entry.size;
            let name = entry.name.clone();
            let panel = snapshot
                .message_panel
                .as_mut()
                .filter(|panel| panel.context == context)
                .ok_or(SubmitError::StaleScope)?;
            if panel
                .uploads
                .as_slice()
                .iter()
                .any(|item| item.name == name)
            {
                return Ok(());
            }
            panel
                .uploads
                .try_push(talk_core::ui::messaging::UploadView {
                    id: 8000 + panel.uploads.as_slice().len() as u64,
                    name,
                    bytes: size,
                    state: talk_core::ui::messaging::UploadState::Selected,
                    submitted: false,
                    transferred: 0,
                    uploaded: true,
                    server_message: None,
                })
                .map_err(|_| SubmitError::Busy)?;
            Ok(())
        }
        ContentAction::StageCloudFiles { context, paths } => {
            for path in paths.as_slice() {
                self::action(
                    snapshot,
                    ContentAction::StageCloudFile {
                        context,
                        path: path.clone(),
                    },
                )?;
            }
            Ok(())
        }
        ContentAction::Load(target)
            if snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == target) =>
        {
            Ok(())
        }
        ContentAction::Load(target) => {
            snapshot
                .content_panel
                .assets
                .try_push(AssetView {
                    target,
                    state: AssetState::Unsupported,
                    emoji: None,
                    color: None,
                    document_pages: None,
                })
                .map_err(|_| SubmitError::Busy)?;
            Ok(())
        }
        ContentAction::Preview(attachment)
            if snapshot.selected == Some(attachment.context)
                && snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .any(|a| a.target == AssetTarget::Image(attachment.clone())) =>
        {
            snapshot.content_panel.preview = Some(attachment);
            Ok(())
        }
        ContentAction::ClosePreview => {
            snapshot.content_panel.preview = None;
            Ok(())
        }
        ContentAction::Acknowledge => {
            snapshot.content_panel.result = None;
            Ok(())
        }
        ContentAction::CancelDownload => {
            let transfer = snapshot
                .content_panel
                .transfer
                .as_mut()
                .ok_or(SubmitError::StaleScope)?;
            transfer.state = DownloadState::Cancelled;
            println!("UI_CONTENT_FIXTURE cancelled-projection-only; no file or server operation");
            Ok(())
        }
        _ => Err(SubmitError::Unavailable(UnavailableReason::Unconfigured)),
    }
}

pub(super) fn cloud_files(snapshot: &mut PresentationSnapshot, path: &str) {
    use talk_core::ui::content::{CloudFileView, FileBrowserView};
    let Some(account) = snapshot.active_account else {
        return;
    };
    let names: &[(&str, bool, Option<u64>)] = if path == "/" {
        &[
            ("Entwürfe", true, None),
            ("Team", true, None),
            ("Projektübersicht.pdf", false, Some(1_240_000)),
            ("Workshop.png", false, Some(485_000)),
        ]
    } else {
        &[
            ("Konzept.pdf", false, Some(288_000)),
            ("Notizen.txt", false, Some(4200)),
        ]
    };
    snapshot.content_panel.file_browser = Some(FileBrowserView {
        account,
        path: text(path),
        section: talk_core::ui::content::FileSection::All,
        sort: talk_core::ui::content::FileSort::Name,
        pagination: None,
        loading: false,
        offline: false,
        error: None,
        entries: page(names.iter().map(|(name, directory, size)| CloudFileView {
            name: text(name),
            path: text(&format!(
                "{}{}{}",
                if path == "/" { "" } else { path },
                "/",
                name
            )),
            directory: *directory,
            size: *size,
            modified: Some(text("04 Nov 2026")),
            modified_unix: Some(1793750400),
            mime: None,
            file_id: None,
        })),
    });
}

pub(crate) fn images(scenario: Option<&str>) -> Vec<super::super::accounts::content::ImageJob> {
    if !matches!(
        scenario,
        Some(
            "chat-content"
                | "chat-attachments"
                | "chat-upload"
                | "chat-upload-uncertain"
                | "chat-content-progress"
                | "chat-content-loading"
                | "chat-content-failed"
        )
    ) {
        return Vec::new();
    }
    [
        (
            1,
            include_bytes!("../../../assets/fixtures/content-avatar.png").as_slice(),
        ),
        (
            2,
            include_bytes!("../../../assets/fixtures/content-photo.png").as_slice(),
        ),
        (
            3,
            include_bytes!("../../../assets/fixtures/content-document.png").as_slice(),
        ),
    ]
    .into_iter()
    .filter(|(key, _)| {
        *key == 1
            || !matches!(
                scenario,
                Some("chat-content-loading" | "chat-content-failed")
            )
    })
    .map(|(key, bytes)| super::super::accounts::content::ImageJob {
        cached: false,
        local: None,
        attachment: None,
        preview_generation: None,
        account: scope(),
        target: if key == 1 {
            AssetTarget::SelfAvatar
        } else {
            AssetTarget::Image(AttachmentRef {
                context: context(1),
                message: if key == 2 { 101 } else { 102 },
                placeholder: text(if key == 2 { "photo" } else { "document" }),
            })
        },
        key,
        bytes: talk_core::protocol::content::ImageBytes::new(bytes.to_vec())
            .expect("bounded authored fixture"),
    })
    .collect()
}

pub(super) fn pending_upload(snapshot: &mut PresentationSnapshot, uncertain: bool) {
    use talk_core::ui::messaging::{UploadState, UploadView};
    let context = snapshot.selected.unwrap();
    let file = UploadView {
        id: 1,
        name: text("Meetingecke.png"),
        bytes: Some(190313),
        state: if uncertain {
            UploadState::Uncertain
        } else {
            UploadState::Uploading
        },
        submitted: true,
        transferred: 78000,
        uploaded: uncertain,
        server_message: None,
    };
    let mut row = snapshot.messages.as_slice()[0].clone();
    row.local_id = file.message_id();
    row.author = text("Fictional author");
    row.initials = text("FA");
    row.own = true;
    row.body = text("My favourite");
    row.attachments = Page::empty();
    row.metadata = Default::default();
    row.delivery = talk_core::ui::presentation::DeliveryState::Pending;
    snapshot.messages = page([row]);
    snapshot
        .content_panel
        .assets
        .try_push(AssetView {
            target: AssetTarget::Thumbnail(AttachmentRef {
                context,
                message: file.message_id(),
                placeholder: text("upload"),
            }),
            state: AssetState::Ready { key: 2 },
            emoji: None,
            color: None,
            document_pages: None,
        })
        .unwrap();
    if let Some(panel) = snapshot.message_panel.as_mut() {
        panel.uploads = page([file]);
        panel.uploads_running = !uncertain;
    }
}
