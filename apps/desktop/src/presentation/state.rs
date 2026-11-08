//! Local presentation decisions only: bounded drafts, admission feedback and view routing.
use talk_core::ui::presentation::{
    Completion, ContentState, ConversationKind, ConversationRef, ConversationView,
    NativeDesktopPort, PresentationSnapshot, RequestId, SubmissionResult, UiIntent,
};
use talk_core::ui::{CallScope, Decision, SubmitError, Text, UnavailableReason};

pub(super) fn account_connection_failure(
    accounts: &talk_core::ui::Page<talk_core::ui::presentation::AccountView, 8>,
    scope: talk_core::ui::AccountScope,
) -> Option<talk_core::ui::presentation::ConnectionFailure> {
    accounts
        .as_slice()
        .iter()
        .find(|account| account.scope == scope)
        .and_then(|account| account.connection_failure)
}

pub(super) fn composer_send_decision(
    snapshot: &PresentationSnapshot,
    row: &ConversationView,
) -> Decision {
    if snapshot
        .message_panel
        .as_ref()
        .is_some_and(|p| p.context == row.context && p.queue_available)
        && matches!(
            snapshot.content,
            ContentState::Ready | ContentState::Offline
        )
        && row.send != Decision::Denied
    {
        Decision::Allowed
    } else if snapshot.content == ContentState::Ready
        && account_connection_failure(&snapshot.accounts, row.context.account).is_none()
    {
        row.send
    } else {
        Decision::Unknown
    }
}

pub(super) fn connection_failure_copy(
    failure: talk_core::ui::presentation::ConnectionFailure,
) -> (&'static str, &'static str) {
    use talk_core::ui::presentation::ConnectionFailure;
    match failure {
        ConnectionFailure::Unreachable => (
            "Server nicht erreichbar.",
            "Die Verbindung zum Server ist fehlgeschlagen. Prüfe Netzwerk, VPN und Serveradresse.",
        ),
        ConnectionFailure::TimedOut => (
            "Server antwortet nicht rechtzeitig.",
            "Die Verbindung hat zu lange gedauert. Bitte versuche es später erneut.",
        ),
        ConnectionFailure::AccessDenied => (
            "Server verweigert den Zugriff.",
            "Prüfe, ob du auf diesen Server zugreifen darfst.",
        ),
        ConnectionFailure::ServerUnavailable => (
            "Server vorübergehend nicht verfügbar.",
            "Der Server meldet einen Fehler. Bitte versuche es später erneut.",
        ),
        ConnectionFailure::RateLimited => (
            "Zu viele Anfragen.",
            "Der Server begrenzt die Anfragen. Bitte später erneut versuchen.",
        ),
        ConnectionFailure::InvalidResponse => (
            "Unerwartete Serverantwort.",
            "Die Serverantwort konnte nicht verarbeitet werden. Prüfe die Serveradresse oder versuche es später erneut.",
        ),
    }
}

pub(super) fn login_failure_detail(
    failure: talk_core::ui::presentation::LoginFailure,
) -> &'static str {
    use talk_core::ui::presentation::LoginFailure;
    match failure {
        LoginFailure::AccountStorage(failure) => account_load_failure_copy(failure).1,
        LoginFailure::Connection(failure) => connection_failure_copy(failure).1,
        LoginFailure::Network => {
            connection_failure_copy(talk_core::ui::presentation::ConnectionFailure::Unreachable).1
        }
        LoginFailure::Expired => {
            "Die Anmeldung hat zu lange gedauert. Mit einem neuen Link kannst du weitermachen."
        }
        LoginFailure::BrowserUnavailable => "Prüfe deinen Standardbrowser und versuche es erneut.",
        LoginFailure::Tls => {
            "Die Identität des Servers konnte nicht bestätigt werden. Prüfe die Serveradresse."
        }
        LoginFailure::Denied => {
            "Dein Konto wurde nicht verbunden. Du kannst die Anmeldung erneut starten."
        }
        LoginFailure::Storage => "Der gespeicherte Anmeldestatus muss wiederhergestellt werden.",
        LoginFailure::AccountExists => {
            "Dieses Konto ist bereits auf diesem Gerät eingetragen. Öffne die Konten und verbinde es dort erneut."
        }
        LoginFailure::Unconfigured => {
            "Diese Vorschau kann sich noch nicht mit eurer NextCloud verbinden."
        }
    }
}

pub(super) fn account_load_failure_copy(
    failure: talk_core::ui::presentation::AccountLoadFailure,
) -> (&'static str, &'static str) {
    use talk_core::ui::presentation::AccountLoadFailure;
    match failure {
        AccountLoadFailure::DeviceChanged => (
            "Kontospeicher muss neu zugeordnet werden.",
            "Das Laufwerk wird jetzt unter einer anderen Kennung geführt. Benutzerkonto und Datenordner stimmen überein. Wähle „Kontospeicher wieder verwenden“, um deine Konten wieder zu laden.",
        ),
        AccountLoadFailure::InUse => (
            "Kontospeicher wird bereits verwendet.",
            "Eine andere Instanz von Talk verwendet die gespeicherten Konten. Beende sie und klicke auf „Konten erneut laden“.",
        ),
        AccountLoadFailure::AccessDenied => (
            "Kein Zugriff auf gespeicherte Konten.",
            "Talk kann den lokalen Kontospeicher wegen seiner Zugriffsrechte nicht öffnen. Prüfe die Rechte des App-Datenordners und lade die Konten erneut.",
        ),
        AccountLoadFailure::Unavailable => (
            "Gespeicherte Konten nicht erreichbar.",
            "Talk kann den lokalen Kontospeicher gerade nicht öffnen. Prüfe, ob dein Benutzerordner verfügbar und genügend Speicherplatz frei ist. Lade die Konten anschließend erneut.",
        ),
        AccountLoadFailure::UnsafeLocation => (
            "Kontospeicher am falschen Speicherort.",
            "Der App-Datenordner liegt außerhalb des unterstützten lokalen Benutzerordners. Stelle den ursprünglichen Speicherort wieder her und lade die Konten erneut.",
        ),
        AccountLoadFailure::Incomplete => (
            "Lokale Kontodaten sind unvollständig.",
            "Im Kontospeicher fehlt eine benötigte Datei. Stelle den vollständigen App-Datenordner aus einer Sicherung wieder her und lade die Konten erneut.",
        ),
        AccountLoadFailure::BindingMismatch => (
            "Kontospeicher gehört zu einem anderen Profil.",
            "Die lokalen Kontodaten passen nicht zu diesem App-Datenordner. Das kann nach Kopieren oder Wiederherstellen auftreten. Stelle den ursprünglichen Ordner wieder her und lade die Konten erneut.",
        ),
        AccountLoadFailure::UnsupportedVersion => (
            "Kontospeicher benötigt eine andere App-Version.",
            "Diese App-Version kann das Format der gespeicherten Konten nicht lesen. Verwende die zuletzt funktionierende oder eine neuere Version von Talk.",
        ),
        AccountLoadFailure::InvalidData => (
            "Lokale Kontodaten konnten nicht gelesen werden.",
            "Die Kontodaten sind beschädigt oder haben ein unerwartetes Format. Stelle den App-Datenordner aus einer Sicherung wieder her. Ohne Sicherung benötigst du Hilfe zur Wiederherstellung.",
        ),
        AccountLoadFailure::RecoveryRequired => (
            "Gespeicherte Konten konnten nicht geladen werden.",
            "Talk kann die lokalen Kontodaten nicht zuordnen und startet deshalb keine Anmeldung. Lade die Konten erneut. Bleibt der Fehler bestehen, benötigst du Hilfe zur Wiederherstellung.",
        ),
    }
}

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

pub const MAX_DRAFTS: usize = 32;

/// The admitted edit keeps its identity and text until its own receipt arrives.
/// A repeated Enter or an unrelated result must not replace that ownership.
pub struct PendingEdit {
    pub request: RequestId,
    pub message: u128,
    pub generation: u64,
    pub editor_generation: u64,
    pub body: Text<4096>,
}
impl PendingEdit {
    /// Some(true) restores the prior draft. Other correlated outcomes release
    /// admission while retaining the current edit, including newer typed text.
    /// A missing editor generation denotes active composition and never restores.
    pub fn complete(
        pending: &mut Option<Self>,
        request: RequestId,
        result: Completion,
        editing: Option<(u128, u64)>,
        body: &str,
        editor_generation: Option<u64>,
    ) -> Option<bool> {
        let submitted = pending.as_ref().filter(|edit| edit.request == request)?;
        let restore = matches!(result, Completion::ServerAccepted | Completion::Queued)
            && editing == Some((submitted.message, submitted.generation))
            && body == submitted.body.as_str()
            && editor_generation == Some(submitted.editor_generation);
        *pending = None;
        Some(restore)
    }
}

#[derive(Clone, Debug)]
struct Draft {
    context: ConversationRef,
    text: Text<4096>,
    target: talk_core::ui::messaging::ComposeTarget,
    pending: Option<(RequestId, Text<4096>)>,
    pending_origin: Option<ConversationRef>,
    pending_editor: Option<u64>,
    pending_may_clear: bool,
    uncertain: bool,
}
#[derive(Clone, Default)]
pub struct Drafts {
    entries: Vec<Draft>,
    next_request: u64,
    targets: Vec<(ConversationRef, talk_core::ui::messaging::ComposeTarget)>,
}
impl Drafts {
    pub fn restore_durable(
        &mut self,
        context: ConversationRef,
        values: &[talk_core::ui::messaging::ComposerDraft],
        thread_editor: bool,
    ) -> bool {
        let mut changed = false;
        for value in values
            .iter()
            .filter(|d| d.thread.is_some() == thread_editor)
        {
            let target = talk_core::ui::messaging::ComposeTarget {
                thread: value.thread,
                reply_to: value.reply,
                ..Default::default()
            };
            // A local entry, even cleared or pending, always wins a late load.
            if self
                .entries
                .iter()
                .any(|d| d.context == context && d.target.thread == value.thread)
                || self.entries.len() >= MAX_DRAFTS
            {
                continue;
            }
            self.entries.push(Draft {
                context,
                text: value.body.clone(),
                target: target.clone(),
                pending: None,
                pending_origin: None,
                pending_editor: None,
                pending_may_clear: false,
                uncertain: false,
            });
            if !thread_editor && self.target(context) == Default::default() {
                self.set_target(context, target);
            }
            changed = true;
        }
        changed
    }
    pub fn restored_thread_target(
        &self,
        context: ConversationRef,
        root: u128,
    ) -> talk_core::ui::messaging::ComposeTarget {
        self.entries
            .iter()
            .rev()
            .find(|d| d.context == context && d.target.thread == Some(root))
            .map(|d| d.target.clone())
            .unwrap_or(talk_core::ui::messaging::ComposeTarget {
                thread: Some(root),
                ..Default::default()
            })
    }
    pub fn persist(&self, port: &mut dyn NativeDesktopPort) -> Option<ConversationRef> {
        let mut failed = None;
        let mut written = Vec::with_capacity(MAX_DRAFTS);
        for draft in self.entries.iter().rev() {
            let key = (draft.context, draft.target.thread);
            if written.contains(&key) {
                continue;
            }
            let active = self.target(draft.context);
            if active.thread == draft.target.thread && !same_draft_target(&active, &draft.target) {
                continue;
            }
            written.push(key);
            // Cross-room private replies are deliberately memory-only. Persist
            // no quoted excerpts, attachment paths or private source identity.
            if draft.target.private_reply.is_some() {
                continue;
            }
            if port
                .save_composer_draft(
                    draft.context,
                    talk_core::ui::messaging::ComposerDraft {
                        thread: draft.target.thread,
                        reply: draft.target.reply_to,
                        body: draft.text.clone(),
                    },
                )
                .is_err()
                && !draft.text.as_str().is_empty()
            {
                failed = Some(draft.context);
            }
        }
        failed
    }
    /// Separate request namespace for the concurrently mounted thread editor.
    pub fn thread_editor() -> Self {
        Self {
            next_request: 1_u64 << 63,
            ..Self::default()
        }
    }
    /// Cached contexts carry no dispatch authority. Only an explicit stable
    /// room key can move their local text to a newly projected context.
    pub fn rebind_cached(&mut self, previous: &PresentationSnapshot, next: &PresentationSnapshot) {
        let mut contexts = Vec::with_capacity(MAX_DRAFTS);
        for context in self
            .entries
            .iter()
            .map(|draft| draft.context)
            .chain(self.targets.iter().map(|(context, _)| *context))
        {
            if contexts.len() < MAX_DRAFTS && !contexts.contains(&context) {
                contexts.push(context);
            }
        }
        for old in contexts {
            let Some(new) = cached_draft_context(previous, next, old) else {
                continue;
            };
            while let Some(index) = self.entries.iter().position(|draft| draft.context == old) {
                let mut draft = self.entries.remove(index);
                if draft.pending.is_some() && draft.pending_origin.is_none() {
                    draft.pending_origin = Some(old);
                }
                draft.context = new;
                if let Some(destination) = self.entries.iter_mut().find(|current| {
                    current.context == new && same_draft_target(&current.target, &draft.target)
                }) {
                    // The destination already owns newer text and its active
                    // compose options. Transfer receipt ownership only.
                    if draft.pending.is_some() && destination.pending.is_none() {
                        destination.pending = draft.pending;
                        destination.pending_origin = draft.pending_origin;
                        destination.pending_editor = draft.pending_editor;
                        destination.pending_may_clear = false;
                        destination.uncertain = draft.uncertain;
                    } else if let Some((_, submitted)) = draft.pending.as_ref() {
                        // Two admitted operations keep separate receipts. The
                        // existing destination stays first for editor lookup.
                        draft.text = submitted.clone();
                        draft.pending_may_clear = true;
                        self.entries.push(draft);
                    }
                } else {
                    self.entries.insert(index.min(self.entries.len()), draft);
                }
            }
            if let Some(index) = self.targets.iter().position(|(context, _)| *context == old) {
                let (_, target) = self.targets.remove(index);
                if !self.targets.iter().any(|(context, _)| *context == new) {
                    self.targets.push((new, target));
                }
            }
        }
    }

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
    pub fn has_text_for_target(
        &self,
        context: ConversationRef,
        target: &talk_core::ui::messaging::ComposeTarget,
    ) -> bool {
        self.entries.iter().any(|draft| {
            draft.context == context
                && same_draft_target(&draft.target, target)
                && !draft.text.as_str().is_empty()
        })
    }
    pub fn uncertain(&self, context: ConversationRef) -> bool {
        self.entries
            .iter()
            .any(|d| d.context == context && d.uncertain)
    }
    #[cfg(feature = "dev-probes")]
    pub(super) fn probe_request_sequence(&self) -> u64 {
        self.next_request
    }
    pub fn pending(&self, context: ConversationRef) -> bool {
        self.entries
            .iter()
            .any(|d| d.context == context && (d.pending.is_some() || d.uncertain))
    }
    /// Restart must not discard another room's draft, quoted/thread draft, or
    /// an empty editor whose previous submission is still unresolved.
    pub fn has_unsent(&self) -> bool {
        self.entries.iter().any(|draft| {
            !draft.text.as_str().is_empty() || draft.pending.is_some() || draft.uncertain
        })
    }
    /// Removing one account must explain only the drafts and unresolved sends
    /// that will actually be retired with that account.
    pub fn has_unsent_for_account(&self, account: talk_core::ui::AccountId) -> bool {
        self.entries.iter().any(|draft| {
            draft.context.account.account == account
                && (!draft.text.as_str().is_empty() || draft.pending.is_some() || draft.uncertain)
        })
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
                pending_origin: None,
                pending_editor: None,
                pending_may_clear: false,
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
        if self.pending(context) {
            return Err(SubmitError::Busy);
        }
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
        let next = self.next_request.checked_add(1).ok_or(SubmitError::Busy)?;
        if (next ^ self.next_request) & (1_u64 << 63) != 0 {
            return Err(SubmitError::Busy);
        }
        let request = RequestId(next);
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
        draft.pending_origin = Some(context);
        draft.pending_editor = None;
        draft.pending_may_clear = true;
        Ok(request)
    }
    /// Capture the editor revision immediately after a successful send, only
    /// for the admitted request in the current compose target.
    pub fn capture_editor(
        &mut self,
        context: ConversationRef,
        request: RequestId,
        generation: u64,
    ) {
        let target = self.target(context);
        if let Some(draft) = self.entries.iter_mut().find(|draft| {
            draft.context == context
                && same_draft_target(&draft.target, &target)
                && draft
                    .pending
                    .as_ref()
                    .is_some_and(|(pending, _)| *pending == request)
        }) {
            draft.pending_editor = Some(generation);
        }
    }
    /// Gate an editor reset and retain committed text when preedit or a newer
    /// revision intervenes, even if undo restores the submitted body.
    pub fn observe_editor(
        &mut self,
        context: ConversationRef,
        generation: u64,
        composing: bool,
    ) -> bool {
        let target = self.target(context);
        let Some(draft) = self
            .entries
            .iter_mut()
            .find(|draft| draft.context == context && same_draft_target(&draft.target, &target))
        else {
            return !composing;
        };
        let may_clear = !composing
            && draft
                .pending_editor
                .is_none_or(|pending| pending == generation);
        if !may_clear && draft.pending.is_some() {
            draft.pending_may_clear = false;
        }
        may_clear
    }
    pub fn submission_context(&self, result: SubmissionResult) -> Option<ConversationRef> {
        self.entries
            .iter()
            .find(|draft| {
                draft.pending_origin.unwrap_or(draft.context) == result.context
                    && draft
                        .pending
                        .as_ref()
                        .is_some_and(|(request, _)| *request == result.request)
            })
            .map(|draft| draft.context)
    }
    pub fn complete(&mut self, result: SubmissionResult) -> bool {
        let Some(draft) = self.entries.iter_mut().find(|d| {
            d.pending_origin.unwrap_or(d.context) == result.context
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
        let mut completed_reply = None;
        match result.result {
            Completion::Queued | Completion::ServerAccepted => {
                // Never erase text the user changed while the operation was in flight.
                if draft.pending_may_clear && draft.text == *submitted {
                    draft.text = Text::new("").expect("empty bounded text");
                    if draft.target.reply_to.is_some()
                        || draft.target.private_reply.is_some()
                        || draft.target.new_thread
                    {
                        completed_reply = Some((draft.context, draft.target.clone()));
                    }
                }
                draft.pending = None;
                draft.pending_origin = None;
                draft.pending_editor = None;
                draft.pending_may_clear = false;
                draft.uncertain = false;
            }
            Completion::Failed(_) => {
                draft.pending = None;
                draft.pending_origin = None;
                draft.pending_editor = None;
                draft.pending_may_clear = false;
                draft.uncertain = false;
            }
            Completion::Uncertain => {
                draft.uncertain = true;
            }
        }
        if let Some((context, submitted_target)) = completed_reply {
            let mut target = self.target(context);
            if same_draft_target(&target, &submitted_target) {
                // A quote belongs to one submission. Keep the current thread
                // and delivery option, without retiring a newer reply target.
                target.reply_to = None;
                target.private_reply = None;
                target.new_thread = false;
                self.set_target(context, target);
            }
        }
        true
    }
}

/// Defensive renderer validation. Producers still own full permission/lifecycle decisions.
pub fn valid_snapshot(snapshot: &PresentationSnapshot) -> bool {
    if snapshot
        .threads
        .account
        .is_some_and(|account| Some(account) != snapshot.active_account)
        || snapshot.threads.entries.as_slice().iter().any(|thread| {
            Some(thread.context.account) != snapshot.threads.account
                || !thread.context.is_consistent()
                || thread.root == 0
        })
    {
        return false;
    }
    if snapshot
        .reference_previews
        .as_slice()
        .iter()
        .any(|preview| {
            Some(preview.context) != snapshot.selected
                || Some(preview.context.account) != snapshot.active_account
                || !preview.context.is_consistent()
                || preview.message == 0
                || preview.ordinal >= 32
                || !snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .chain(
                        snapshot
                            .message_panel
                            .as_ref()
                            .filter(|panel| panel.context == preview.context)
                            .into_iter()
                            .flat_map(|panel| {
                                panel
                                    .thread_root_message
                                    .iter()
                                    .chain(panel.thread_messages.as_slice())
                            }),
                    )
                    .any(|message| message.local_id == preview.message && !message.metadata.deleted)
        })
    {
        return false;
    }
    if snapshot.discover_panel.as_ref().is_some_and(|panel| {
        Some(panel.account) != snapshot.active_account
            || panel
                .rooms
                .as_slice()
                .iter()
                .any(|room| room.context.account != panel.account || !room.context.is_consistent())
            || panel.joining.is_some_and(|context| {
                context.account != panel.account
                    || !context.is_consistent()
                    || !panel
                        .rooms
                        .as_slice()
                        .iter()
                        .any(|room| room.context == context)
            })
    }) {
        return false;
    }
    if snapshot.search_panel.as_ref().is_some_and(|panel| {
        Some(panel.account) != snapshot.active_account
            || panel
                .filters
                .conversation
                .is_some_and(|c| c.account != panel.account || !c.is_consistent())
            || panel
                .filters
                .person
                .is_some_and(|p| p.account != panel.account)
            || panel.hits.as_slice().iter().any(|hit| {
                hit.context.account != panel.account
                    || !hit.context.is_consistent()
                    || hit.message == 0
                    || !snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .any(|r| r.context == hit.context)
            })
    }) {
        return false;
    }
    if snapshot
        .federation_invitations
        .as_ref()
        .is_some_and(|view| Some(view.scope) != snapshot.active_account)
    {
        return false;
    }
    if snapshot.account_reminders.as_ref().is_some_and(|view| {
        Some(view.scope) != snapshot.active_account
            || view.reminders.as_slice().iter().any(|reminder| {
                reminder.context.account != view.scope || !reminder.context.is_consistent()
            })
    }) {
        return false;
    }
    if snapshot.create_panel.as_ref().is_some_and(|panel| {
        Some(panel.account) != snapshot.active_account
            || panel
                .people
                .as_slice()
                .iter()
                .any(|person| person.id.account != panel.account)
            || panel
                .invites
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

pub fn error_label(error: SubmitError) -> Option<&'static str> {
    Some(match error {
        SubmitError::Unavailable(UnavailableReason::Connection(failure)) => {
            connection_failure_copy(failure).1
        }
        SubmitError::Busy => return None,
        SubmitError::Unavailable(UnavailableReason::Busy) => {
            "Die Aktion konnte nicht abgeschlossen werden. Bitte erneut versuchen."
        }
        SubmitError::StaleScope => {
            "Diese Ansicht ist nicht mehr aktuell. Bitte öffne sie erneut und wiederhole die Aktion."
        }
        SubmitError::Unavailable(UnavailableReason::PermissionDenied) => {
            "Keine Berechtigung für diese Aktion."
        }
        SubmitError::Unavailable(UnavailableReason::PolicyUnknown) => {
            "Die Verfügbarkeit ist nicht bekannt. Bitte neu laden."
        }
        SubmitError::Unavailable(UnavailableReason::PolicyStale) => {
            "Berechtigungen müssen erneut geprüft werden."
        }
        SubmitError::Unavailable(UnavailableReason::ReauthenticationRequired) => {
            "Bitte erneut anmelden. Dein Text wurde nicht gesendet."
        }
        SubmitError::Unavailable(UnavailableReason::Unconfigured) => {
            "Die Aktion konnte nicht gestartet werden. Bitte neu laden und erneut versuchen."
        }
        SubmitError::Unavailable(UnavailableReason::Unsupported) => {
            "Diese Aktion ist für dieses Ziel derzeit nicht verfügbar."
        }
    })
}

fn same_draft_target(
    a: &talk_core::ui::messaging::ComposeTarget,
    b: &talk_core::ui::messaging::ComposeTarget,
) -> bool {
    a.reply_to == b.reply_to
        && a.thread == b.thread
        && a.private_reply.as_ref().map(|p| (&p.source, p.message))
            == b.private_reply.as_ref().map(|p| (&p.source, p.message))
}

pub(super) fn cached_draft_context(
    previous: &PresentationSnapshot,
    next: &PresentationSnapshot,
    old: ConversationRef,
) -> Option<ConversationRef> {
    if old.account.runtime.as_local() != 0 && old.conversation.host.as_local() != 0 {
        return None;
    }
    let mut source = previous
        .conversations
        .as_slice()
        .iter()
        .filter(|row| row.context == old);
    let key = source.next()?.navigation_key.as_ref()?;
    if source.any(|row| row.navigation_key.as_ref() != Some(key)) {
        return None;
    }
    let mut destinations = next.conversations.as_slice().iter().filter(|row| {
        row.context.is_consistent()
            && row.context.account.account == old.account.account
            && row.navigation_key.as_ref() == Some(key)
            && next
                .accounts
                .as_slice()
                .iter()
                .any(|account| account.scope == row.context.account)
    });
    let destination = destinations.next()?.context;
    if destination == old || destinations.any(|row| row.context != destination) {
        return None;
    }
    Some(destination)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    fn test_page<T, const N: usize>(
        values: impl IntoIterator<Item = T>,
    ) -> talk_core::ui::Page<T, N> {
        let mut page = talk_core::ui::Page::empty();
        for value in values {
            assert!(page.try_push(value).is_ok());
        }
        page
    }
    pub(in crate::presentation) fn draft_snapshot() -> PresentationSnapshot {
        use talk_core::ui::presentation::{
            AccountConnectionState, AccountView, ConversationListState, LoginPhase, LoginView,
            NativeStoreState, Presence, StorageStatus,
        };
        let context = context(1);
        let row = ConversationView {
            action_error: None,
            context,
            navigation_key: None,
            title: Text::new("Synthetic room").unwrap(),
            initials: Text::new("S").unwrap(),
            subtitle: Text::new("").unwrap(),
            updated_label: Text::new("").unwrap(),
            kind: ConversationKind::Group,
            unread: 0,
            unread_direct_mention: None,
            last_read_message: None,
            unread_mention: None,
            notification_muted: None,
            last_activity: None,
            favorite: None,
            tags: None,
            organization: Default::default(),
            archived: Some(false),
            archive: Decision::Unknown,
            presence: Presence::Unknown,
            status_icon: None,
            status_message: None,
            participants: None,
            participant_names: Page::empty(),
            send: Decision::Unknown,
            attach: Decision::Unknown,
            draft_clipboard: Decision::Unknown,
            call: Decision::Unknown,
            call_active: None,
            call_scope: None,
        };
        PresentationSnapshot {
            pending_intents: 0,
            content_panel: Default::default(),
            file_export: None,
            create_panel: None,
            discover_panel: None,
            search_panel: None,
            reference_previews: Page::empty(),
            revision: 1,
            login: LoginView {
                phase: LoginPhase::EnterServer,
                attempt: None,
                server_label: Text::new("").unwrap(),
                account_label: Text::new("").unwrap(),
                native_store: NativeStoreState::Unknown,
                storage: StorageStatus::NotSaved,
                storage_failure: None,
            },
            accounts: test_page([AccountView {
                scope: context.account,
                navigation_key: None,
                connection: AccountConnectionState::Disconnected,
                connection_failure: None,
                label: Text::new("Synthetic account").unwrap(),
                server_label: Text::new("example.invalid").unwrap(),
                self_name: Text::new("Synthetic user").unwrap(),
                initials: Text::new("S").unwrap(),
                presence: Presence::Unknown,
                storage: StorageStatus::NotSaved,
            }]),
            saved_accounts: Page::empty(),
            account_removal: None,
            active_account: Some(context.account),
            conversations: test_page([row]),
            threads: Default::default(),
            conversation_list: ConversationListState {
                content: ContentState::Ready,
                complete: true,
            },
            tags: Page::empty(),
            tag_state: ContentState::Ready,
            organization_result: None,
            account_reminders: None,
            federation_invitations: None,
            selected: Some(context),
            content: ContentState::Ready,
            messages: Page::empty(),
            files: Page::empty(),
            participants: Page::empty(),
            conversation_panel: None,
            message_panel: None,
            call: None,
            submission: None,
            mutation: None,
        }
    }
    fn cached_snapshots(
        count: usize,
    ) -> (
        PresentationSnapshot,
        PresentationSnapshot,
        PresentationSnapshot,
    ) {
        let mut cached = draft_snapshot();
        let template = cached.conversations.as_slice()[0].clone();
        let mut account = cached.accounts.as_slice()[0].clone();
        account.scope.runtime = RuntimeGeneration::from_local(0);
        let cached_scope = account.scope;
        cached.accounts = test_page([account]);
        cached.active_account = Some(cached_scope);
        cached.conversations = test_page((1..=count).map(|id| {
            let mut row = template.clone();
            row.context.account = cached_scope;
            row.context.conversation.host = HostId::from_local(0);
            row.context.conversation.local = id as u128;
            row.navigation_key = Some(
                talk_core::preferences::navigation::NavigationRoomKey::new(&format!(
                    "r1:{id:032x}"
                ))
                .unwrap(),
            );
            row
        }));
        cached.selected = cached
            .conversations
            .as_slice()
            .first()
            .map(|row| row.context);
        let mut restored = cached.clone();
        let mut account = restored.accounts.as_slice()[0].clone();
        account.scope.runtime = RuntimeGeneration::from_local(9);
        let live_scope = account.scope;
        restored.accounts = test_page([account]);
        restored.active_account = Some(live_scope);
        restored.conversations = test_page(restored.conversations.as_slice().iter().cloned().map(
            |mut row| {
                row.context.account = live_scope;
                row.context.conversation.local += 100;
                row
            },
        ));
        restored.selected = restored
            .conversations
            .as_slice()
            .first()
            .map(|row| row.context);
        let mut fresh = restored.clone();
        fresh.conversations = test_page(fresh.conversations.as_slice().iter().cloned().map(
            |mut row| {
                row.context.conversation.host = HostId::from_local(7);
                row.context.conversation.local += 100;
                row
            },
        ));
        fresh.selected = fresh
            .conversations
            .as_slice()
            .first()
            .map(|row| row.context);
        (cached, restored, fresh)
    }

    #[test]
    fn eight_cached_drafts_and_reply_targets_rebind_by_room_key_through_both_restore_stages() {
        let (cached, restored, fresh) = cached_snapshots(MAX_DRAFTS);
        let mut drafts = Drafts::default();
        for (index, row) in cached.conversations.as_slice().iter().enumerate() {
            assert!(drafts.set_target(
                row.context,
                talk_core::ui::messaging::ComposeTarget {
                    reply_to: Some(7 + index as u128),
                    silent: true,
                    ..Default::default()
                }
            ));
            drafts.save(row.context, &format!("draft {index}")).unwrap();
        }
        drafts.rebind_cached(&cached, &restored);
        drafts.retain_accounts(&restored);
        drafts.rebind_cached(&restored, &fresh);
        drafts.retain_accounts(&fresh);
        assert_eq!(drafts.entries.len(), MAX_DRAFTS);
        assert_eq!(drafts.targets.len(), MAX_DRAFTS);
        for (index, row) in fresh.conversations.as_slice().iter().enumerate() {
            assert_eq!(drafts.text(row.context), format!("draft {index}"));
            assert_eq!(drafts.target(row.context).reply_to, Some(7 + index as u128));
            assert!(drafts.target(row.context).silent);
        }
        assert!(
            cached
                .conversations
                .as_slice()
                .iter()
                .all(|row| drafts.text(row.context).is_empty())
        );
    }

    #[test]
    fn an_original_context_receipt_settles_a_rebound_send_without_double_send_or_erasing_newer_text()
     {
        for newer_text in [false, true] {
            let (cached, restored, fresh) = cached_snapshots(1);
            let old = cached.selected.unwrap();
            let current = fresh.selected.unwrap();
            let mut drafts = Drafts::default();
            let mut port = Port {
                fail: None,
                submissions: 0,
            };
            drafts.save(old, "submitted body").unwrap();
            let request = drafts.send(old, Decision::Allowed, &mut port).unwrap();
            drafts.capture_editor(old, request, 1);
            drafts.rebind_cached(&cached, &restored);
            drafts.retain_accounts(&restored);
            drafts.rebind_cached(&restored, &fresh);
            drafts.retain_accounts(&fresh);
            assert!(drafts.pending(current));
            assert_eq!(drafts.entries[0].pending_editor, Some(1));
            assert_eq!(
                drafts.send(current, Decision::Allowed, &mut port),
                Err(SubmitError::Busy)
            );
            assert!(!drafts.complete(SubmissionResult {
                request,
                context: current,
                result: Completion::Queued
            }));
            if newer_text {
                drafts.save(current, "newer body").unwrap();
            }
            assert_eq!(
                drafts.observe_editor(current, if newer_text { 2 } else { 1 }, false),
                !newer_text
            );
            let result = SubmissionResult {
                request,
                context: old,
                result: Completion::Queued,
            };
            assert_eq!(drafts.submission_context(result), Some(current));
            assert!(drafts.complete(result));
            assert!(!drafts.complete(result));
            assert_eq!(
                drafts.text(current),
                if newer_text { "newer body" } else { "" }
            );
            assert!(!drafts.pending(current));
            assert_eq!(port.submissions, 1);
        }
    }

    #[test]
    fn rebinding_never_replaces_a_newer_destination_text_or_its_compose_options() {
        let (cached, _, fresh) = cached_snapshots(1);
        let old = cached.selected.unwrap();
        let current = fresh.selected.unwrap();
        let mut drafts = Drafts::default();
        let mut port = Port {
            fail: None,
            submissions: 0,
        };
        drafts
            .save(old, "same body deliberately typed again")
            .unwrap();
        let request = drafts.send(old, Decision::Allowed, &mut port).unwrap();
        drafts.capture_editor(old, request, 1);
        assert!(drafts.set_target(
            current,
            talk_core::ui::messaging::ComposeTarget {
                silent: true,
                ..Default::default()
            }
        ));
        drafts
            .save(current, "same body deliberately typed again")
            .unwrap();
        drafts.rebind_cached(&cached, &fresh);
        drafts.retain_accounts(&fresh);
        assert!(drafts.target(current).silent);
        assert!(drafts.pending(current));
        assert_eq!(drafts.entries[0].pending_editor, Some(1));
        assert!(!drafts.observe_editor(current, 2, false));
        assert!(drafts.complete(SubmissionResult {
            request,
            context: old,
            result: Completion::Queued
        }));
        assert_eq!(drafts.text(current), "same body deliberately typed again");
        assert!(drafts.target(current).silent);
        assert_eq!(drafts.entries.len(), 1);
    }

    #[test]
    fn two_preexisting_pending_receipts_keep_their_origins_and_never_clear_each_others_editor() {
        let (cached, _, fresh) = cached_snapshots(1);
        let old = cached.selected.unwrap();
        let current = fresh.selected.unwrap();
        let mut drafts = Drafts::default();
        let mut port = Port {
            fail: None,
            submissions: 0,
        };
        drafts.save(old, "older submitted body").unwrap();
        let old_request = drafts.send(old, Decision::Allowed, &mut port).unwrap();
        drafts.capture_editor(old, old_request, 1);
        drafts.save(current, "newer submitted body").unwrap();
        let current_request = drafts.send(current, Decision::Allowed, &mut port).unwrap();
        drafts.capture_editor(current, current_request, 2);
        drafts.rebind_cached(&cached, &fresh);
        drafts.retain_accounts(&fresh);
        assert_eq!(drafts.text(current), "newer submitted body");
        assert!(drafts.observe_editor(current, 2, false));
        assert!(drafts.complete(SubmissionResult {
            request: old_request,
            context: old,
            result: Completion::Queued
        }));
        assert_eq!(drafts.text(current), "newer submitted body");
        assert!(drafts.pending(current));
        assert!(drafts.observe_editor(current, 2, false));
        assert!(drafts.complete(SubmissionResult {
            request: current_request,
            context: current,
            result: Completion::Queued
        }));
        assert_eq!(drafts.text(current), "");
        assert!(!drafts.has_unsent());
        assert_eq!(port.submissions, 2);
    }

    #[test]
    fn real_runtime_changes_ambiguous_keys_other_accounts_and_forgotten_accounts_do_not_rebind() {
        let (cached, _, fresh) = cached_snapshots(1);
        let old = cached.selected.unwrap();
        let mut wrong_key = fresh.clone();
        wrong_key.conversations = test_page(fresh.conversations.as_slice().iter().cloned().map(
            |mut row| {
                row.navigation_key = Some(
                    talk_core::preferences::navigation::NavigationRoomKey::new(
                        "r1:ffffffffffffffffffffffffffffffff",
                    )
                    .unwrap(),
                );
                row
            },
        ));
        assert!(cached_draft_context(&cached, &wrong_key, old).is_none());
        let mut other_account = fresh.clone();
        let mut account = other_account.accounts.as_slice()[0].clone();
        account.scope.account = AccountId::from_local(2);
        let foreign_scope = account.scope;
        other_account.accounts = test_page([account]);
        other_account.conversations = test_page(
            fresh
                .conversations
                .as_slice()
                .iter()
                .cloned()
                .map(|mut row| {
                    row.context.account = foreign_scope;
                    row.context.conversation.account = foreign_scope.account;
                    row
                }),
        );
        assert!(cached_draft_context(&cached, &other_account, old).is_none());
        let mut ambiguous = fresh.clone();
        let one = fresh.conversations.as_slice()[0].clone();
        let mut two = one.clone();
        two.context.conversation.local += 1;
        ambiguous.conversations = test_page([one, two]);
        assert!(cached_draft_context(&cached, &ambiguous, old).is_none());
        assert!(cached_draft_context(&fresh, &fresh, fresh.selected.unwrap()).is_none());
        let mut forgotten = fresh.clone();
        forgotten.accounts = Page::empty();
        forgotten.saved_accounts = Page::empty();
        let mut drafts = Drafts::default();
        drafts.save(old, "retire with forgotten account").unwrap();
        drafts.rebind_cached(&cached, &forgotten);
        drafts.retain_accounts(&forgotten);
        assert!(!drafts.has_unsent());
    }
    #[test]
    fn recovery_detects_a_hidden_reply_draft_without_replacing_the_current_target() {
        let room = context(1);
        let reply = talk_core::ui::messaging::ComposeTarget {
            reply_to: Some(7),
            ..Default::default()
        };
        let mut drafts = Drafts::default();
        assert!(drafts.set_target(room, reply.clone()));
        drafts.save(room, "Existing reply text").unwrap();
        assert!(drafts.set_target(room, Default::default()));
        assert!(drafts.text(room).is_empty());
        assert!(drafts.has_text_for_target(room, &reply));
        assert!(!drafts.has_text_for_target(room, &Default::default()));
        assert!(!drafts.has_text_for_target(context(2), &reply));
        assert_eq!(drafts.target(room), Default::default());
        assert!(drafts.set_target(room, reply));
        assert_eq!(drafts.text(room), "Existing reply text");
    }
    #[test]
    fn edit_receipts_only_restore_the_unchanged_admitted_editor() {
        let pending = || {
            Some(PendingEdit {
                request: RequestId(7),
                message: 42,
                generation: 1,
                editor_generation: 1,
                body: Text::new("submitted edit").unwrap(),
            })
        };
        let mut edit = pending();
        assert_eq!(
            PendingEdit::complete(
                &mut edit,
                RequestId(8),
                Completion::ServerAccepted,
                Some((42, 1)),
                "submitted edit",
                Some(1)
            ),
            None
        );
        assert!(
            edit.is_some(),
            "an unrelated receipt cannot release the admitted edit"
        );
        for completion in [Completion::ServerAccepted, Completion::Queued] {
            let mut edit = pending();
            assert_eq!(
                PendingEdit::complete(
                    &mut edit,
                    RequestId(7),
                    completion,
                    Some((42, 1)),
                    "submitted edit",
                    Some(1)
                ),
                Some(true)
            );
            assert!(edit.is_none());
        }
        for (message, body) in [
            (Some((42, 2)), "submitted edit"),
            (Some((42, 1)), "newer edit"),
            (Some((43, 1)), "submitted edit"),
            (None, "submitted edit"),
        ] {
            for completion in [Completion::ServerAccepted, Completion::Queued] {
                let mut edit = pending();
                assert_eq!(
                    PendingEdit::complete(
                        &mut edit,
                        RequestId(7),
                        completion,
                        message,
                        body,
                        Some(1)
                    ),
                    Some(false)
                );
                assert!(edit.is_none());
            }
        }
        for completion in [
            Completion::Failed(UnavailableReason::PermissionDenied),
            Completion::Uncertain,
        ] {
            let mut edit = pending();
            assert_eq!(
                PendingEdit::complete(
                    &mut edit,
                    RequestId(7),
                    completion,
                    Some((42, 1)),
                    "submitted edit",
                    Some(1)
                ),
                Some(false)
            );
            assert!(
                edit.is_none(),
                "a settled failure retains text and permits a deliberate new edit"
            );
        }
    }
    #[test]
    fn edit_receipts_never_restore_over_preedit_or_a_newer_revision_with_the_same_body() {
        for completion in [Completion::ServerAccepted, Completion::Queued] {
            for editor_generation in [None, Some(2), Some(3)] {
                let mut edit = Some(PendingEdit {
                    request: RequestId(7),
                    message: 42,
                    generation: 1,
                    editor_generation: 1,
                    body: Text::new("submitted edit").unwrap(),
                });
                assert_eq!(
                    PendingEdit::complete(
                        &mut edit,
                        RequestId(7),
                        completion,
                        Some((42, 1)),
                        "submitted edit",
                        editor_generation
                    ),
                    Some(false)
                );
                assert!(edit.is_none(), "the correlated receipt releases admission");
            }
        }
    }
    use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, Page, RuntimeGeneration};
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
    fn removal_warning_is_scoped_to_the_account_that_owns_the_draft() {
        let mut drafts = Drafts::default();
        let first = context(1);
        let mut other = context(2);
        other.account.account = AccountId::from_local(2);
        other.conversation.account = other.account.account;
        drafts
            .save(first, "Only this account loses this draft")
            .unwrap();
        assert!(drafts.has_unsent_for_account(first.account.account));
        assert!(!drafts.has_unsent_for_account(other.account.account));
        drafts.save(other, "Separate account").unwrap();
        drafts.save(first, "").unwrap();
        assert!(!drafts.has_unsent_for_account(first.account.account));
        assert!(drafts.has_unsent_for_account(other.account.account));
    }
    #[test]
    fn restart_guard_covers_other_rooms_targets_and_unresolved_empty_drafts() {
        let mut drafts = Drafts::default();
        let first = context(1);
        let second = context(2);
        let mut port = Port {
            fail: None,
            submissions: 0,
        };
        assert!(!drafts.has_unsent());
        drafts.save(first, "Retained in another room").unwrap();
        drafts.save(second, "").unwrap();
        assert!(drafts.has_unsent());
        drafts.save(first, "").unwrap();
        assert!(!drafts.has_unsent());
        assert!(drafts.set_target(
            first,
            talk_core::ui::messaging::ComposeTarget {
                reply_to: Some(42),
                ..Default::default()
            }
        ));
        drafts.save(first, "Reply draft").unwrap();
        assert!(drafts.set_target(first, Default::default()));
        assert_eq!(drafts.text(first), "");
        assert!(drafts.has_unsent());
        assert!(drafts.set_target(
            first,
            talk_core::ui::messaging::ComposeTarget {
                reply_to: Some(42),
                ..Default::default()
            }
        ));
        let request = drafts.send(first, Decision::Allowed, &mut port).unwrap();
        drafts.save(first, "").unwrap();
        assert!(drafts.has_unsent());
        assert!(drafts.complete(SubmissionResult {
            request,
            context: first,
            result: Completion::Uncertain,
        }));
        assert!(drafts.has_unsent());
        assert!(drafts.complete(SubmissionResult {
            request,
            context: first,
            result: Completion::ServerAccepted,
        }));
        assert!(!drafts.has_unsent());
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
        drafts.capture_editor(c, RequestId(1), 1);
        assert!(drafts.observe_editor(c, 2, false));
        assert_eq!(drafts.entries[0].pending_editor, None);
        assert!(drafts.send(c, Decision::Unknown, &mut port).is_err());
        assert_eq!(port.submissions, 1);
    }
    #[test]
    fn send_receipts_keep_the_same_committed_body_after_preedit_or_undo() {
        for completion in [Completion::ServerAccepted, Completion::Queued] {
            for composing in [true, false] {
                let mut drafts = Drafts::default();
                let c = context(1);
                let mut port = Port {
                    fail: None,
                    submissions: 0,
                };
                drafts.save(c, "A").unwrap();
                let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
                drafts.capture_editor(c, request, 1);
                if !composing {
                    drafts.save(c, "AB").unwrap();
                    drafts.save(c, "A").unwrap();
                }
                assert!(!drafts.observe_editor(c, if composing { 2 } else { 3 }, composing));
                assert_eq!(drafts.text(c), "A");
                assert!(drafts.complete(SubmissionResult {
                    request,
                    context: c,
                    result: completion,
                }));
                assert_eq!(drafts.text(c), "A");
                assert!(!drafts.pending(c));
                assert_eq!(drafts.entries[0].pending_editor, None);
                assert_eq!(port.submissions, 1);
            }
        }
    }
    #[test]
    fn editor_observation_and_capture_only_affect_the_matching_context_target_and_request() {
        use talk_core::ui::messaging::ComposeTarget;
        let mut drafts = Drafts::default();
        let c = context(1);
        let mut port = Port {
            fail: None,
            submissions: 0,
        };
        drafts.save(c, "A").unwrap();
        let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
        drafts.capture_editor(c, request, 1);
        let mut other_host = c;
        other_host.conversation.host = HostId::from_local(8);
        let mut other_runtime = c;
        other_runtime.account.runtime = RuntimeGeneration::from_local(2);
        for other in [context(2), other_host, other_runtime] {
            drafts.capture_editor(other, request, 2);
            assert!(!drafts.observe_editor(other, 2, true));
            assert!(drafts.observe_editor(other, 2, false));
            assert!(!drafts.complete(SubmissionResult {
                request,
                context: other,
                result: Completion::ServerAccepted,
            }));
        }
        let other_request = RequestId(request.0 + 1);
        drafts.capture_editor(c, other_request, 2);
        assert!(!drafts.complete(SubmissionResult {
            request: other_request,
            context: c,
            result: Completion::ServerAccepted,
        }));
        assert!(drafts.set_target(
            c,
            ComposeTarget {
                reply_to: Some(7),
                ..Default::default()
            }
        ));
        drafts.save(c, "reply draft").unwrap();
        drafts.capture_editor(c, request, 2);
        assert!(!drafts.observe_editor(c, 2, true));
        assert_eq!(drafts.entries.len(), 2);
        assert!(drafts.set_target(c, ComposeTarget::default()));
        assert_eq!(drafts.text(c), "A");
        assert!(drafts.pending(c));
        assert!(drafts.observe_editor(c, 1, false));
        assert!(drafts.complete(SubmissionResult {
            request,
            context: c,
            result: Completion::ServerAccepted,
        }));
        assert_eq!(drafts.text(c), "");
        assert!(drafts.has_text_for_target(
            c,
            &ComposeTarget {
                reply_to: Some(7),
                ..Default::default()
            }
        ));
        assert_eq!(port.submissions, 1);
    }
    #[test]
    fn unchanged_editor_revisions_clear_normally_and_retire_with_their_submission() {
        for completion in [Completion::ServerAccepted, Completion::Queued] {
            let mut drafts = Drafts::default();
            let c = context(1);
            let mut port = Port {
                fail: None,
                submissions: 0,
            };
            drafts.save(c, "A").unwrap();
            let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
            drafts.capture_editor(c, request, 1);
            assert!(drafts.observe_editor(c, 1, false));
            assert!(drafts.complete(SubmissionResult {
                request,
                context: c,
                result: completion,
            }));
            assert_eq!(drafts.text(c), "");
            assert!(!drafts.pending(c));
            assert!(drafts.observe_editor(c, 2, false));
            drafts.save(c, "B").unwrap();
            let next_request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
            assert!(drafts.observe_editor(c, 2, false));
            drafts.capture_editor(c, next_request, 2);
            assert!(drafts.observe_editor(c, 2, false));
            assert!(drafts.complete(SubmissionResult {
                request: next_request,
                context: c,
                result: completion,
            }));
            assert_eq!(drafts.text(c), "");
            assert_eq!(port.submissions, 2);
        }
    }
    #[test]
    fn successful_reply_retires_the_quote_and_restores_the_destination_draft() {
        use talk_core::ui::messaging::ComposeTarget;
        for completion in [Completion::Queued, Completion::ServerAccepted] {
            for thread in [None, Some(9)] {
                for silent in [false, true] {
                    let c = context(1);
                    let mut drafts = Drafts::default();
                    let mut port = Port {
                        fail: None,
                        submissions: 0,
                    };
                    let destination = ComposeTarget {
                        thread,
                        silent,
                        ..Default::default()
                    };
                    assert!(drafts.set_target(c, destination.clone()));
                    drafts.save(c, "Unsent destination draft").unwrap();
                    let reply = ComposeTarget {
                        reply_to: Some(7),
                        ..destination.clone()
                    };
                    assert!(drafts.set_target(c, reply.clone()));
                    drafts.save(c, "Quoted reply").unwrap();
                    let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
                    drafts.capture_editor(c, request, 1);
                    assert_eq!(drafts.target(c), reply, "admission alone retains the quote");
                    assert!(drafts.observe_editor(c, 1, false));
                    assert!(drafts.complete(SubmissionResult {
                        request,
                        context: c,
                        result: completion,
                    }));
                    assert_eq!(drafts.target(c), destination);
                    assert_eq!(drafts.text(c), "Unsent destination draft");
                    assert!(!drafts.has_text_for_target(c, &reply));
                    assert!(!drafts.pending(c));
                }
            }
        }
    }
    #[test]
    fn reply_admission_failure_and_uncertain_receipts_keep_the_quote_for_recovery() {
        use talk_core::ui::messaging::ComposeTarget;
        let c = context(1);
        let mut drafts = Drafts::default();
        let mut port = Port {
            fail: Some(SubmitError::Busy),
            submissions: 0,
        };
        let reply = ComposeTarget {
            reply_to: Some(7),
            ..Default::default()
        };
        assert!(drafts.set_target(c, reply.clone()));
        drafts.save(c, "Quoted reply").unwrap();
        assert_eq!(
            drafts.send(c, Decision::Allowed, &mut port),
            Err(SubmitError::Busy)
        );
        assert_eq!(drafts.target(c), reply);
        assert_eq!(drafts.text(c), "Quoted reply");
        assert!(!drafts.pending(c));
        port.fail = None;
        for completion in [
            Completion::Failed(UnavailableReason::Unconfigured),
            Completion::Uncertain,
        ] {
            let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
            assert!(drafts.complete(SubmissionResult {
                request,
                context: c,
                result: completion,
            }));
            assert_eq!(drafts.target(c), reply);
            assert_eq!(drafts.text(c), "Quoted reply");
            assert_eq!(drafts.pending(c), completion == Completion::Uncertain);
            if completion == Completion::Uncertain {
                assert!(drafts.complete(SubmissionResult {
                    request,
                    context: c,
                    result: Completion::ServerAccepted,
                }));
                assert_eq!(drafts.target(c), ComposeTarget::default());
                assert_eq!(drafts.text(c), "");
            }
        }
    }
    #[test]
    fn reply_receipts_do_not_retire_newer_text_preedit_or_undone_edits() {
        use talk_core::ui::messaging::ComposeTarget;
        for completion in [Completion::Queued, Completion::ServerAccepted] {
            for (body, generation, composing) in [
                ("New reply draft", 2, false),
                ("Quoted reply", 2, true),
                ("Quoted reply", 3, false),
            ] {
                let c = context(1);
                let mut drafts = Drafts::default();
                let mut port = Port {
                    fail: None,
                    submissions: 0,
                };
                let reply = ComposeTarget {
                    reply_to: Some(7),
                    ..Default::default()
                };
                assert!(drafts.set_target(c, reply.clone()));
                drafts.save(c, "Quoted reply").unwrap();
                let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
                drafts.capture_editor(c, request, 1);
                drafts.save(c, body).unwrap();
                assert!(!drafts.observe_editor(c, generation, composing));
                assert!(drafts.complete(SubmissionResult {
                    request,
                    context: c,
                    result: completion,
                }));
                assert_eq!(drafts.target(c), reply);
                assert_eq!(drafts.text(c), body);
                assert!(!drafts.pending(c));
            }
        }
    }
    #[test]
    fn reply_receipts_do_not_switch_another_active_compose_target() {
        use talk_core::ui::messaging::ComposeTarget;
        for next_target in [
            ComposeTarget::default(),
            ComposeTarget {
                reply_to: Some(8),
                ..Default::default()
            },
            ComposeTarget {
                thread: Some(9),
                ..Default::default()
            },
        ] {
            let c = context(1);
            let mut drafts = Drafts::default();
            let mut port = Port {
                fail: None,
                submissions: 0,
            };
            let reply = ComposeTarget {
                reply_to: Some(7),
                ..Default::default()
            };
            assert!(drafts.set_target(c, reply.clone()));
            drafts.save(c, "Quoted reply").unwrap();
            let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
            drafts.capture_editor(c, request, 1);
            assert!(drafts.set_target(c, next_target.clone()));
            drafts.save(c, "Different draft").unwrap();
            assert!(drafts.observe_editor(c, 2, false));
            assert!(drafts.complete(SubmissionResult {
                request,
                context: c,
                result: Completion::Queued,
            }));
            assert_eq!(drafts.target(c), next_target);
            assert_eq!(drafts.text(c), "Different draft");
            assert!(!drafts.has_text_for_target(c, &reply));
        }
    }
    #[test]
    fn creating_a_thread_reuses_the_draft_and_clears_only_after_its_admission() {
        use talk_core::ui::messaging::ComposeTarget;
        let c = context(1);
        let mut drafts = Drafts::default();
        let mut port = Port {
            fail: None,
            submissions: 0,
        };
        drafts.save(c, "A new discussion").unwrap();
        assert!(drafts.set_target(
            c,
            ComposeTarget {
                new_thread: true,
                ..Default::default()
            }
        ));
        assert_eq!(drafts.text(c), "A new discussion");
        drafts.save(c, "A new discussion").unwrap();
        let request = drafts.send(c, Decision::Allowed, &mut port).unwrap();
        assert!(drafts.target(c).new_thread);
        assert!(drafts.complete(SubmissionResult {
            request,
            context: c,
            result: Completion::Queued
        }));
        assert!(!drafts.target(c).new_thread);
        assert!(drafts.text(c).is_empty());
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
        drafts.capture_editor(c, request, 1);
        drafts.complete(SubmissionResult {
            request,
            context: c,
            result: Completion::Uncertain,
        });
        assert!(!drafts.observe_editor(c, 2, false));
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
        assert!(drafts.observe_editor(c, 2, false));
    }
    #[test]
    fn draft_capacity_rejects_navigation_without_eviction_and_scopes_hosts() {
        let mut drafts = Drafts::default();
        for i in 1..=MAX_DRAFTS as u128 {
            drafts.save(context(i), "Wichtig").unwrap();
        }
        drafts.capture_editor(context(MAX_DRAFTS as u128 + 1), RequestId(1), 1);
        assert!(drafts.observe_editor(context(MAX_DRAFTS as u128 + 1), 1, false));
        assert!(!drafts.observe_editor(context(MAX_DRAFTS as u128 + 1), 1, true));
        assert_eq!(drafts.entries.len(), MAX_DRAFTS);
        assert_eq!(
            drafts.save(context(MAX_DRAFTS as u128 + 1), "Neun"),
            Err(SubmitError::Busy)
        );
        for i in 1..=MAX_DRAFTS as u128 {
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
    fn pane_editor_admission_and_receipts_preserve_the_independent_room_draft() {
        use talk_core::ui::messaging::{ComposeTarget, MessageAction};
        #[derive(Default)]
        struct Capture(Vec<UiIntent>);
        impl NativeDesktopPort for Capture {
            fn try_submit(&mut self, intent: UiIntent) -> Result<(), SubmitError> {
                self.0.push(intent);
                Ok(())
            }
            fn take_snapshot(&mut self) -> Option<PresentationSnapshot> {
                None
            }
        }
        let c = context(1);
        let mut room = Drafts::default();
        room.save(c, "Unsent room draft").unwrap();
        let mut pane = Drafts::thread_editor();
        let target = ComposeTarget {
            thread: Some(7),
            reply_to: Some(8),
            ..Default::default()
        };
        assert!(pane.set_target(c, target.clone()));
        pane.save(c, "Quoted pane reply").unwrap();
        let mut port = Capture::default();
        let request = pane.send(c, Decision::Allowed, &mut port).unwrap();
        assert!(
            matches!(&port.0[0], UiIntent::Message { context, action: MessageAction::Send { body, target: sent }, .. }
            if *context == c && body.as_str() == "Quoted pane reply" && sent == &target)
        );
        assert_eq!(
            pane.send(c, Decision::Allowed, &mut port),
            Err(SubmitError::Busy)
        );
        assert_eq!(port.0.len(), 1);
        let mut retired = c;
        retired.account.runtime = talk_core::ui::RuntimeGeneration::from_local(99);
        assert!(!pane.complete(SubmissionResult {
            request,
            context: retired,
            result: Completion::Queued
        }));
        assert_eq!(pane.text(c), "Quoted pane reply");
        assert!(pane.complete(SubmissionResult {
            request,
            context: c,
            result: Completion::Queued
        }));
        assert_eq!(pane.text(c), "");
        assert_eq!(pane.target(c).thread, Some(7));
        assert!(pane.target(c).reply_to.is_none());
        assert_eq!(room.text(c), "Unsent room draft");
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
            action_error: None,
            tags: None,
            context: c,
            navigation_key: None,
            title: Text::new("Room").unwrap(),
            initials: Text::new("R").unwrap(),
            subtitle: Text::new("").unwrap(),
            updated_label: Text::new("").unwrap(),
            kind: ConversationKind::Direct,
            unread: 0,
            unread_direct_mention: None,
            last_read_message: None,
            unread_mention: None,
            notification_muted: None,
            last_activity: None,
            favorite: None,
            organization: Default::default(),
            archived: None,
            archive: Decision::Unknown,
            presence: talk_core::ui::presentation::Presence::Unknown,
            status_icon: None,
            status_message: None,
            participants: None,
            participant_names: Page::empty(),
            send: Decision::Unknown,
            attach: Decision::Unknown,
            draft_clipboard: Decision::Unknown,
            call: Decision::Allowed,
            call_active: None,
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

    #[test]
    fn connection_failures_belong_only_to_the_observed_account_runtime() {
        use talk_core::ui::presentation::{
            AccountConnectionState, AccountView, ConnectionFailure, Presence, StorageStatus,
        };
        let scope = context(1).account;
        let mut accounts = talk_core::ui::Page::empty();
        accounts
            .try_push(AccountView {
                scope,
                navigation_key: None,
                connection: AccountConnectionState::Disconnected,
                connection_failure: Some(ConnectionFailure::AccessDenied),
                label: Text::new("Fixture").unwrap(),
                server_label: Text::new("Fixture server").unwrap(),
                self_name: Text::new("Fixture").unwrap(),
                initials: Text::new("F").unwrap(),
                presence: Presence::Unknown,
                storage: StorageStatus::NotSaved,
            })
            .unwrap();
        assert_eq!(
            account_connection_failure(&accounts, scope),
            Some(ConnectionFailure::AccessDenied)
        );
        let mut other = scope;
        other.account = AccountId::from_local(2);
        assert_eq!(account_connection_failure(&accounts, other), None);
        other = scope;
        other.runtime = RuntimeGeneration::from_local(99);
        assert_eq!(account_connection_failure(&accounts, other), None);
    }

    #[test]
    fn offline_sends_stage_multiple_drafts_despite_connection_failure_without_granting_dispatch() {
        use talk_core::ui::{messaging::MessagePanel, presentation::ConnectionFailure};
        let mut snapshot = draft_snapshot();
        let c = snapshot.selected.unwrap();
        snapshot.content = ContentState::Offline;
        snapshot.accounts = test_page(snapshot.accounts.as_slice().iter().cloned().map(|mut a| {
            a.connection_failure = Some(ConnectionFailure::Unreachable);
            a
        }));
        let mut panel = MessagePanel::new(c);
        panel.queue_available = true;
        snapshot.message_panel = Some(panel);
        let mut row = snapshot.conversations.as_slice()[0].clone();
        row.send = Decision::Unknown;
        let mut drafts = Drafts::default();
        let mut port = Port {
            fail: None,
            submissions: 0,
        };
        for body in ["First offline message", "Second offline message"] {
            drafts.save(c, body).unwrap();
            let decision = composer_send_decision(&snapshot, &row);
            assert_eq!(
                decision,
                Decision::Allowed,
                "local staging is independent of transport"
            );
            let request = drafts.send(c, decision, &mut port).unwrap();
            assert_eq!(drafts.text(c), body, "keep text until staging completes");
            assert_eq!(drafts.send(c, decision, &mut port), Err(SubmitError::Busy));
            assert!(drafts.complete(SubmissionResult {
                request,
                context: c,
                result: Completion::Queued,
            }));
            assert_eq!(drafts.text(c), "");
        }
        assert_eq!(port.submissions, 2);
        row.send = Decision::Denied;
        assert_ne!(composer_send_decision(&snapshot, &row), Decision::Allowed);
        row.send = Decision::Unknown;
        snapshot.content = ContentState::Denied;
        assert_ne!(composer_send_decision(&snapshot, &row), Decision::Allowed);
        snapshot.content = ContentState::Offline;
        snapshot.message_panel.as_mut().unwrap().queue_available = false;
        assert_ne!(composer_send_decision(&snapshot, &row), Decision::Allowed);
        snapshot.message_panel.as_mut().unwrap().queue_available = true;
        snapshot.message_panel.as_mut().unwrap().context = context(2);
        assert_ne!(composer_send_decision(&snapshot, &row), Decision::Allowed);
    }
}

#[cfg(test)]
mod durable_draft_tests {
    use super::*;
    #[test]
    fn late_disk_draft_never_overwrites_local_text_and_threads_remain_separate() {
        let c = ConversationRef {
            account: talk_core::ui::AccountScope {
                account: talk_core::ui::AccountId::from_local(1),
                runtime: talk_core::ui::RuntimeGeneration::from_local(2),
            },
            conversation: talk_core::ui::ConversationId {
                account: talk_core::ui::AccountId::from_local(1),
                host: talk_core::ui::HostId::from_local(3),
                local: 4,
            },
        };
        let disk = talk_core::ui::messaging::ComposerDraft {
            thread: None,
            reply: Some(9),
            body: Text::new("Disk").unwrap(),
        };
        let mut drafts = Drafts::default();
        drafts.save(c, "Newer").unwrap();
        assert!(!drafts.restore_durable(c, std::slice::from_ref(&disk), false));
        assert_eq!(drafts.text(c), "Newer");
        let mut restored = Drafts::default();
        assert!(restored.restore_durable(c, &[disk], false));
        assert_eq!(restored.text(c), "Disk");
        assert_eq!(restored.target(c).reply_to, Some(9));
        let thread = talk_core::ui::messaging::ComposerDraft {
            thread: Some(7),
            reply: Some(8),
            body: Text::new("Thread").unwrap(),
        };
        assert!(!restored.restore_durable(c, std::slice::from_ref(&thread), false));
        let mut threads = Drafts::thread_editor();
        assert!(threads.restore_durable(c, &[thread], true));
        threads.set_target(c, threads.restored_thread_target(c, 7));
        assert_eq!(threads.text(c), "Thread");
    }
}
