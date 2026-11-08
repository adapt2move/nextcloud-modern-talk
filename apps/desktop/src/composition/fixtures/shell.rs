//! Fictional navigation and status fixtures for the application shell:
//! a denser sidebar, the Activity page, list menus and scoped error states.
//! No server, credential or media operation occurs.
use super::{call_scope, context, page, populate, snapshot, text};
use talk_core::ui::Decision;
use talk_core::ui::presentation::{
    AccountReminderView, AccountRemindersView, ConferencePreset, ContentState,
    ConversationCategory, ConversationKind, ConversationView, FeatureSupport,
    FederationInvitationView, FederationInvitationsView, PresentationSnapshot, ThreadView,
};

/// A realistic day: favorites, several channels with unread counts, direct
/// messages and one archived room. Built on the ordinary `chat` fixture.
fn rich() -> PresentationSnapshot {
    let mut state = snapshot(Some("chat"));
    let template = state.conversations.as_slice()[0].clone();
    let room = |id: u128,
                title: &str,
                initials: &str,
                kind: ConversationKind,
                category: ConversationCategory,
                unread: u32,
                mention: bool,
                archived: bool| {
        let mut row: ConversationView = template.clone();
        row.context = context(id);
        row.navigation_key = None;
        row.title = text(title);
        row.initials = text(initials);
        row.subtitle = text(match id {
            11 => "Jana: Neue Kampagne ab Montag, bitte kurz prüfen.",
            12 => "Tim: Der Build läuft wieder grün.",
            13 => "Bot: 12 neue Tickets in der Warteschlange.",
            14 => "Moderation: Agenda für Donnerstag ist online.",
            15 => "Lea: Hast du die Folien schon gesehen?",
            _ => "Archiviert im Frühjahr.",
        });
        row.kind = kind;
        row.favorite = Some(false);
        row.organization.category = category;
        row.unread = unread;
        row.unread_mention = Some(mention);
        row.unread_direct_mention = Some(mention && unread > 0);
        row.last_read_message = None;
        row.archived = Some(archived);
        row.notification_muted = Some(false);
        row.last_activity = Some(90 - id as u64);
        row.call_active = Some(false);
        row.call_scope = Some(call_scope(id));
        row.presence = if kind == ConversationKind::Direct {
            talk_core::ui::presentation::Presence::Available
        } else {
            talk_core::ui::presentation::Presence::Unknown
        };
        row
    };
    let channel = ConversationCategory::Channel;
    let none = ConversationCategory::Unknown;
    let extra = [
        room(
            11,
            "Marketing",
            "MK",
            ConversationKind::Group,
            channel,
            5,
            true,
            false,
        ),
        room(
            12,
            "Engineering",
            "EN",
            ConversationKind::Group,
            channel,
            0,
            false,
            false,
        ),
        room(
            13,
            "Support-Queue",
            "SQ",
            ConversationKind::Group,
            channel,
            12,
            false,
            false,
        ),
        room(
            14,
            "Release 4.2",
            "R4",
            ConversationKind::Group,
            ConversationCategory::Conference(ConferencePreset::Standard),
            0,
            false,
            false,
        ),
        room(
            15,
            "Lea Neumann",
            "LN",
            ConversationKind::Direct,
            none,
            1,
            false,
            false,
        ),
        room(
            16,
            "Alt-Projekt 2024",
            "AP",
            ConversationKind::Group,
            channel,
            0,
            false,
            true,
        ),
    ];
    state.conversations = page(
        state
            .conversations
            .as_slice()
            .iter()
            .cloned()
            .map(|mut row| {
                if row.context == context(1) {
                    row.unread = 3;
                    row.unread_mention = Some(true);
                    row.unread_direct_mention = Some(true);
                    row.last_read_message = Some(1);
                }
                row
            })
            .chain(extra),
    );
    state
}

/// The Activity page needs its feature views to expose every filter.
fn with_activity(mut state: PresentationSnapshot) -> PresentationSnapshot {
    let scope = super::scope();
    let rooms: Vec<_> = state
        .conversations
        .as_slice()
        .iter()
        .map(|row| row.context)
        .collect();
    state.threads.account = Some(scope);
    state.threads.support = FeatureSupport::Supported;
    state.threads.content = ContentState::Ready;
    state.threads.entries = page([
        ThreadView {
            context: rooms[0],
            root: 1,
            title: text("Design review"),
            replies: 2,
            last_message: Some(41),
        },
        ThreadView {
            context: rooms[2],
            root: 2,
            title: text("Offsite planen"),
            replies: 7,
            last_message: Some(52),
        },
    ]);
    state.account_reminders = Some(AccountRemindersView {
        scope,
        support: FeatureSupport::Supported,
        state: ContentState::Ready,
        complete: true,
        reminders: page(rooms.iter().take(2).enumerate().map(|(index, context)| {
            AccountReminderView {
                context: *context,
                message: 45 + index as u128,
                preview: Some(text(if index == 0 {
                    "Die neue Navigation ist bereit für einen letzten Blick."
                } else {
                    "Den Entwurf besprechen wir morgen."
                })),
                timestamp: 1_800_003_600 + index as u64 * 86_400,
            }
        })),
    });
    state.federation_invitations = Some(FederationInvitationsView {
        scope,
        support: FeatureSupport::Supported,
        pending: false,
        failed: false,
        invitations: page([FederationInvitationView {
            id: 101,
            room: text("Gemeinschaft planen"),
            inviter: text("Alex Morgan"),
        }]),
    });
    state
}

pub(super) fn snapshot_for(scenario: Option<&str>) -> Option<PresentationSnapshot> {
    Some(match scenario? {
        "sidebar-rich" | "sidebar-new-menu" | "sidebar-list-menu" | "sidebar-archive" => rich(),
        "sidebar-activity" | "sidebar-activity-mentions" => with_activity(rich()),
        "sidebar-load-failed" => {
            let mut state = snapshot(Some("chat"));
            state.conversations = talk_core::ui::Page::empty();
            state.conversation_list.content = ContentState::Failed;
            state.conversation_list.complete = false;
            // Nothing was ever loaded, so no room can be open.
            state.selected = None;
            state.messages = talk_core::ui::Page::empty();
            state.files = talk_core::ui::Page::empty();
            state.participants = talk_core::ui::Page::empty();
            state.message_panel = None;
            state.conversation_panel = None;
            state
        }
        "connection-details" => snapshot(Some("offline-with-queue")),
        // A guest-link room whose access was withdrawn: no messages, no composer.
        "chat-access-denied" => {
            let mut state = snapshot(Some("chat"));
            populate(&mut state, context(4));
            state.content = ContentState::Denied;
            state.messages = talk_core::ui::Page::empty();
            state.files = talk_core::ui::Page::empty();
            state.participants = talk_core::ui::Page::empty();
            state
        }
        // A room the account can read but not write to.
        "chat-read-only" => {
            let mut state = snapshot(Some("chat"));
            state.conversations = page(state.conversations.as_slice().iter().cloned().map(
                |mut row| {
                    if Some(row.context) == state.selected {
                        row.send = Decision::Denied;
                    }
                    row
                },
            ));
            state
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL_SCENARIOS: &[&str] = &[
        "sidebar-rich",
        "sidebar-activity",
        "sidebar-activity-mentions",
        "sidebar-new-menu",
        "sidebar-list-menu",
        "sidebar-archive",
        "sidebar-load-failed",
        "connection-details",
        "chat-access-denied",
        "chat-read-only",
    ];

    #[test]
    fn every_shell_scenario_is_registered_and_a_consistent_projection() {
        for name in SHELL_SCENARIOS {
            assert!(
                crate::composition::fixtures::SCENARIOS.contains(name),
                "{name} is not a selectable scenario"
            );
            let state = snapshot_for(Some(name)).expect("shell scenario");
            assert!(
                crate::presentation::valid_snapshot(&state),
                "{name} is inconsistent"
            );
        }
        assert!(snapshot_for(Some("chat")).is_none());
        assert!(snapshot_for(None).is_none());
    }

    #[test]
    fn rich_sidebar_exposes_unread_mentions_and_an_archived_room() {
        let state = rich();
        let rooms = state.conversations.as_slice();
        assert!(
            rooms
                .iter()
                .any(|r| r.unread > 0 && r.unread_mention == Some(true))
        );
        assert!(rooms.iter().any(|r| r.archived == Some(true)));
        assert!(rooms.len() >= 10);
        let titles: std::collections::HashSet<_> = rooms.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(
            titles.len(),
            rooms.len(),
            "fixture rooms must be distinguishable"
        );
    }

    #[test]
    fn activity_fixture_enables_every_filter() {
        let state = with_activity(rich());
        assert_eq!(state.threads.support, FeatureSupport::Supported);
        assert!(state.account_reminders.is_some());
        assert!(
            state
                .federation_invitations
                .as_ref()
                .is_some_and(|view| !view.invitations.as_slice().is_empty())
        );
    }

    #[test]
    fn access_denied_room_has_no_readable_content() {
        let state = snapshot_for(Some("chat-access-denied")).expect("scenario");
        assert_eq!(state.content, ContentState::Denied);
        assert!(state.messages.as_slice().is_empty());
        let selected = state.selected.expect("selected room");
        let row = state
            .conversations
            .as_slice()
            .iter()
            .find(|row| row.context == selected)
            .expect("room row");
        assert_eq!(row.kind, ConversationKind::GuestAccessible);
    }

    #[test]
    fn read_only_room_keeps_its_timeline_and_denies_only_sending() {
        let state = snapshot_for(Some("chat-read-only")).expect("scenario");
        assert_eq!(state.content, ContentState::Ready);
        assert!(!state.messages.as_slice().is_empty());
        let selected = state.selected.expect("selected room");
        let denied: Vec<_> = state
            .conversations
            .as_slice()
            .iter()
            .filter(|row| row.send == Decision::Denied)
            .map(|row| row.context)
            .collect();
        assert_eq!(denied, vec![selected]);
    }
}
