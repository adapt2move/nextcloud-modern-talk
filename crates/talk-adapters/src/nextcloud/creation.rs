//! Presets, staged directory selections and a single, journaled creation write.
use super::{projection::Project, *};
use nextcloud_talk_api as api;
use talk_core::protocol::{
    FeatureSupport,
    conversation::RecipientId,
    creation::{Options, Preset, Room},
};

fn merged(
    presets: &[api::creation::Preset],
    selected: Option<&str>,
    public: bool,
) -> Option<Vec<(String, i64)>> {
    let default = presets.iter().find(|p| p.identifier == "default")?;
    let forced = presets.iter().find(|p| p.identifier == "forced")?;
    let mut values = std::collections::BTreeMap::new();
    for (key, value) in &default.parameters {
        values.insert(key.clone(), *value);
    }
    if let Some(selected) = selected {
        if !matches!(selected, "webinar" | "presentation" | "voiceroom") {
            return None;
        }
        let preset = presets.iter().find(|p| {
            p.identifier == selected && p.identifier != "default" && p.identifier != "forced"
        })?;
        for (key, value) in &preset.parameters {
            values.insert(key.clone(), *value);
        }
    }
    values.insert("roomType".into(), if public { 3 } else { 2 });
    values.insert("listable".into(), if public { 1 } else { 0 });
    for (key, value) in &forced.parameters {
        values.insert(key.clone(), *value);
    }
    Some(values.into_iter().collect())
}

impl NextcloudClient {
    pub(super) async fn inspect_creation(
        &mut self,
        c: &OperationContext,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        self.creation_presets = None;
        let mut shown = Page::empty();
        let presets = if caps.has("conversation-presets") {
            let api::Reply::CreationPresets(presets) = self
                .transport
                .api_request(c, api::Request::CreationPresets)
                .await?
            else {
                return Err(self.error(ErrorKind::Malformed));
            };
            if merged(&presets, None, false).is_none() {
                return Err(self.error(ErrorKind::Malformed));
            }
            for preset in presets.iter().filter(|p| {
                matches!(
                    p.identifier.as_str(),
                    "webinar" | "presentation" | "voiceroom"
                )
            }) {
                shown
                    .try_push(Preset {
                        id: Text::new(&preset.identifier)
                            .map_err(|_| self.error(ErrorKind::TooLarge))?,
                        name: Text::new(&preset.name)
                            .map_err(|_| self.error(ErrorKind::TooLarge))?,
                        description: Text::new(&preset.description)
                            .map_err(|_| self.error(ErrorKind::TooLarge))?,
                    })
                    .map_err(|_| self.error(ErrorKind::TooLarge))?;
            }
            presets
        } else {
            vec![
                api::creation::Preset {
                    identifier: "default".into(),
                    name: String::new(),
                    description: String::new(),
                    parameters: vec![],
                },
                api::creation::Preset {
                    identifier: "forced".into(),
                    name: String::new(),
                    description: String::new(),
                    parameters: vec![],
                },
            ]
        };
        let forced_listable = presets
            .iter()
            .find(|p| p.identifier == "forced")
            .and_then(|p| p.parameters.iter().find(|(k, _)| k == "listable"))
            .map(|(_, v)| *v);
        let forced_type = presets
            .iter()
            .find(|p| p.identifier == "forced")
            .and_then(|p| p.parameters.iter().find(|(k, _)| k == "roomType"))
            .map(|(_, v)| *v);
        let options = Options {
            group: caps.can_create,
            extended: caps.feature("conversation-creation-all"),
            public: if caps.has("conversation-creation-all") {
                caps.feature("listable-rooms")
            } else {
                FeatureSupport::Unsupported
            },
            private_allowed: forced_listable.is_none_or(|value| value == 0)
                && forced_type.is_none_or(|value| value == 2),
            public_allowed: forced_listable.is_none_or(|value| value > 0)
                && forced_type.is_none_or(|value| value == 3),
            presets: shown,
        };
        self.creation_presets = Some(presets);
        Ok(ProtocolReply::CreationOptions(options))
    }

    pub(super) fn stage_creation_person(
        &mut self,
        c: &OperationContext,
        id: RecipientId,
    ) -> Result<ProtocolReply, PortError> {
        let now = self.transport.now();
        let mut person = self
            .people
            .iter()
            .find(|p| {
                p.id == id
                    && id.scope == c.scope
                    && p.expires.epoch == now.epoch
                    && now.millis < p.expires.millis
            })
            .cloned()
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        self.creation_people
            .retain(|p| p.expires.epoch == now.epoch && now.millis < p.expires.millis);
        if self
            .creation_people
            .iter()
            .any(|p| p.id != id && p.actor_id == person.actor_id)
        {
            return Err(self.error(ErrorKind::Conflict));
        }
        person.expires.millis = now.millis.saturating_add(600_000);
        if !self.creation_people.iter().any(|p| p.id == id) {
            if self.creation_people.len() >= 20 {
                return Err(self.error(ErrorKind::Busy));
            }
            self.creation_people.push(person);
        }
        Ok(ProtocolReply::CreationPerson(id))
    }

    pub(super) fn unstage_creation_person(
        &mut self,
        c: &OperationContext,
        id: RecipientId,
    ) -> Result<ProtocolReply, PortError> {
        if id.scope != c.scope {
            return Err(self.error(ErrorKind::WrongScope));
        }
        self.creation_people.retain(|p| p.id != id);
        Ok(ProtocolReply::CreationPerson(id))
    }

    pub(super) async fn create_conversation(
        &mut self,
        c: &OperationContext,
        room: Room,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        if self.creation_uncertain {
            return Err(self.error(ErrorKind::Uncertain));
        }
        self.allowed(caps.can_create)?;
        let presets = self
            .creation_presets
            .as_ref()
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        let parameters = merged(presets, room.preset.as_ref().map(Text::as_str), room.public)
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        let listable = parameters
            .iter()
            .find(|(k, _)| k == "listable")
            .map(|(_, v)| *v);
        let room_type = parameters
            .iter()
            .find(|(key, _)| key == "roomType")
            .map(|(_, value)| *value);
        if room.public && (listable == Some(0) || !caps.has("listable-rooms"))
            || !room.public && listable.is_some_and(|v| v != 0)
            || room_type != Some(if room.public { 3 } else { 2 })
        {
            return Err(self.error(ErrorKind::PermissionDenied));
        }
        let now = self.transport.now();
        let mut participants = Vec::new();
        for id in room.participants.as_slice() {
            let binding = self
                .creation_people
                .iter()
                .find(|p| {
                    p.id == *id
                        && id.scope == c.scope
                        && p.expires.epoch == now.epoch
                        && now.millis < p.expires.millis
                })
                .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
            if participants.contains(&binding.actor_id) {
                return Err(self.error(ErrorKind::Malformed));
            }
            participants.push(binding.actor_id.clone());
        }
        let extended = caps.has("conversation-creation-all");
        if !extended
            && (room.public
                || !room.description.as_str().is_empty()
                || room.preset.is_some()
                || !participants.is_empty())
        {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let request = if extended {
            api::Request::CreateConversation(api::creation::Room {
                name: room.name.as_str().into(),
                description: room.description.as_str().into(),
                participants,
                preset: room.preset.as_ref().map(|p| p.as_str().into()),
                parameters,
            })
        } else {
            api::Request::CreateGroup {
                name: room.name.as_str().into(),
            }
        };
        self.creation_uncertain = true;
        let reply = match self.transport.api_request(c, request).await {
            Ok(value) => value,
            Err(error) => {
                if matches!(
                    error.kind,
                    ErrorKind::PermissionDenied
                        | ErrorKind::NotFound
                        | ErrorKind::Conflict
                        | ErrorKind::RateLimited
                ) {
                    self.creation_uncertain = false;
                }
                return Err(error);
            }
        };
        let api::Reply::Room(created) = reply else {
            return Err(self.error(ErrorKind::Uncertain));
        };
        let incomplete_invitations = created.incomplete_invitations;
        let data = created.project();
        let id = self
            .bind_room(&data)
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        let mut created =
            wire::room(&data, id, caps).map_err(|_| self.error(ErrorKind::Uncertain))?;
        self.project_room_tags(&mut created, &data, caps)
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        self.creation_uncertain = false;
        self.creation_people.clear();
        Ok(ProtocolReply::CreatedConversation {
            room: created,
            incomplete_invitations,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn server_forced_values_win_after_defaults_preset_and_user_selection() {
        let preset = |id: &str, params: &[(&str, i64)]| api::creation::Preset {
            identifier: id.into(),
            name: id.into(),
            description: String::new(),
            parameters: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
        };
        let presets = [
            preset(
                "default",
                &[("roomType", 2), ("listable", 0), ("permissions", 0)],
            ),
            preset("webinar", &[("roomType", 3), ("permissions", 389)]),
            preset("forced", &[("listable", 0), ("permissions", 1)]),
        ];
        let values = merged(&presets, Some("webinar"), true).unwrap();
        assert!(values.contains(&("listable".into(), 0)));
        assert!(values.contains(&("permissions".into(), 1)));
        assert!(values.contains(&("roomType".into(), 3)));
        assert!(merged(&presets, Some("unknown"), true).is_none());
    }
}
