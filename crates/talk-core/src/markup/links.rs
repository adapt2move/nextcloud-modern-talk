use std::ops::Range;

use url::Url;

use super::{MAX_LINKS, Style, Text};

const MAX_DESTINATION_BYTES: usize = 2048;

/// Validate an explicit web destination or a `www.` address. No I/O occurs.
/// Reject length, credentials and ambiguous/control characters before URL
/// canonicalization; the canonical ASCII/IDN destination is bounded again.
pub fn web_destination(raw: &str) -> Option<Text<2048>> {
    if raw.len() > MAX_DESTINATION_BYTES || raw.chars().any(forbidden) {
        return None;
    }
    if encoded_forbidden(raw) {
        return None;
    }
    let www = prefix(raw, "www.");
    if www && raw.len().checked_add(8)? > MAX_DESTINATION_BYTES {
        return None;
    }
    let input;
    let raw = if www {
        input = format!("https://{raw}");
        input.as_str()
    } else {
        raw
    };
    let scheme_end = raw.find("://")?;
    let scheme = raw.get(..scheme_end)?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = raw.get(scheme_end + 3..)?.split(['/', '?', '#']).next()?;
    // Url normalizes empty userinfo away, so reject even `https://@host/` here.
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?;
    if host.is_empty() || (www && host.trim_end_matches('.').eq_ignore_ascii_case("www")) {
        return None;
    }
    Text::new(url.as_str()).ok()
}

fn forbidden(character: char) -> bool {
    character.is_control() || character.is_whitespace() || character == '\\' || bidi(character)
}

fn bidi(character: char) -> bool {
    matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn encoded_forbidden(raw: &str) -> bool {
    // At most 2048 bytes, checked before this buffer is allocated. Inspect
    // decoded bytes as well: URL canonicalization retains encoded controls.
    let mut decoded = Vec::with_capacity(raw.len());
    let mut input = raw.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let mut lookahead = input.clone();
            if let (Some(high), Some(low)) = (
                lookahead.next().and_then(hex),
                lookahead.next().and_then(hex),
            ) {
                decoded.push(high * 16 + low);
                input = lookahead;
                continue;
            }
        }
        decoded.push(byte);
    }
    decoded
        .iter()
        .any(|byte| byte.is_ascii_control() || *byte == b'\\')
        || decoded.windows(2).any(|bytes| bytes == [0xd8, 0x9c])
        || decoded.windows(3).any(|bytes| {
            matches!(
                bytes,
                [0xe2, 0x80, 0x8e | 0x8f | 0xaa..=0xae] | [0xe2, 0x81, 0xa6..=0xa9]
            )
        })
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn prefix(text: &str, candidate: &str) -> bool {
    text.get(..candidate.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(candidate))
}

pub(super) struct Recognized {
    pub range: Range<usize>,
    pub destination: Text<2048>,
}

/// A mask prevents both explicit Markdown labels and code/HTML/image text
/// from being reinterpreted as bare URLs. It also lets display recognition
/// join text events separated by character-entity decoding.
pub(super) fn recognize(text: &str, allowed: &[bool]) -> Option<Vec<Recognized>> {
    let mut links = Vec::new();
    let mut consumed = 0;
    for (start, character) in text.char_indices() {
        if start < consumed || !matches!(character, 'h' | 'H' | 'w' | 'W') {
            continue;
        }
        if text
            .get(..start)?
            .chars()
            .next_back()
            .is_some_and(|previous| previous.is_alphanumeric() || matches!(previous, '_' | '@'))
        {
            continue;
        }
        let remaining = text.get(start..)?;
        if allowed.get(start) != Some(&true) {
            continue;
        }
        if !prefix(remaining, "https://")
            && !prefix(remaining, "http://")
            && !prefix(remaining, "www.")
        {
            continue;
        }
        let length = remaining
            .char_indices()
            .find_map(|(offset, character)| {
                (character.is_whitespace()
                    || character.is_control()
                    || matches!(
                        character,
                        '<' | '>' | '"' | '\'' | '`' | '\\' | '“' | '”' | '‘' | '’'
                    ))
                .then_some(offset)
            })
            .unwrap_or(remaining.len());
        let permitted = allowed
            .get(start..)?
            .iter()
            .position(|value| !*value)
            .unwrap_or(remaining.len());
        let candidate = trim_end(remaining.get(..length.min(permitted))?);
        let end = start + candidate.len();
        if candidate.is_empty() || !allowed.get(start..end)?.iter().all(|allowed| *allowed) {
            continue;
        }
        let Some(destination) = web_destination(candidate) else {
            continue;
        };
        if links.len() == MAX_LINKS {
            return None;
        }
        links.push(Recognized {
            range: start..end,
            destination,
        });
        consumed = end;
    }
    Some(links)
}

fn trim_end(mut text: &str) -> &str {
    loop {
        let Some(last) = text.chars().next_back() else {
            return text;
        };
        let unbalanced = match last {
            ')' => unbalanced(text, '(', ')'),
            ']' => unbalanced(text, '[', ']'),
            '}' => unbalanced(text, '{', '}'),
            _ => false,
        };
        if !unbalanced && !matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '…') {
            return text;
        }
        text = text.get(..text.len() - last.len_utf8()).unwrap_or_default();
    }
}

fn unbalanced(text: &str, opening: char, closing: char) -> bool {
    text.chars()
        .filter(|character| *character == closing)
        .count()
        > text
            .chars()
            .filter(|character| *character == opening)
            .count()
}

pub(super) fn mark_link(styles: &mut [Style], range: Range<usize>) -> Option<()> {
    for style in styles.get_mut(range)? {
        style.link = true;
    }
    Some(())
}
