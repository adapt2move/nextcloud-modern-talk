//! Bounded Talk 24/25 creation presets and extended room creation.
//! See spreed `lib/Controller/PresetController.php` and `RoomController.php`.
use crate::{ErrorKind, Kind, Plan, bounded};
use reqwest::Method;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Preset {
    pub identifier: String,
    pub name: String,
    pub description: String,
    pub parameters: Vec<(String, i64)>,
}

#[derive(Clone, Debug)]
pub struct Room {
    pub name: String,
    pub description: String,
    pub participants: Vec<String>,
    pub preset: Option<String>,
    pub parameters: Vec<(String, i64)>,
}

fn parameter(name: &str, value: i64) -> Option<&'static str> {
    let max = match name {
        "roomType" => 3,
        "readOnly" | "lobbyState" | "mentionPermissions" => 1,
        "listable" | "sipEnabled" | "recordingConsent" => 2,
        "permissions" => 511,
        "messageExpiration" => i64::from(u32::MAX),
        _ => return None,
    };
    if value < 0 || value > max || (name == "roomType" && value < 2) {
        return None;
    }
    Some(match name {
        "roomType" => "roomType",
        "readOnly" => "readOnly",
        "lobbyState" => "lobbyState",
        "mentionPermissions" => "mentionPermissions",
        "listable" => "listable",
        "sipEnabled" => "sipEnabled",
        "recordingConsent" => "recordingConsent",
        "permissions" => "permissions",
        "messageExpiration" => "messageExpiration",
        _ => return None,
    })
}

pub(crate) fn decode(value: &Value) -> Result<crate::Reply, ErrorKind> {
    let entries = value.as_array().ok_or(ErrorKind::Malformed)?;
    if entries.len() > 32 {
        return Err(ErrorKind::TooLarge);
    }
    let mut presets: Vec<Preset> = Vec::new();
    for entry in entries {
        let get = |key: &str, limit| -> Result<String, ErrorKind> {
            let s = entry
                .get(key)
                .and_then(Value::as_str)
                .ok_or(ErrorKind::Malformed)?;
            bounded(s, limit)?;
            Ok(s.to_owned())
        };
        let identifier = get("identifier", 128)?;
        if identifier.is_empty()
            || !identifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || presets.iter().any(|p| p.identifier == identifier)
        {
            return Err(ErrorKind::Malformed);
        }
        let mut parameters = Vec::new();
        let values = entry.get("parameters").ok_or(ErrorKind::Malformed)?;
        // PHP encodes an empty associative array as [].
        if !values.as_array().is_some_and(Vec::is_empty) {
            let values = values.as_object().ok_or(ErrorKind::Malformed)?;
            if values.len() > 16 {
                return Err(ErrorKind::TooLarge);
            }
            for (key, value) in values {
                let value = value.as_i64().ok_or(ErrorKind::Malformed)?;
                parameter(key, value).ok_or(ErrorKind::Unsupported)?;
                parameters.push((key.clone(), value));
            }
        }
        presets.push(Preset {
            identifier,
            name: get("name", 256)?,
            description: get("description", 1024)?,
            parameters,
        });
    }
    Ok(crate::Reply::CreationPresets(presets))
}

pub(crate) fn plan(room: &Room) -> Result<Plan, ErrorKind> {
    bounded(&room.name, 256)?;
    bounded(&room.description, 2000)?;
    if room.name.trim().is_empty() || room.participants.len() > 20 || room.parameters.len() > 16 {
        return Err(ErrorKind::Malformed);
    }
    let mut form = vec![
        ("roomName", room.name.clone()),
        ("description", room.description.clone()),
    ];
    if let Some(preset) = &room.preset {
        bounded(preset, 128)?;
        if preset.is_empty()
            || !preset
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ErrorKind::Malformed);
        }
        form.push(("preset", preset.clone()));
    }
    for user in &room.participants {
        bounded(user, 512)?;
        if user.trim().is_empty() || user.chars().any(char::is_control) {
            return Err(ErrorKind::Malformed);
        }
        form.push(("participants[users][]", user.clone()));
    }
    for (name, value) in &room.parameters {
        let key = parameter(name, *value).ok_or(ErrorKind::Malformed)?;
        if form.iter().any(|(existing, _)| *existing == key) {
            return Err(ErrorKind::Malformed);
        }
        form.push((key, value.to_string()));
    }
    if !form.iter().any(|(key, _)| *key == "roomType") {
        return Err(ErrorKind::Malformed);
    }
    Ok((
        Method::POST,
        "ocs/v2.php/apps/spreed/api/v4/room".into(),
        vec![],
        form,
        Kind::Room,
        true,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn presets_and_creation_are_bounded_and_use_grouped_invitation_fields() {
        let crate::Reply::CreationPresets(presets) = decode(&json!([
            {"identifier":"default","name":"Default","description":"","parameters":{"roomType":2,"listable":0}},
            {"identifier":"forced","name":"Forced","description":"","parameters":[]}
        ])).unwrap() else { panic!("presets") };
        assert_eq!(presets.len(), 2);
        assert!(decode(&json!([{"identifier":"default","name":"A","description":"","parameters":{"unknown":1}}])).is_err());
        let room = Room {
            name: "Fixture".into(),
            description: "Description".into(),
            participants: vec!["synthetic-user".into()],
            preset: None,
            parameters: vec![("roomType".into(), 2), ("listable".into(), 0)],
        };
        let (_, _, _, form, _, mutation) = plan(&room).unwrap();
        assert!(mutation);
        assert!(form.contains(&("participants[users][]", "synthetic-user".into())));
        assert!(
            plan(&Room {
                parameters: vec![("roomType".into(), 9)],
                ..room.clone()
            })
            .is_err()
        );
        assert!(
            plan(&Room {
                participants: vec!["bad\nidentity".into()],
                ..room
            })
            .is_err()
        );
    }
}
