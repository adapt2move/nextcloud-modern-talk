//! Explicit permission actions, correlated to the current Devices entry.
use super::{
    ButtonStyle, Desktop, LocalAction, Overlay, controls::Availability, preferences::SettingsTab,
};
use gpui::{Context, Div, FontWeight, div, prelude::*};
use talk_core::preferences::permissions::{
    PermissionAction, PermissionError, PermissionGuidance, PermissionIntent, PermissionKind,
    PermissionObservation, PermissionOperation, PermissionSnapshot, PermissionStatus,
};
use talk_core::preferences::{RequestId, RuntimeState, SaveState};

#[derive(Clone, Copy)]
struct Pending {
    request: RequestId,
    kind: PermissionKind,
    action: PermissionAction,
    entry: u64,
    setup: Option<(talk_core::ui::AccountScope, u64)>,
}
pub(super) struct PermissionUi {
    pub snapshot: Result<PermissionSnapshot, PermissionError>,
    request: u64,
    pending: Option<Pending>,
    checks: [bool; 3],
    entry: u64,
    pub microphone_ready: bool,
    error: Option<PermissionError>,
}
impl PermissionUi {
    pub fn new(snapshot: Result<PermissionSnapshot, PermissionError>) -> Self {
        Self {
            snapshot,
            request: 0,
            pending: None,
            checks: [false; 3],
            entry: 0,
            microphone_ready: false,
            error: None,
        }
    }

    fn feedback(&self, _kind: PermissionKind) -> Option<&'static str> {
        // A failed snapshot must stay visible even when no correlated completion
        // arrived. Do not infer a grant or consume an uncorrelated operation here.
        if let Some(message) = self
            .snapshot
            .as_ref()
            .err()
            .copied()
            .and_then(permission_error_key)
            .or_else(|| self.error.and_then(permission_error_key))
        {
            return Some(message);
        }
        if self
            .snapshot
            .as_ref()
            .is_ok_and(|s| s.runtime != RuntimeState::Running)
        {
            return Some("Die Zugriffsprüfung ist nicht verfügbar.");
        }
        None
    }

    fn record_submission(&mut self, pending: Pending, result: Result<(), PermissionError>) {
        match result {
            Ok(()) => {
                self.pending = Some(pending);
                self.error = None;
                if pending.kind == PermissionKind::Microphone {
                    self.microphone_ready = false;
                }
            }
            Err(PermissionError::Busy) => {}
            Err(error) => self.error = Some(error),
        }
    }

    fn apply_snapshot(
        &mut self,
        snapshot: Result<PermissionSnapshot, PermissionError>,
    ) -> Option<(Pending, Option<PermissionError>)> {
        self.snapshot = snapshot;
        let pending = self.pending?;
        let snapshot = self.snapshot.as_ref().ok()?;
        let (request, kind, action, error) = match snapshot.operation {
            PermissionOperation::Finished {
                request,
                kind,
                action,
            } => (request, kind, action, None),
            PermissionOperation::Failed {
                request,
                kind,
                action,
                error,
            } => (request, kind, action, Some(error)),
            _ => return None,
        };
        if request != pending.request || kind != pending.kind || action != pending.action {
            return None;
        }
        self.pending = None;
        self.error = error;
        Some((pending, error))
    }

    fn availability(&self) -> Availability {
        if self.pending.is_some() {
            Availability::pending()
        } else {
            Availability::when(
                self.snapshot
                    .as_ref()
                    .is_ok_and(|s| s.runtime == RuntimeState::Running),
                "Die Zugriffsprüfung ist nicht verfügbar.",
            )
        }
    }
}
const KINDS: [PermissionKind; 3] = [
    PermissionKind::Microphone,
    PermissionKind::Camera,
    PermissionKind::ScreenSharing,
];
fn index(kind: PermissionKind) -> usize {
    match kind {
        PermissionKind::Microphone => 0,
        PermissionKind::Camera => 1,
        PermissionKind::ScreenSharing => 2,
    }
}

struct PermissionHelp {
    status: &'static str,
    explanation: Option<&'static str>,
    steps: &'static [&'static str],
    request_label: Option<&'static str>,
    note: Option<&'static str>,
}

pub(super) fn permission_identity(kind: PermissionKind) -> (&'static str, &'static str) {
    match kind {
        PermissionKind::Microphone => (
            "mic",
            "Mikrofonzugriff wird benötigt, damit andere dich hören.",
        ),
        PermissionKind::Camera => (
            "video",
            "Kamerazugriff wird benötigt, damit andere dich sehen.",
        ),
        PermissionKind::ScreenSharing => (
            "monitor-up",
            "Bildschirmaufnahme wird benötigt, um deinen Bildschirm zu teilen.",
        ),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PermissionPlatform {
    MacOS,
    Windows,
    Linux,
    Other,
}
impl PermissionPlatform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOS
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

/// Platform copy never grants access: action capabilities still come from the
/// permission owner, and only a new observation can confirm a grant.
fn permission_help(
    observation: PermissionObservation,
    platform: PermissionPlatform,
) -> PermissionHelp {
    let macos = platform == PermissionPlatform::MacOS;
    let status = match observation.status {
        PermissionStatus::Unknown => "Zugriffsstatus unbekannt.",
        PermissionStatus::NotDetermined => "Zugriff noch nicht freigegeben.",
        PermissionStatus::Granted => "Zugriff erlaubt.",
        PermissionStatus::Denied => "Zugriff verweigert.",
        PermissionStatus::Restricted => "Zugriff durch das System eingeschränkt.",
        PermissionStatus::PerSession => "Für jede Bildschirmfreigabe wird eine Auswahl benötigt.",
        PermissionStatus::NotRequired => "Der Gerätezugriff wird beim Öffnen geprüft.",
        PermissionStatus::Unsupported => "Dieser Zugriff kann hier nicht geprüft werden.",
        PermissionStatus::Unavailable => "Die Zugriffsprüfung ist nicht verfügbar.",
    };
    let request_label = if observation.can_request {
        match observation.status {
            PermissionStatus::NotDetermined => Some("Zugriff erlauben"),
            // The screen helper can support a request while returning Unknown.
            // Keep it explicit without presenting that observation as a first grant.
            PermissionStatus::Unknown => Some("Zugriff anfragen"),
            _ => None,
        }
    } else {
        None
    };
    let mut explanation = match observation.status {
        PermissionStatus::Restricted => Some(
            "Eine System- oder Verwaltungsrichtlinie sperrt den Zugriff. Wende dich an die Administration.",
        ),
        PermissionStatus::Unknown => {
            Some("Prüfe den Zugriff erneut, um den aktuellen Status zu erfahren.")
        }
        PermissionStatus::Unavailable => {
            Some("Die Prüfung ist derzeit nicht möglich. Bitte versuche es später erneut.")
        }
        PermissionStatus::NotDetermined | PermissionStatus::Denied if !macos => {
            if observation.can_open_settings {
                Some(
                    "Prüfe die Zugriffsrechte in den Systemeinstellungen und wähle danach „Erneut prüfen“.",
                )
            } else {
                None
            }
        }
        _ => None,
    };
    let mut steps: &'static [&'static str] = if macos {
        match observation.status {
            PermissionStatus::NotDetermined if request_label.is_some() => &[
                "Wähle „Zugriff erlauben“.",
                "Bestätige die Anfrage von macOS, wenn sie erscheint.",
                "Kehre hierher zurück und wähle „Erneut prüfen“.",
            ],
            PermissionStatus::NotDetermined | PermissionStatus::Denied => match observation.kind {
                PermissionKind::Microphone => &[
                    "Öffne „Datenschutz & Sicherheit“ in den Systemeinstellungen.",
                    "Wähle „Mikrofon“ und aktiviere NextCloud Modern Talk.",
                    "Kehre hierher zurück und wähle „Erneut prüfen“.",
                ],
                PermissionKind::Camera => &[
                    "Öffne „Datenschutz & Sicherheit“ in den Systemeinstellungen.",
                    "Wähle „Kamera“ und aktiviere NextCloud Modern Talk.",
                    "Kehre hierher zurück und wähle „Erneut prüfen“.",
                ],
                PermissionKind::ScreenSharing => &[
                    "Öffne „Datenschutz & Sicherheit“ in den Systemeinstellungen.",
                    "Wähle „Aufnahme von Bildschirm & Systemaudio“ und aktiviere NextCloud Modern Talk.",
                    "Kehre hierher zurück und wähle „Erneut prüfen“.",
                ],
            },
            // Screen preflight cannot distinguish denial from never requested.
            // An unavailable helper also says nothing about the OS grant. Offer
            // inspection, not a claim that changing Settings repairs the check.
            PermissionStatus::Unknown | PermissionStatus::Unavailable
                if observation.can_open_settings =>
            {
                match observation.kind {
                    PermissionKind::Microphone => &[
                        "Öffne „Datenschutz & Sicherheit“ in den Systemeinstellungen.",
                        "Wähle „Mikrofon“ und prüfe den Zugriff für NextCloud Modern Talk.",
                        "Kehre hierher zurück und wähle „Erneut prüfen“.",
                    ],
                    PermissionKind::Camera => &[
                        "Öffne „Datenschutz & Sicherheit“ in den Systemeinstellungen.",
                        "Wähle „Kamera“ und prüfe den Zugriff für NextCloud Modern Talk.",
                        "Kehre hierher zurück und wähle „Erneut prüfen“.",
                    ],
                    PermissionKind::ScreenSharing => &[
                        "Öffne „Datenschutz & Sicherheit“ in den Systemeinstellungen.",
                        "Wähle „Aufnahme von Bildschirm & Systemaudio“ und prüfe den Zugriff für NextCloud Modern Talk.",
                        "Kehre hierher zurück und wähle „Erneut prüfen“.",
                    ],
                }
            }
            _ => &[],
        }
    } else {
        &[]
    };
    if platform == PermissionPlatform::Windows
        && observation.can_open_settings
        && matches!(
            observation.status,
            PermissionStatus::NotDetermined | PermissionStatus::Denied | PermissionStatus::Unknown
        )
    {
        steps = match observation.kind {
            PermissionKind::Microphone => &[
                "Öffne in den Windows-Einstellungen „Datenschutz & Sicherheit → Mikrofon“.",
                "Erlaube den Mikrofonzugriff für Desktop-Apps.",
                "Kehre hierher zurück und wähle „Erneut prüfen“.",
            ],
            PermissionKind::Camera => &[
                "Öffne in den Windows-Einstellungen „Datenschutz & Sicherheit → Kamera“.",
                "Erlaube den Kamerazugriff für Desktop-Apps.",
                "Kehre hierher zurück und wähle „Erneut prüfen“.",
            ],
            PermissionKind::ScreenSharing => &[],
        };
        explanation = None;
    }
    if platform == PermissionPlatform::Linux {
        explanation = match (observation.kind, observation.status) {
            (_, PermissionStatus::NotRequired) => {
                Some("Linux prüft den Gerätezugriff beim Starten von Mikrofon oder Kamera.")
            }
            (PermissionKind::ScreenSharing, PermissionStatus::PerSession) => Some(
                "Unter Linux wählst du das Fenster oder den Bildschirm für jede Freigabe im Systemdialog aus.",
            ),
            (
                PermissionKind::Microphone | PermissionKind::Camera,
                PermissionStatus::Unknown | PermissionStatus::Denied,
            ) => Some(
                "Prüfe die Gerätefreigabe in den Einstellungen deiner Desktop-Umgebung oder Sandbox.",
            ),
            _ => explanation,
        };
    }
    let note = match observation.guidance {
        PermissionGuidance::RestartRequired
            if macos
                && observation.kind == PermissionKind::ScreenSharing
                && observation.status == PermissionStatus::Unknown =>
        {
            Some(
                "macOS hat die Freigabe bestätigt. Starte die App neu, damit dieser Prozess sie verwenden kann.",
            )
        }
        PermissionGuidance::RestartAfterGrant
            if macos
                && observation.kind == PermissionKind::ScreenSharing
                && observation.status == PermissionStatus::Granted =>
        {
            Some("Starte die App nur neu, wenn macOS dich nach der Freigabe dazu auffordert.")
        }
        PermissionGuidance::HelperUnavailable => {
            Some("Die native Zugriffsprüfung ist auf diesem System noch nicht eingerichtet.")
        }
        PermissionGuidance::BackendUnavailable => {
            Some("Die native Bildschirmfreigabe ist noch nicht verfügbar.")
        }
        PermissionGuidance::SelectForEachShare if platform == PermissionPlatform::Windows => Some(
            "Unter Windows wird das Fenster oder der Bildschirm erst beim Start einer Freigabe ausgewählt.",
        ),
        _ => None,
    };
    PermissionHelp {
        status,
        explanation,
        steps,
        request_label,
        note,
    }
}

fn permission_error_key(error: PermissionError) -> Option<&'static str> {
    Some(match error {
        PermissionError::Stale => {
            "Die Zugriffsanfrage ist nicht mehr aktuell. Bitte erneut prüfen."
        }
        PermissionError::Busy => return None,
        PermissionError::Closed => "Die Zugriffsprüfung wurde beendet.",
        PermissionError::Unsupported => "Diese Zugriffsaktion wird hier nicht unterstützt.",
        PermissionError::Unavailable => "Die Zugriffsprüfung ist nicht verfügbar.",
        PermissionError::Failed => "Die Zugriffsaktion ist fehlgeschlagen. Bitte erneut prüfen.",
        PermissionError::TimedOut => {
            "Die Zugriffsanfrage hat zu lange gedauert. Bitte erneut prüfen."
        }
        PermissionError::Cancelled => "Die Zugriffsanfrage wurde abgebrochen.",
    })
}

impl Desktop {
    pub(super) fn permission_operation_idle(&self) -> bool {
        self.permission_ui.pending.is_none()
            && self
                .permission_ui
                .error
                .and_then(permission_error_key)
                .is_none()
            && self
                .permission_ui
                .snapshot
                .as_ref()
                .is_ok_and(|snapshot| snapshot.runtime == RuntimeState::Running)
    }
    pub fn devices_open(&self) -> bool {
        self.overlay == Some(Overlay::Settings)
            && self.settings_tab == SettingsTab::Devices
            && !self.incoming_visible()
    }
    pub(super) fn enter_device_page(&mut self) {
        let Some(entry) = self.permission_ui.entry.checked_add(1) else {
            return;
        };
        self.permission_ui.entry = entry;
        self.permission_ui.microphone_ready = false;
        self.microphone_auto_attempted = false;
        self.queue_permission_check(PermissionKind::Microphone);
        self.queue_permission_check(PermissionKind::Camera);
        self.enumerate_devices();
    }
    pub(super) fn observation(&self, kind: PermissionKind) -> PermissionObservation {
        self.permission_ui
            .snapshot
            .as_ref()
            .map_or(PermissionObservation::unknown(kind), |s| match kind {
                PermissionKind::Microphone => s.microphone,
                PermissionKind::Camera => s.camera,
                PermissionKind::ScreenSharing => s.screen_sharing,
            })
    }
    pub(super) fn queue_permission_check(&mut self, kind: PermissionKind) {
        self.permission_ui.checks[index(kind)] = true;
        self.dispatch_permission_check();
    }
    fn dispatch_permission_check(&mut self) {
        if self.permission_ui.pending.is_some() {
            return;
        }
        if let Some(kind) = KINDS
            .into_iter()
            .find(|k| self.permission_ui.checks[index(*k)])
        {
            self.permission_ui.checks[index(kind)] = false;
            self.permission_action(kind, PermissionAction::Check);
        }
    }
    pub(super) fn permission_action(&mut self, kind: PermissionKind, action: PermissionAction) {
        if self.permission_ui.pending.is_some() {
            return;
        }
        let Ok(snapshot) = &self.permission_ui.snapshot else {
            return;
        };
        if snapshot.runtime != RuntimeState::Running {
            return;
        }
        let observation = self.observation(kind);
        if (action == PermissionAction::Request && !observation.can_request)
            || (action == PermissionAction::OpenSystemSettings && !observation.can_open_settings)
        {
            return;
        }
        let Some(next) = self.permission_ui.request.checked_add(1) else {
            return;
        };
        self.permission_ui.request = next;
        let request = RequestId(next);
        let result = self.settings_port.submit_permission(PermissionIntent {
            generation: snapshot.generation,
            request,
            kind,
            action,
        });
        let pending = Pending {
            request,
            kind,
            action,
            entry: self.permission_ui.entry,
            setup: self.onboarding_permission_owner(kind),
        };
        self.permission_ui.record_submission(pending, result);
    }
    pub(super) fn refresh_permissions(&mut self, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.settings_port.take_permission_snapshot() {
            if let Some((pending, error)) = self.permission_ui.apply_snapshot(snapshot) {
                let kind = pending.kind;
                let action = pending.action;
                if let Some(owner) = pending.setup {
                    self.onboarding_permission_completed(owner, kind, action, error.is_none());
                }
                if error.is_none()
                    && kind == PermissionKind::Microphone
                    && pending.entry == self.permission_ui.entry
                    && action != PermissionAction::OpenSystemSettings
                    && self.devices_open()
                {
                    self.permission_ui.microphone_ready =
                        self.observation(kind).allows_device_preview();
                    self.microphone_auto_attempted = false;
                }
            }
            cx.notify();
        }
        self.dispatch_permission_check();
    }
    /// Compact device feedback. A successful observation adds no success row or
    /// repeated settings/recheck actions; a capture denial still remains visible.
    pub(super) fn device_permission_notice(
        &self,
        kind: PermissionKind,
        capture_denied: bool,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let observation = self.observation(kind);
        let help = permission_help(observation, PermissionPlatform::current());
        let feedback = self.permission_ui.feedback(kind);
        let failed = feedback.is_some();
        let attention = capture_denied
            || failed
            || matches!(
                observation.status,
                PermissionStatus::Denied
                    | PermissionStatus::Restricted
                    | PermissionStatus::Unsupported
                    | PermissionStatus::Unavailable
            );
        if observation.allows_device_preview() && !attention {
            return None;
        }
        let availability = self.permission_ui.availability();
        let status = if capture_denied {
            "Zugriff verweigert."
        } else if failed {
            feedback.unwrap_or("Die Zugriffsprüfung ist nicht verfügbar.")
        } else {
            help.status
        };
        let link = |id: &'static str, label: &'static str, action| {
            self.button(
                (id, index(kind)),
                label,
                ButtonStyle::Quiet,
                availability,
                LocalAction::Permission(kind, action),
                cx,
            )
            .min_h(gpui::px(24.))
            .px_1()
            .py_0()
            .text_size(self.sp(12.))
            .text_color(self.theme.accent)
        };
        let mut notice = div().flex().items_center().flex_wrap().gap_2().child(
            self.hint(self.l(status)).text_color(if attention {
                self.theme.danger
            } else {
                self.theme.muted
            }),
        );
        if let Some(label) = help.request_label {
            notice = notice.child(link(
                "device-permission-request",
                label,
                PermissionAction::Request,
            ));
        }
        if attention && observation.can_open_settings {
            notice = notice.child(link(
                "device-permission-settings",
                "Systemeinstellungen öffnen",
                PermissionAction::OpenSystemSettings,
            ));
        }
        if attention {
            notice = notice.child(link(
                "device-permission-check",
                "Erneut prüfen",
                PermissionAction::Check,
            ));
        }
        Some(notice)
    }
    pub(super) fn permission_panel(&self, kind: PermissionKind, cx: &Context<Self>) -> Div {
        let observation = self.observation(kind);
        let help = permission_help(observation, PermissionPlatform::current());
        let (_, purpose) = permission_identity(kind);
        let feedback = self.permission_ui.feedback(kind);
        let availability = self.permission_ui.availability();
        // One borderless text column keeps purpose, status, instructions and
        // actions aligned; paragraphs determine their height at large text sizes.
        let mut panel = div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .whitespace_normal()
            .text_color(self.theme.text)
            .text_size(self.sp(12.))
            .line_height(self.sp(18.))
            .child(self.hint(self.l(purpose)).w_full().whitespace_normal())
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .mt_3()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.l(help.status)),
            );
        if let Some(explanation) = help.explanation {
            panel = panel.child(
                self.hint(self.l(explanation))
                    .w_full()
                    .whitespace_normal()
                    .mt_2(),
            );
        }
        for (step, key) in help.steps.iter().enumerate() {
            panel = panel.child(
                div()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .mt_2()
                    .text_size(self.sp(12.))
                    .line_height(self.sp(18.))
                    .child(format!("{}. {}", step + 1, self.l(key))),
            );
        }
        panel = panel.child(
            div()
                .w_full()
                .min_w_0()
                .mt_3()
                .when_some(help.request_label, |d, label| {
                    d.child(
                        self.button(
                            ("permission-request", index(kind)),
                            "",
                            ButtonStyle::Primary,
                            availability,
                            LocalAction::Permission(kind, PermissionAction::Request),
                            cx,
                        )
                        .w_full()
                        .min_w_0()
                        .whitespace_normal()
                        .text_center()
                        .gap_0()
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .whitespace_normal()
                                .child(self.l(label)),
                        )
                        .mb_2(),
                    )
                })
                .when(observation.can_open_settings, |d| {
                    d.child(
                        self.button(
                            ("permission-settings", index(kind)),
                            "",
                            ButtonStyle::Secondary,
                            availability,
                            LocalAction::Permission(kind, PermissionAction::OpenSystemSettings),
                            cx,
                        )
                        .w_full()
                        .min_w_0()
                        .whitespace_normal()
                        .text_center()
                        .gap_0()
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .whitespace_normal()
                                .child(self.l("Systemeinstellungen öffnen")),
                        )
                        .mb_2(),
                    )
                })
                .child(
                    self.button(
                        ("permission-check", index(kind)),
                        "",
                        ButtonStyle::Quiet,
                        availability,
                        LocalAction::Permission(kind, PermissionAction::Check),
                        cx,
                    )
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .text_center()
                    .gap_0()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .child(self.l("Erneut prüfen")),
                    ),
                ),
        );
        if let Some(note) = help.note {
            panel = panel.child(self.hint(self.l(note)).w_full().whitespace_normal().mt_2());
        }
        if let Some(feedback) = feedback {
            panel = panel.child(
                self.hint(self.l(feedback))
                    .w_full()
                    .whitespace_normal()
                    .mt_2(),
            );
        }
        panel
    }

    pub(super) fn onboarding_permission_status(&self, kind: PermissionKind) -> &'static str {
        permission_help(self.observation(kind), PermissionPlatform::current()).status
    }

    pub(super) fn onboarding_permission_panel(
        &self,
        kind: PermissionKind,
        cx: &Context<Self>,
    ) -> Div {
        let observation = self.observation(kind);
        let help = permission_help(observation, PermissionPlatform::current());
        let (_, purpose) = permission_identity(kind);
        let explanation = match (PermissionPlatform::current(), kind) {
            (PermissionPlatform::MacOS, PermissionKind::ScreenSharing) => {
                "macOS fragt nach der Bildschirmfreigabe. Falls es einen Neustart verlangt, geht es danach hier weiter."
            }
            (PermissionPlatform::MacOS, _) => {
                "Bestätige die Anfrage von macOS, wenn sie erscheint. Nach der Freigabe geht es automatisch weiter."
            }
            (PermissionPlatform::Windows, PermissionKind::ScreenSharing) => {
                "Unter Windows wird das Fenster oder der Bildschirm erst beim Start einer Freigabe ausgewählt."
            }
            (PermissionPlatform::Windows, _) => {
                "Windows verwaltet den Gerätezugriff in „Datenschutz & Sicherheit“. Bereits erlaubter Zugriff wird automatisch erkannt."
            }
            (PermissionPlatform::Linux, PermissionKind::ScreenSharing) => {
                "Unter Linux wählst du das Fenster oder den Bildschirm für jede Freigabe im Systemdialog aus."
            }
            (PermissionPlatform::Linux, _) => {
                "Linux prüft den Gerätezugriff beim Starten von Mikrofon oder Kamera."
            }
            _ => {
                "Prüfe die Zugriffsrechte in den Systemeinstellungen und wähle danach „Erneut prüfen“."
            }
        };
        let availability = self.permission_ui.availability();
        let settings_ready =
            kind != PermissionKind::ScreenSharing || self.onboarding_checkpoint_ready();
        let settings_availability = if settings_ready {
            availability
        } else if matches!(self.preferences.save, SaveState::Pending { .. }) {
            Availability::pending()
        } else {
            Availability::when(
                false,
                "Der Einrichtungsfortschritt konnte nicht gespeichert werden. Bitte erneut versuchen.",
            )
        };
        let mut panel = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.hint(self.l(purpose)))
            .child(self.hint(self.l(explanation)))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.l(help.status)),
            );
        if observation.status == PermissionStatus::Restricted
            && let Some(explanation) = help.explanation
        {
            panel = panel.child(self.hint(self.l(explanation)));
        }
        if let Some(feedback) = self.permission_ui.feedback(kind) {
            panel = panel.child(self.hint(self.l(feedback)));
        }
        if observation.guidance == PermissionGuidance::RestartRequired {
            panel = panel.child(self.hint(self.l(
                "macOS hat die Freigabe bestätigt. Starte die App neu, damit dieser Prozess sie verwenden kann.",
            )));
        }
        // The interactive step itself authorizes one request. Recovery actions
        // only inspect the current OS state or open its Settings page.
        panel
            .when(
                observation.can_open_settings && observation.status != PermissionStatus::Granted,
                |d| {
                    d.child(
                        self.button(
                            ("onboarding-permission-settings", index(kind)),
                            "Systemeinstellungen öffnen",
                            ButtonStyle::Secondary,
                            settings_availability,
                            LocalAction::Permission(kind, PermissionAction::OpenSystemSettings),
                            cx,
                        )
                        .w_full(),
                    )
                },
            )
            .when(
                !matches!(
                    observation.status,
                    PermissionStatus::Granted
                        | PermissionStatus::NotRequired
                        | PermissionStatus::PerSession
                ),
                |d| {
                    d.child(
                        self.button(
                            ("onboarding-permission-check", index(kind)),
                            "Erneut prüfen",
                            ButtonStyle::Quiet,
                            availability,
                            LocalAction::Permission(kind, PermissionAction::Check),
                            cx,
                        )
                        .w_full(),
                    )
                },
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::localization::text;
    use talk_core::preferences::Locale;

    const STATUSES: [PermissionStatus; 9] = [
        PermissionStatus::Unknown,
        PermissionStatus::NotDetermined,
        PermissionStatus::Granted,
        PermissionStatus::Denied,
        PermissionStatus::Restricted,
        PermissionStatus::PerSession,
        PermissionStatus::NotRequired,
        PermissionStatus::Unsupported,
        PermissionStatus::Unavailable,
    ];

    fn observation(kind: PermissionKind, status: PermissionStatus) -> PermissionObservation {
        PermissionObservation {
            kind,
            status,
            guidance: PermissionGuidance::SystemSettings,
            can_request: false,
            can_open_settings: true,
        }
    }

    #[test]
    fn only_supported_first_requests_offer_allow_access() {
        for kind in KINDS {
            for status in STATUSES {
                let mut observation = observation(kind, status);
                assert!(
                    permission_help(observation, PermissionPlatform::MacOS)
                        .request_label
                        .is_none()
                );
                observation.can_request = true;
                let help = permission_help(observation, PermissionPlatform::MacOS);
                assert_eq!(
                    help.request_label,
                    match status {
                        PermissionStatus::NotDetermined => Some("Zugriff erlauben"),
                        PermissionStatus::Unknown => Some("Zugriff anfragen"),
                        _ => None,
                    },
                );
                if status == PermissionStatus::NotDetermined {
                    assert_eq!(help.steps.first(), Some(&"Wähle „Zugriff erlauben“."));
                }
            }
        }
    }

    #[test]
    fn macos_settings_steps_name_the_affected_permission_and_end_with_a_recheck() {
        for (kind, pane) in [
            (PermissionKind::Microphone, "Microphone"),
            (PermissionKind::Camera, "Camera"),
            (
                PermissionKind::ScreenSharing,
                "Screen & System Audio Recording",
            ),
        ] {
            for status in [PermissionStatus::Denied, PermissionStatus::NotDetermined] {
                let help = permission_help(observation(kind, status), PermissionPlatform::MacOS);
                let steps: Vec<_> = help
                    .steps
                    .iter()
                    .map(|key| text(Locale::English, key))
                    .collect();
                assert_eq!(steps.len(), 3);
                assert!(
                    steps[0].contains("System Settings") && steps[0].contains("Privacy & Security")
                );
                assert!(steps[1].contains(pane) && steps[1].contains("NextCloud Modern Talk"));
                assert!(steps[2].contains("Check again"));
            }
        }
    }

    #[test]
    fn restrictions_and_unsupported_checks_do_not_offer_futile_settings_steps() {
        for kind in KINDS {
            for status in [PermissionStatus::Restricted, PermissionStatus::Unsupported] {
                let help = permission_help(observation(kind, status), PermissionPlatform::MacOS);
                assert!(help.steps.is_empty());
                assert!(help.request_label.is_none());
                if status == PermissionStatus::Restricted {
                    assert!(
                        text(Locale::English, help.explanation.unwrap()).contains("administrator")
                    );
                }
            }
        }
    }

    #[test]
    fn unknown_and_unavailable_checks_offer_supported_inspection_without_claiming_denial() {
        for kind in KINDS {
            for status in [PermissionStatus::Unknown, PermissionStatus::Unavailable] {
                let mut observation = observation(kind, status);
                let help = permission_help(observation, PermissionPlatform::MacOS);
                assert_ne!(help.status, "Zugriff verweigert.");
                assert!(help.request_label.is_none());
                assert_eq!(help.steps.len(), 3);
                assert!(text(Locale::English, help.steps[1]).contains("review"));
                observation.can_open_settings = false;
                assert!(
                    permission_help(observation, PermissionPlatform::MacOS)
                        .steps
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn other_platforms_never_get_apple_steps_or_restart_advice() {
        for kind in KINDS {
            for status in STATUSES {
                let mut observation = observation(kind, status);
                observation.guidance = PermissionGuidance::RestartAfterGrant;
                let help = permission_help(observation, PermissionPlatform::Linux);
                assert!(help.steps.is_empty());
                assert!(help.note.is_none());
                assert_eq!(
                    permission_help(observation, PermissionPlatform::MacOS)
                        .note
                        .is_some(),
                    kind == PermissionKind::ScreenSharing && status == PermissionStatus::Granted,
                );
            }
        }
    }

    #[test]
    fn windows_and_linux_use_their_own_permission_routes() {
        for (kind, pane) in [
            (PermissionKind::Microphone, "Microphone"),
            (PermissionKind::Camera, "Camera"),
        ] {
            let help = permission_help(
                observation(kind, PermissionStatus::Unknown),
                PermissionPlatform::Windows,
            );
            assert_eq!(help.steps.len(), 3);
            assert!(text(Locale::English, help.steps[0]).contains(pane));
            assert!(text(Locale::English, help.steps[1]).contains("desktop apps"));
            assert!(
                !help
                    .steps
                    .iter()
                    .any(|key| text(Locale::English, key).contains("macOS"))
            );
        }
        let linux = permission_help(
            observation(PermissionKind::ScreenSharing, PermissionStatus::PerSession),
            PermissionPlatform::Linux,
        );
        assert!(linux.steps.is_empty());
        assert!(text(Locale::English, linux.explanation.unwrap()).contains("each share"));
        assert!(linux.request_label.is_none());
        for platform in [
            PermissionPlatform::Windows,
            PermissionPlatform::Linux,
            PermissionPlatform::Other,
        ] {
            let restricted = permission_help(
                observation(PermissionKind::Camera, PermissionStatus::Restricted),
                platform,
            );
            assert!(restricted.steps.is_empty());
            assert!(
                text(Locale::English, restricted.explanation.unwrap()).contains("administrator")
            );
        }
    }

    #[test]
    fn snapshot_failure_remains_visible_while_an_action_is_pending() {
        let mut ui = PermissionUi::new(Err(PermissionError::Unavailable));
        ui.pending = Some(Pending {
            request: RequestId(1),
            kind: PermissionKind::Camera,
            action: PermissionAction::OpenSystemSettings,
            entry: 1,
            setup: None,
        });
        assert_eq!(
            ui.feedback(PermissionKind::Camera),
            permission_error_key(PermissionError::Unavailable),
        );
        ui.snapshot = Ok(PermissionSnapshot {
            generation: 1,
            runtime: RuntimeState::Running,
            microphone: PermissionObservation::unknown(PermissionKind::Microphone),
            camera: observation(PermissionKind::Camera, PermissionStatus::Denied),
            screen_sharing: PermissionObservation::unknown(PermissionKind::ScreenSharing),
            operation: PermissionOperation::Idle,
        });
        assert_eq!(ui.feedback(PermissionKind::Camera), None,);
        ui.pending = None;
        ui.error = Some(PermissionError::TimedOut);
        assert_eq!(
            ui.feedback(PermissionKind::Camera),
            permission_error_key(PermissionError::TimedOut),
        );
    }

    #[test]
    fn busy_admission_does_not_start_an_operation_or_replace_a_failure() {
        assert_eq!(permission_error_key(PermissionError::Busy), None);
        let mut ui = PermissionUi::new(Ok(PermissionSnapshot {
            generation: 1,
            runtime: RuntimeState::Running,
            microphone: PermissionObservation::unknown(PermissionKind::Microphone),
            camera: observation(PermissionKind::Camera, PermissionStatus::Denied),
            screen_sharing: PermissionObservation::unknown(PermissionKind::ScreenSharing),
            operation: PermissionOperation::Idle,
        }));
        let pending = Pending {
            request: RequestId(4),
            kind: PermissionKind::Camera,
            action: PermissionAction::Request,
            entry: 2,
            setup: None,
        };
        ui.record_submission(pending, Err(PermissionError::Busy));
        assert!(ui.pending.is_none());
        assert_eq!(ui.feedback(PermissionKind::Camera), None);
        ui.error = Some(PermissionError::TimedOut);
        ui.record_submission(pending, Err(PermissionError::Busy));
        assert_eq!(ui.error, Some(PermissionError::TimedOut));
        assert_eq!(
            ui.feedback(PermissionKind::Camera),
            permission_error_key(PermissionError::TimedOut),
        );
        ui.record_submission(pending, Ok(()));
        assert!(ui.error.is_none());
        assert_eq!(ui.pending.unwrap().request, RequestId(4));
        assert_eq!(ui.feedback(PermissionKind::Camera), None);
        let mut snapshot = ui.snapshot.clone().unwrap();
        for (request, kind, action) in [
            (
                RequestId(3),
                PermissionKind::Camera,
                PermissionAction::Request,
            ),
            (
                RequestId(4),
                PermissionKind::Microphone,
                PermissionAction::Request,
            ),
            (
                RequestId(4),
                PermissionKind::Camera,
                PermissionAction::Check,
            ),
        ] {
            snapshot.operation = PermissionOperation::Failed {
                request,
                kind,
                action,
                error: PermissionError::Failed,
            };
            assert!(ui.apply_snapshot(Ok(snapshot.clone())).is_none());
            assert_eq!(ui.pending.unwrap().request, RequestId(4));
            assert!(ui.error.is_none());
        }
        assert!(
            ui.apply_snapshot(Err(PermissionError::Unavailable))
                .is_none()
        );
        assert_eq!(ui.pending.unwrap().request, RequestId(4));
        snapshot.operation = PermissionOperation::Failed {
            request: pending.request,
            kind: pending.kind,
            action: pending.action,
            error: PermissionError::TimedOut,
        };
        assert!(ui.apply_snapshot(Ok(snapshot)).is_some());
        assert!(ui.pending.is_none());
        assert_eq!(ui.error, Some(PermissionError::TimedOut));
    }

    #[test]
    fn help_copy_has_english_and_spanish_resources() {
        let localized = |key: &str| {
            assert!(!key.is_empty());
            for locale in [Locale::English, Locale::Spanish] {
                assert_ne!(text(locale, key), key, "missing {locale:?} resource: {key}");
            }
        };
        for kind in KINDS {
            localized(permission_identity(kind).1);
            for status in STATUSES {
                for platform in [
                    PermissionPlatform::MacOS,
                    PermissionPlatform::Windows,
                    PermissionPlatform::Linux,
                    PermissionPlatform::Other,
                ] {
                    for can_request in [false, true] {
                        let help = permission_help(
                            PermissionObservation {
                                can_request,
                                ..observation(kind, status)
                            },
                            platform,
                        );
                        for key in std::iter::once(help.status)
                            .chain(help.explanation)
                            .chain(help.steps.iter().copied())
                            .chain(help.request_label)
                        {
                            localized(key);
                        }
                    }
                }
            }
        }
        for guidance in [
            PermissionGuidance::RestartAfterGrant,
            PermissionGuidance::HelperUnavailable,
            PermissionGuidance::BackendUnavailable,
        ] {
            let help = permission_help(
                PermissionObservation {
                    guidance,
                    ..observation(PermissionKind::ScreenSharing, PermissionStatus::Granted)
                },
                PermissionPlatform::MacOS,
            );
            localized(help.note.unwrap());
        }
        for error in [
            PermissionError::Stale,
            PermissionError::Busy,
            PermissionError::Closed,
            PermissionError::Unsupported,
            PermissionError::Unavailable,
            PermissionError::Failed,
            PermissionError::TimedOut,
            PermissionError::Cancelled,
        ] {
            if let Some(key) = permission_error_key(error) {
                localized(key);
            }
        }
        for key in [
            "Berechtigungen · {step} von 3",
            "Später einrichten",
            "Du kannst ohne diese Freigaben chatten und sie später in den Einstellungen ändern.",
            "Unter Windows wird das Fenster oder der Bildschirm erst beim Start einer Freigabe ausgewählt.",
            "Erneut prüfen",
        ] {
            localized(key);
        }
    }
}
