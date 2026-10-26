//! Fictional read-only projection for native dialog layout checks.
use super::*;
use talk_core::ui::conversation::{
    Availability, ConversationPanel, NotificationLevel, RoomDetails, RoomSetting,
};
use talk_core::ui::presentation::FeatureSupport;

pub(super) fn configure(snapshot: &mut PresentationSnapshot) {
    let context = snapshot.selected.expect("explicit chat fixture");
    let mut panel = ConversationPanel::new(context);
    panel.details = Some(RoomDetails {
        name: text("Produktteam"),
        description: Some(text(
            "Fiktiver Raum für gemeinsame Ideen und Projektabsprachen.",
        )),
        read_only: Some(false),
        notifications: Some(NotificationLevel::Mention),
        call_notifications: Some(true),
        important: Some(false),
        sensitive: false,
        classified: false,
        preserved: Some(false),
        message_expiration: Some(0),
        default_permissions: Some(511),
        server_default_permissions: Some(511),
        effective_permissions: Some(511),
        lobby: Some(false),
        has_password: Some(false),
        participant_count: Some(7),
        guest_access: Some(false),
        leave: Decision::Allowed,
        controls: page(
            [
                RoomSetting::Metadata,
                RoomSetting::Avatar,
                RoomSetting::ReadOnly,
                RoomSetting::Notifications,
                RoomSetting::Important,
                RoomSetting::Sensitive,
                RoomSetting::Expiration,
                RoomSetting::Preserve,
                RoomSetting::GuestAccess,
                RoomSetting::Password,
                RoomSetting::Discoverability,
                RoomSetting::Mentions,
                RoomSetting::Permissions,
                RoomSetting::Lobby,
                RoomSetting::RecordingConsent,
                RoomSetting::Delete,
                RoomSetting::ClearHistory,
                RoomSetting::Keep,
                RoomSetting::Members,
                RoomSetting::Invite,
                RoomSetting::CsvImport,
                RoomSetting::Roles,
                RoomSetting::Bots,
                RoomSetting::Bans,
                RoomSetting::Breakouts,
            ]
            .map(|setting| {
                (
                    setting,
                    Availability {
                        implemented: true,
                        server: FeatureSupport::Supported,
                        allowed: Decision::Allowed,
                    },
                )
            }),
        ),
    });
    snapshot.conversation_panel = Some(panel);
}
