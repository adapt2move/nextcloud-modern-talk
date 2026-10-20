use super::controls::{Availability, icon};
use super::{ButtonStyle, Desktop, LocalAction};
use gpui::{Context, Div, Focusable, FontWeight, Window, div, prelude::*, px};
use talk_core::ui::presentation::{
    LoginFailure, LoginPhase, NativeStoreState, StorageChoice, StorageStatus, UiIntent,
};

impl Desktop {
    pub fn onboarding(&self, window: &Window, cx: &Context<Self>) -> gpui::Stateful<Div> {
        let t = self.theme;
        let login = &self.snapshot.login;
        let narrow = window.viewport_size().width < px(960.) || self.scale >= 150;
        let sheet_height =
            f32::from(window.viewport_size().height) - 26. - 24. - if narrow { 69. } else { 0. };
        let step = match login.phase {
            LoginPhase::EnterServer | LoginPhase::Validating => 1,
            LoginPhase::Ready => 3,
            _ => 2,
        };
        let mut card = div()
            .w_full()
            .max_w(px(if narrow { 640. } else { 416. }))
            .flex_none()
            .flex()
            .flex_col()
            .gap_4();
        match login.phase {
            LoginPhase::EnterServer | LoginPhase::Validating => {
                card = card
                    .child(
                        div()
                            .text_size(self.sp(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.muted)
                            .child(self.l("WILLKOMMEN BEI TALK")),
                    )
                    .child(
                        div()
                            .text_size(self.sp(27.))
                            .line_height(self.sp(34.))
                            .font_weight(FontWeight::BOLD)
                            .child(self.l("Mit deiner Cloud verbinden")),
                    )
                    .child(
                        div()
                            .text_size(self.sp(15.))
                            .text_color(t.muted)
                            .child(self.l("Gib die Adresse eurer NextCloud ein.")),
                    )
                    .child(
                        div()
                            .mt_4()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(self.sp(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.l("Serveradresse")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .px_3()
                                    .min_h(px(48.))
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(t.input_border)
                                    .relative()
                                    .child(self.field_ring(self.server.focus_handle(cx), 8.))
                                    .child(self.scaled_icon("globe", 18., t.muted))
                                    .child(div().flex_1().min_w_0().child(self.server.clone())),
                            )
                            .child(self.hint(self.l(if login.phase == LoginPhase::Validating {
                                "Die Serveradresse wird geprüft. Bitte warte."
                            } else {
                                "Zum Beispiel die Adresse, unter der du eure Dateien öffnest."
                            }))),
                    )
                    .child(
                        self.button_with_icon(
                            "login",
                            ("Weiter", "arrow-right"),
                            ButtonStyle::Primary,
                            Availability::when(
                                login.phase == LoginPhase::EnterServer,
                                "Die Serveradresse wird geprüft. Bitte warte.",
                            )
                            .explained_elsewhere(),
                            LocalAction::BeginLogin,
                            cx,
                        )
                        .mt_2()
                        .min_h(px(46.)),
                    )
                    .child(div().mt_1().text_center().child(self.hint(
                        self.l("Im nächsten Schritt meldest du dich in deinem Browser an."),
                    )));
                if login.phase == LoginPhase::Validating
                    && let Some(attempt) = login.attempt
                {
                    card = card.child(self.button(
                        "cancel-validating",
                        "Anmeldung abbrechen",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Intent(UiIntent::CancelLogin(attempt)),
                        cx,
                    ));
                }
            }
            LoginPhase::BrowserWaiting => {
                card=card.child(self.onboarding_symbol("external",false))
                    .child(self.title(self.l("Im Browser anmelden.")))
                    .child(self.hint(self.l("Melde dich bei eurer NextCloud an und erlaube Talk den Zugriff auf dein Konto.")))
                    .child(self.server_chip(login.server_label.as_str()));
                if let Some(attempt) = login.attempt {
                    card = card
                        .child(
                            self.button_with_icon(
                                "reopen-browser",
                                ("Browser erneut öffnen", "external"),
                                ButtonStyle::Primary,
                                true,
                                LocalAction::Intent(UiIntent::ReopenBrowser(attempt)),
                                cx,
                            )
                            .min_h(px(46.)),
                        )
                        .child(self.button(
                            "cancel-login",
                            "Anmeldung abbrechen",
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::Intent(UiIntent::CancelLogin(attempt)),
                            cx,
                        ));
                }
                card = card.child(
                    div()
                        .flex()
                        .gap_2()
                        .justify_center()
                        .items_center()
                        .child(self.scaled_icon("clock", 16., t.muted))
                        .child(self.hint(self.l("Warte auf deine Anmeldung …"))),
                );
            }
            LoginPhase::ChooseStorage => {
                let pending = matches!(
                    login.storage,
                    StorageStatus::Saving
                        | StorageStatus::CleanupPending
                        | StorageStatus::RecoveryRequired
                );
                card=card.child(self.onboarding_symbol("lock",false)).child(self.title(self.l("Deine Anmeldung speichern.")))
                    .child(self.hint(self.l(match login.native_store {NativeStoreState::Locked=>"Der sichere Schlüsselspeicher ist gesperrt.",NativeStoreState::Denied=>"Der Zugriff auf den sicheren Schlüsselspeicher wurde abgelehnt.",NativeStoreState::Missing=>"Der sichere Schlüsselspeicher ist auf diesem Gerät nicht verfügbar.",_=>"Wähle, wie du dich auf diesem Gerät anmelden möchtest."})))
                    .child(self.storage_option("native-store","Sicherer Schlüsselspeicher","Die empfohlene Ablage deines Betriebssystems.",StorageChoice::NativeStore,!pending && matches!(login.native_store,NativeStoreState::Available|NativeStoreState::Unknown),cx))
                    .child(self.storage_option("private-file","Auf diesem Gerät speichern","Dauerhaft in einer unverschlüsselten privaten Datei. Nur dein Betriebssystem-Benutzer erhält Zugriff.",StorageChoice::PrivateFile,!pending,cx));
                if pending {
                    card=card.child(div().p_3().rounded_lg().bg(if login.storage==StorageStatus::Saving {t.subtle}else{t.warning_bg}).text_color(if login.storage==StorageStatus::Saving {t.muted}else{t.warning}).child(self.l(match login.storage {
                    StorageStatus::Saving=>"Deine Anmeldung wird gespeichert …",
                    StorageStatus::CleanupPending=>"Eine frühere Speicherung wird noch bereinigt. Bitte warte.",
                    _=>"Der Speicherzustand muss geklärt werden. Wähle bis dahin keine neue Ablage.",
                })));
                }
                if matches!(
                    login.native_store,
                    NativeStoreState::Locked | NativeStoreState::Denied
                ) && !pending
                {
                    card = card.child(self.button(
                        "retry-native",
                        "Erneut versuchen",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Storage(StorageChoice::NativeStore),
                        cx,
                    ));
                }
                if let Some(attempt) = login.attempt {
                    card = card.child(self.button(
                        "cancel-storage",
                        "Abbrechen",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Intent(UiIntent::CancelLogin(attempt)),
                        cx,
                    ));
                }
            }
            LoginPhase::Ready => {
                use talk_core::preferences::permissions::PermissionKind;
                let kind = self.permission_ui.setup_kind;
                let (step, title, previous, next) = match kind {
                    PermissionKind::Microphone => {
                        (1, "Mikrofon", None, Some(PermissionKind::Camera))
                    }
                    PermissionKind::Camera => (
                        2,
                        "Kamera",
                        Some(PermissionKind::Microphone),
                        Some(PermissionKind::ScreenSharing),
                    ),
                    PermissionKind::ScreenSharing => {
                        (3, "Bildschirmfreigabe", Some(PermissionKind::Camera), None)
                    }
                };
                let (symbol, _) = super::permissions::permission_identity(kind);
                let connected = self.snapshot.active_account.is_some();
                card = card
                    .child(self.hint(&self.lf("Berechtigungen · {step} von 3", &[("step", &step.to_string())])))
                    .child(div().w_full().flex().gap_2().children((1..=3).map(|position| {
                        div().flex_1().h(px(3.)).rounded_full().bg(if position <= step { t.accent } else { t.border })
                    })))
                    .child(div().mt_4().flex().items_center().gap_3()
                        .child(div().flex_shrink_0().child(self.scaled_icon(symbol, 28., t.focus)))
                        .child(div().min_w_0().flex_1().child(self.title(self.l(title)))))
                    .child(self.permission_panel(kind, cx))
                    .child(self.button(
                        "permission-setup-next",
                        if next.is_some() { "Weiter" } else { "Zu deinen Gesprächen" },
                        ButtonStyle::Primary,
                        Availability::when(connected, "Deine Gespräche sind noch nicht bereit."),
                        next.map_or(LocalAction::FinishOnboarding, LocalAction::PermissionSetup),
                        cx,
                    ).w_full().mt_4())
                    .when_some(previous, |d, previous| d.child(self.button(
                        "permission-setup-back", "Zurück", ButtonStyle::Quiet, true,
                        LocalAction::PermissionSetup(previous), cx,
                    ).w_full()))
                    .when(next.is_some(), |d| d.child(self.button(
                        "permission-setup-skip", "Später einrichten", ButtonStyle::Quiet,
                        Availability::when(connected, "Deine Gespräche sind noch nicht bereit."),
                        LocalAction::FinishOnboarding, cx,
                    ).w_full()))
                    .child(self.hint(self.l("Du kannst ohne diese Freigaben chatten und sie später in den Einstellungen ändern.")))
                    .child(self.hint(self.l(match login.storage {
                        StorageStatus::NativeStored => {
                            "Anmeldung im sicheren Schlüsselspeicher gespeichert."
                        }
                        StorageStatus::PrivateFileStored => {
                            "Auf diesem Gerät gespeichert · unverschlüsselte private Datei."
                        }
                        StorageStatus::SessionOnly => {
                            "Nur für diese Sitzung angemeldet. Nach dem Beenden erneut anmelden."
                        }
                        StorageStatus::CleanupPending => {
                            "Die Bereinigung einer früheren Speicherung ist noch offen."
                        }
                        StorageStatus::RecoveryRequired => {
                            "Der Speicherzustand muss noch geklärt werden."
                        }
                        StorageStatus::Saving => "Speichern läuft …",
                        StorageStatus::NotSaved => "Deine Anmeldung ist nicht gespeichert.",
                    })));
            }
            LoginPhase::Failed(reason) => {
                let (title, detail) = match reason {
                    LoginFailure::Expired => (
                        "Der Link ist abgelaufen.",
                        "Die Anmeldung hat zu lange gedauert. Mit einem neuen Link kannst du weitermachen.",
                    ),
                    LoginFailure::Tls => (
                        "Verbindung nicht vertrauenswürdig.",
                        "Die Identität des Servers konnte nicht bestätigt werden. Prüfe die Serveradresse.",
                    ),
                    LoginFailure::Denied => (
                        "Freigabe abgelehnt.",
                        "Dein Konto wurde nicht verbunden. Du kannst die Anmeldung erneut starten.",
                    ),
                    LoginFailure::BrowserUnavailable => (
                        "Browser konnte nicht geöffnet werden.",
                        "Prüfe deinen Standardbrowser und versuche es erneut.",
                    ),
                    LoginFailure::Storage => (
                        "Speicherung nicht bestätigt.",
                        "Deine Anmeldung wurde nicht als gespeichert bestätigt. Bitte versuche es erneut.",
                    ),
                    LoginFailure::Network => (
                        "Server nicht erreichbar.",
                        "Prüfe deine Internetverbindung und versuche es erneut.",
                    ),
                    LoginFailure::Unconfigured => (
                        "Verbindung noch nicht verfügbar.",
                        "Diese Vorschau kann sich noch nicht mit eurer NextCloud verbinden.",
                    ),
                };
                card = card
                    .child(self.onboarding_symbol(
                        if reason == LoginFailure::Expired {
                            "clock"
                        } else {
                            "info"
                        },
                        true,
                    ))
                    .child(self.title(self.l(title)))
                    .child(self.hint(self.l(detail)))
                    .child(self.server_chip(login.server_label.as_str()))
                    .child(
                        self.button_with_icon(
                            "retry-login",
                            ("Erneut anmelden", "refresh"),
                            ButtonStyle::Primary,
                            true,
                            LocalAction::BeginLogin,
                            cx,
                        )
                        .min_h(px(46.)),
                    );
                if let Some(attempt) = login.attempt {
                    card = card.child(self.button(
                        "change-server",
                        "Anderen Server verwenden",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Intent(UiIntent::CancelLogin(attempt)),
                        cx,
                    ));
                }
            }
        }
        card = card.when(self.local_feedback_visible(), |d| {
            d.child(self.feedback_view(cx))
        });
        let wordmark = || {
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .relative()
                        .size(px(39.))
                        .flex_none()
                        .child(icon("brand", 39., t.focus).absolute().inset_0())
                        .child(icon("brand-front", 39., t.brand_front).absolute().inset_0()),
                )
                .child(
                    div()
                        .text_size(self.sp(26.))
                        .font_weight(FontWeight::BOLD)
                        .child(self.l("Talk")),
                )
        };
        let aside = div()
            .w(px(340.))
            .h_full()
            .flex_none()
            .px(px(34.))
            .pt(px(30.))
            .pb(px(28.))
            .flex()
            .flex_col()
            .child(wordmark())
            .child(
                div()
                    .mt(px(80.))
                    .text_size(self.sp(35.))
                    .line_height(self.sp(43.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.l("Euer Team."))
                    .child(div().child(self.l("Eure Cloud."))),
            )
            .child(
                div()
                    .mt_5()
                    .text_size(self.sp(15.))
                    .line_height(self.sp(26.))
                    .text_color(t.sidebar_secondary)
                    .child(self.l("Ein ruhiger Platz für alles,
was ihr miteinander besprecht.")),
            )
            .child(
                div().mt(px(42.)).flex().flex_col().gap_5().children(
                    ["Cloud verbinden", "Sicher anmelden", "Ankommen"]
                        .into_iter()
                        .enumerate()
                        .map(|(i, label)| {
                            let current = step == i + 1;
                            let done = step > i + 1;
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .text_size(self.sp(13.))
                                .font_weight(if current {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(if current {
                                    t.focus
                                } else {
                                    t.sidebar_secondary
                                })
                                .child(
                                    div()
                                        .size(px(24.))
                                        .flex_none()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(if current {
                                            t.accent
                                        } else {
                                            t.input_border
                                        })
                                        .bg(if current {
                                            t.accent
                                        } else {
                                            gpui::transparent_black()
                                        })
                                        .text_color(if current {
                                            t.on_accent
                                        } else {
                                            t.sidebar_secondary
                                        })
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .when(done, |d| {
                                            d.child(self.scaled_icon(
                                                "check",
                                                14.,
                                                t.sidebar_secondary,
                                            ))
                                        })
                                        .when(!done, |d| d.child((i + 1).to_string())),
                                )
                                .child(self.l(label))
                        }),
                ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(t.sidebar_secondary)
                    .text_size(self.sp(12.))
                    .child(self.scaled_icon("cloud", 14., t.sidebar_secondary))
                    .child(self.l("Teamarbeit mit NextCloud.")),
            );
        div()
            .id("onboarding")
            .size_full()
            .flex()
            .when(narrow, |d| d.flex_col())
            .when(!narrow, |d| d.child(aside))
            .when(narrow, |d| {
                d.child(
                    div()
                        .flex_none()
                        .px_5()
                        .py_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(wordmark())
                        .when(login.phase != LoginPhase::Ready, |d| {
                            d.child(self.hint(
                                &self.lf("Schritt {step} von 3", &[("step", &step.to_string())]),
                            ))
                        }),
                )
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .my_3()
                    .mr_3()
                    .when(narrow, |d| d.ml_3())
                    .rounded(px(13.))
                    .bg(t.surface)
                    .overflow_hidden()
                    .child(
                        self.scroll_area(super::navigation::Region::Onboarding)
                            .size_full()
                            // Card is explicitly non-shrinking; it determines scroll extent at large text.
                            .child(
                                div()
                                    .min_h(px(sheet_height))
                                    .w_full()
                                    .px_8()
                                    .py_8()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .justify_center()
                                    .child(card),
                            ),
                    ),
            )
    }

    fn onboarding_symbol(&self, name: &'static str, warning: bool) -> Div {
        div()
            .size(px(64.))
            .rounded(px(19.))
            .bg(if warning {
                self.theme.warning_bg
            } else {
                self.theme.subtle
            })
            .flex()
            .items_center()
            .justify_center()
            .child(self.scaled_icon(
                name,
                26.,
                if warning {
                    self.theme.warning
                } else {
                    self.theme.accent
                },
            ))
    }
    fn server_chip(&self, label: &str) -> Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(7.))
            .px_3()
            .py_2()
            .bg(self.theme.subtle)
            .text_color(self.theme.muted)
            .text_size(self.sp(12.))
            .child(self.scaled_icon("lock", 14., self.theme.muted))
            .child(div().min_w_0().child(label.to_owned()))
    }
    fn storage_option(
        &self,
        id: &'static str,
        title: &'static str,
        detail: &'static str,
        choice: StorageChoice,
        enabled: bool,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        self.option_row(
            id,
            (
                match choice {
                    StorageChoice::NativeStore => "lock",
                    StorageChoice::PrivateFile => "file",
                    StorageChoice::SessionOnly => "clock",
                },
                title,
                detail,
            ),
            Availability::when(enabled, "Diese Speicherwahl ist derzeit nicht verfügbar."),
            LocalAction::Storage(choice),
            cx,
        )
    }
}
