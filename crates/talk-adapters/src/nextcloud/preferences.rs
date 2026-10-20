//! Released Talk24.0.6 / Talk25.0.6 routes and semantics, not guessed config keys.
//! Primary sources (both commits were read):
//! https://github.com/nextcloud/spreed/blob/0d736588dcfb42becf0e5688e0847e35d5b5efa2/lib/Controller/SettingsController.php
//! https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/lib/Settings/BeforePreferenceSetEventListener.php
//! https://github.com/nextcloud/spreed/blob/0d736588dcfb42becf0e5688e0847e35d5b5efa2/lib/Controller/RoomController.php
//! https://github.com/nextcloud/spreed/blob/eed360d176edcb6888dad60965cb694e6bf62ae3/lib/Capabilities.php
//!
//! Settings are read from the authenticated local server's capabilities. A single
//! key is written, then re-read: no multi-key pseudo-transaction or automatic retry.
//! Timeout/cancellation after dispatch remains uncertain until explicit readback.
//! attachment_folder is the account-owned uploads/received-share base, optionally
//! with server conversation subfolders. Server rejects shared/non-directory targets
//! and creates absent folders. This does not implement or promise a file uploader.
use super::{NextcloudClient, wire::Capabilities};
use reqwest::Method;
use serde_json::Value;
use talk_core::{
    ids::{AccountScope, ConversationId},
    ports::{ErrorKind, OperationContext, PortError, ProtocolReply},
    preferences::{
        RequestId,
        account::{AccountPreferenceChange, AccountPreferenceResult, AccountPreferences},
    },
};

pub(super) fn public_privacy(value: &Value) -> Option<bool> {
    // Published constants: 0 public, 1 private. No truthy default for unknown data.
    match super::wire::integer(Some(value)) {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}
impl Capabilities {
    pub(super) fn account_preferences(&self, scope: AccountScope) -> AccountPreferences {
        AccountPreferences {
            scope,
            send_read_receipts: self
                .has("chat-read-status")
                .then_some(self.read_receipts)
                .flatten(),
            share_typing_status: self
                .has("typing-privacy")
                .then_some(self.typing_status)
                .flatten(),
            attachment_folder: self.attachment_folder.clone(),
            attachments_allowed: self.attachments_allowed,
            conversation_subfolders: self.conversation_subfolders,
        }
    }
}
impl NextcloudClient {
    // Both released RoomController variants return the updated room. As with
    // archive, reconcile after a successful mutation and never replay uncertainty.
    pub(super) async fn set_favorite(
        &mut self,
        context: &OperationContext,
        conversation: ConversationId,
        favorite: bool,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        use talk_core::protocol::FeatureSupport;
        match caps.feature("favorites") {
            FeatureSupport::Unsupported => return Err(self.error(ErrorKind::Unsupported)),
            FeatureSupport::Unknown => return Err(self.error(ErrorKind::Unconfigured)),
            FeatureSupport::Supported => {}
        }
        let before = self.room(context, conversation, caps).await?;
        if before.organization.favorite_change == talk_core::policy::Decision::Unknown {
            return Err(self.error(ErrorKind::Unconfigured));
        }
        self.allowed(before.organization.favorite_change)?;
        let route = self.route(conversation, "v4/room", "/favorite")?;
        self.write(
            context,
            if favorite {
                Method::POST
            } else {
                Method::DELETE
            },
            &route,
            &[],
        )
        .await?;
        let room = self
            .room(context, conversation, caps)
            .await
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        if room.favorite != Some(favorite) {
            return Err(self.error(ErrorKind::Uncertain));
        }
        Ok(ProtocolReply::Conversation(room))
    }
    pub(super) async fn set_preference(
        &mut self,
        context: &OperationContext,
        request: RequestId,
        change: AccountPreferenceChange,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        let before = caps.account_preferences(context.scope);
        let (key, value, available) = match &change {
            AccountPreferenceChange::SendReadReceipts(enabled) => (
                "read_status_privacy",
                if *enabled { "0".into() } else { "1".into() },
                before.send_read_receipts.is_some(),
            ),
            AccountPreferenceChange::ShareTypingStatus(enabled) => (
                "typing_privacy",
                if *enabled { "0".into() } else { "1".into() },
                before.share_typing_status.is_some(),
            ),
            AccountPreferenceChange::AttachmentFolder(folder) => (
                "attachment_folder",
                folder.as_str().to_owned(),
                before.attachments_allowed == Some(true) && before.attachment_folder.is_some(),
            ),
        };
        if !available {
            return Err(self.error(ErrorKind::Unsupported));
        }
        self.write(
            context,
            Method::POST,
            "ocs/v2.php/apps/spreed/api/v1/settings/user",
            &[("key", key.into()), ("value", value)],
        )
        .await?;
        let values = self
            .capabilities(context)
            .await
            .map_err(|_| self.error(ErrorKind::Uncertain))?
            .account_preferences(context.scope);
        let matches = match &change {
            AccountPreferenceChange::SendReadReceipts(value) => {
                values.send_read_receipts == Some(*value)
            }
            AccountPreferenceChange::ShareTypingStatus(value) => {
                values.share_typing_status == Some(*value)
            }
            AccountPreferenceChange::AttachmentFolder(value) => {
                values.attachment_folder.as_ref() == Some(value)
            }
        };
        if !matches {
            return Err(self.error(ErrorKind::Uncertain));
        }
        // Private read markers may still clear the account's unread count. The
        // server hides publication; history projection also hides incoming statuses.
        // Typing publication and display must use AccountPreferences::send_typing /
        // show_typing_indicators at the application/signaling boundary.
        Ok(ProtocolReply::Preferences(AccountPreferenceResult {
            request,
            values,
        }))
    }
    pub(super) async fn set_archived(
        &mut self,
        context: &OperationContext,
        conversation: ConversationId,
        archived: bool,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        let before = self.room(context, conversation, caps).await?;
        self.allowed(before.archive)?;
        let route = self.route(conversation, "v4/room", "/archive")?;
        self.write(
            context,
            if archived {
                Method::POST
            } else {
                Method::DELETE
            },
            &route,
            &[],
        )
        .await?;
        let room = self
            .room(context, conversation, caps)
            .await
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        if room.archived != Some(archived) {
            return Err(self.error(ErrorKind::Uncertain));
        }
        Ok(ProtocolReply::Conversation(room))
    }
}
