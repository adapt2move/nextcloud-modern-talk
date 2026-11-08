use super::*;
use serde_json::Value;

impl Shared {
    pub(super) fn incoming(&self, value: &Value) -> Result<(), ErrorKind> {
        let kind = wire::text(&value["type"], 32)?;
        let mut state = self.state.lock().map_err(|_| ErrorKind::Uncertain)?;
        match kind {
            "welcome" => {}
            "event" => {
                let own_sfu = state.session.clone().ok_or(ErrorKind::WrongScope)?;
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
                        room.membership(&own_sfu)?;
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
                            if let Some(talk) = room
                                .members
                                .iter()
                                .find(|m| m.sfu == sfu)
                                .map(|m| m.talk.clone())
                            {
                                room.typing.retain(|(session, _)| session != &talk);
                            }
                            room.members.retain(|m| m.sfu != sfu);
                            room.in_call.retain(|p| *p != sfu);
                            room.selected.retain(|p| p.sfu != sfu);
                        }
                        room.membership(&own_sfu)?;
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
                        if room.purpose == RoomPurpose::Call {
                            if update["all"].as_bool() == Some(true) {
                                let active = update["incall"]
                                    .as_bool()
                                    .or_else(|| {
                                        crate::models::integer(update.get("incall"))
                                            .map(|v| v & 1 != 0)
                                    })
                                    .ok_or(ErrorKind::Malformed)?;
                                if !active {
                                    room.cancellation.cancel();
                                    return Ok(());
                                }
                                room.in_call = room.members.iter().map(|m| m.sfu.clone()).collect();
                                if !room.in_call.contains(&own_sfu) && room.in_call.len() < 100 {
                                    room.in_call.push(own_sfu.clone());
                                }
                            } else if let Some(users) = update["users"].as_array() {
                                if users.len() > 100 {
                                    return Err(ErrorKind::TooLarge);
                                }
                                let mut active = Vec::with_capacity(users.len());
                                for user in users {
                                    let in_call = user["inCall"].as_bool().unwrap_or_else(|| {
                                        crate::models::integer(user.get("inCall"))
                                            .is_some_and(|v| v & 1 != 0)
                                    });
                                    if in_call && let Some(id) = user["sessionId"].as_str() {
                                        let id = PeerSession::parse(id)?;
                                        if !active.contains(&id) {
                                            active.push(id);
                                        }
                                    }
                                }
                                room.in_call = active;
                            }
                            room.membership(&own_sfu)?;
                            room.push(Event::PolicyChanged)?;
                        }
                    }
                    (Some("roomlist"), Some("disinvite")) => {
                        if event["disinvite"]["roomid"].as_str() == Some(room.token.as_str()) {
                            room.cancellation.cancel();
                        }
                    }
                    (Some("roomlist"), Some("update"))
                        if event["update"]["roomid"].as_str() == Some(room.token.as_str())
                            && room.purpose == RoomPurpose::Call =>
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
                if room.purpose == RoomPurpose::Chat {
                    if let Some((sender, started)) = wire::typing(&value["message"])?
                        && let Some(member) = room
                            .members
                            .iter()
                            .find(|member| member.sfu == sender && member.talk != room.own)
                    {
                        let talk = member.talk.clone();
                        room.typing.retain(|(session, _)| session != &talk);
                        if started && room.typing.len() < 100 {
                            room.typing.push((talk, std::time::Instant::now()));
                        }
                    }
                    return Ok(());
                }
                if data["roomType"].as_str() == Some("screen")
                    && data["type"].as_str() == Some("unshareScreen")
                {
                    let sender = PeerSession::parse(wire::text(
                        &value["message"]["sender"]["sessionid"],
                        512,
                    )?)?;
                    if let Some(member) = room.members.iter().find(|m| m.sfu == sender) {
                        let from = member.talk.clone();
                        let event = Event::ScreenStopped {
                            from,
                            negotiation: None,
                        };
                        room.push(event)?;
                    }
                    return Ok(());
                }
                if data["type"].as_str() == Some("control") {
                    let message = &value["message"];
                    if message["sender"]["type"].as_str() != Some("session")
                        || data["roomType"].as_str() != Some("video")
                        || data["payload"]["action"].as_str() != Some("forceMute")
                        || data["payload"]["peerId"].as_str() != Some(own_sfu.0.as_str())
                    {
                        return Ok(());
                    }
                    let Ok(sender) = wire::text(&message["sender"]["sessionid"], 512)
                        .and_then(PeerSession::parse)
                    else {
                        return Ok(());
                    };
                    if data.get("from").is_some()
                        && data["from"].as_str() != Some(sender.0.as_str())
                    {
                        return Ok(());
                    }
                    if let Some(member) = room.members.iter().find(|member| member.sfu == sender) {
                        room.push(Event::ForceMuteBy {
                            from: member.talk.clone(),
                        })?;
                    }
                    return Ok(());
                }
                if let Some(payload) = crate::signaling::encryption_payload(data)? {
                    let message = &value["message"];
                    if message["sender"]["type"].as_str() != Some("session") {
                        return Ok(());
                    }
                    let Ok(sender) = wire::text(&message["sender"]["sessionid"], 512)
                        .and_then(PeerSession::parse)
                    else {
                        return Ok(());
                    };
                    if data.get("from").is_some()
                        && data["from"].as_str() != Some(sender.0.as_str())
                    {
                        return Ok(());
                    }
                    if let Some(member) = room
                        .members
                        .iter()
                        .find(|member| member.sfu == sender && member.talk != room.own)
                    {
                        let from = member.talk.clone();
                        room.push_encryption(from, sender, payload);
                    }
                    return Ok(());
                }
                if matches!(data["type"].as_str(), Some("raiseHand" | "reaction")) {
                    let message = &value["message"];
                    if message["sender"]["type"].as_str() != Some("session")
                        || data["roomType"].as_str() != Some("video")
                    {
                        return Ok(());
                    }
                    let Ok(sender) = wire::text(&message["sender"]["sessionid"], 512)
                        .and_then(PeerSession::parse)
                    else {
                        return Ok(());
                    };
                    if data.get("from").is_some()
                        && data["from"].as_str() != Some(sender.0.as_str())
                    {
                        return Ok(());
                    }
                    let Some(member) = room.members.iter().find(|member| member.sfu == sender)
                    else {
                        return Ok(());
                    };
                    let kind = data["type"].as_str().unwrap_or("");
                    if let Ok(activity) =
                        crate::signaling::activity::Activity::decode(kind, &data["payload"])
                    {
                        room.push_activity(member.talk.clone(), activity);
                    }
                    return Ok(());
                }
                let Some((from, sid, signal, stream)) = wire::signal(&value["message"])? else {
                    return Ok(());
                };
                if room
                    .retired_subscriptions
                    .iter()
                    .any(|(peer, kind, old)| peer == &from && *kind == stream && old == &sid)
                {
                    return Ok(());
                }
                // HPB sendoffer pushes the first screen offer before a subscriber
                // knows the stream exists. Admit only a mapped current room member;
                // the media owner still requires fresh REST membership.
                if stream == StreamKind::Screen
                    && !room
                        .selected
                        .iter()
                        .any(|p| p.sfu == from && p.stream == stream)
                    && room.retired_screens.len() < 32
                    && !room
                        .retired_screens
                        .iter()
                        .any(|(peer, old)| peer == &from && old == &sid)
                    && matches!(&signal, Signal::Description(s) if s.kind() == crate::signaling::DescriptionKind::Offer)
                    && let Some(member) = room
                        .members
                        .iter()
                        .find(|m| m.sfu == from && m.talk != room.own)
                    && room.selected.len() < 20
                {
                    room.selected.push(Selection {
                        stream,
                        talk: member.talk.clone(),
                        sfu: from.clone(),
                        sid: None,
                        publisher: false,
                        request_id: None,
                    });
                }
                let Some(selected) = room
                    .selected
                    .iter_mut()
                    .find(|p| p.sfu == from && p.stream == stream)
                else {
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
                    selected.request_id = None;
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
                    stream,
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
                    && room.purpose == RoomPurpose::Call
                {
                    room.push(Event::ForceMute)?;
                }
            }
            "room" if value.get("id").is_none() => {
                if let Some(room) = state.room.as_mut() {
                    if value["room"]["roomid"].as_str() != Some(room.token.as_str()) {
                        room.cancellation.cancel();
                    } else if room.purpose == RoomPurpose::Call {
                        room.push(Event::PolicyChanged)?;
                    }
                }
                state.events.push(LiveEvent::RefreshAll);
            }
            "error" => {
                if value["error"]["code"].as_str() == Some("not_allowed")
                    && let Some(id) = value["id"].as_str()
                    && let Some(room) = state
                        .room
                        .as_mut()
                        .filter(|r| r.joined && !r.cancellation.is_cancelled())
                    && room.subscription_requests.iter().any(|old| old == id)
                {
                    if let Some(selected) = room
                        .selected
                        .iter_mut()
                        .find(|p| p.request_id.as_deref() == Some(id))
                    {
                        selected.request_id = None;
                        let event = Event::SubscriptionRejected {
                            from: selected.talk.clone(),
                            stream: selected.stream,
                        };
                        room.push(event)?;
                    }
                    // A retired request cannot affect a replacement subscriber.
                    return Ok(());
                }
                return Err(wire::failure(&value["error"]));
            }
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
        if let Some(room) = state.room.as_mut() {
            room.typing.clear();
        }
        if let Some(room) = state.room.as_mut().filter(|r| {
            r.purpose == RoomPurpose::Call && r.joined && !r.cancellation.is_cancelled()
        }) {
            room.push(Event::TransportInterrupted)?;
        }
        Ok(())
    }
    pub(super) fn resumed(&self) -> Result<(), ErrorKind> {
        let mut state = self.state.lock().map_err(|_| ErrorKind::Uncertain)?;
        state.status = LiveState::Connected;
        state.events.push(LiveEvent::RefreshAll);
        if let Some(room) = state.room.as_mut().filter(|r| {
            r.purpose == RoomPurpose::Call && r.joined && !r.cancellation.is_cancelled()
        }) {
            room.push(Event::TransportResumed)?;
        }
        Ok(())
    }
    pub(super) fn invalidate_room(&self, replaced: bool) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(room) = state.room.take() {
                room.replaced.store(replaced, Ordering::Release);
                room.cancellation.cancel();
            }
            state.events.push(LiveEvent::RefreshAll);
        }
    }
}
