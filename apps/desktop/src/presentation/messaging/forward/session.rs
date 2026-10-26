//! One bounded approval. Projected identity and policy are rechecked at every activation.
use talk_core::ui::Decision;
use talk_core::ui::messaging::MessageFeature;
use talk_core::ui::presentation::{
    ContentState, ConversationRef, ConversationView, DeliveryState, MessageView,
    PresentationSnapshot,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Target {
    pub context: ConversationRef,
    pub message: u128,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Confirmation {
    pub target: Target,
    pub destination: ConversationRef,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Denial {
    Source,
    Connection,
    Destination,
    Selection,
}

pub(super) struct Approval {
    pub target: Target,
    // At most one projected message, already capped at 4096 UTF-8 bytes. Compare
    // source fields rather than reaction/read counters, which do not change it.
    pub original: MessageView,
    pub destination: Option<ConversationRef>,
}

#[derive(Default)]
pub(super) struct Session {
    generation: u64,
    pub active: Option<Approval>,
}
impl Session {
    pub fn open(&mut self, context: ConversationRef, message: &MessageView) -> Option<Target> {
        self.active = None;
        self.generation = self.generation.checked_add(1)?;
        let target = Target {
            context,
            message: message.local_id,
            generation: self.generation,
        };
        self.active = Some(Approval {
            target,
            original: message.clone(),
            destination: None,
        });
        Some(target)
    }
    pub fn target(&self) -> Option<Target> {
        self.active.as_ref().map(|approval| approval.target)
    }
    pub fn close(&mut self) {
        self.active = None;
    }
    pub fn select(&mut self, target: Target, destination: ConversationRef) -> bool {
        let Some(approval) = self.active.as_mut().filter(|a| a.target == target) else {
            return false;
        };
        if !same_account_destination(target.context, destination) {
            return false;
        }
        approval.destination = Some(destination);
        true
    }
    pub fn source_current(&self, snapshot: &PresentationSnapshot) -> bool {
        self.active.as_ref().is_some_and(|approval| {
            source(snapshot, approval.target.context, approval.target.message)
                .is_some_and(|current| same_source(&approval.original, current))
        })
    }
    pub fn reconcile(&mut self, snapshot: &PresentationSnapshot) {
        if self.active.is_some() && !self.source_current(snapshot) {
            self.close();
        }
    }
    pub fn confirm(
        &self,
        confirmation: Confirmation,
        snapshot: &PresentationSnapshot,
    ) -> Result<(), Denial> {
        let approval = self
            .active
            .as_ref()
            .filter(|a| a.target == confirmation.target)
            .ok_or(Denial::Source)?;
        if !self.source_current(snapshot) {
            return Err(Denial::Source);
        }
        if approval.destination != Some(confirmation.destination) {
            return Err(Denial::Selection);
        }
        let panel = snapshot.message_panel.as_ref().ok_or(Denial::Source)?;
        // Offline confirmation only admits an intent to the existing bounded
        // queue. It never grants send authority. The owner revalidates the
        // approved source and destination before dispatch after reconnect.
        let ready = snapshot.content == ContentState::Ready
            && snapshot.conversation_list.content == ContentState::Ready;
        let queued = panel.queue_available
            && matches!(
                snapshot.content,
                ContentState::Ready | ContentState::Offline
            )
            && matches!(
                snapshot.conversation_list.content,
                ContentState::Ready | ContentState::Offline
            );
        if !ready && !queued {
            return Err(Denial::Connection);
        }
        if !destinations(snapshot, confirmation.target.context)
            .any(|room| room.context == confirmation.destination)
        {
            return Err(Denial::Destination);
        }
        if panel.context != confirmation.target.context
            || panel.state.pending() && !panel.queue_available
        {
            return Err(Denial::Connection);
        }
        Ok(())
    }
}

pub(super) fn source(
    snapshot: &PresentationSnapshot,
    context: ConversationRef,
    message: u128,
) -> Option<&MessageView> {
    if !context.is_consistent()
        || snapshot.active_account != Some(context.account)
        || snapshot.selected != Some(context)
        || !matches!(
            snapshot.content,
            ContentState::Ready | ContentState::Offline
        )
        || !snapshot
            .accounts
            .as_slice()
            .iter()
            .any(|a| a.scope == context.account)
        || !snapshot.conversations.as_slice().iter().any(|room| {
            room.context == context
                && room.organization.classified == Some(false)
                && room.send == Decision::Allowed
        })
        || snapshot.message_panel.as_ref().is_none_or(|panel| {
            panel.context != context || !panel.availability(MessageFeature::Forward).enabled()
        })
    {
        return None;
    }
    snapshot.messages.as_slice().iter().find(|row| {
        row.local_id == message
            && !row.metadata.deleted
            && !row.metadata.system
            && !row.metadata.truncated
            && row.metadata.rich.as_slice().is_empty()
            && row.attachments.as_slice().is_empty()
            && matches!(
                row.delivery,
                DeliveryState::Received | DeliveryState::ServerAccepted
            )
    })
}

fn same_source(original: &MessageView, current: &MessageView) -> bool {
    original.local_id == current.local_id
        && original.author == current.author
        && original.body == current.body
        && original.metadata.author_key == current.metadata.author_key
        && original.metadata.timestamp == current.metadata.timestamp
        && original.metadata.edited == current.metadata.edited
        && original.metadata.expires_at == current.metadata.expires_at
        && original.metadata.local_operation == current.metadata.local_operation
}

fn same_account_destination(source: ConversationRef, destination: ConversationRef) -> bool {
    source.is_consistent()
        && destination.is_consistent()
        && source != destination
        && source.account == destination.account
        && source.conversation.host == destination.conversation.host
}

pub(super) fn destinations(
    snapshot: &PresentationSnapshot,
    source: ConversationRef,
) -> impl Iterator<Item = &ConversationView> {
    // The projection owns the 100-room cap. No network search or retained rows.
    snapshot
        .conversations
        .as_slice()
        .iter()
        .filter(move |room| {
            same_account_destination(source, room.context)
                && room.send == Decision::Allowed
                && room.organization.classified == Some(false)
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReturnSurface {
    Timeline,
    CallChat,
    Thread,
}
pub(super) fn return_surface(
    previous: ReturnSurface,
    source: ConversationRef,
    current: Option<ConversationRef>,
    owns_overlay: bool,
) -> Option<ReturnSurface> {
    (owns_overlay && current == Some(source)).then_some(previous)
}

/// Selection is explicit; navigation moves focus without changing approval.
pub(super) fn next_row(current: Option<usize>, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match key {
        "home" => Some(0),
        "end" => Some(count - 1),
        "up" => Some(current.unwrap_or(0).checked_sub(1).unwrap_or(count - 1)),
        "down" => Some(current.map_or(0, |index| (index + 1) % count)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;
