//! One bounded history row in the versioned computer preference file.
use talk_core::{
    bounded::Text,
    preferences::{
        PreferenceError,
        reactions::{RECENT_REACTIONS, RecentReactions},
    },
};

pub(super) fn encode(recent: &RecentReactions, output: &mut String) {
    use std::fmt::Write;
    output.push_str("recent_reactions=");
    if recent.as_slice().is_empty() {
        output.push_str("none");
    } else {
        for (index, emoji) in recent.as_slice().iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            for byte in emoji.as_str().bytes() {
                let _ = write!(output, "{byte:02x}");
            }
        }
    }
    output.push('\n');
}

pub(super) fn decode(value: &str) -> Result<RecentReactions, PreferenceError> {
    if value == "none" {
        return Ok(RecentReactions::default());
    }
    if value.len() > RECENT_REACTIONS * 128 + RECENT_REACTIONS - 1 {
        return Err(PreferenceError::TooLarge);
    }
    let mut keys = Vec::with_capacity(RECENT_REACTIONS);
    for hex in value.split(',') {
        if keys.len() == RECENT_REACTIONS || hex.is_empty() || hex.len() > 128 || hex.len() % 2 != 0
        {
            return Err(PreferenceError::InvalidData);
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for pair in hex.as_bytes().as_chunks::<2>().0 {
            let high = (pair[0] as char)
                .to_digit(16)
                .ok_or(PreferenceError::InvalidData)?;
            let low = (pair[1] as char)
                .to_digit(16)
                .ok_or(PreferenceError::InvalidData)?;
            bytes.push((high * 16 + low) as u8);
        }
        let emoji = std::str::from_utf8(&bytes).map_err(|_| PreferenceError::InvalidData)?;
        keys.push(Text::new(emoji).map_err(|_| PreferenceError::InvalidData)?);
    }
    RecentReactions::from_ordered(keys).map_err(|_| PreferenceError::InvalidData)
}
