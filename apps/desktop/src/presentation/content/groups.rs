//! Pure media runs over one room's bounded message projection. A run changes
//! layout only; its members keep their own IDs, permissions and actions.
use talk_core::ui::presentation::{DeliveryState, MessageView};

const MAX_ROWS: usize = 100;
const MAX_GROUP: usize = 8;
const MAX_PINS: usize = 50;

/// Original row indices; `end` is exclusive. Only runs of two or more exist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::presentation) struct Span {
    pub start: usize,
    pub end: usize,
}

pub(in crate::presentation) fn spans(
    rows: &[MessageView],
    pins: &[u128],
    same_day: impl Fn(u64, u64) -> bool,
) -> Vec<Span> {
    // Unknown overflow must not conceal a pin that falls beyond the UI's
    // normal Page<_, 50> bound. Leave those messages individually rendered.
    if pins.len() > MAX_PINS {
        return Vec::new();
    }
    let mut spans = Vec::with_capacity(rows.len().min(MAX_ROWS) / 2);
    let mut start = 0;
    let mut length = 0;
    let mut previous: Option<&MessageView> = None;
    for (index, row) in rows.iter().take(MAX_ROWS).enumerate() {
        let eligible = eligible(row, pins);
        let continues = eligible
            && length < MAX_GROUP
            && previous.is_some_and(|previous| adjacent(previous, row, &same_day));
        if !continues {
            if length >= 2 {
                spans.push(Span {
                    start,
                    end: start + length,
                });
            }
            start = index;
            length = 0;
        }
        if eligible {
            length += 1;
            previous = Some(row);
        } else {
            previous = None;
        }
    }
    if length >= 2 {
        spans.push(Span {
            start,
            end: start + length,
        });
    }
    spans
}

fn adjacent(
    previous: &MessageView,
    current: &MessageView,
    same_day: &impl Fn(u64, u64) -> bool,
) -> bool {
    previous.own == current.own
        && previous.metadata.author_key == current.metadata.author_key
        && current
            .metadata
            .timestamp
            .checked_sub(previous.metadata.timestamp)
            .is_some_and(|gap| gap <= 120)
        && same_day(previous.metadata.timestamp, current.metadata.timestamp)
}

fn eligible(message: &MessageView, pins: &[u128]) -> bool {
    let meta = &message.metadata;
    if message.local_id == 0
        || !matches!(
            message.delivery,
            DeliveryState::Received | DeliveryState::ServerAccepted
        )
        || meta.deleted
        || meta.edited
        || meta.truncated
        || meta.timestamp == 0
        || meta.parent.is_some()
        || meta.thread.is_some()
        || message.reply_count != 0
        || !meta.reactions.as_slice().is_empty()
        || !message.attachments.as_slice().is_empty()
        || pins.contains(&message.local_id)
        || meta
            .author_key
            .as_ref()
            .is_none_or(|key| key.as_str().trim().is_empty())
    {
        return false;
    }
    let [file] = meta.rich.as_slice() else {
        return false;
    };
    if file.kind.as_str() != "file"
        || !file.mime_type.as_ref().is_some_and(|mime| {
            mime.as_str()
                .strip_prefix("image/")
                .or_else(|| mime.as_str().strip_prefix("video/"))
                .is_some_and(|subtype| !subtype.is_empty())
        })
    {
        return false;
    }
    let body = message.body.as_str().trim();
    // The generic system flag cannot distinguish a file-share template from
    // meaningful text. Keep captions intact even on system-tagged rows.
    body.is_empty()
        || body == file.name.as_str()
        || body
            .strip_prefix('{')
            .and_then(|body| body.strip_suffix('}'))
            .is_some_and(|key| !key.is_empty() && key == file.placeholder.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{
        Page, Text,
        messaging::{MessageMetadata, Reaction, Reply, RichObject},
    };

    fn photo(id: u128, timestamp: u64) -> MessageView {
        let mut rich = Page::empty();
        rich.try_push(RichObject {
            id: Some(Text::new(&id.to_string()).unwrap()),
            mime_type: Some(Text::new("image/jpeg").unwrap()),
            size: Some(100),
            placeholder: Text::new("file").unwrap(),
            kind: Text::new("file").unwrap(),
            name: Text::new("original.jpg").unwrap(),
            link: None,
        })
        .unwrap();
        MessageView {
            local_id: id,
            author: Text::new("Same display name").unwrap(),
            initials: Text::new("SD").unwrap(),
            time_label: Text::new("10:00").unwrap(),
            body: Text::new("{file}").unwrap(),
            own: false,
            delivery: DeliveryState::Received,
            attachments: Page::empty(),
            reply_count: 0,
            metadata: MessageMetadata {
                author_key: Some(Text::new("5:users:synthetic-one").unwrap()),
                timestamp,
                rich,
                ..Default::default()
            },
        }
    }
    fn group(rows: &[MessageView]) -> Vec<Span> {
        spans(rows, &[], |_, _| true)
    }

    #[test]
    fn two_adjacent_photos_group_by_actor_identity_without_replacing_member_ids() {
        let mut rows = vec![photo(1, 100), photo(2, 220)];
        rows[1].metadata.system = true;
        rows[1].body = Text::new("original.jpg").unwrap();
        assert_eq!(group(&rows), vec![Span { start: 0, end: 2 }]);
        assert_eq!(
            rows.iter().map(|row| row.local_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        rows[1].author = Text::new("Renamed display name").unwrap();
        assert_eq!(group(&rows), vec![Span { start: 0, end: 2 }]);
        rows[1].metadata.author_key = Some(Text::new("5:users:synthetic-two").unwrap());
        assert!(group(&rows).is_empty());
        rows[1].author = rows[0].author.clone();
        assert!(
            group(&rows).is_empty(),
            "equal display names never establish identity"
        );
    }

    #[test]
    fn runs_are_disjoint_at_most_eight_members_and_only_first_hundred_rows_are_examined() {
        let rows: Vec<_> = (1..=101).map(|id| photo(id, id as u64)).collect();
        let result = group(&rows);
        let expected: Vec<_> = (0..=11)
            .map(|batch| Span {
                start: batch * 8,
                end: (batch + 1) * 8,
            })
            .chain([Span {
                start: 96,
                end: 100,
            }])
            .collect();
        assert_eq!(result, expected);
        assert!(result.windows(2).all(|pair| pair[0].end <= pair[1].start));
        assert!(group(&rows[..1]).is_empty());
        assert!(group(&[]).is_empty());
    }

    #[test]
    fn author_own_day_and_forward_time_boundaries_split_runs() {
        let first = photo(1, 100);
        for timestamp in [99, 221] {
            assert!(group(&[first.clone(), photo(2, timestamp)]).is_empty());
        }
        assert!(spans(&[first.clone(), photo(2, 101)], &[], |_, _| false).is_empty());
        for key in [None, Some(""), Some(" "), Some("5:guests:synthetic-one")] {
            let mut second = photo(2, 101);
            second.metadata.author_key = key.map(|key| Text::new(key).unwrap());
            assert!(group(&[first.clone(), second]).is_empty());
        }
        let mut second = photo(2, 101);
        second.own = true;
        assert!(group(&[first, second]).is_empty());
    }

    #[test]
    fn captions_quotes_threads_replies_edits_reactions_and_noncanonical_rows_are_barriers() {
        let original = photo(2, 101);
        let mut barriers = Vec::new();
        for state in [
            DeliveryState::Pending,
            DeliveryState::Failed,
            DeliveryState::Uncertain,
        ] {
            let mut row = original.clone();
            row.delivery = state;
            barriers.push(row);
        }
        for field in 0..9 {
            let mut row = original.clone();
            match field {
                0 => row.metadata.deleted = true,
                1 => row.metadata.edited = true,
                2 => row.metadata.truncated = true,
                3 => row.metadata.thread = Some(7),
                4 => row.reply_count = 1,
                5 => {
                    row.metadata.parent = Some(Reply {
                        id: 7,
                        author: Text::new("Author").unwrap(),
                        text: Text::new("Quote").unwrap(),
                        unavailable: false,
                    })
                }
                6 => row
                    .metadata
                    .reactions
                    .try_push(Reaction {
                        emoji: Text::new("👍").unwrap(),
                        count: 1,
                        own: false,
                    })
                    .unwrap(),
                7 => row.body = Text::new("A meaningful caption {file}").unwrap(),
                8 => {
                    row.metadata.system = true;
                    row.body = Text::new("Meaningful system text {file}").unwrap();
                }
                _ => unreachable!(),
            }
            barriers.push(row);
        }
        for barrier in barriers {
            assert!(group(&[photo(1, 100), barrier, photo(3, 102)]).is_empty());
        }
    }

    #[test]
    fn pins_and_nonmedia_or_additional_content_remain_separate() {
        let rows = [photo(1, 100), photo(2, 101), photo(3, 102), photo(4, 103)];
        assert_eq!(
            spans(&rows, &[2], |_, _| true),
            vec![Span { start: 2, end: 4 }]
        );
        assert!(spans(&rows, &[0; MAX_PINS + 1], |_, _| true).is_empty());
        for mime in ["application/pdf", "image/", "", "text/plain"] {
            let mut row = photo(2, 101);
            let mut object = row.metadata.rich.as_slice().first().unwrap().clone();
            object.mime_type = Some(Text::new(mime).unwrap());
            row.metadata.rich = Page::empty();
            row.metadata.rich.try_push(object).unwrap();
            assert!(group(&[photo(1, 100), row]).is_empty());
        }
        let mut row = photo(2, 101);
        let object = row.metadata.rich.as_slice().first().unwrap().clone();
        row.metadata.rich.try_push(object).unwrap();
        assert!(group(&[photo(1, 100), row]).is_empty());
        let mut video = photo(2, 101);
        let mut object = video.metadata.rich.as_slice().first().unwrap().clone();
        object.mime_type = Some(Text::new("video/mp4").unwrap());
        video.metadata.rich = Page::empty();
        video.metadata.rich.try_push(object).unwrap();
        video.body = Text::new("").unwrap();
        assert_eq!(
            group(&[photo(1, 100), video]),
            vec![Span { start: 0, end: 2 }]
        );
    }
}
