use super::*;
use serde_json::Value;

impl Shared {
    pub(super) fn incoming(&self, value: &Value) -> Result<(), ErrorKind> {
        let kind = wire::text(&value["type"], 32)?;
        let mut state = self.state.lock().map_err(|_| ErrorKind::Uncertain)?;
        match kind {
            "welcome" => {}
            "event" => {
                let event = &value["event"];
                let current = state.room.as_ref().filter(|r| r.joined).map(|r| &r.token);
                if let Some(hint) = wire::hint(event, current)? {
                    state.events.push(hint);
                }
                let Some(room) = state
                    .room
                    .as_mut()
                    .filter(|r| r.joined && !r.cancellation.is_cancelled())
                else {
                    return Ok(());
                };
                match (event["target"].as_str(), event["type"].as_str()) {
                    (Some("room"), Some("join")) => {
                        let values = event["join"].as_array().ok_or(ErrorKind::Malformed)?;
                        if values.len() > 100 || room.members.len() + values.len() > 100 {
                            return Err(ErrorKind::TooLarge);
                        }
                        for v in values {
                            let member = wire::member(v)?;
                            if let Some(old) = room
                                .members
                                .iter()
                                .find(|m| m.sfu == member.sfu || m.talk == member.talk)
                            {
                                if old.sfu != member.sfu || old.talk != member.talk {
                                    return Err(ErrorKind::WrongScope);
                                }
                            } else {
                                room.members.push(member);
                            }
                        }
                        let known = room.members.iter().map(|m| m.talk.clone()).collect();
                        room.push(Event::SfuMembership(known))?;
                    }
                    (Some("room"), Some("leave")) => {
                        let values = event["leave"].as_array().ok_or(ErrorKind::Malformed)?;
                        if values.len() > 100 {
                            return Err(ErrorKind::TooLarge);
                        }
                        for v in values {
                            let sfu = PeerSession::parse(wire::text(v, 512)?)?;
                            if room
                                .members
                                .iter()
                                .any(|m| m.sfu == sfu && m.talk == room.own)
                            {
                                room.cancellation.cancel();
                                return Err(ErrorKind::WrongScope);
                            }
                            room.members.retain(|m| m.sfu != sfu);
                            room.selected.retain(|p| p.sfu != sfu);
                        }
                        room.push(Event::SfuMembership(
                            room.members.iter().map(|m| m.talk.clone()).collect(),
                        ))?;
                    }
                    (Some("room"), Some("switchto")) => {
                        // A breakout/federation switch is not a grant for a new
                        // room, nor an invitation to move this call's devices.
                        room.cancellation.cancel();
                    }
                    (Some("participants"), Some("update")) => {
                        let update = &event["update"];
                        if update["roomid"].as_str() != Some(room.token.as_str()) {
                            return Ok(());
                        }
                        if update["all"].as_bool() == Some(true)
                            && crate::models::integer(update.get("incall"))
                                .is_some_and(|v| v & 1 == 0)
                        {
                            room.cancellation.cancel();
                        }
                        room.push(Event::PolicyChanged)?;
                    }
                    (Some("roomlist"), Some("disinvite")) => {
                        if event["disinvite"]["roomid"].as_str() == Some(room.token.as_str()) {
                            room.cancellation.cancel();
                        }
                    }
                    (Some("roomlist"), Some("update"))
                        if event["update"]["roomid"].as_str() == Some(room.token.as_str()) =>
                    {
                        room.push(Event::PolicyChanged)?;
                    }
                    _ => {}
                }
            }
            "message" => {
                let own_sfu = state.session.clone().ok_or(ErrorKind::WrongScope)?;
                let Some(room) = state
                    .room
                    .as_mut()
                    .filter(|r| r.joined && !r.cancellation.is_cancelled())
                else {
                    return Ok(());
                };
                let data = &value["message"]["data"];
                if data.get("to").is_some() && data["to"].as_str() != Some(own_sfu.0.as_str()) {
                    return Err(ErrorKind::WrongScope);
                }
                let Some((from, sid, signal)) = wire::signal(&value["message"])? else {
                    return Ok(());
                };
                let Some(selected) = room.selected.iter_mut().find(|p| p.sfu == from) else {
                    // Unsolicited offers/candidates never create subscribers.
                    return Ok(());
                };
                if let Some(expected) = &selected.sid {
                    if expected != &sid {
                        return Ok(());
                    }
                } else if matches!(&signal, Signal::Description(s) if s.kind() == crate::signaling::DescriptionKind::Offer)
                {
                    selected.sid = Some(sid.clone());
                } else {
                    // The first subscriber offer establishes its SFU SID.
                    return Ok(());
                }
                if let Signal::Description(s) = &signal {
                    let expected = if selected.publisher {
                        crate::signaling::DescriptionKind::Answer
                    } else {
                        crate::signaling::DescriptionKind::Offer
                    };
                    if s.kind() != expected {
                        return Err(ErrorKind::WrongScope);
                    }
                }
                let from = selected.talk.clone();
                room.push(Event::Signal {
                    from,
                    negotiation: sid,
                    signal,
                })?;
            }
            "control" => {
                let own = state.session.as_ref().map(|s| s.0.as_str());
                let data = &value["control"]["data"];
                if data["action"].as_str() == Some("forceMute")
                    && data["peerId"].as_str() == own
                    && let Some(room) = state.room.as_mut()
                {
                    room.push(Event::ForceMute)?;
                }
            }
            "room" if value.get("id").is_none() => {
                if let Some(room) = state.room.as_mut() {
                    if value["room"]["roomid"].as_str() != Some(room.token.as_str()) {
                        room.cancellation.cancel();
                    } else {
                        room.push(Event::PolicyChanged)?;
                    }
                }
                state.events.push(LiveEvent::RefreshAll);
            }
            "error" => return Err(wire::failure(&value["error"])),
            "bye" => return Err(ErrorKind::Cancelled),
            // Unknown extensions cannot authorize actions and do not prevent a
            // newer server from adding unrelated non-authoritative events.
            _ => {}
        }
        Ok(())
    }

    pub(super) fn interrupt(&self) -> Result<(), ErrorKind> {
        let mut state = self.state.lock().map_err(|_| ErrorKind::Uncertain)?;
        state.status = LiveState::Reconnecting;
        state.events.push(LiveEvent::RefreshAll);
        if let Some(room) = state
            .room
            .as_mut()
            .filter(|r| r.joined && !r.cancellation.is_cancelled())
        {
            room.push(Event::TransportInterrupted)?;
        }
        Ok(())
    }
    pub(super) fn resumed(&self) -> Result<(), ErrorKind> {
        let mut state = self.state.lock().map_err(|_| ErrorKind::Uncertain)?;
        state.status = LiveState::Connected;
        state.events.push(LiveEvent::RefreshAll);
        if let Some(room) = state
            .room
            .as_mut()
            .filter(|r| r.joined && !r.cancellation.is_cancelled())
        {
            room.push(Event::TransportResumed)?;
        }
        Ok(())
    }
    pub(super) fn invalidate_room(&self) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(room) = state.room.take() {
                room.cancellation.cancel();
            }
            state.events.push(LiveEvent::RefreshAll);
        }
    }
}
