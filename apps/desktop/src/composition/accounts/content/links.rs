//! A single, short-lived deliberate web handoff. A rendered label never grants
//! access: resolve against the current source again immediately before opening.
use super::*;
use talk_core::ui::presentation::PresentationSnapshot;

pub(super) struct Pending {
    context: ConversationRef,
    message: u128,
    target: Target,
    expires: Instant,
}

enum Target {
    Message {
        source: Text<4096>,
        ordinal: u16,
    },
    Location {
        placeholder: Text<128>,
        id: Text<1024>,
    },
}

fn destination(
    snapshot: &PresentationSnapshot,
    context: ConversationRef,
    message: u128,
    source: &Text<4096>,
    ordinal: u16,
    now: u64,
) -> Result<Text<2048>, SubmitError> {
    let row = accessible_message(snapshot, context, message, now)?;
    if row.body != *source {
        return Err(SubmitError::StaleScope);
    }
    talk_core::ui::markup::parse(source.as_str())
        .links
        .as_slice()
        .get(usize::from(ordinal))
        .map(|link| link.destination.clone())
        .ok_or(SubmitError::Unavailable(UnavailableReason::Unsupported))
}

fn location_destination(
    snapshot: &PresentationSnapshot,
    context: ConversationRef,
    message: u128,
    placeholder: &Text<128>,
    id: &Text<1024>,
    now: u64,
) -> Result<Text<2048>, SubmitError> {
    let row = accessible_message(snapshot, context, message, now)?;
    let current = row.metadata.rich.as_slice().iter().any(|object| {
        object.kind.as_str() == "geo-location"
            && object.placeholder == *placeholder
            && object.id.as_ref() == Some(id)
    });
    if !current {
        return Err(SubmitError::StaleScope);
    }
    let (lat, lon) = talk_core::ui::markup::location_coordinates(id.as_str())
        .ok_or(SubmitError::Unavailable(UnavailableReason::Unsupported))?;
    Text::new(&format!(
        "https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=16/{lat}/{lon}"
    ))
    .map_err(|_| SubmitError::Unavailable(UnavailableReason::Unsupported))
}

fn accessible_message(
    snapshot: &PresentationSnapshot,
    context: ConversationRef,
    message: u128,
    now: u64,
) -> Result<&talk_core::ui::presentation::MessageView, SubmitError> {
    if snapshot.active_account != Some(context.account)
        || snapshot.selected != Some(context)
        || snapshot.content != ContentState::Ready
        || !context.is_consistent()
    {
        return Err(SubmitError::StaleScope);
    }
    if !snapshot
        .conversations
        .as_slice()
        .iter()
        .any(|room| room.context == context && room.draft_clipboard == Decision::Allowed)
    {
        return Err(SubmitError::Unavailable(
            UnavailableReason::PermissionDenied,
        ));
    }
    let row = snapshot
        .messages
        .as_slice()
        .iter()
        .chain(
            snapshot
                .message_panel
                .as_ref()
                .filter(|p| p.context == context)
                .into_iter()
                .flat_map(|p| {
                    p.shared_files
                        .as_slice()
                        .iter()
                        .chain(p.thread_messages.as_slice())
                        .chain(p.pinned_messages.as_slice())
                }),
        )
        .find(|row| row.local_id == message)
        .ok_or(SubmitError::StaleScope)?;
    if row.metadata.deleted
        || row
            .metadata
            .expires_at
            .is_some_and(|at| at != 0 && at <= now)
    {
        return Err(SubmitError::StaleScope);
    }
    Ok(row)
}

impl Accounts {
    pub(crate) fn open_message_link(
        &mut self,
        context: ConversationRef,
        message: u128,
        source: Text<4096>,
        ordinal: u16,
    ) -> Result<(), SubmitError> {
        self.current(context.account)?;
        if self
            .content_owner
            .message_link
            .as_ref()
            .is_some_and(|p| p.expires > Instant::now())
        {
            return Err(SubmitError::Busy);
        }
        destination(
            &self.snapshot,
            context,
            message,
            &source,
            ordinal,
            history::now_unix(),
        )?;
        self.content_owner.message_link = Some(Pending {
            context,
            message,
            target: Target::Message { source, ordinal },
            expires: Instant::now() + Duration::from_secs(2),
        });
        Ok(())
    }

    pub(crate) fn open_location(
        &mut self,
        context: ConversationRef,
        message: u128,
        placeholder: Text<128>,
        id: Text<1024>,
    ) -> Result<(), SubmitError> {
        self.current(context.account)?;
        if self
            .content_owner
            .message_link
            .as_ref()
            .is_some_and(|p| p.expires > Instant::now())
        {
            return Err(SubmitError::Busy);
        }
        location_destination(
            &self.snapshot,
            context,
            message,
            &placeholder,
            &id,
            history::now_unix(),
        )?;
        self.content_owner.message_link = Some(Pending {
            context,
            message,
            target: Target::Location { placeholder, id },
            expires: Instant::now() + Duration::from_secs(2),
        });
        Ok(())
    }

    pub(crate) fn take_message_link(&mut self) -> Option<Text<2048>> {
        let pending = self.content_owner.message_link.take()?;
        if pending.expires <= Instant::now() || self.current(pending.context.account).is_err() {
            return None;
        }
        match pending.target {
            Target::Message { source, ordinal } => destination(
                &self.snapshot,
                pending.context,
                pending.message,
                &source,
                ordinal,
                history::now_unix(),
            ),
            Target::Location { placeholder, id } => location_destination(
                &self.snapshot,
                pending.context,
                pending.message,
                &placeholder,
                &id,
                history::now_unix(),
            ),
        }
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> (PresentationSnapshot, ConversationRef, Text<4096>) {
        let mut state = crate::composition::fixtures::snapshot(Some("chat"));
        let context = state.selected.unwrap();
        state.content = ContentState::Ready;
        state.conversations = page(state.conversations.as_slice().iter().cloned().map(
            |mut row| {
                row.draft_clipboard = Decision::Allowed;
                row
            },
        ));
        let mut row = state.messages.as_slice()[0].clone();
        row.local_id = 77;
        row.body = text(
            "See [the guide](https://example.invalid/guide) and https://example.invalid/help.",
        );
        row.metadata.deleted = false;
        row.metadata.expires_at = None;
        let source = row.body.clone();
        state.messages = page([row]);
        (state, context, source)
    }

    #[test]
    fn activation_uses_current_body_and_exact_link_ordinal_without_accepting_a_view_url() {
        let (mut state, context, source) = snapshot();
        assert_eq!(
            destination(&state, context, 77, &source, 0, 100)
                .unwrap()
                .as_str(),
            "https://example.invalid/guide"
        );
        assert_eq!(
            destination(&state, context, 77, &source, 1, 100)
                .unwrap()
                .as_str(),
            "https://example.invalid/help"
        );
        assert!(destination(&state, context, 77, &source, 2, 100).is_err());
        assert!(destination(&state, context, 78, &source, 0, 100).is_err());
        let mut edited = state.messages.as_slice()[0].clone();
        edited.body = text("https://example.invalid/changed");
        state.messages = page([edited]);
        assert!(destination(&state, context, 77, &source, 0, 100).is_err());
    }

    #[test]
    fn location_handoff_revalidates_object_and_never_uses_a_server_url() {
        let (mut state, context, _) = snapshot();
        let mut row = state.messages.as_slice()[0].clone();
        let placeholder = text("place");
        let id = text("geo:52.5,13.4");
        row.metadata.rich = page([talk_core::ui::messaging::RichObject {
            placeholder: placeholder.clone(),
            kind: text("geo-location"),
            name: text("Fixture"),
            id: Some(id.clone()),
            mime_type: None,
            size: None,
            link: Some(text("https://evil.invalid/")),
        }]);
        state.messages = page([row.clone()]);
        assert_eq!(
            location_destination(&state, context, 77, &placeholder, &id, 100)
                .unwrap()
                .as_str(),
            "https://www.openstreetmap.org/?mlat=52.5&mlon=13.4#map=16/52.5/13.4"
        );
        row.metadata.rich = Page::empty();
        state.messages = page([row]);
        assert!(location_destination(&state, context, 77, &placeholder, &id, 100).is_err());
    }

    #[test]
    fn stale_context_denial_expiry_and_main_tombstone_cannot_open_a_secondary_link() {
        let (state, context, source) = snapshot();
        for case in 0..6 {
            let mut changed = state.clone();
            match case {
                0 => changed.active_account = None,
                1 => changed.selected = None,
                2 => changed.content = ContentState::Denied,
                3 => {
                    changed.conversations = page(
                        changed
                            .conversations
                            .as_slice()
                            .iter()
                            .cloned()
                            .map(|mut room| {
                                room.draft_clipboard = Decision::Denied;
                                room
                            }),
                    )
                }
                4 | 5 => {
                    let mut panel = talk_core::ui::messaging::MessagePanel::new(context);
                    panel.pinned_messages = page(changed.messages.as_slice().iter().cloned());
                    changed.message_panel = Some(panel);
                    let mut row = changed.messages.as_slice()[0].clone();
                    if case == 4 {
                        row.metadata.deleted = true;
                    } else {
                        row.metadata.expires_at = Some(100);
                    }
                    changed.messages = page([row]);
                }
                _ => unreachable!(),
            }
            assert!(destination(&changed, context, 77, &source, 0, 100).is_err());
        }
    }
}
