//! Modal settings use native miniature compositions derived from the actual color roles.
use super::controls::{Availability, icon};
use super::preferences::SettingsTab;
use super::theme::{Appearance, Palette, Theme};
use super::{ButtonStyle, Desktop, LocalAction, Overlay};
use gpui::{
    Context, Div, FontWeight, Stateful, Window, WindowAppearance, div, prelude::*, px, relative,
};

fn pill(color: gpui::Hsla) -> Div {
    div().rounded_full().bg(color)
}
fn miniature(t: Theme, w: f32) -> Div {
    div()
        .relative()
        .w(px(w))
        .h(px(88.))
        .overflow_hidden()
        .rounded(px(6.))
        .bg(t.chrome())
        .child(
            div()
                .absolute()
                .left(px(5.))
                .top(px(8.))
                .w(px(w * 0.22))
                .h(px(7.))
                .rounded_full()
                .border_1()
                .border_color(t.border)
                .bg(t.surface),
        )
        .children((0..3).map(|i| {
            pill(if i == 0 { t.selected } else { t.subtle })
                .absolute()
                .left(px(5.))
                .top(px(23. + i as f32 * 10.))
                .w(px(w * 0.22))
                .h(px(6.))
        }))
        .child(
            div()
                .absolute()
                .left(px(w * 0.31))
                .right(px(3.))
                .top(px(3.))
                .bottom(px(3.))
                .rounded(px(4.))
                .bg(t.surface)
                .child(
                    pill(t.accent)
                        .absolute()
                        .right(px(7.))
                        .top(px(9.))
                        .w(px(w * 0.22))
                        .h(px(7.)),
                )
                .child(
                    pill(t.border)
                        .absolute()
                        .left(px(7.))
                        .top(px(26.))
                        .w(px(w * 0.26))
                        .h(px(5.)),
                )
                .child(
                    pill(t.border)
                        .absolute()
                        .left(px(7.))
                        .top(px(36.))
                        .w(px(w * 0.20))
                        .h(px(5.)),
                )
                .child(
                    div()
                        .absolute()
                        .right(px(7.))
                        .top(px(23.))
                        .w(px(w * 0.22))
                        .h(px(26.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.subtle)
                        .children([t.success, t.focus, t.warning].into_iter().enumerate().map(
                            |(i, c)| {
                                div()
                                    .absolute()
                                    .left(px(4.))
                                    .top(px(4. + 7. * i as f32))
                                    .flex()
                                    .items_center()
                                    .gap(px(3.))
                                    .child(pill(c).size(px(3.)))
                                    .child(pill(t.input_border).w(px(w * 0.11)).h(px(2.)))
                            },
                        )),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(7.))
                        .right(px(7.))
                        .bottom(px(6.))
                        .h(px(12.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(t.border)
                        .child(
                            pill(t.border)
                                .absolute()
                                .left(px(4.))
                                .top(px(4.))
                                .w(px(w * 0.24))
                                .h(px(3.)),
                        )
                        .child(
                            pill(t.accent)
                                .absolute()
                                .right(px(3.))
                                .top(px(2.))
                                .size(px(6.)),
                        ),
                ),
        )
}
/// One modal layout for app and conversation settings: navigation rail with
/// leading icons, a scrolling page and an optional status footer. The geometry
/// is that of the app settings, which every settings-like dialog now shares.
pub(super) struct DialogFrame {
    pub id: &'static str,
    pub close: (&'static str, &'static str),
    pub region: super::navigation::Region,
    /// Wide layout: the rail. Narrow layout: compact navigation above the page.
    pub navigation: Div,
    pub page: gpui::AnyElement,
    pub footer: Option<Div>,
}
const DIALOG_WIDTH: f32 = 842.;
const DIALOG_HEIGHT: f32 = 632.;
const DIALOG_RAIL: f32 = 211.;
const DIALOG_PADDING: f32 = 24.;
impl Desktop {
    pub(super) fn dialog_narrow(&self, window: &Window) -> bool {
        window.viewport_size().width < px(900.) || self.scale >= 150
    }
    fn dialog_size(window: &Window) -> (f32, f32) {
        (
            (f32::from(window.viewport_size().width) - 48.).min(DIALOG_WIDTH),
            (f32::from(window.viewport_size().height) - 74.).min(DIALOG_HEIGHT),
        )
    }
    pub(super) fn dialog_frame(
        &self,
        frame: DialogFrame,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        let t = self.theme;
        let narrow = self.dialog_narrow(window);
        let (width, height) = Self::dialog_size(window);
        let page = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .when(!narrow, |d| d.h_full())
            .flex()
            .flex_col()
            .child(
                self.scroll_area(frame.region)
                    .flex_1()
                    .min_h_0()
                    .px(px(DIALOG_PADDING))
                    .pt(px(20.))
                    .pb(px(DIALOG_PADDING))
                    .child(frame.page),
            )
            .when_some(frame.footer, |d, footer| {
                d.child(
                    div()
                        .flex_none()
                        .px(px(DIALOG_PADDING))
                        .py(px(10.))
                        .border_t_1()
                        .border_color(t.border)
                        .child(footer),
                )
            });
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(t.scrim)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id(frame.id)
                    .relative()
                    .occlude()
                    .w(px(width))
                    .h(px(height))
                    .min_h_0()
                    .rounded(px(13.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(t.border)
                    .shadow_xl()
                    .bg(t.surface)
                    .track_focus(&self.modal_focus)
                    .tab_group()
                    .flex()
                    .when(narrow, |d| d.flex_col())
                    .child(frame.navigation)
                    .child(page)
                    .child(
                        div()
                            .absolute()
                            .top(px(8.))
                            .right(px(8.))
                            .child(self.icon_button(
                                frame.close.0,
                                ("x", frame.close.1),
                                ButtonStyle::Quiet,
                                Availability::available(),
                                LocalAction::Close,
                                cx,
                            )),
                    ),
            )
    }
    /// Wide navigation. The header names the dialog and, where relevant, its context.
    pub(super) fn dialog_rail(
        &self,
        header: impl IntoElement,
        items: impl IntoIterator<Item = Stateful<Div>>,
    ) -> Div {
        div()
            .flex_none()
            .w(px(DIALOG_RAIL))
            .h_full()
            .flex()
            .flex_col()
            .p(px(16.))
            // GPUI's overflow mask is rectangular. Each opaque child at the
            // frame edge must draw the same inner corner as its parent.
            .rounded_tl(px(12.))
            .rounded_bl(px(12.))
            .bg(self.theme.subtle)
            .border_r_1()
            .border_color(self.theme.border)
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .min_w_0()
                    .px_2()
                    .py_3()
                    .mb_3()
                    .child(header),
            )
            .children(items)
    }
    /// Narrow navigation sits above the page and leaves room for the close button.
    pub(super) fn dialog_bar(&self, content: impl IntoElement) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .pr(px(48.))
            .py_2()
            .border_b_1()
            .border_color(self.theme.border)
            .child(content)
    }
    pub(super) fn dialog_title(&self, title: &str) -> Div {
        div()
            .text_size(self.sp(16.))
            .font_weight(FontWeight::MEDIUM)
            .child(self.l(title).to_owned())
    }
    /// Navigation entries: icon first, then label; selection is a quiet tint.
    pub(super) fn dialog_nav_item(
        &self,
        id: impl Into<gpui::ElementId>,
        content: (&'static str, &'static str),
        active: bool,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let (label, symbol) = content;
        self.control(id, ButtonStyle::Quiet, true, action, cx)
            .w_full()
            .justify_start()
            .min_h(px(37.))
            .mb_1()
            .bg(if active {
                self.theme.selected
            } else {
                gpui::transparent_black()
            })
            .text_color(self.theme.text)
            .font_weight(if active {
                FontWeight::MEDIUM
            } else {
                FontWeight::NORMAL
            })
            .child(self.scaled_icon(symbol, 17., self.theme.muted))
            .child(self.l(label).to_owned())
    }
    /// Every page starts with the same title scale; descriptions stay one quiet line.
    pub(super) fn dialog_page_header(&self, title: &str, description: Option<&str>) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .pr(px(32.))
            .child(
                div()
                    .text_size(self.sp(20.))
                    .line_height(self.sp(28.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.l(title).to_owned()),
            )
            .when_some(description, |d, text| d.child(self.hint(self.l(text))))
    }
    /// A page is a column of groups separated by space, not by cards.
    pub(super) fn dialog_page(&self) -> Div {
        div().flex().flex_col().gap_6().w_full()
    }
    /// A group of related rows under a short label.
    pub(super) fn dialog_section(&self, label: Option<&str>) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(label, |d, label| {
                d.child(
                    div()
                        .text_size(self.sp(13.))
                        .line_height(self.sp(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.l(label).to_owned()),
                )
            })
    }
    /// Label and explanation on the left, the single control on the right.
    pub(super) fn dialog_row(
        &self,
        label: &str,
        detail: Option<&str>,
        control: impl IntoElement,
    ) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .py(px(10.))
            .border_b_1()
            .border_color(self.theme.border)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(self.sp(13.))
                            .line_height(self.sp(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.l(label).to_owned()),
                    )
                    .when_some(detail, |d, detail| d.child(self.hint(self.l(detail)))),
            )
            .child(control)
    }
    /// Buttons keep their intrinsic width instead of stretching across the page.
    pub(super) fn dialog_actions(&self) -> Div {
        div().flex_none().flex().flex_wrap().items_center().gap_2()
    }
    /// Container for one exclusive choice; options keep their natural width.
    pub(super) fn segmented(&self) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .p(px(2.))
            .rounded(px(8.))
            .bg(self.theme.subtle)
    }
    pub(super) fn segment(
        &self,
        id: impl Into<gpui::ElementId>,
        label: &'static str,
        selected: bool,
        availability: Availability,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let enabled = availability.is_enabled();
        self.button(id, label, ButtonStyle::Quiet, availability, action, cx)
            .min_h(px(32.))
            .py_0()
            .px_3()
            .text_size(self.sp(12.))
            .when(selected, |d| {
                d.bg(t.segment)
                    .border_color(t.input_border)
                    .font_weight(FontWeight::MEDIUM)
                    .when(enabled, |d| d.text_color(t.text))
            })
    }
    pub fn settings_view(&self, window: &Window, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let narrow = self.dialog_narrow(window);
        let (width, _) = Self::dialog_size(window);
        let content_width = width - if narrow { 0. } else { DIALOG_RAIL } - 2. * DIALOG_PADDING;
        let tab = if self.overlay == Some(Overlay::Accounts) {
            SettingsTab::Accounts
        } else {
            self.settings_tab
        };
        let appearance = tab == SettingsTab::Appearance;
        let mut content = self.dialog_page();
        if appearance {
            content = content
                .gap(px(12.))
                .child(self.dialog_page_header(
                    "Darstellung",
                    Some("Ein Arbeitsplatz, der sich nach dir anfühlt."),
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(self.settings_label(self.l("Erscheinungsbild")))
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .when(narrow, |d| d.flex_col())
                                .children(
                                    [Appearance::System, Appearance::Light, Appearance::Dark]
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, a)| {
                                            self.appearance_card(
                                                i,
                                                a,
                                                if narrow {
                                                    96.
                                                } else {
                                                    (content_width - 24.) / 3. - 16.
                                                },
                                                narrow,
                                                cx,
                                            )
                                        }),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(self.settings_label(self.l("Farbwelt")))
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .when(narrow, |d| d.flex_col())
                                .children(
                                    [Palette::Blue, Palette::Sage]
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, p)| self.theme_card(i, p, cx)),
                                ),
                        ),
                )
                .child(
                    div()
                        .border_t_1()
                        .border_color(t.border)
                        .pt_2()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            div()
                                .child(self.settings_label(self.l("Textgröße")))
                                .child(self.hint(self.l("Für alle Gespräche."))),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .flex_wrap()
                                .p(px(2.))
                                .rounded(px(8.))
                                .bg(t.subtle)
                                .children([100u16, 125, 150, 200].into_iter().enumerate().map(
                                    |(i, n)| {
                                        self.chip(
                                            ("text-scale", usize::from(n)),
                                            format!("{n} %"),
                                            (self.scale == n, i, 4),
                                            LocalAction::Scale(n),
                                            cx,
                                        )
                                    },
                                )),
                        ),
                )
                .child(
                    div()
                        .border_t_1()
                        .border_color(t.border)
                        .pt_2()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            div()
                                .child(self.settings_label(self.l("Seitenleiste")))
                                .child(self.hint(self.l("Abstand der Gesprächszeilen. Der Nachrichtenverlauf bleibt unverändert."))),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .p(px(2.))
                                .rounded(px(8.))
                                .bg(t.subtle)
                                .child(self.chip(
                                    "density-compact",
                                    "Kompakt",
                                    (self.compact, 0, 2),
                                    LocalAction::Density(true),
                                    cx,
                                ))
                                .child(self.chip(
                                    "density-relaxed",
                                    "Entspannt",
                                    (!self.compact, 1, 2),
                                    LocalAction::Density(false),
                                    cx,
                                )),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .py(px(if self.compact { 6. } else { 10. }))
                        .rounded(px(9.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.subtle)
                        .when(!self.compact, |d| d.child(self.person_avatar("?", 32.)))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .text_size(self.sp(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(self.l("Du")),
                                )
                                .child(
                                    div()
                                        .text_size(self.sp(13.))
                                        .child(self.l("So sehen deine Nachrichten aus.")),
                                ),
                        ),
                );
        } else if tab == SettingsTab::Accounts {
            content = content.child(self.dialog_page_header("Konten", None));
            let quick = self.preferences.active.quick_switcher;
            let mut switcher = self.dialog_section(None).child(self.setting_toggle("quick-switcher", "Schneller Kontowechsel",
                "Automatisch bei mehreren angemeldeten Konten. Die Auswahl bleibt hier in den Einstellungen.",
                quick.effective(self.admitted_account_count()),
                super::preferences::PreferenceEdit::QuickSwitcher(if quick.effective(self.admitted_account_count()) {
                    talk_core::preferences::QuickSwitcherPreference::Off
                } else { talk_core::preferences::QuickSwitcherPreference::On }), cx));
            if quick != talk_core::preferences::QuickSwitcherPreference::Auto {
                switcher = switcher.child(self.dialog_actions().child(self.button(
                    "quick-switcher-auto",
                    "Automatisch auswählen",
                    ButtonStyle::Quiet,
                    self.settings_availability(),
                    LocalAction::Preference(super::preferences::PreferenceEdit::QuickSwitcher(
                        talk_core::preferences::QuickSwitcherPreference::Auto,
                    )),
                    cx,
                )));
            }
            content = content.child(switcher);
            let mut accounts = self.dialog_section(None).gap_0();
            for a in self.snapshot.accounts.as_slice() {
                accounts = accounts.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .py_3()
                        .border_b_1()
                        .border_color(t.border)
                        .child(if Some(a.scope) == self.snapshot.active_account {
                            self.content_avatar(talk_core::ui::content::AssetTarget::SelfAvatar, a.initials.as_str(), 36.)
                        } else {
                            self.person_avatar(a.initials.as_str(), 36.)
                        })
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .child(div().font_weight(FontWeight::MEDIUM).child(self.account_name(a)))
                                .child(self.hint(a.server_label.as_str()))
                                .when(a.storage != talk_core::ui::presentation::StorageStatus::Saving, |d| d.child(self.hint(self.l(match a.storage {
                                    talk_core::ui::presentation::StorageStatus::NativeStored => "Anmeldung im sicheren Schlüsselspeicher gespeichert.",
                                    talk_core::ui::presentation::StorageStatus::PrivateFileStored => "Auf diesem Gerät gespeichert · unverschlüsselte private Datei.",
                                    talk_core::ui::presentation::StorageStatus::SessionOnly => "Nur für diese Sitzung angemeldet. Nach dem Beenden erneut anmelden.",
                                    talk_core::ui::presentation::StorageStatus::CleanupPending => "Die Bereinigung einer früheren Speicherung ist noch offen.",
                                    talk_core::ui::presentation::StorageStatus::RecoveryRequired => "Der Speicherzustand muss noch geklärt werden.",
                                    talk_core::ui::presentation::StorageStatus::Saving => "",
                                    talk_core::ui::presentation::StorageStatus::NotSaved => "Deine Anmeldung ist nicht gespeichert.",
                                }))))
                                .child(self.hint(self.l(match a.connection {
                                    talk_core::ui::presentation::AccountConnectionState::Connected => "Verbunden",
                                    talk_core::ui::presentation::AccountConnectionState::Guest => "Gastzugang",
                                    talk_core::ui::presentation::AccountConnectionState::Disconnected => "Nicht verbunden",
                                }))),
                        )
                        .child(
                            self.button(
                                super::focus::account_id("account", a.scope),
                                if Some(a.scope) == self.snapshot.active_account {
                                    "Ausgewählt"
                                } else {
                                    "Öffnen"
                                },
                                ButtonStyle::Secondary,
                                Availability::when(
                                    Some(a.scope) != self.snapshot.active_account,
                                    "Dieses Konto ist bereits ausgewählt.",
                                )
                                .below_control(),
                                LocalAction::Intent(
                                    talk_core::ui::presentation::UiIntent::SelectAccount(a.scope),
                                ),
                                cx,
                            ),
                        )
                        .child(self.icon_button(
                            super::focus::account_id("remove-account", a.scope),
                            ("trash-2", "Konto aus dieser App entfernen"), ButtonStyle::Quiet,
                            Availability::when(!self.account_removal.busy(), "Bitte warte auf den laufenden Vorgang."),
                            LocalAction::RemoveAccount(a.scope.account, Some(a.scope)), cx)),
                );
            }
            for saved in self
                .snapshot
                .saved_accounts
                .as_slice()
                .iter()
                .filter(|a| a.phase != talk_core::ui::presentation::SavedAccountPhase::Active)
            {
                use talk_core::ui::presentation::SavedAccountPhase;
                let status = match saved.phase {
                    SavedAccountPhase::Dormant => "Gespeichert · nicht verbunden",
                    SavedAccountPhase::Restoring => "Verbindung wird wiederhergestellt …",
                    SavedAccountPhase::WaitingForBrowser => "Warte auf die Browseranmeldung …",
                    SavedAccountPhase::AwaitingStorageChoice => "Speicherwahl erforderlich",
                    SavedAccountPhase::ReauthenticationRequired => "Erneute Anmeldung erforderlich",
                    SavedAccountPhase::RecoveryRequired => "Wiederherstellung erforderlich",
                    SavedAccountPhase::Active => "Verbunden",
                };
                accounts = accounts.child(div().flex().items_center().gap_3().py_3().border_b_1().border_color(t.border)
                    .child(div().flex_1().min_w_0()
                        .child(div().font_weight(FontWeight::MEDIUM).child(saved.label.as_ref().map_or_else(|| self.l("Gespeichertes Konto").to_owned(), |s| s.as_str().to_owned())))
                        .when_some(saved.server_label.as_ref(), |d, label| d.child(self.hint(label.as_str())))
                        .child(self.hint(self.l(status)))
                        .when(saved.failure.is_some(), |d| d.child(self.hint(self.l("Das Konto konnte nicht verbunden werden. Bitte erneut versuchen."))))
                        .when(saved.cleanup_pending, |d| d.child(self.hint(self.l("Aufräumen des Schlüsselspeichers noch ausstehend."))))
                        .when(saved.suppression_unconfirmed, |d| d.child(self.hint(self.l("Der gespeicherte Anmeldestatus muss wiederhergestellt werden.")))))
                    .child(self.button(gpui::SharedString::from(format!("restore-account-{}", saved.id.as_local())), "Verbinden", ButtonStyle::Secondary,
                        Availability::when(matches!(saved.phase, SavedAccountPhase::Dormant | SavedAccountPhase::ReauthenticationRequired),
                            "Die Verbindung ist noch nicht bereit."), if saved.phase == SavedAccountPhase::ReauthenticationRequired { LocalAction::Reauthenticate(saved.id) } else { LocalAction::Intent(talk_core::ui::presentation::UiIntent::RestoreAccount(saved.id)) }, cx))
                    .child(self.icon_button(gpui::SharedString::from(format!("remove-saved-account-{}", saved.id.as_local())),
                        ("trash-2", "Konto aus dieser App entfernen"), ButtonStyle::Quiet,
                        Availability::when(!self.account_removal.busy() && matches!(saved.phase, SavedAccountPhase::Dormant | SavedAccountPhase::ReauthenticationRequired | SavedAccountPhase::RecoveryRequired),
                            "Bitte warte auf den laufenden Vorgang."), LocalAction::RemoveAccount(saved.id, None), cx)));
            }
            if self.snapshot.accounts.as_slice().is_empty()
                && self.snapshot.saved_accounts.as_slice().is_empty()
            {
                accounts = accounts.child(self.hint(self.l("Noch kein Konto verbunden.")));
            }
            content = content
                .child(accounts.child(self.account_removal_feedback(cx)))
                .child(self.dialog_actions().child(self.button(
                    "add-account",
                    "Konto hinzufügen",
                    ButtonStyle::Secondary,
                    true,
                    LocalAction::AddAccount,
                    cx,
                )));
        }
        if !appearance && tab != SettingsTab::Accounts {
            content = self.settings_page(tab, cx);
        }
        if narrow && self.settings_navigation_open {
            content = div().flex().flex_col().gap_1().children(
                SettingsTab::ALL
                    .into_iter()
                    .map(|item| self.settings_nav(item, item == tab, cx)),
            );
        } else if matches!(
            tab,
            SettingsTab::General
                | SettingsTab::Appearance
                | SettingsTab::Devices
                | SettingsTab::Notifications
        ) {
            // Computer persistence cannot confirm account-owned values or sources.
            content = content.child(self.preference_status(cx));
        }
        content = content.when(self.feedback.is_some_and(|f| f.context.is_none()), |d| {
            d.child(self.feedback_view(cx))
        });
        let navigation = if narrow {
            self.dialog_bar(self.button_with_leading_icon(
                "settings-menu",
                (self.l(tab.key()), "chevron-down"),
                ButtonStyle::Secondary,
                Availability::available(),
                LocalAction::SettingsMenu,
                cx,
            ))
        } else {
            self.dialog_rail(
                self.dialog_title("Einstellungen"),
                SettingsTab::ALL
                    .into_iter()
                    .map(|item| self.settings_nav(item, item == tab, cx)),
            )
        };
        self.dialog_frame(
            DialogFrame {
                id: "settings-modal",
                close: ("close-settings", "Einstellungen schließen"),
                region: super::navigation::Region::Settings,
                navigation,
                page: self.reveal(("settings-page", tab as usize), content),
                footer: None,
            },
            window,
            cx,
        )
    }
    fn settings_label(&self, text: &str) -> Div {
        div()
            .text_size(self.sp(13.))
            .font_weight(FontWeight::MEDIUM)
            .child(self.l(text).to_owned())
    }
    fn settings_nav(&self, tab: SettingsTab, active: bool, cx: &Context<Self>) -> Stateful<Div> {
        self.dialog_nav_item(
            ("settings-tab", tab as usize),
            (tab.key(), tab.icon()),
            active,
            LocalAction::SettingsTab(tab),
            cx,
        )
    }
    fn choice(
        &self,
        id: impl Into<gpui::ElementId>,
        selected: bool,
        action: LocalAction,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        self.control_with_geometry(id, ButtonStyle::Quiet, true, action, (2., 10.), cx)
            .relative()
            .min_w_0()
            .rounded(px(10.))
            .p(px(6.))
            .border_2()
            .border_color(if selected { t.focus } else { t.border })
            .bg(if selected { t.subtle } else { t.surface })
    }
    fn check_badge(&self) -> Div {
        div()
            .absolute()
            .right(px(5.))
            .top(px(5.))
            .size(px(18.))
            .rounded_full()
            .bg(self.theme.accent)
            .flex()
            .items_center()
            .justify_center()
            .child(icon("check", 12., self.theme.on_accent))
    }
    fn appearance_card(
        &self,
        i: usize,
        a: Appearance,
        w: f32,
        narrow: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let selected = self.appearance == a;
        let t = self.theme;
        let light = Theme::resolve(self.palette, Appearance::Light, WindowAppearance::Light);
        let dark = Theme::resolve(self.palette, Appearance::Dark, WindowAppearance::Dark);
        let picture = div()
            .relative()
            .w(px(w))
            .h(px(88.))
            .flex_none()
            .overflow_hidden()
            .rounded(px(6.))
            .when(a != Appearance::System, |d| {
                d.child(miniature(
                    if a == Appearance::Dark { dark } else { light },
                    w,
                ))
            })
            .when(a == Appearance::System, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .w(relative(0.5))
                        .h_full()
                        .overflow_hidden()
                        .child(miniature(light, w)),
                )
                .child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .w(relative(0.5))
                        .h_full()
                        .overflow_hidden()
                        .child(miniature(dark, w).absolute().left(px(-w / 2.))),
                )
            });
        let card = self
            .choice(
                ("appearance-card", i),
                selected,
                LocalAction::Appearance(a),
                cx,
            )
            .flex_1()
            .flex_col()
            .gap_1()
            .when(narrow, |d| d.flex_row().justify_start().gap_4())
            .child(picture)
            .when(selected, |d| {
                d.child(self.check_badge().top_auto().bottom(px(4.)).right(px(4.)))
            })
            .child(
                div()
                    .text_size(self.sp(13.))
                    .text_color(t.text)
                    .child(self.l(match a {
                        Appearance::System => "System",
                        Appearance::Light => "Hell",
                        Appearance::Dark => "Dunkel",
                    })),
            );
        self.roving(card, super::navigation::Group::Appearance, i, cx)
    }
    fn theme_card(&self, i: usize, p: Palette, cx: &Context<Self>) -> Stateful<Div> {
        let selected = self.palette == p;
        let t = self.theme;
        let card = self
            .choice(("palette-card", i), selected, LocalAction::Theme(p), cx)
            .flex_1()
            .min_h(px(76.))
            .items_center()
            .px_3()
            .py_2()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_1()
                    .children([false, true].into_iter().map(|dark| {
                        let preview = Theme::resolve(
                            p,
                            if dark {
                                Appearance::Dark
                            } else {
                                Appearance::Light
                            },
                            WindowAppearance::Light,
                        );
                        let active = selected && dark == t.dark;
                        div()
                            .relative()
                            .size(px(48.))
                            .rounded_full()
                            .border_2()
                            .border_color(if active {
                                t.focus
                            } else {
                                gpui::transparent_black()
                            })
                            .p(px(3.))
                            .child(
                                div()
                                    .relative()
                                    .size_full()
                                    .rounded_full()
                                    .bg(gpui::linear_gradient(
                                        135.,
                                        gpui::linear_color_stop(
                                            if dark { preview.stage } else { preview.canvas },
                                            0.,
                                        ),
                                        gpui::linear_color_stop(
                                            if dark {
                                                preview.focus
                                            } else {
                                                preview.chrome_end
                                            },
                                            1.,
                                        ),
                                    ))
                                    .child(
                                        div()
                                            .absolute()
                                            .left(px(6.))
                                            .top(px(5.))
                                            .w(px(20.))
                                            .h(px(11.))
                                            .rounded_full()
                                            .bg(gpui::white().opacity(if dark {
                                                0.10
                                            } else {
                                                0.35
                                            })),
                                    ),
                            )
                            .when(active, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .right(px(-2.))
                                        .bottom(px(-2.))
                                        .size(px(20.))
                                        .rounded_full()
                                        .bg(t.surface)
                                        .border_1()
                                        .border_color(t.input_border)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(icon(
                                            if dark { "moon" } else { "sun" },
                                            14.,
                                            t.text,
                                        )),
                                )
                            })
                    })),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(self.sp(14.))
                            .text_color(t.text)
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.l(if p == Palette::Blue { "Blau" } else { "Salbei" })),
                    )
                    .child(self.hint(self.l(if p == Palette::Blue {
                        "Ruhig und klar."
                    } else {
                        "Natürlich und ruhig."
                    }))),
            )
            .when(selected, |d| d.child(self.check_badge()));
        self.roving(card, super::navigation::Group::Palette, i, cx)
    }
}
