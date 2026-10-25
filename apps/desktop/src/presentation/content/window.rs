//! A finite navigation window over already loaded, room-scoped attachments.
//! Main history dominates secondary copies even when its file was removed.
use talk_core::ui::{
    Text,
    content::AttachmentRef,
    presentation::{ContentState, ConversationRef, MessageView, PresentationSnapshot},
};

const MAX_ITEMS: usize = 128;
const BEFORE_CURRENT: usize = 64;

pub(in crate::presentation) fn entries(
    snapshot: &PresentationSnapshot,
    current: Option<&AttachmentRef>,
) -> Vec<AttachmentRef> {
    let Some(context) = context(snapshot) else {
        return Vec::new();
    };
    let (position, total) = location(snapshot, context, current);
    let start = position.map_or(0, |index| {
        index
            .saturating_sub(BEFORE_CURRENT)
            .min(total.saturating_sub(MAX_ITEMS))
    });
    let mut window = Vec::with_capacity(MAX_ITEMS);
    for (message, placeholder) in files(snapshot, context).skip(start).take(MAX_ITEMS) {
        window.push(AttachmentRef {
            context,
            message,
            placeholder: placeholder.clone(),
        });
    }
    window
}

/// Zero-based position in the complete deduplicated loaded collection, plus its
/// total. A clipped navigation window must not become a misleading file count.
pub(in crate::presentation) fn global_position(
    snapshot: &PresentationSnapshot,
    current: &AttachmentRef,
) -> Option<(usize, usize)> {
    let context = context(snapshot)?;
    let (position, total) = location(snapshot, context, Some(current));
    position.map(|position| (position, total))
}

fn context(snapshot: &PresentationSnapshot) -> Option<ConversationRef> {
    snapshot.selected.filter(|context| {
        context.is_consistent()
            && snapshot.active_account == Some(context.account)
            && snapshot.content != ContentState::Denied
    })
}

fn location(
    snapshot: &PresentationSnapshot,
    context: ConversationRef,
    current: Option<&AttachmentRef>,
) -> (Option<usize>, usize) {
    let mut position = None;
    let mut total = 0;
    for (message, placeholder) in files(snapshot, context) {
        if current.is_some_and(|current| {
            current.context == context
                && current.message == message
                && current.placeholder == *placeholder
        }) {
            position = Some(total);
        }
        total += 1;
    }
    (position, total)
}

fn files(
    snapshot: &PresentationSnapshot,
    context: ConversationRef,
) -> impl Iterator<Item = (u128, &Text<128>)> {
    let main = snapshot.messages.as_slice();
    let panel = snapshot
        .message_panel
        .as_ref()
        .filter(|panel| panel.context == context);
    let shared = panel.map_or(&[][..], |panel| panel.shared_files.as_slice());
    let thread = panel.map_or(&[][..], |panel| panel.thread_messages.as_slice());
    let pins = panel.map_or(&[][..], |panel| panel.pinned_messages.as_slice());
    // The pages already bound this scan to 250 rows with 32 rich objects each.
    // Iterator chains and borrowed placeholders avoid an all-files allocation.
    distinct(main, [&[], &[], &[]])
        .chain(distinct(shared, [main, &[], &[]]))
        .chain(distinct(thread, [main, shared, &[]]))
        .chain(distinct(pins, [main, shared, thread]))
        .filter(|row| row.local_id != 0 && !row.metadata.deleted)
        .flat_map(|row| {
            let rich = row.metadata.rich.as_slice();
            rich.iter().enumerate().filter_map(move |(index, object)| {
                (object.kind.as_str() == "file"
                    && !rich.iter().take(index).any(|previous| {
                        previous.kind.as_str() == "file"
                            && previous.placeholder == object.placeholder
                    }))
                .then_some((row.local_id, &object.placeholder))
            })
        })
}

fn distinct<'a>(
    rows: &'a [MessageView],
    higher: [&'a [MessageView]; 3],
) -> impl Iterator<Item = &'a MessageView> {
    rows.iter().enumerate().filter_map(move |(index, row)| {
        // Presence, not the number of surviving rich objects, owns this ID.
        // A canonical tombstone or removed attachment therefore cannot fall
        // back to a stale shared/thread/pinned copy.
        (!rows
            .iter()
            .take(index)
            .any(|previous| previous.local_id == row.local_id)
            && !higher.iter().any(|rows| {
                rows.iter()
                    .any(|previous| previous.local_id == row.local_id)
            }))
        .then_some(row)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{
        AccountId, AccountScope, ConversationId, HostId, Page, RuntimeGeneration,
        messaging::{MessageMetadata, MessagePanel, RichObject},
        presentation::{
            ConversationListState, DeliveryState, LoginPhase, LoginView, NativeStoreState,
            StorageStatus,
        },
    };

    fn room(account: u128, conversation: u128) -> ConversationRef {
        let account = AccountId::from_local(account);
        ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: conversation,
            },
        }
    }
    fn snapshot() -> PresentationSnapshot {
        let context = room(1, 1);
        PresentationSnapshot {
            content_panel: Default::default(),
            create_panel: None,
            revision: 1,
            login: LoginView {
                phase: LoginPhase::Ready,
                attempt: None,
                server_label: Text::new("").unwrap(),
                account_label: Text::new("").unwrap(),
                native_store: NativeStoreState::Unknown,
                storage: StorageStatus::NotSaved,
                storage_failure: None,
            },
            accounts: Page::empty(),
            saved_accounts: Page::empty(),
            account_removal: None,
            active_account: Some(context.account),
            conversations: Page::empty(),
            conversation_list: ConversationListState {
                content: ContentState::Ready,
                complete: true,
            },
            tags: Page::empty(),
            tag_state: ContentState::Ready,
            organization_result: None,
            selected: Some(context),
            content: ContentState::Ready,
            messages: Page::empty(),
            files: Page::empty(),
            participants: Page::empty(),
            conversation_panel: None,
            message_panel: Some(MessagePanel::new(context)),
            call: None,
            submission: None,
            mutation: None,
        }
    }
    fn message(id: u128, files: usize) -> MessageView {
        let mut rich = Page::empty();
        for index in 0..files {
            rich.try_push(RichObject {
                id: Some(Text::new(&id.to_string()).unwrap()),
                mime_type: Some(Text::new("image/jpeg").unwrap()),
                size: Some(100),
                placeholder: Text::new(&format!("file-{index}")).unwrap(),
                kind: Text::new("file").unwrap(),
                name: Text::new("same filename.jpg").unwrap(),
                link: None,
            })
            .unwrap();
        }
        MessageView {
            local_id: id,
            author: Text::new("Synthetic").unwrap(),
            initials: Text::new("S").unwrap(),
            time_label: Text::new("10:00").unwrap(),
            body: Text::new("").unwrap(),
            own: false,
            delivery: DeliveryState::Received,
            attachments: Page::empty(),
            reply_count: 0,
            metadata: MessageMetadata {
                rich,
                ..Default::default()
            },
        }
    }
    fn reference(message: u128, index: usize) -> AttachmentRef {
        AttachmentRef {
            context: room(1, 1),
            message,
            placeholder: Text::new(&format!("file-{index}")).unwrap(),
        }
    }

    #[test]
    fn last_and_middle_files_beyond_first_window_keep_current_and_adjacent_neighbors() {
        let mut state = snapshot();
        for id in 1..=100 {
            state.messages.try_push(message(id, 3)).unwrap();
        }
        let last = reference(100, 2);
        let window = entries(&state, Some(&last));
        assert_eq!(window.len(), MAX_ITEMS);
        assert_eq!(window.last(), Some(&last));
        assert_eq!(window.iter().rev().nth(1), Some(&reference(100, 1)));
        assert_eq!(global_position(&state, &last), Some((299, 300)));
        let middle = reference(51, 0);
        let window = entries(&state, Some(&middle));
        assert_eq!(
            window.iter().position(|item| item == &middle),
            Some(BEFORE_CURRENT)
        );
        assert_eq!(window.len(), MAX_ITEMS);
        assert_eq!(global_position(&state, &middle), Some((150, 300)));
        assert_eq!(entries(&state, None).first(), Some(&reference(1, 0)));
        state
            .message_panel
            .as_mut()
            .unwrap()
            .pinned_messages
            .try_push(message(101, 2))
            .unwrap();
        let last_pin = reference(101, 1);
        assert_eq!(entries(&state, Some(&last_pin)).last(), Some(&last_pin));
        assert_eq!(global_position(&state, &last_pin), Some((301, 302)));
    }

    #[test]
    fn main_tombstone_or_removed_rich_content_never_falls_back_to_secondary_copies() {
        for deleted in [false, true] {
            let mut state = snapshot();
            let stale = message(1, 1);
            let mut main = stale.clone();
            if deleted {
                main.metadata.deleted = true;
            } else {
                main.metadata.rich = Page::empty();
            }
            state.messages.try_push(main).unwrap();
            let panel = state.message_panel.as_mut().unwrap();
            panel.shared_files.try_push(stale.clone()).unwrap();
            panel.thread_messages.try_push(stale.clone()).unwrap();
            panel.pinned_messages.try_push(stale).unwrap();
            assert!(entries(&state, Some(&reference(1, 0))).is_empty());
            assert_eq!(global_position(&state, &reference(1, 0)), None);
        }
    }

    #[test]
    fn exact_reference_identity_deduplicates_without_using_the_filename() {
        let mut state = snapshot();
        let mut main = message(1, 2);
        let duplicate = main.metadata.rich.as_slice().first().unwrap().clone();
        main.metadata.rich.try_push(duplicate).unwrap();
        state.messages.try_push(main.clone()).unwrap();
        let panel = state.message_panel.as_mut().unwrap();
        panel.shared_files.try_push(main).unwrap();
        panel.shared_files.try_push(message(2, 1)).unwrap();
        panel.thread_messages.try_push(message(2, 2)).unwrap();
        panel.thread_messages.try_push(message(3, 1)).unwrap();
        panel.pinned_messages.try_push(message(4, 1)).unwrap();
        assert_eq!(
            entries(&state, None),
            vec![
                reference(1, 0),
                reference(1, 1),
                reference(2, 0),
                reference(3, 0),
                reference(4, 0)
            ]
        );
    }

    #[test]
    fn denied_and_mismatched_account_or_conversation_cannot_contribute_entries() {
        let mut state = snapshot();
        state.messages.try_push(message(1, 1)).unwrap();
        for foreign_context in [room(2, 2), room(1, 2)] {
            let mut foreign = MessagePanel::new(foreign_context);
            foreign.shared_files.try_push(message(2, 1)).unwrap();
            state.message_panel = Some(foreign);
            assert_eq!(entries(&state, None), vec![reference(1, 0)]);
        }
        let mut current = reference(1, 0);
        current.context = room(2, 2);
        assert_eq!(entries(&state, Some(&current)), vec![reference(1, 0)]);
        assert_eq!(global_position(&state, &current), None);
        state.active_account = Some(room(2, 1).account);
        assert!(entries(&state, None).is_empty());
        state.active_account = Some(room(1, 1).account);
        state.content = ContentState::Denied;
        assert!(entries(&state, None).is_empty());
        state.content = ContentState::Offline;
        assert_eq!(entries(&state, None), vec![reference(1, 0)]);
    }
}
