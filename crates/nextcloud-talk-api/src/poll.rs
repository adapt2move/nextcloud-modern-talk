//! Published polls from the pinned Talk 24.0.6 / 25.0.6 v1 contract.
//! Draft deletion is deliberately a separate, unsupported operation here.
use crate::{
    Client, Error, ErrorKind,
    models::{RoomToken, integer, string},
    scope::OperationContext,
};
use reqwest::Method;
use serde_json::Value;

pub const MAX_OPTIONS: usize = 100;
#[derive(Clone, Debug)]
pub enum Command {
    Read(u64),
    Create {
        question: String,
        options: Vec<String>,
        hidden_results: bool,
        max_votes: u16,
    },
    Vote {
        poll: u64,
        options: Vec<u16>,
    },
    Close(u64),
}
#[derive(Clone, Debug)]
pub struct Voter {
    pub name: String,
    pub option: u16,
}
#[derive(Clone, Debug)]
pub struct Poll {
    pub id: u64,
    pub question: String,
    pub options: Vec<String>,
    pub votes: Option<Vec<u32>>,
    pub own: Vec<u16>,
    pub status: u8,
    pub hidden_results: bool,
    pub max_votes: u16,
    pub voters: Option<u32>,
    pub actor_type: String,
    pub actor_id: String,
    pub details: Vec<Voter>,
    pub details_complete: bool,
}
fn id(id: u64) -> Result<u64, ErrorKind> {
    if id == 0 {
        Err(ErrorKind::Malformed)
    } else {
        Ok(id)
    }
}
type PollPlan = (Method, String, Vec<(&'static str, String)>);

fn plan(room: &RoomToken, command: &Command) -> Result<PollPlan, ErrorKind> {
    let route = format!("ocs/v2.php/apps/spreed/api/v1/poll/{}", room.as_str());
    Ok(match command {
        Command::Read(n) => (Method::GET, format!("{route}/{}", id(*n)?), vec![]),
        Command::Close(n) => (Method::DELETE, format!("{route}/{}", id(*n)?), vec![]),
        Command::Vote { poll, options } => {
            if options.len() > MAX_OPTIONS
                || options
                    .iter()
                    .enumerate()
                    .any(|(i, n)| options[..i].contains(n))
            {
                return Err(ErrorKind::Malformed);
            }
            let mut form = options
                .iter()
                .map(|n| ("optionIds[]", n.to_string()))
                .collect::<Vec<_>>();
            // PHP's default empty array applies when no selection is supplied.
            if form.is_empty() {
                form.push(("format", "json".into()));
            }
            (Method::POST, format!("{route}/{}", id(*poll)?), form)
        }
        Command::Create {
            question,
            options,
            hidden_results,
            max_votes,
        } => {
            if question.trim().is_empty()
                || question.len() > 4000
                || !(2..=MAX_OPTIONS).contains(&options.len())
                || options
                    .iter()
                    .any(|s| s.trim().is_empty() || s.len() > 1000)
                || usize::from(*max_votes) > options.len()
                || serde_json::to_vec(options)
                    .map_err(|_| ErrorKind::Malformed)?
                    .len()
                    > 60_000
            {
                return Err(ErrorKind::TooLarge);
            }
            let mut form = vec![
                ("question", question.clone()),
                ("resultMode", u8::from(*hidden_results).to_string()),
                ("maxVotes", max_votes.to_string()),
                ("draft", "false".into()),
            ];
            form.extend(options.iter().map(|s| ("options[]", s.clone())));
            (Method::POST, route, form)
        }
    })
}
impl Poll {
    pub(crate) fn decode(v: &Value) -> Result<Self, ErrorKind> {
        let options = v["options"].as_array().ok_or(ErrorKind::Malformed)?;
        if !(2..=MAX_OPTIONS).contains(&options.len()) {
            return Err(ErrorKind::TooLarge);
        }
        let options = options
            .iter()
            .map(|v| string(Some(v), 1000)?.ok_or(ErrorKind::Malformed))
            .collect::<Result<Vec<_>, _>>()?;
        let status = u8::try_from(integer(v.get("status")).ok_or(ErrorKind::Malformed)?)
            .map_err(|_| ErrorKind::Malformed)?;
        if status > 2 {
            return Err(ErrorKind::Unsupported);
        }
        let hidden_results = match integer(v.get("resultMode")) {
            Some(0) => false,
            Some(1) => true,
            _ => return Err(ErrorKind::Unsupported),
        };
        let selected = v["votedSelf"].as_array().ok_or(ErrorKind::Malformed)?;
        if selected.len() > options.len() {
            return Err(ErrorKind::Malformed);
        }
        let mut own = Vec::with_capacity(selected.len());
        for n in selected {
            let n = u16::try_from(integer(Some(n)).ok_or(ErrorKind::Malformed)?)
                .map_err(|_| ErrorKind::Malformed)?;
            if usize::from(n) >= options.len() || own.contains(&n) {
                return Err(ErrorKind::Malformed);
            }
            own.push(n);
        }
        let visible = status == 1 || (!hidden_results && !own.is_empty());
        let raw = &v["votes"];
        if !raw.is_object() && !raw.as_array().is_some_and(Vec::is_empty) {
            return Err(ErrorKind::Malformed);
        }
        let mut votes = vec![0; options.len()];
        if let Some(map) = raw.as_object() {
            if map.len() > options.len() {
                return Err(ErrorKind::Malformed);
            }
            for (key, value) in map {
                let index = key
                    .strip_prefix("option-")
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|i| *i < options.len())
                    .ok_or(ErrorKind::Malformed)?;
                votes[index] = u32::try_from(integer(Some(value)).ok_or(ErrorKind::Malformed)?)
                    .map_err(|_| ErrorKind::Malformed)?;
            }
        }
        let mut details = Vec::new();
        let mut details_complete = true;
        if let Some(values) = v.get("details").and_then(Value::as_array) {
            // The complete response body was already bounded. Keep at most 100 permitted rows.
            if !hidden_results && status == 1 {
                details_complete = values.len() <= 100;
                for row in values.iter().take(100) {
                    let option =
                        u16::try_from(integer(row.get("optionId")).ok_or(ErrorKind::Malformed)?)
                            .map_err(|_| ErrorKind::Malformed)?;
                    if usize::from(option) >= options.len() {
                        return Err(ErrorKind::Malformed);
                    }
                    details.push(Voter {
                        name: string(row.get("actorDisplayName"), 256)?
                            .ok_or(ErrorKind::Malformed)?,
                        option,
                    });
                }
            }
        }
        Ok(Self {
            id: id(integer(v.get("id")).ok_or(ErrorKind::Malformed)?)?,
            question: string(v.get("question"), 4000)?.ok_or(ErrorKind::Malformed)?,
            options,
            votes: visible.then_some(votes),
            own,
            status,
            hidden_results,
            max_votes: u16::try_from(integer(v.get("maxVotes")).ok_or(ErrorKind::Malformed)?)
                .map_err(|_| ErrorKind::TooLarge)?,
            voters: integer(v.get("numVoters"))
                .map(|n| u32::try_from(n).map_err(|_| ErrorKind::TooLarge))
                .transpose()?,
            actor_type: string(v.get("actorType"), 64)?.ok_or(ErrorKind::Malformed)?,
            actor_id: string(v.get("actorId"), 512)?.ok_or(ErrorKind::Malformed)?,
            details,
            details_complete,
        })
    }
}
impl Client {
    pub async fn poll(
        &mut self,
        c: &OperationContext,
        room: &RoomToken,
        command: Command,
    ) -> Result<Poll, Error> {
        self.validate(c)?;
        let (method, route, form) = plan(room, &command).map_err(|k| self.error(k))?;
        let mutation = method != Method::GET;
        let create = matches!(command, Command::Create { .. });
        if create {
            if self.poll_creation_uncertain.contains(room) {
                return Err(self.error(ErrorKind::Uncertain));
            }
            if self.poll_creation_uncertain.len() >= 100 {
                return Err(self.error(ErrorKind::Busy));
            }
            self.poll_creation_uncertain.push(room.clone());
        }
        let result = async {
            let response = self
                .transport
                .ocs(c, method, &route, &[], &form, false)
                .await?;
            let poll = Poll::decode(&response.data).map_err(|k| self.error(k))?;
            let expected = match command {
                Command::Read(n) | Command::Close(n) | Command::Vote { poll: n, .. } => Some(n),
                _ => None,
            };
            if expected.is_some_and(|n| n != poll.id) {
                return Err(self.error(ErrorKind::WrongScope));
            }
            self.validate(c)?;
            Ok(poll)
        }
        .await
        .map_err(|e| if mutation { self.mutation_error(e) } else { e });
        if create
            && result
                .as_ref()
                .err()
                .is_none_or(|e| e.kind != ErrorKind::Uncertain)
        {
            self.poll_creation_uncertain.retain(|r| r != room);
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn value() -> Value {
        json!({"id":3,"question":"Where?","options":["A","B"],"votes":[],"votedSelf":[],"status":0,"resultMode":1,"maxVotes":0,"numVoters":0,"actorType":"users","actorId":"author"})
    }
    #[test]
    fn hidden_summary_is_absent_until_closed() {
        let mut v = value();
        v["votes"] = json!({"option-0":4});
        assert!(Poll::decode(&v).unwrap().votes.is_none());
        v["status"] = json!(1);
        let p = Poll::decode(&v).unwrap();
        assert_eq!(p.votes, Some(vec![4, 0]));
    }
    #[test]
    fn invalid_indices_and_unknown_status_fail_closed() {
        let mut v = value();
        v["votes"] = json!({"option-3":1});
        assert!(Poll::decode(&v).is_err());
        v = value();
        v["status"] = json!(7);
        assert!(Poll::decode(&v).is_err());
    }
    #[test]
    fn anonymous_poll_never_exposes_accidental_details() {
        let mut v = value();
        v["status"] = json!(1);
        v["details"] = json!([{"actorDisplayName":"private","optionId":0}]);
        assert!(Poll::decode(&v).unwrap().details.is_empty());
    }
    #[test]
    fn routes_and_options_are_typed() {
        let room = RoomToken::parse("abcd").unwrap();
        let p = plan(
            &room,
            &Command::Vote {
                poll: 5,
                options: vec![0, 1],
            },
        )
        .unwrap();
        assert_eq!(p.1, "ocs/v2.php/apps/spreed/api/v1/poll/abcd/5");
        assert_eq!(p.2[0], ("optionIds[]", "0".into()));
        assert!(
            plan(
                &room,
                &Command::Vote {
                    poll: 5,
                    options: vec![1, 1]
                }
            )
            .is_err()
        );
    }
}
