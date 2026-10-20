//! Default-off lab executable. Real browser-granted credentials enter only private identity.
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
use talk_adapters::identity::{
    AccountConnection, ConnectionCursor, ConnectionRequestId, ConnectionState, IdentityError,
    RuntimeCommand, RuntimeConfig, RuntimeReply, StoredState,
};
use talk_core::{
    bounded::Text,
    identity::StorageChoice,
    ids::{AccountId, AccountScope, LoginAttemptId},
    policy::Decision,
    ports::{ErrorKind, ProtocolReply, ProtocolRequest},
    preferences::{
        RequestId,
        account::{AccountPreferenceChange, NextcloudFolder},
    },
    protocol::{RoomKind, SendMessage, SendState},
};
struct Connection {
    owner: AccountConnection,
    cursor: ConnectionCursor,
    request: u64,
}
impl Connection {
    fn start(config: RuntimeConfig) -> Result<Self, IdentityError> {
        let mut owner = AccountConnection::start(config)?;
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            match owner.poll(ConnectionCursor::default())?.snapshot.state {
                ConnectionState::Ready => {
                    return Ok(Self {
                        owner,
                        cursor: ConnectionCursor::default(),
                        request: 0,
                    });
                }
                ConnectionState::Failed(error) => return Err(error),
                _ if Instant::now() >= until => return Err(ErrorKind::Deadline.into()),
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}
fn wait(runtime: &mut Connection, command: RuntimeCommand) -> Result<RuntimeReply, IdentityError> {
    runtime.owner.poll(runtime.cursor)?;
    runtime.request += 1;
    let request = ConnectionRequestId(runtime.request);
    let scoped = matches!(command, RuntimeCommand::Request { .. });
    runtime.owner.try_submit(request, command)?;
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let update = runtime.owner.poll(runtime.cursor)?;
        if let Some(done) = update
            .completions
            .as_slice()
            .iter()
            .find(|done| done.request == request)
        {
            runtime.cursor = update.cursor;
            if scoped && done.result.is_ok() && !done.scope_is_current {
                return Err(ErrorKind::Cancelled.into());
            }
            return done.result.clone();
        }
        if Instant::now() >= until {
            return Err(IdentityError::Request(ErrorKind::Deadline));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn protocol(
    runtime: &mut Connection,
    scope: AccountScope,
    request: ProtocolRequest,
) -> Result<ProtocolReply, IdentityError> {
    match wait(runtime, RuntimeCommand::Request { scope, request })? {
        RuntimeReply::Protocol(reply) => Ok(reply),
        _ => Err(ErrorKind::Malformed.into()),
    }
}
fn account_path(runtime: &mut Connection, scope: AccountScope) -> Result<Value, IdentityError> {
    let ProtocolReply::Conversations(list) =
        protocol(runtime, scope, ProtocolRequest::ListConversations)?
    else {
        return Err(ErrorKind::Malformed.into());
    };
    let keys = list
        .rooms
        .as_slice()
        .iter()
        .filter(|room| room.navigation_key.is_some())
        .count();
    let room = list
        .rooms
        .as_slice()
        .iter()
        .find(|room| room.kind == RoomKind::Group && room.actions.send_message == Decision::Allowed)
        .ok_or(ErrorKind::NotFound)?
        .clone();
    let favorite = room.favorite.ok_or(ErrorKind::Unconfigured)?;
    for value in [!favorite, favorite] {
        let ProtocolReply::Conversation(result) = protocol(
            runtime,
            scope,
            ProtocolRequest::SetFavorite {
                conversation: room.id,
                favorite: value,
            },
        )?
        else {
            return Err(ErrorKind::Malformed.into());
        };
        if result.favorite != Some(value) {
            return Err(ErrorKind::Uncertain.into());
        }
    }
    let archived = room.archived.ok_or(ErrorKind::Unconfigured)?;
    for value in [!archived, archived] {
        let ProtocolReply::Conversation(result) = protocol(
            runtime,
            scope,
            ProtocolRequest::SetArchived {
                conversation: room.id,
                archived: value,
            },
        )?
        else {
            return Err(ErrorKind::Malformed.into());
        };
        if result.archived != Some(value) {
            return Err(ErrorKind::Uncertain.into());
        }
    }
    let ProtocolReply::Preferences(before) = protocol(
        runtime,
        scope,
        ProtocolRequest::ReadPreferences {
            request: RequestId(1),
        },
    )?
    else {
        return Err(ErrorKind::Malformed.into());
    };
    let read = before
        .values
        .send_read_receipts
        .ok_or(ErrorKind::Unconfigured)?;
    let typing = before
        .values
        .share_typing_status
        .ok_or(ErrorKind::Unconfigured)?;
    let folder = before
        .values
        .attachment_folder
        .ok_or(ErrorKind::Unconfigured)?;
    for (index, change) in [
        AccountPreferenceChange::SendReadReceipts(!read),
        AccountPreferenceChange::ShareTypingStatus(!typing),
        AccountPreferenceChange::AttachmentFolder(
            NextcloudFolder::new("/Facade account uploads").map_err(|_| ErrorKind::Malformed)?,
        ),
        AccountPreferenceChange::SendReadReceipts(read),
        AccountPreferenceChange::ShareTypingStatus(typing),
        AccountPreferenceChange::AttachmentFolder(folder),
    ]
    .into_iter()
    .enumerate()
    {
        if !matches!(
            protocol(
                runtime,
                scope,
                ProtocolRequest::SetPreference {
                    request: RequestId(index as u64 + 2),
                    change
                }
            )?,
            ProtocolReply::Preferences(_)
        ) {
            return Err(ErrorKind::Malformed.into());
        }
    }
    let operation = runtime.owner.new_operation_id(scope)?;
    let ProtocolReply::Send(receipt) = protocol(
        runtime,
        scope,
        ProtocolRequest::Send(SendMessage {
            conversation: room.id,
            operation,
            text: Text::new("Account facade integration").unwrap(),
        }),
    )?
    else {
        return Err(ErrorKind::Malformed.into());
    };
    if !matches!(receipt.state, SendState::ServerAccepted(_)) {
        return Err(ErrorKind::Uncertain.into());
    }
    if !matches!(
        protocol(
            runtime,
            scope,
            ProtocolRequest::History {
                conversation: room.id,
                before: None
            }
        )?,
        ProtocolReply::History(_)
    ) {
        return Err(ErrorKind::Malformed.into());
    }
    let ProtocolReply::TagCreated(tag) = protocol(
        runtime,
        scope,
        ProtocolRequest::CreateTag {
            name: Text::new("Facade group").unwrap(),
        },
    )?
    else {
        return Err(ErrorKind::Malformed.into());
    };
    protocol(
        runtime,
        scope,
        ProtocolRequest::RenameTag {
            tag: tag.id,
            name: Text::new("Facade renamed").unwrap(),
        },
    )?;
    protocol(
        runtime,
        scope,
        ProtocolRequest::SetTagCollapsed {
            tag: tag.id,
            collapsed: true,
        },
    )?;
    let ProtocolReply::Tags(catalog) = protocol(runtime, scope, ProtocolRequest::ReadTags)? else {
        return Err(ErrorKind::Malformed.into());
    };
    let mut order = talk_core::bounded::Page::empty();
    for item in catalog.tags.as_slice().iter().rev() {
        order.try_push(item.id).map_err(|_| ErrorKind::TooLarge)?;
    }
    protocol(runtime, scope, ProtocolRequest::ReorderTags { tags: order })?;
    let mut tags = talk_core::bounded::Page::empty();
    tags.try_push(tag.id).unwrap();
    protocol(
        runtime,
        scope,
        ProtocolRequest::SetConversationTags {
            conversation: room.id,
            tags,
        },
    )?;
    protocol(
        runtime,
        scope,
        ProtocolRequest::SetConversationTags {
            conversation: room.id,
            tags: talk_core::bounded::Page::empty(),
        },
    )?;
    protocol(runtime, scope, ProtocolRequest::DeleteTag(tag.id))?;
    Ok(
        json!({"phase":"features","rooms":list.rooms.as_slice().len(),"keys":keys,"preferences":true,"favorite":true,"archive":true,"tags":true,"send":true}),
    )
}
fn main() {
    let code = match run() {
        Ok(()) => 0,
        Err(_) => {
            println!("{{\"fatal\":\"identity-lab\"}}");
            1
        }
    };
    std::process::exit(code);
}
fn run() -> Result<(), ()> {
    let path = PathBuf::from(std::env::args_os().nth(1).ok_or(())?);
    if !path.is_absolute() {
        return Err(());
    }
    let config: Value =
        serde_json::from_slice(&std::fs::read(&path).map_err(|_| ())?).map_err(|_| ())?;
    let origin = Text::new(config["origin"].as_str().ok_or(())?).map_err(|_| ())?;
    let directory = PathBuf::from(config["directory"].as_str().ok_or(())?);
    let mut runtime =
        Connection::start(RuntimeConfig::isolated_fixture(directory)).map_err(|_| ())?;
    let mut account: Option<AccountId> = runtime
        .owner
        .snapshot()
        .accounts
        .as_slice()
        .first()
        .map(|a| a.summary.account);
    let mut attempt: Option<LoginAttemptId> = None;
    let mut scope: Option<AccountScope> = None;
    println!("{{\"ready\":true}}");
    io::stdout().flush().map_err(|_| ())?;
    let input = io::stdin();
    for line in input.lock().lines().take(256) {
        let line = line.map_err(|_| ())?;
        if line.len() > 32 {
            return Err(());
        }
        if line == "quit" {
            break;
        }
        if line == "features" {
            let result = account_path(&mut runtime, scope.ok_or(())?);
            let response = match result {
                Ok(value) => value,
                Err(error) => json!({"error":format!("{error:?}")}),
            };
            println!("{response}");
            io::stdout().flush().map_err(|_| ())?;
            continue;
        }
        let command = match line.as_str() {
            "begin" => RuntimeCommand::Begin {
                server: origin.clone(),
                replace: account,
            },
            "poll" => RuntimeCommand::Poll {
                account: account.ok_or(())?,
                attempt: attempt.ok_or(())?,
            },
            "save-file" | "save-native" | "session" => RuntimeCommand::Save {
                account: account.ok_or(())?,
                attempt: attempt.ok_or(())?,
                choice: match line.as_str() {
                    "save-file" => StorageChoice::PrivateFile,
                    "save-native" => StorageChoice::Native,
                    _ => StorageChoice::SessionOnly,
                },
            },
            "begin-save" => RuntimeCommand::BeginSave {
                account: account.ok_or(())?,
            },
            "retire-all" => RuntimeCommand::RetireAll {
                account: account.ok_or(())?,
            },
            "forget" => RuntimeCommand::ForgetAccount {
                account: account.ok_or(())?,
            },
            "restore" => RuntimeCommand::Restore {
                account: account.ok_or(())?,
            },
            "probe" => RuntimeCommand::Request {
                scope: scope.ok_or(())?,
                request: ProtocolRequest::ProbeAccount,
            },
            "maintain" => RuntimeCommand::Maintain {
                account: account.ok_or(())?,
            },
            "logout" => {
                runtime.owner.logout(account.ok_or(())?).map_err(|_| ())?;
                scope = None;
                RuntimeCommand::Accounts
            }
            "state" => RuntimeCommand::Accounts,
            _ => return Err(()),
        };
        let result = wait(&mut runtime, command);
        let response = match result {
            Ok(RuntimeReply::Begun {
                account: a,
                attempt: t,
            }) => {
                account = Some(a);
                attempt = Some(t);
                json!({"phase":"begun"})
            }
            Ok(RuntimeReply::Pending { .. }) => json!({"phase":"pending"}),
            Ok(RuntimeReply::BrowserOpened { .. }) => json!({"phase":"browser-opened"}),
            Ok(RuntimeReply::Authorized { attempt: t, .. }) => {
                attempt = Some(t);
                json!({"phase":"authorized"})
            }
            Ok(RuntimeReply::Active { summary, probe }) => {
                scope = summary.scope;
                json!({"phase":"active","storage":match summary.storage{StoredState::Eligible(i)=>if i.provider.backend==talk_core::identity::StorageBackend::PrivateFile{"file"}else{"native"},StoredState::SessionOnly=>"session",_=>"unexpected"},"family":format!("{:?}",probe.family)})
            }
            Ok(RuntimeReply::Protocol(_)) => json!({"phase":"authenticated"}),
            Ok(RuntimeReply::Accounts(values)) => {
                json!({"phase":"state","accounts":values.as_slice().len(),"suppressed":values.as_slice().iter().all(|a|a.storage==StoredState::Suppressed&&a.scope.is_none()),"suppression_unconfirmed":values.as_slice().iter().any(|a|a.suppression_unconfirmed)})
            }
            Ok(RuntimeReply::StorageStatus { .. }) => json!({"phase":"storage-status"}),
            Ok(RuntimeReply::Forgotten(_)) => {
                account = None;
                scope = None;
                attempt = None;
                json!({"phase":"forgotten"})
            }
            Ok(RuntimeReply::State(_)) => json!({"phase":"maintained"}),
            Err(e) => json!({"error":format!("{e:?}")}),
        };
        println!("{response}");
        io::stdout().flush().map_err(|_| ())?;
    }
    if !runtime.owner.join() {
        return Err(());
    }
    Ok(())
}
