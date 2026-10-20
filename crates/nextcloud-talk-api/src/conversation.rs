//! Explicit room administration routes. Server policy is revalidated by the application.
use crate::{
    ErrorKind, Kind, Plan, bounded,
    models::{RoomToken, integer, string},
};
use reqwest::Method;
use serde_json::Value;
#[derive(Clone, Copy, Debug)]
pub enum InvitationSource {
    User,
    Group,
    Team,
    Email,
    Federated,
    Phone,
}
impl InvitationSource {
    pub fn wire(self) -> &'static str {
        match self {
            Self::User => "users",
            Self::Group => "groups",
            Self::Team => "teams",
            Self::Email => "emails",
            Self::Federated => "federated_users",
            Self::Phone => "phones",
        }
    }
}
#[derive(Clone, Debug)]
pub enum Command {
    Rename(String),
    Description(String),
    EmojiAvatar {
        emoji: String,
        color: Option<String>,
    },
    DeleteAvatar,
    ReadOnly(bool),
    NotificationLevel(i8),
    CallNotifications(bool),
    Important(bool),
    Sensitive(bool),
    Expiration(u32),
    Preserve(bool),
    Public {
        enabled: bool,
        password: String,
    },
    Password(String),
    Listable(u8),
    MentionAll(bool),
    DefaultPermissions(u16),
    AttendeePermissions {
        attendee: u64,
        permissions: u16,
    },
    Lobby {
        enabled: bool,
        timer: Option<u64>,
    },
    RecordingConsent(bool),
    Delete,
    ClearHistory,
    Keep,
    Invite {
        source: InvitationSource,
        id: String,
    },
    RemoveAttendee(u64),
    Role {
        attendee: u64,
        moderator: bool,
        owner: bool,
    },
    Bots,
    Bot {
        id: u64,
        enabled: bool,
    },
    Bans,
    Ban {
        actor_type: String,
        actor_id: String,
        note: String,
    },
    Unban(u64),
    ResendInvitations(Option<u64>),
    BreakoutRooms,
    ConfigureBreakouts {
        amount: u8,
        automatic: bool,
    },
    StartBreakouts,
    StopBreakouts,
    RemoveBreakouts,
    Directory {
        search: String,
        source: InvitationSource,
    },
}
#[derive(Clone, Debug)]
pub struct DirectoryEntry {
    pub id: String,
    pub label: String,
    pub source: String,
    pub details: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Bot {
    pub id: u64,
    pub name: String,
    pub description: Option<String>,
    pub state: u64,
}
#[derive(Clone, Debug)]
pub struct Ban {
    pub id: u64,
    pub name: String,
    pub note: String,
    pub timestamp: u64,
}
#[derive(Clone, Debug)]
pub enum Response {
    Directory(Vec<DirectoryEntry>),
    Bots(Vec<Bot>),
    Bans(Vec<Ban>),
    Ban(Ban),
}
#[derive(Clone, Copy)]
pub(crate) enum ResponseKind {
    Directory,
    Bots,
    Bans,
    Ban,
}
fn required(v: &Value, k: &str, max: usize) -> Result<String, ErrorKind> {
    string(v.get(k), max)?.ok_or(ErrorKind::Malformed)
}
fn number(v: &Value, k: &str) -> Result<u64, ErrorKind> {
    integer(v.get(k)).ok_or(ErrorKind::Malformed)
}
fn ban(v: &Value) -> Result<Ban, ErrorKind> {
    Ok(Ban {
        id: number(v, "id")?,
        name: required(v, "bannedDisplayName", 256)?,
        note: required(v, "internalNote", 4000)?,
        timestamp: number(v, "bannedTime")?,
    })
}
pub(crate) fn decode(kind: ResponseKind, v: &Value) -> Result<crate::Reply, ErrorKind> {
    use ResponseKind as K;
    Ok(crate::Reply::Conversation(match kind {
        K::Directory => Response::Directory(crate::list(v, 100, |v| {
            Ok(DirectoryEntry {
                id: required(v, "id", 512)?,
                label: required(v, "label", 256)?,
                source: required(v, "source", 64)?,
                details: string(v.get("subline").or_else(|| v.get("details")), 512)?,
            })
        })?),
        K::Bots => Response::Bots(crate::list(v, 100, |v| {
            Ok(Bot {
                id: number(v, "id")?,
                name: required(v, "name", 256)?,
                description: string(v.get("description"), 4000)?,
                state: number(v, "state")?,
            })
        })?),
        K::Bans => Response::Bans(crate::list(v, 100, ban)?),
        K::Ban => Response::Ban(ban(v)?),
    }))
}
fn positive(v: u64) -> Result<(), ErrorKind> {
    if v == 0 {
        Err(ErrorKind::Malformed)
    } else {
        Ok(())
    }
}
fn text(s: &str, max: usize) -> Result<(), ErrorKind> {
    bounded(s, max)?;
    if s.trim().is_empty() {
        Err(ErrorKind::Malformed)
    } else {
        Ok(())
    }
}
pub(crate) fn plan(room: &RoomToken, c: &Command) -> Result<Plan, ErrorKind> {
    use Command as C;
    use ResponseKind as K;
    let base = format!("ocs/v2.php/apps/spreed/api/v4/room/{}", room.as_str());
    let mut route = base;
    let mut method = Method::PUT;
    let mut q = Vec::new();
    let mut f = Vec::new();
    let mut kind = Kind::Room;
    let flag = |v: bool| u8::from(v).to_string();
    match c {
        C::Rename(name) => {
            text(name, 256)?;
            f.push(("roomName", name.clone()))
        }
        C::Description(value) => {
            bounded(value, 4000)?;
            route.push_str("/description");
            f.push(("description", value.clone()))
        }
        C::EmojiAvatar { emoji, color } => {
            text(emoji, 64)?;
            method = Method::POST;
            route = format!(
                "ocs/v2.php/apps/spreed/api/v1/room/{}/avatar/emoji",
                room.as_str()
            );
            f.push(("emoji", emoji.clone()));
            if let Some(color) = color {
                let color = color.strip_prefix('#').unwrap_or(color);
                if color.len() != 6 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(ErrorKind::Malformed);
                }
                f.push(("color", color.into()))
            }
        }
        C::DeleteAvatar => {
            method = Method::DELETE;
            route = format!(
                "ocs/v2.php/apps/spreed/api/v1/room/{}/avatar",
                room.as_str()
            )
        }
        C::ReadOnly(value) => {
            route.push_str("/read-only");
            f.push(("state", flag(*value)))
        }
        C::NotificationLevel(level) => {
            if !(-2..=3).contains(level) {
                return Err(ErrorKind::Malformed);
            }
            method = Method::POST;
            route.push_str("/notify");
            f.push(("level", level.to_string()))
        }
        C::CallNotifications(value) => {
            method = Method::POST;
            route.push_str("/notify-calls");
            f.push(("level", flag(*value)))
        }
        C::Important(value) | C::Sensitive(value) | C::Preserve(value) => {
            method = if *value { Method::POST } else { Method::DELETE };
            route.push_str(match c {
                C::Important(_) => "/important",
                C::Sensitive(_) => "/sensitive",
                _ => "/preserve",
            })
        }
        C::Expiration(seconds) => {
            method = Method::POST;
            route.push_str("/message-expiration");
            f.push(("seconds", seconds.to_string()))
        }
        C::Public { enabled, password } => {
            bounded(password, 512)?;
            method = if *enabled {
                Method::POST
            } else {
                Method::DELETE
            };
            route.push_str("/public");
            if *enabled {
                f.push(("password", password.clone()))
            }
        }
        C::Password(password) => {
            bounded(password, 512)?;
            route.push_str("/password");
            f.push(("password", password.clone()))
        }
        C::Listable(scope) => {
            if *scope > 2 {
                return Err(ErrorKind::Malformed);
            }
            route.push_str("/listable");
            f.push(("scope", scope.to_string()))
        }
        C::MentionAll(value) => {
            route.push_str("/mention-permissions");
            f.push(("mentionPermissions", if *value { "0" } else { "1" }.into()))
        }
        C::DefaultPermissions(bits) => {
            if bits & !511 != 0 {
                return Err(ErrorKind::Unsupported);
            }
            route.push_str("/permissions/default");
            f.push(("permissions", bits.to_string()))
        }
        C::AttendeePermissions {
            attendee,
            permissions,
        } => {
            positive(*attendee)?;
            if permissions & !511 != 0 {
                return Err(ErrorKind::Unsupported);
            }
            route.push_str("/attendees/permissions");
            f.extend([
                ("attendeeId", attendee.to_string()),
                ("method", "set".into()),
                ("permissions", permissions.to_string()),
            ]);
            kind = Kind::Ack
        }
        C::Lobby { enabled, timer } => {
            route.push_str("/webinar/lobby");
            f.push(("state", flag(*enabled)));
            if let Some(t) = timer {
                f.push(("timer", t.to_string()))
            }
        }
        C::RecordingConsent(value) => {
            route.push_str("/recording-consent");
            f.push(("recordingConsent", flag(*value)))
        }
        C::Delete => {
            method = Method::DELETE;
            kind = Kind::Ack
        }
        C::ClearHistory => {
            method = Method::DELETE;
            route = format!("ocs/v2.php/apps/spreed/api/v1/chat/{}", room.as_str());
            kind = Kind::Message
        }
        C::Keep => {
            method = Method::DELETE;
            route.push_str("/object")
        }
        C::Invite { source, id } => {
            text(id, 512)?;
            method = Method::POST;
            route.push_str("/participants");
            f.extend([
                ("newParticipant", id.clone()),
                ("source", source.wire().into()),
            ]);
            kind = Kind::Ack
        }
        C::RemoveAttendee(id) => {
            positive(*id)?;
            method = Method::DELETE;
            route.push_str("/attendees");
            q.push(("attendeeId", id.to_string()));
            kind = Kind::Ack
        }
        C::Role {
            attendee,
            moderator,
            owner,
        } => {
            positive(*attendee)?;
            method = if *moderator || *owner {
                Method::POST
            } else {
                Method::DELETE
            };
            route.push_str("/moderators");
            let values = vec![
                ("attendeeId", attendee.to_string()),
                (
                    "participantType",
                    if *owner {
                        "1"
                    } else if *moderator {
                        "2"
                    } else {
                        "3"
                    }
                    .into(),
                ),
            ];
            if method == Method::DELETE {
                q = values
            } else {
                f = values
            }
            kind = Kind::Ack
        }
        C::Bots => {
            method = Method::GET;
            route = format!("ocs/v2.php/apps/spreed/api/v1/bot/{}", room.as_str());
            kind = Kind::Conversation(K::Bots)
        }
        C::Bot { id, enabled } => {
            positive(*id)?;
            method = if *enabled {
                Method::POST
            } else {
                Method::DELETE
            };
            route = format!("ocs/v2.php/apps/spreed/api/v1/bot/{}/{id}", room.as_str());
            kind = Kind::Ack
        }
        C::Bans => {
            method = Method::GET;
            route = format!("ocs/v2.php/apps/spreed/api/v1/ban/{}", room.as_str());
            kind = Kind::Conversation(K::Bans)
        }
        C::Ban {
            actor_type,
            actor_id,
            note,
        } => {
            if !matches!(actor_type.as_str(), "users" | "guests" | "emails" | "ip") {
                return Err(ErrorKind::Unsupported);
            }
            text(actor_id, 512)?;
            bounded(note, 4000)?;
            method = Method::POST;
            route = format!("ocs/v2.php/apps/spreed/api/v1/ban/{}", room.as_str());
            f.extend([
                ("actorType", actor_type.clone()),
                ("actorId", actor_id.clone()),
                ("internalNote", note.clone()),
            ]);
            kind = Kind::Conversation(K::Ban)
        }
        C::Unban(id) => {
            positive(*id)?;
            method = Method::DELETE;
            route = format!("ocs/v2.php/apps/spreed/api/v1/ban/{}/{id}", room.as_str());
            kind = Kind::Ack
        }
        C::ResendInvitations(attendee) => {
            method = Method::POST;
            route.push_str("/participants/resend-invitations");
            if let Some(id) = attendee {
                positive(*id)?;
                f.push(("attendeeId", id.to_string()))
            }
            kind = Kind::Ack
        }
        C::BreakoutRooms => {
            method = Method::GET;
            route.push_str("/breakout-rooms");
            kind = Kind::Rooms
        }
        C::ConfigureBreakouts { amount, automatic } => {
            if !(1..=20).contains(amount) {
                return Err(ErrorKind::Malformed);
            }
            method = Method::POST;
            route = format!(
                "ocs/v2.php/apps/spreed/api/v1/breakout-rooms/{}",
                room.as_str()
            );
            f.extend([
                ("mode", if *automatic { "1" } else { "2" }.into()),
                ("amount", amount.to_string()),
            ]);
            kind = Kind::Rooms
        }
        C::StartBreakouts | C::StopBreakouts | C::RemoveBreakouts => {
            method = if matches!(c, C::StartBreakouts) {
                Method::POST
            } else {
                Method::DELETE
            };
            route = format!(
                "ocs/v2.php/apps/spreed/api/v1/breakout-rooms/{}{}",
                room.as_str(),
                if matches!(c, C::RemoveBreakouts) {
                    ""
                } else {
                    "/rooms"
                }
            );
            kind = if matches!(c, C::RemoveBreakouts) {
                Kind::Room
            } else {
                Kind::Rooms
            }
        }
        C::Directory { search, source } => {
            text(search, 256)?;
            method = Method::GET;
            route = "ocs/v2.php/core/autocomplete/get".into();
            q.extend([
                ("search", search.clone()),
                ("itemType", "call".into()),
                ("itemId", room.as_str().into()),
                ("limit", "100".into()),
                (
                    "shareTypes[]",
                    match source {
                        InvitationSource::User => "0",
                        InvitationSource::Group => "1",
                        InvitationSource::Email => "4",
                        InvitationSource::Federated => "6",
                        InvitationSource::Team => "7",
                        InvitationSource::Phone => return Err(ErrorKind::Unsupported),
                    }
                    .into(),
                ),
            ]);
            kind = Kind::Conversation(K::Directory)
        }
    }
    let mutation = method != Method::GET;
    Ok((method, route, q, f, kind, mutation))
}
