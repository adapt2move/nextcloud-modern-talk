//! Chat routes from the pinned Talk 24/25 contract. No rendering or permission policy.
//! Future reads use timeout=0: the caller schedules finite polling without monopolizing
//! the serialized account owner. Reading never advances markers or notifications.
use crate::{
    ErrorKind, Kind, Plan, bounded,
    models::{Message, RoomToken, integer, string},
};
use reqwest::Method;
use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct ComposeOptions {
    pub reply_to: Option<u64>,
    pub thread: Option<u64>,
    pub thread_title: Option<String>,
    pub silent: bool,
}
#[derive(Clone, Debug)]
pub enum Command {
    History {
        before: Option<u64>,
        thread: Option<u64>,
    },
    Poll {
        after: u64,
        thread: Option<u64>,
    },
    Context(u64),
    Send {
        message: String,
        reference: String,
        options: ComposeOptions,
    },
    Edit {
        message: u64,
        text: String,
    },
    Delete(u64),
    React {
        message: u64,
        reaction: String,
        add: bool,
    },
    ReactionAuthors {
        message: u64,
        reaction: String,
    },
    MarkUnread,
    Mentions(String),
    Search {
        query: String,
        cursor: Option<String>,
    },
    Pin {
        message: u64,
        until: u64,
    },
    Unpin(u64),
    HidePin(u64),
    Reminder {
        message: u64,
        timestamp: Option<u64>,
    },
    ReadReminder(u64),
    Reminders,
    Scheduled,
    Schedule {
        message: String,
        send_at: u64,
        options: ComposeOptions,
    },
    EditScheduled {
        id: u64,
        message: String,
        send_at: u64,
        silent: bool,
    },
    CancelScheduled(u64),
    Threads,
    Thread(u64),
    RenameThread {
        id: u64,
        title: String,
    },
    ThreadNotifications {
        id: u64,
        level: u8,
    },
    SharedObjects {
        object_type: String,
        before: Option<u64>,
    },
    ShareObject {
        object_type: String,
        object_id: String,
        reference: String,
        thread: Option<u64>,
    },
    AttachmentFolder {
        file_names: Vec<String>,
    },
    ShareAttachment {
        path: String,
        file_name: String,
        reference: String,
        caption: String,
    },
}
#[derive(Clone, Debug)]
pub struct Parent {
    pub id: u64,
    pub token: Option<String>,
    pub author: String,
    pub message: String,
    pub unavailable: bool,
    pub expires: Option<u64>,
}
impl Parent {
    pub(crate) fn optional(value: Option<&Value>) -> Result<Option<Self>, ErrorKind> {
        let Some(v) = value.filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        if !v.is_object() || v.get("parent").is_some() {
            return Err(ErrorKind::Malformed);
        }
        let unavailable = v["deleted"].as_bool() == Some(true)
            || v["messageType"].as_str() == Some("comment_deleted");
        Ok(Some(Self {
            id: number(v, "id")?,
            token: string(v.get("token"), 30)?,
            author: if unavailable {
                String::new()
            } else {
                string(v.get("actorDisplayName"), 256)?.unwrap_or_default()
            },
            message: if unavailable {
                String::new()
            } else {
                string(v.get("message"), 131072)?.unwrap_or_default()
            },
            unavailable,
            expires: integer(v.get("expirationTimestamp")),
        }))
    }
}
#[derive(Clone, Debug)]
pub struct RichObject {
    pub placeholder: String,
    pub kind: String,
    pub id: Option<String>,
    pub name: String,
    pub link: Option<String>,
    pub path: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct Reaction {
    pub emoji: String,
    pub count: u32,
    pub own: bool,
}
pub(crate) fn reactions(v: &Value) -> Result<Vec<Reaction>, ErrorKind> {
    let Some(map) = v.get("reactions").filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    // Nextcloud serializes an empty PHP map as an empty array.
    if map.as_array().is_some_and(Vec::is_empty) {
        return Ok(Vec::new());
    }
    let map = map.as_object().ok_or(ErrorKind::Malformed)?;
    if map.len() > 64 {
        return Err(ErrorKind::TooLarge);
    }
    let own = v.get("reactionsSelf").and_then(Value::as_array);
    if own.is_some_and(|a| a.len() > 64) {
        return Err(ErrorKind::TooLarge);
    }
    map.iter()
        .map(|(emoji, count)| {
            bounded(emoji, 64)?;
            Ok(Reaction {
                emoji: emoji.clone(),
                count: u32::try_from(integer(Some(count)).ok_or(ErrorKind::Malformed)?)
                    .map_err(|_| ErrorKind::Malformed)?,
                own: own.is_some_and(|a| a.iter().any(|v| v.as_str() == Some(emoji))),
            })
        })
        .collect()
}
pub(crate) fn rich(v: Option<&Value>) -> Result<Vec<RichObject>, ErrorKind> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    if v.as_array().is_some_and(Vec::is_empty) {
        return Ok(Vec::new());
    }
    let map = v.as_object().ok_or(ErrorKind::Malformed)?;
    if map.len() > 32 {
        return Err(ErrorKind::TooLarge);
    }
    map.iter()
        .map(|(key, v)| {
            bounded(key, 128)?;
            Ok(RichObject {
                placeholder: key.clone(),
                kind: string(v.get("type"), 128)?.unwrap_or_else(|| "unknown".into()),
                id: string(v.get("id"), 1024)?
                    .or_else(|| integer(v.get("id")).map(|n| n.to_string())),
                name: string(v.get("name"), 1024)?.unwrap_or_else(|| "[unknown object]".into()),
                link: string(v.get("link"), 2048)?,
                path: string(v.get("path"), 16384)?,
                mime_type: string(v.get("mimetype"), 256)?,
                size: integer(v.get("size")),
            })
        })
        .collect()
}
#[derive(Clone, Debug)]
pub struct Mention {
    pub id: String,
    pub label: String,
    pub source: String,
}
#[derive(Clone, Debug)]
pub struct ReactionAuthor {
    pub emoji: String,
    pub name: String,
    pub actor_id: String,
    pub actor_type: String,
}
#[derive(Clone, Debug)]
pub struct Reminder {
    pub message: u64,
    pub token: String,
    pub timestamp: u64,
}
#[derive(Clone, Debug)]
pub struct ScheduledMessage {
    pub id: u64,
    pub message: String,
    pub send_at: u64,
    pub options: ComposeOptions,
}
#[derive(Clone, Debug)]
pub struct Thread {
    pub id: u64,
    pub token: String,
    pub title: String,
    pub replies: u32,
    pub notification_level: Option<u8>,
}
#[derive(Clone, Debug)]
pub struct AttachmentFolder {
    pub folder: String,
    pub renames: Vec<(String, String)>,
}
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub message: u64,
    pub token: String,
    pub title: String,
    pub excerpt: String,
}
#[derive(Clone, Debug)]
pub enum Response {
    Search {
        hits: Vec<SearchHit>,
        cursor: Option<String>,
    },
    Mentions(Vec<Mention>),
    ReactionAuthors(Vec<ReactionAuthor>),
    Reminder(Reminder),
    Reminders(Vec<Reminder>),
    Scheduled(Vec<ScheduledMessage>),
    Schedule(ScheduledMessage),
    Threads(Vec<Thread>),
    Thread(Thread),
    AttachmentFolder(AttachmentFolder),
    AttachmentShared(Vec<(String, String)>),
}
#[derive(Clone, Copy)]
pub(crate) enum ResponseKind {
    Search,
    Mentions,
    ReactionAuthors,
    Reminder,
    Reminders,
    Scheduled,
    Schedule,
    Threads,
    Thread,
    AttachmentFolder,
    AttachmentShared,
    SharedObjects,
}
fn number(v: &Value, key: &str) -> Result<u64, ErrorKind> {
    integer(v.get(key)).ok_or(ErrorKind::Malformed)
}
fn required(v: &Value, key: &str, max: usize) -> Result<String, ErrorKind> {
    string(v.get(key), max)?.ok_or(ErrorKind::Malformed)
}
fn array<T>(
    v: &Value,
    max: usize,
    f: impl Fn(&Value) -> Result<T, ErrorKind>,
) -> Result<Vec<T>, ErrorKind> {
    crate::list(v, max, f)
}
fn reminder(v: &Value) -> Result<Reminder, ErrorKind> {
    let token = required(
        v,
        if v.get("roomToken").is_some() {
            "roomToken"
        } else {
            "token"
        },
        30,
    )?;
    RoomToken::parse(&token)?;
    Ok(Reminder {
        message: number(v, "messageId")?,
        token,
        timestamp: integer(v.get("reminderTimestamp"))
            .or_else(|| integer(v.get("timestamp")))
            .ok_or(ErrorKind::Malformed)?,
    })
}
fn schedule(v: &Value) -> Result<ScheduledMessage, ErrorKind> {
    Ok(ScheduledMessage {
        id: number(v, "id")?,
        message: required(v, "message", 131072)?,
        send_at: number(v, "sendAt")?,
        options: ComposeOptions {
            reply_to: integer(v.get("parent").and_then(|p| p.get("id"))).filter(|n| *n > 0),
            thread: integer(v.get("threadId")).filter(|n| *n > 0),
            thread_title: string(v.get("threadTitle"), 256)?.filter(|s| !s.is_empty()),
            silent: v
                .get("silent")
                .and_then(Value::as_bool)
                .ok_or(ErrorKind::Malformed)?,
        },
    })
}
fn thread(v: &Value) -> Result<Thread, ErrorKind> {
    let t = &v["thread"];
    let token = required(t, "roomToken", 30)?;
    RoomToken::parse(&token)?;
    Ok(Thread {
        id: number(t, "id")?,
        token,
        title: required(t, "title", 256)?,
        replies: u32::try_from(number(t, "numReplies")?).map_err(|_| ErrorKind::TooLarge)?,
        notification_level: integer(v.get("attendee").and_then(|a| a.get("notificationLevel")))
            .and_then(|n| u8::try_from(n).ok()),
    })
}
fn renames(v: &Value) -> Result<Vec<(String, String)>, ErrorKind> {
    let entries = v.as_array().ok_or(ErrorKind::Malformed)?;
    if entries.len() > 100 {
        return Err(ErrorKind::TooLarge);
    }
    let mut result = Vec::new();
    for entry in entries {
        let map = entry.as_object().ok_or(ErrorKind::Malformed)?;
        for (k, v) in map {
            if result.len() >= 100 {
                return Err(ErrorKind::TooLarge);
            }
            bounded(k, 255)?;
            result.push((k.clone(), required(&serde_json::json!({"v":v}), "v", 255)?));
        }
    }
    Ok(result)
}
pub(crate) fn decode(kind: ResponseKind, v: &Value) -> Result<crate::Reply, ErrorKind> {
    use Response as R;
    use ResponseKind as K;
    Ok(crate::Reply::Chat(match kind {
        K::Search => {
            let hits = array(&v["entries"], 50, |e| {
                let token = required(&e["attributes"], "conversation", 30)?;
                RoomToken::parse(&token)?;
                Ok(SearchHit {
                    message: number(&e["attributes"], "messageId")?,
                    token,
                    title: required(e, "title", 256)?,
                    excerpt: required(e, "subline", 4096)?,
                })
            })?;
            let cursor = match v.get("cursor") {
                Some(Value::String(s)) => {
                    bounded(s, 1024)?;
                    Some(s.clone())
                }
                Some(Value::Number(n)) => Some(n.to_string()),
                Some(Value::Null) | None => None,
                _ => return Err(ErrorKind::Malformed),
            };
            R::Search { hits, cursor }
        }
        K::Mentions => R::Mentions(array(v, 100, |v| {
            Ok(Mention {
                id: required(v, "mentionId", 512)?,
                label: required(v, "label", 256)?,
                source: required(v, "source", 64)?,
            })
        })?),
        K::ReactionAuthors => {
            if v.as_array().is_some_and(Vec::is_empty) {
                return Ok(crate::Reply::Chat(R::ReactionAuthors(Vec::new())));
            }
            let map = v.as_object().ok_or(ErrorKind::Malformed)?;
            if map.len() > 64 {
                return Err(ErrorKind::TooLarge);
            }
            let mut all = Vec::new();
            for (emoji, authors) in map {
                bounded(emoji, 64)?;
                for a in array(authors, 100, |a| {
                    Ok(ReactionAuthor {
                        emoji: emoji.clone(),
                        name: required(a, "actorDisplayName", 256)?,
                        actor_id: required(a, "actorId", 256)?,
                        actor_type: required(a, "actorType", 128)?,
                    })
                })? {
                    if all.len() >= 100 {
                        return Err(ErrorKind::TooLarge);
                    }
                    all.push(a)
                }
            }
            R::ReactionAuthors(all)
        }
        K::Reminder => R::Reminder(reminder(v)?),
        K::Reminders => R::Reminders(array(v, 100, reminder)?),
        K::Scheduled => R::Scheduled(array(v, 100, schedule)?),
        K::Schedule => R::Schedule(schedule(v)?),
        K::Threads => R::Threads(array(v, 50, thread)?),
        K::Thread => R::Thread(thread(v)?),
        K::AttachmentFolder => R::AttachmentFolder(AttachmentFolder {
            folder: required(v, "folder", 16384)?,
            renames: renames(&v["renames"])?,
        }),
        K::AttachmentShared => R::AttachmentShared(renames(&v["renames"])?),
        K::SharedObjects => {
            if v.as_array().is_some_and(Vec::is_empty) {
                return Ok(crate::Reply::History {
                    messages: Vec::new(),
                    last_given: None,
                    common_read: None,
                });
            }
            let map = v.as_object().ok_or(ErrorKind::Malformed)?;
            if map.len() > 50 {
                return Err(ErrorKind::TooLarge);
            }
            let messages = map
                .values()
                .map(Message::decode)
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(crate::Reply::History {
                messages,
                last_given: None,
                common_read: None,
            });
        }
    }))
}
fn positive(id: u64) -> Result<(), ErrorKind> {
    if id == 0 {
        Err(ErrorKind::Malformed)
    } else {
        Ok(())
    }
}
fn text(value: &str, max: usize) -> Result<(), ErrorKind> {
    bounded(value, max)?;
    if value.trim().is_empty() {
        Err(ErrorKind::Malformed)
    } else {
        Ok(())
    }
}
fn options(o: &ComposeOptions, f: &mut Vec<(&'static str, String)>) -> Result<(), ErrorKind> {
    if o.reply_to.is_some() && o.thread.is_some()
        || o.thread_title.is_some() && (o.reply_to.is_some() || o.thread.is_some())
    {
        return Err(ErrorKind::Malformed);
    }
    if let Some(id) = o.reply_to {
        positive(id)?;
        f.push(("replyTo", id.to_string()))
    }
    if let Some(id) = o.thread {
        positive(id)?;
        f.push(("threadId", id.to_string()))
    }
    if let Some(t) = &o.thread_title {
        text(t, 256)?;
        f.push(("threadTitle", t.clone()))
    }
    f.push(("silent", o.silent.to_string()));
    Ok(())
}
pub(crate) fn plan(room: &RoomToken, c: &Command) -> Result<Plan, ErrorKind> {
    use Command as C;
    use ResponseKind as K;
    let base = format!("ocs/v2.php/apps/spreed/api/v1/chat/{}", room.as_str());
    let mut method = Method::GET;
    let mut route = base.clone();
    let mut q = Vec::new();
    let mut f = Vec::new();
    let mut kind = Kind::Ack;
    match c {
        C::History { thread, .. } | C::Poll { thread, .. } => {
            let future = matches!(c, C::Poll { .. });
            let cursor = match c {
                C::Poll { after, .. } => Some(*after),
                C::History { before, .. } => *before,
                _ => None,
            };
            q.extend([
                ("lookIntoFuture", u8::from(future).to_string()),
                ("limit", "50".into()),
                ("setReadMarker", "0".into()),
                ("markNotificationsAsRead", "0".into()),
                ("noStatusUpdate", "1".into()),
                ("timeout", "0".into()),
            ]);
            if let Some(id) = cursor {
                positive(id)?;
                q.push(("lastKnownMessageId", id.to_string()))
            }
            if let Some(id) = thread {
                positive(*id)?;
                q.push(("threadId", id.to_string()))
            }
            kind = Kind::History;
        }
        C::Context(id) => {
            positive(*id)?;
            route.push_str(&format!("/{id}/context"));
            q.push(("limit", "25".into()));
            kind = Kind::History
        }
        C::Send {
            message,
            reference,
            options: o,
        } => {
            text(message, 131072)?;
            text(reference, 256)?;
            method = Method::POST;
            f.extend([
                ("message", message.clone()),
                ("referenceId", reference.clone()),
            ]);
            options(o, &mut f)?;
            kind = Kind::Message
        }
        C::Edit {
            message,
            text: value,
        } => {
            positive(*message)?;
            text(value, 131072)?;
            method = Method::PUT;
            route.push_str(&format!("/{message}"));
            f.push(("message", value.clone()));
            kind = Kind::Message
        }
        C::Delete(id) => {
            positive(*id)?;
            method = Method::DELETE;
            route.push_str(&format!("/{id}"));
            kind = Kind::Message
        }
        C::React {
            message,
            reaction,
            add,
        } => {
            positive(*message)?;
            text(reaction, 64)?;
            route = format!(
                "ocs/v2.php/apps/spreed/api/v1/reaction/{}/{message}",
                room.as_str()
            );
            method = if *add { Method::POST } else { Method::DELETE };
            if *add {
                f.push(("reaction", reaction.clone()))
            } else {
                q.push(("reaction", reaction.clone()))
            }
        }
        C::ReactionAuthors { message, reaction } => {
            positive(*message)?;
            bounded(reaction, 64)?;
            route = format!(
                "ocs/v2.php/apps/spreed/api/v1/reaction/{}/{message}",
                room.as_str()
            );
            q.push(("reaction", reaction.clone()));
            kind = Kind::Chat(K::ReactionAuthors)
        }
        C::Search { query, cursor } => {
            text(query, 256)?;
            route = "ocs/v2.php/search/providers/talk-message/search".into();
            q.extend([
                ("term", query.clone()),
                ("limit", "50".into()),
                ("conversation", room.as_str().into()),
            ]);
            if let Some(cursor) = cursor {
                bounded(cursor, 1024)?;
                q.push(("cursor", cursor.clone()))
            }
            kind = Kind::Chat(K::Search)
        }
        C::MarkUnread => {
            method = Method::DELETE;
            route.push_str("/read")
        }
        C::Mentions(search) => {
            bounded(search, 256)?;
            route.push_str("/mentions");
            q.extend([
                ("search", search.clone()),
                ("limit", "100".into()),
                ("includeStatus", "false".into()),
            ]);
            kind = Kind::Chat(K::Mentions)
        }
        C::Pin { message, until } => {
            positive(*message)?;
            method = Method::POST;
            route.push_str(&format!("/{message}/pin"));
            f.push(("pinUntil", until.to_string()));
            kind = Kind::OptionalMessage
        }
        C::Unpin(id) | C::HidePin(id) => {
            positive(*id)?;
            method = Method::DELETE;
            route.push_str(&format!(
                "/{id}/pin{}",
                if matches!(c, C::HidePin(_)) {
                    "/self"
                } else {
                    ""
                }
            ));
            if matches!(c, C::Unpin(_)) {
                kind = Kind::OptionalMessage
            }
        }
        C::Reminder { message, timestamp } => {
            positive(*message)?;
            route.push_str(&format!("/{message}/reminder"));
            method = if timestamp.is_some() {
                Method::POST
            } else {
                Method::DELETE
            };
            if let Some(t) = timestamp {
                f.push(("timestamp", t.to_string()));
            }
        }
        C::ReadReminder(id) => {
            positive(*id)?;
            route.push_str(&format!("/{id}/reminder"));
            kind = Kind::Chat(K::Reminder)
        }
        C::Reminders => {
            route = "ocs/v2.php/apps/spreed/api/v1/chat/upcoming-reminders".into();
            kind = Kind::Chat(K::Reminders)
        }
        C::Scheduled => {
            route.push_str("/schedule");
            kind = Kind::Chat(K::Scheduled)
        }
        C::Schedule {
            message,
            send_at,
            options: o,
        } => {
            text(message, 131072)?;
            positive(*send_at)?;
            method = Method::POST;
            route.push_str("/schedule");
            f.extend([
                ("message", message.clone()),
                ("sendAt", send_at.to_string()),
            ]);
            options(o, &mut f)?;
            kind = Kind::Chat(K::Schedule)
        }
        C::EditScheduled {
            id,
            message,
            send_at,
            silent,
        } => {
            positive(*id)?;
            text(message, 131072)?;
            positive(*send_at)?;
            method = Method::POST;
            route.push_str(&format!("/schedule/{id}"));
            f.extend([
                ("message", message.clone()),
                ("sendAt", send_at.to_string()),
                ("silent", silent.to_string()),
            ]);
            kind = Kind::Chat(K::Schedule)
        }
        C::CancelScheduled(id) => {
            positive(*id)?;
            method = Method::DELETE;
            route.push_str(&format!("/schedule/{id}"));
        }
        C::Threads => {
            route.push_str("/threads/recent");
            q.push(("limit", "50".into()));
            kind = Kind::Chat(K::Threads)
        }
        C::Thread(id) => {
            positive(*id)?;
            route.push_str(&format!("/threads/{id}"));
            kind = Kind::Chat(K::Thread)
        }
        C::RenameThread { id, title } => {
            positive(*id)?;
            text(title, 256)?;
            method = Method::PUT;
            route.push_str(&format!("/threads/{id}"));
            f.push(("threadTitle", title.clone()));
            kind = Kind::Chat(K::Thread)
        }
        C::ThreadNotifications { id, level } => {
            positive(*id)?;
            if *level > 3 {
                return Err(ErrorKind::Malformed);
            }
            method = Method::POST;
            route.push_str(&format!("/threads/{id}/notify"));
            f.push(("level", level.to_string()));
            kind = Kind::Chat(K::Thread)
        }
        C::SharedObjects {
            object_type,
            before,
        } => {
            text(object_type, 128)?;
            route.push_str("/share");
            q.extend([("objectType", object_type.clone()), ("limit", "50".into())]);
            if let Some(id) = before {
                positive(*id)?;
                q.push(("lastKnownMessageId", id.to_string()))
            }
            kind = Kind::Chat(K::SharedObjects)
        }
        C::ShareObject {
            object_type,
            object_id,
            reference,
            thread,
        } => {
            text(object_type, 128)?;
            text(object_id, 1024)?;
            text(reference, 256)?;
            method = Method::POST;
            route.push_str("/share");
            f.extend([
                ("objectType", object_type.clone()),
                ("objectId", object_id.clone()),
                ("referenceId", reference.clone()),
            ]);
            if let Some(id) = thread {
                positive(*id)?;
                f.push(("threadId", id.to_string()))
            }
            kind = Kind::Message
        }
        C::AttachmentFolder { file_names } => {
            if file_names.len() > 100 {
                return Err(ErrorKind::TooLarge);
            }
            method = Method::POST;
            route.push_str("/attachment/folder");
            for name in file_names {
                text(name, 255)?;
                if name.contains(['/', '\\']) || matches!(name.as_str(), "." | "..") {
                    return Err(ErrorKind::Malformed);
                }
                f.push(("fileNames[]", name.clone()));
            }
            f.push(("allowUpdate", "false".into()));
            kind = Kind::Chat(K::AttachmentFolder)
        }
        C::ShareAttachment {
            path,
            file_name,
            reference,
            caption,
        } => {
            text(path, 16384)?;
            text(file_name, 255)?;
            if file_name.contains(['/', '\\']) || matches!(file_name.as_str(), "." | "..") {
                return Err(ErrorKind::Malformed);
            }
            text(reference, 256)?;
            bounded(caption, 4096)?;
            method = Method::POST;
            route.push_str("/attachment");
            f.extend([
                ("filePath", path.clone()),
                ("fileName", file_name.clone()),
                ("referenceId", reference.clone()),
                ("allowUpdate", "false".into()),
            ]);
            if !caption.is_empty() {
                f.push((
                    "talkMetaData",
                    serde_json::json!({"caption":caption}).to_string(),
                ))
            }
            kind = Kind::Chat(K::AttachmentShared)
        }
    }
    let mutation = method != Method::GET;
    Ok((method, route, q, f, kind, mutation))
}

pub(crate) fn validate(room: &RoomToken, response: &Response) -> Result<(), ErrorKind> {
    let tokens: Vec<&str> = match response {
        Response::Reminder(r) => vec![&r.token],
        Response::Thread(t) => vec![&t.token],
        Response::Threads(ts) => ts.iter().map(|t| t.token.as_str()).collect(),
        Response::Search { hits, .. } => hits.iter().map(|t| t.token.as_str()).collect(),
        _ => vec![],
    };
    if tokens.iter().any(|t| *t != room.as_str()) {
        Err(ErrorKind::WrongScope)
    } else {
        Ok(())
    }
}
