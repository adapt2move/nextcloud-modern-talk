//! Bounded schema-4 rows. Decode validates duplicate keys before applying any upsert.
use std::fmt::Write;
use talk_core::{
    bounded::Text,
    preferences::{PreferenceError, navigation::*},
};

pub(super) fn encode(values: &NavigationPreferences, output: &mut String) {
    for layout in values.accounts() {
        let collapsed: u8 = NavigationSection::ALL
            .iter()
            .filter(|s| layout.collapsed(**s))
            .map(|s| s.bit())
            .sum();
        let _ = writeln!(
            output,
            "layout_sections={}|{}",
            layout.account().as_str(),
            collapsed
        );
        for group in layout.groups() {
            let _ = write!(
                output,
                "layout_group={}|{}|{}|{}|",
                layout.account().as_str(),
                group.id.get(),
                group.order,
                u8::from(group.collapsed)
            );
            for byte in group.name.as_str().bytes() {
                let _ = write!(output, "{byte:02x}");
            }
            output.push('\n');
        }
        for room in layout.placements() {
            let group = room.group.map_or(0, NavigationGroupId::get);
            let favorite = match room.favorite {
                None => "-",
                Some(false) => "0",
                Some(true) => "1",
            };
            let _ = writeln!(
                output,
                "layout_room={}|{}|{}|{}|{}",
                layout.account().as_str(),
                room.room.as_str(),
                group,
                room.order,
                favorite
            );
        }
    }
}

pub(super) fn decode_row(
    values: &mut NavigationPreferences,
    key: &str,
    value: &str,
) -> Result<(), PreferenceError> {
    // Long rows fail before any split allocation; names have a 128-byte UTF-8 bound.
    if value.len() > 400 {
        return Err(PreferenceError::InvalidData);
    }
    let mut parts = value.split('|');
    let invalid = || PreferenceError::InvalidData;
    let account =
        NavigationAccountKey::new(parts.next().ok_or_else(invalid)?).map_err(|_| invalid())?;
    if key == "layout_sections" {
        if values.account(&account).is_some() {
            return Err(invalid());
        }
        let mask: u8 = parts
            .next()
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        if mask > 31 || parts.next().is_some() {
            return Err(invalid());
        }
        for section in NavigationSection::ALL {
            values
                .apply(
                    account.clone(),
                    RemoteNavigationSupport::Unsupported,
                    NavigationEdit::SetSectionCollapsed {
                        section,
                        collapsed: mask & section.bit() != 0,
                    },
                )
                .map_err(|_| invalid())?;
        }
        return Ok(());
    }
    if values.account(&account).is_none() {
        return Err(invalid());
    }
    let item = parts.next().ok_or_else(invalid)?;
    let third = parts.next().ok_or_else(invalid)?;
    let fourth = parts.next().ok_or_else(invalid)?;
    let fifth = parts.next().ok_or_else(invalid)?;
    if parts.next().is_some() {
        return Err(invalid());
    }
    let support = RemoteNavigationSupport::Unsupported;
    let apply = |values: &mut NavigationPreferences, edit| {
        values
            .apply(account.clone(), support, edit)
            .map_err(|_| invalid())
    };
    if key == "layout_group" {
        let id =
            NavigationGroupId::new(item.parse().map_err(|_| invalid())?).map_err(|_| invalid())?;
        if values
            .account(&account)
            .is_some_and(|a| a.groups().iter().any(|g| g.id == id))
        {
            return Err(invalid());
        }
        if fifth.is_empty()
            || fifth.len() > 256
            || fifth.len() % 2 != 0
            || !fifth.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid());
        }
        let mut bytes = Vec::with_capacity(fifth.len() / 2);
        for pair in fifth.as_bytes().as_chunks::<2>().0 {
            let high = (pair[0] as char).to_digit(16).ok_or_else(invalid)?;
            let low = (pair[1] as char).to_digit(16).ok_or_else(invalid)?;
            bytes.push((high * 16 + low) as u8);
        }
        apply(
            values,
            NavigationEdit::UpsertGroup(NavigationGroup {
                id,
                name: Text::new(std::str::from_utf8(&bytes).map_err(|_| invalid())?)
                    .map_err(|_| invalid())?,
                order: third.parse().map_err(|_| invalid())?,
                collapsed: super::boolean(fourth)?,
            }),
        )
    } else {
        let room = NavigationRoomKey::new(item).map_err(|_| invalid())?;
        if values
            .account(&account)
            .is_some_and(|a| a.placements().iter().any(|p| p.room == room))
        {
            return Err(invalid());
        }
        let group = match third {
            "0" => None,
            _ => Some(
                NavigationGroupId::new(third.parse().map_err(|_| invalid())?)
                    .map_err(|_| invalid())?,
            ),
        };
        let favorite = match fifth {
            "-" => None,
            _ => Some(super::boolean(fifth)?),
        };
        apply(
            values,
            NavigationEdit::PlaceRoom {
                room: room.clone(),
                group,
                order: fourth.parse().map_err(|_| invalid())?,
            },
        )?;
        apply(values, NavigationEdit::SetFavorite { room, favorite })
    }
}
