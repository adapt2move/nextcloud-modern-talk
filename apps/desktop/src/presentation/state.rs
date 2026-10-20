//! Local presentation decisions only: bounded drafts, admission feedback and view routing.
use talk_core::ui::presentation::{
    Completion, ContentState, ConversationKind, ConversationRef, ConversationView,
    NativeDesktopPort, PresentationSnapshot, RequestId, SubmissionResult, UiIntent,
};
use talk_core::ui::{CallScope, Decision, SubmitError, Text, UnavailableReason};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallEntry {
    Direct(CallScope),
    Conference(CallScope),
}
pub fn call_entry(row: &ConversationView, content: ContentState) -> Option<CallEntry> {
    if content != ContentState::Ready || row.call != Decision::Allowed {
        return None;
    }
    let call = row.call_scope?;
    if !call.is_consistent()
        || call.account != row.context.account
        || call.conversation != row.context.conversation
    {
        return None;
    }
    if row.organization.category == talk_core::ui::presentation::ConversationCategory::Channel {
        return None;
    }
    match row.kind {
        ConversationKind::Direct => Some(CallEntry::Direct(call)),
        ConversationKind::Group | ConversationKind::GuestAccessible => {
            if matches!(
                row.organization.category,
                talk_core::ui::presentation::ConversationCategory::Conference(_)
            ) {
                Some(CallEntry::Conference(call))
            } else {
                Some(CallEntry::Direct(call))
            }
        }
        ConversationKind::NoteToSelf | ConversationKind::Unknown => None,
    }
}

/// Describes only existing local projection facts; never guesses a server policy.
pub fn call_unavailable_reason(row: &ConversationView, content: ContentState) -> &'static str {
    match content {
        ContentState::Offline => "Du bist offline.",
        ContentState::Loading => "Das Gespräch wird noch geladen.",
        ContentState::Denied => "Du hast derzeit keinen Zugriff auf dieses Gespräch.",
        ContentState::Failed => "Das Gespräch konnte nicht geladen werden.",
        _ if row.kind == ConversationKind::NoteToSelf => {
            "Eine Notiz an dich selbst hat keinen Anruf."
        }
        _ if row.call == Decision::Denied => {
            "Ein Anruf ist für dich in diesem Gespräch nicht erlaubt."
        }
        _ if row.call != Decision::Allowed => "Die Anrufberechtigung ist noch nicht bekannt.",
        _ => "Ein Anruf ist für dieses Gespräch derzeit nicht verfügbar.",
    }
}

pub const MAX_DRAFTS: usize = 8;
#[derive(Clone, Debug)]
struct Draft {
    context: ConversationRef,
    text: Text<4096>,
    target: talk_core::ui::messaging::ComposeTarget,
    pending: Option<(RequestId, Text<4096>)>,
    uncertain: bool,
}
#[derive(Default)]
pub struct Drafts {
    entries: Vec<Draft>,
    next_request: u64,
    targets: Vec<(ConversationRef, talk_core::ui::messaging::ComposeTarget)>,
}
impl Drafts {
    pub fn target(&self, context: ConversationRef) -> talk_core::ui::messaging::ComposeTarget {
        self.targets
            .iter()
            .find(|(c, _)| *c == context)
            .map(|(_, t)| t.clone())
            .unwrap_or_default()
    }
    pub fn set_target(
        &mut self,
        context: ConversationRef,
        target: talk_core::ui::messaging::ComposeTarget,
    ) -> bool {
        if target == Default::default() {
            self.targets.retain(|(c, _)| *c != context);
            return true;
        }
        if let Some((_, value)) = self.targets.iter_mut().find(|(c, _)| *c == context) {
            *value = target;
        } else if self.targets.len() < MAX_DRAFTS {
            self.targets.push((context, target));
        } else {
            return false;
        }
        true
    }

    pub fn retain_accounts(&mut self, snapshot: &PresentationSnapshot) {
        self.targets.retain(|(context, _)| {
            snapshot
                .accounts
                .as_slice()
                .iter()
                .any(|a| a.scope == context.account)
        });
        self.entries.retain(|d| {
            snapshot
                .accounts
                .as_slice()
                .iter()
                .any(|a| a.scope == d.context.account)
        });
    }
    pub fn text(&self, context: ConversationRef) -> &str {
        self.entries
            .iter()
            .find(|d| d.context == context && same_draft_target(&d.target, &self.target(context)))
            .map_or("", |d| d.text.as_str())
    }
    pub fn uncertain(&self, context: ConversationRef) -> bool {
        self.entries
            .iter()
            .any(|d| d.context == context && d.uncertain)
    }
    pub fn pending(&self, context: ConversationRef) -> bool {
        self.entries
            .iter()
            .any(|d| d.context == context && (d.pending.is_some() || d.uncertain))
    }
    pub fn save(&mut self, context: ConversationRef, text: &str) -> Result<(), SubmitError> {
        if !context.is_consistent() {
            return Err(SubmitError::StaleScope);
        }
        let target = self.target(context);
        let text = Text::new(text).map_err(|_| SubmitError::Busy)?;
        if let Some(draft) = self
            .entries
            .iter_mut()
            .find(|d| d.context == context && same_draft_target(&d.target, &target))
        {
            draft.text = text;
            draft.target = target;
        } else if !text.as_str().is_empty() {
            // Empty, non-pending drafts may be reclaimed; unsent text is never evicted.
            self.entries
                .retain(|d| !d.text.as_str().is_empty() || d.pending.is_some() || d.uncertain);
            if self.entries.len() >= MAX_DRAFTS {
                return Err(SubmitError::Busy);
            }
            self.entries.push(Draft {
                context,
                text,
                target,
                pending: None,
                uncertain: false,
            });
        }
        Ok(())
    }
    pub fn send(
        &mut self,
        context: ConversationRef,
        gate: Decision,
        port: &mut dyn NativeDesktopPort,
    ) -> Result<RequestId, SubmitError> {
        match gate {
            Decision::Allowed => (),
            Decision::Denied => {
                return Err(SubmitError::Unavailable(
                    UnavailableReason::PermissionDenied,
                ));
            }
            Decision::Unknown => {
                return Err(SubmitError::Unavailable(UnavailableReason::PolicyUnknown));
            }
            Decision::Stale => {
                return Err(SubmitError::Unavailable(UnavailableReason::PolicyStale));
            }
        }
        let target = self.target(context);
        let draft = self
            .entries
            .iter_mut()
            .find(|d| d.context == context && same_draft_target(&d.target, &target))
            .ok_or(SubmitError::Unavailable(UnavailableReason::Unsupported))?;
        if draft.pending.is_some() || draft.uncertain {
            return Err(SubmitError::Busy);
        }
        if draft.text.as_str().trim().is_empty() {
            return Err(SubmitError::Unavailable(UnavailableReason::Unsupported));
        }
        let request = RequestId(self.next_request.checked_add(1).ok_or(SubmitError::Busy)?);
        // Busy is non-admission: neither the text nor the pending state changes.
        port.try_submit(UiIntent::Message {
            request,
            context,
            action: talk_core::ui::messaging::MessageAction::Send {
                body: draft.text.clone(),
                target: target.clone(),
            },
        })?;
        self.next_request = request.0;
        draft.pending = Some((request, draft.text.clone()));
        Ok(request)
    }
    pub fn complete(&mut self, result: SubmissionResult) -> bool {
        let Some(draft) = self.entries.iter_mut().find(|d| {
            d.context == result.context
                && d.pending
                    .as_ref()
                    .is_some_and(|(request, _)| *request == result.request)
        }) else {
            return false;
        };
        let Some((request, submitted)) = draft.pending.as_ref() else {
            return false;
        };
        if *request != result.request {
            return false;
        }
        match result.result {
            Completion::ServerAccepted => {
                // Never erase text the user changed while the operation was in flight.
                if draft.text == *submitted {
                    draft.text = Text::new("").expect("empty bounded text");
                }
                draft.pending = None;
                draft.uncertain = false;
            }
            Completion::Failed(_) => {
                draft.pending = None;
                draft.uncertain = false;
            }
            Completion::Uncertain => {
                draft.uncertain = true;
            }
        }
        true
    }
}

/// Defensive renderer validation. Producers still own full permission/lifecycle decisions.
pub fn valid_snapshot(snapshot: &PresentationSnapshot) -> bool {
    if snapshot.create_panel.as_ref().is_some_and(|panel| {
        Some(panel.account) != snapshot.active_account
            || panel
                .people
                .as_slice()
                .iter()
                .any(|person| person.id.account != panel.account)
            || panel.result.is_some_and(|result| {
                result.conversation.is_some_and(|context| {
                    context.account != panel.account || !context.is_consistent()
                })
            })
    }) {
        return false;
    }
    if snapshot
        .tags
        .as_slice()
        .iter()
        .any(|tag| Some(tag.id.account) != snapshot.active_account)
        || snapshot.conversations.as_slice().iter().any(|row| {
            row.tags.as_ref().is_some_and(|tags| {
                tags.as_slice()
                    .iter()
                    .any(|tag| tag.account != row.context.account)
            })
        })
    {
        return false;
    }
    if snapshot.content == talk_core::ui::presentation::ContentState::Denied
        && (!snapshot.messages.as_slice().is_empty()
            || !snapshot.files.as_slice().is_empty()
            || !snapshot.participants.as_slice().is_empty())
    {
        return false;
    }
    if let Some(active) = snapshot.active_account {
        if !snapshot
            .accounts
            .as_slice()
            .iter()
            .any(|a| a.scope == active)
        {
            return false;
        }
        if snapshot
            .conversations
            .as_slice()
            .iter()
            .any(|r| !r.context.is_consistent() || r.context.account != active)
        {
            return false;
        }
    } else if !snapshot.conversations.as_slice().is_empty() {
        return false;
    }
    if let Some(selected) = snapshot.selected {
        if Some(selected.account) != snapshot.active_account
            || !snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|r| r.context == selected)
        {
            return false;
        }
    } else if !snapshot.messages.as_slice().is_empty()
        || !snapshot.files.as_slice().is_empty()
        || !snapshot.participants.as_slice().is_empty()
    {
        return false;
    }
    if let Some(call) = &snapshot.call
        && (!call.scope.is_consistent()
            || !snapshot
                .accounts
                .as_slice()
                .iter()
                .any(|a| a.scope == call.scope.account))
    {
        return false;
    }
    true
}

pub fn error_label(error: SubmitError) -> &'static str {
    match error {
        SubmitError::Busy => {
            "Gerade ausgelastet. Dein Entwurf bleibt erhalten; bitte erneut versuchen."
        }
        SubmitError::StaleScope => {
            "Dieser Stand ist überholt. Bitte das Gespräch neu öffnen; dein Text bleibt erhalten."
        }
        SubmitError::Unavailable(UnavailableReason::PermissionDenied) => {
            "Keine Berechtigung für diese Aktion. Dein Entwurf bleibt erhalten."
        }
        SubmitError::Unavailable(
            UnavailableReason::PolicyUnknown | UnavailableReason::PolicyStale,
        ) => "Berechtigungen werden geprüft. Die Aktion ist noch nicht verfügbar.",
        SubmitError::Unavailable(UnavailableReason::ReauthenticationRequired) => {
            "Bitte erneut anmelden. Dein Text wurde nicht gesendet."
        }
        SubmitError::Unavailable(UnavailableReason::Unconfigured) => {
            "Diese Aktion ist in diesem Client noch nicht verfügbar."
        }
        SubmitError::Unavailable(UnavailableReason::Unsupported) => {
            "Diese Funktion ist in diesem Client noch nicht verfügbar."
        }
    }
}

fn same_draft_target(
    a: &talk_core::ui::messaging::ComposeTarget,
    b: &talk_core::ui::messaging::ComposeTarget,
) -> bool {
    a.reply_to == b.reply_to && a.thread == b.thread
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};
    fn context(n: u128) -> ConversationRef {
        ConversationRef {
            account: AccountScope {
                account: AccountId::from_local(1),
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account: AccountId::from_local(1),
                host: HostId::from_local(2),
                local: n,
            },
        }
    }
    struct Port {
        fail: Option<SubmitError>,
        submissions: usize,
    }
    impl NativeDesktopPort for Port {
        fn try_submit(&mut self, _: UiIntent) -> Result<(), SubmitError> {
            self.submissions += 1;
            self.fail.map_or(Ok(()), Err)
        }
        fn take_snapshot(&mut self) -> Option<PresentationSnapshot> {
            None
        }
    }
    #[test]
    fn busy_and_denied_never_erase_text_or_create_pending_work() {
        let mut drafts = Drafts::default();
        let c = context(1);
        drafts.save(c, "Noch nicht gesendet 🧭").unwrap();
        let mut port = Port {
            fail: Some(SubmitError::Busy),
            submissions: 0,
        };
        assert_eq!(
            drafts.send(c, Decision::Allowed, &mut port),
            Err(SubmitError::Busy)
        );
        assert!(!drafts.pending(c));
        assert_eq!(drafts.text(c), "Noch nicht gesendet 🧭");
        assert!(drafts.send(c, Decision::Unknown, &mut port).is_err());
        assert_eq!(port.submissions, 1);
    }
    #[test]
    fn only_correlated_confirmation_clears_unchanged_text() {
        let mut drafts = Drafts::default();
        let c = context(1);
        let mut port = Port {
            fail: None,
            submissions: 0,
        };
        drafts.save(c, "erste Nachricht").unwrap();
        let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
        assert_eq!(drafts.text(c), "erste Nachricht");
        assert!(drafts.send(c, Decision::Allowed, &mut port).is_err());
        assert!(!drafts.complete(SubmissionResult {
            request,
            context: context(2),
            result: Completion::ServerAccepted
        }));
        let mut old = c;
        old.account.runtime = RuntimeGeneration::from_local(2);
        assert!(!drafts.complete(SubmissionResult {
            request,
            context: old,
            result: Completion::ServerAccepted
        }));
        drafts.save(c, "nächster Entwurf").unwrap();
        assert!(drafts.complete(SubmissionResult {
            request,
            context: c,
            result: Completion::ServerAccepted
        }));
        assert_eq!(drafts.text(c), "nächster Entwurf");
        let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
        assert!(drafts.complete(SubmissionResult {
            request,
            context: c,
            result: Completion::ServerAccepted
        }));
        assert_eq!(drafts.text(c), "");
        assert_eq!(port.submissions, 2);
    }
    #[test]
    fn failure_retains_draft_and_uncertainty_blocks_duplicate_retry_until_reconciled() {
        let mut drafts = Drafts::default();
        let c = context(1);
        let mut port = Port {
            fail: None,
            submissions: 0,
        };
        drafts.save(c, "behalten").unwrap();
        let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
        drafts.complete(SubmissionResult {
            request,
            context: c,
            result: Completion::Uncertain,
        });
        assert_eq!(
            drafts.send(c, Decision::Allowed, &mut port),
            Err(SubmitError::Busy)
        );
        drafts.complete(SubmissionResult {
            request,
            context: c,
            result: Completion::Failed(UnavailableReason::Unconfigured),
        });
        assert_eq!(drafts.text(c), "behalten");
        assert!(!drafts.pending(c));
    }
    #[test]
    fn draft_capacity_rejects_navigation_without_eviction_and_scopes_hosts() {
        let mut drafts = Drafts::default();
        for i in 1..=8 {
            drafts.save(context(i), "Wichtig").unwrap();
        }
        assert_eq!(drafts.save(context(9), "Neun"), Err(SubmitError::Busy));
        for i in 1..=8 {
            assert_eq!(drafts.text(context(i)), "Wichtig");
        }
        let mut other = context(1);
        other.conversation.host = HostId::from_local(8);
        assert_eq!(drafts.text(other), "");
        drafts.save(context(1), "").unwrap();
        drafts.save(other, "Anderer Host").unwrap();
        assert_eq!(drafts.text(context(1)), "");
        assert_eq!(drafts.text(other), "Anderer Host");
    }
    #[test]
    fn silent_delivery_changes_do_not_split_or_discard_a_draft() {
        let c = context(1);
        let mut drafts = Drafts::default();
        drafts.save(c, "keep this text").unwrap();
        assert!(drafts.set_target(
            c,
            talk_core::ui::messaging::ComposeTarget {
                silent: true,
                ..Default::default()
            }
        ));
        assert_eq!(drafts.text(c), "keep this text");
        drafts.save(c, "updated text").unwrap();
        assert!(drafts.set_target(c, Default::default()));
        assert_eq!(drafts.text(c), "updated text");
        assert_eq!(drafts.entries.len(), 1);
    }
    #[test]
    fn thread_reply_drafts_and_confirmations_do_not_clear_the_normal_composer() {
        use talk_core::ui::messaging::ComposeTarget;
        let c = context(1);
        let mut drafts = Drafts::default();
        let mut port = Port {
            fail: None,
            submissions: 0,
        };
        drafts.save(c, "normal draft").unwrap();
        assert!(drafts.set_target(
            c,
            ComposeTarget {
                thread: Some(7),
                ..Default::default()
            }
        ));
        drafts.save(c, "thread draft").unwrap();
        let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
        assert!(drafts.set_target(c, ComposeTarget::default()));
        assert_eq!(drafts.text(c), "normal draft");
        assert!(drafts.complete(SubmissionResult {
            request,
            context: c,
            result: Completion::ServerAccepted
        }));
        assert_eq!(drafts.text(c), "normal draft");
        assert!(drafts.set_target(
            c,
            ComposeTarget {
                thread: Some(7),
                ..Default::default()
            }
        ));
        assert_eq!(drafts.text(c), "");
    }
    #[test]
    fn unknown_kind_policy_or_mismatched_call_generation_never_opens_prejoin() {
        let c = context(1);
        let call = CallScope {
            account: c.account,
            conversation: c.conversation,
            call: talk_core::ui::CallGeneration::from_local(1),
        };
        let mut row = ConversationView {
            tags: None,
            context: c,
            navigation_key: None,
            title: Text::new("Room").unwrap(),
            initials: Text::new("R").unwrap(),
            subtitle: Text::new("").unwrap(),
            updated_label: Text::new("").unwrap(),
            kind: ConversationKind::Direct,
            unread: 0,
            favorite: None,
            organization: Default::default(),
            archived: None,
            archive: Decision::Unknown,
            presence: talk_core::ui::presentation::Presence::Unknown,
            participants: None,
            send: Decision::Unknown,
            attach: Decision::Unknown,
            draft_clipboard: Decision::Unknown,
            call: Decision::Allowed,
            call_scope: Some(call),
        };
        assert_eq!(
            call_entry(&row, ContentState::Ready),
            Some(CallEntry::Direct(call))
        );
        row.kind = ConversationKind::Group;
        assert_eq!(
            call_entry(&row, ContentState::Ready),
            Some(CallEntry::Direct(call))
        );
        row.organization.category = talk_core::ui::presentation::ConversationCategory::Channel;
        assert_eq!(call_entry(&row, ContentState::Ready), None);
        row.organization.category = talk_core::ui::presentation::ConversationCategory::Conference(
            talk_core::ui::presentation::ConferencePreset::Standard,
        );
        assert_eq!(
            call_entry(&row, ContentState::Ready),
            Some(CallEntry::Conference(call))
        );
        row.kind = ConversationKind::Unknown;
        assert_eq!(call_entry(&row, ContentState::Ready), None);
        row.kind = ConversationKind::NoteToSelf;
        assert_eq!(call_entry(&row, ContentState::Ready), None);
        row.kind = ConversationKind::Group;
        for policy in [Decision::Unknown, Decision::Denied, Decision::Stale] {
            row.call = policy;
            assert_eq!(call_entry(&row, ContentState::Ready), None);
        }
        row.call = Decision::Allowed;
        row.call_scope.as_mut().unwrap().account.runtime = RuntimeGeneration::from_local(99);
        assert_eq!(call_entry(&row, ContentState::Ready), None);
    }
}
