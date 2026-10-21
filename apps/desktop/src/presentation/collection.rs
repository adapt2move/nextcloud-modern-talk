//! Shared loading/recovery presentation. Retained data never establishes authority.
use super::{ButtonStyle, Desktop, LocalAction};
use gpui::{Context, Div, FontWeight, div, prelude::*, px};
use talk_core::ui::presentation::ContentState;

#[derive(Clone, Copy)]
pub(super) struct CollectionState {
    state: ContentState,
    retained: bool,
}
impl CollectionState {
    pub fn new(state: ContentState, retained: bool) -> Self {
        Self { state, retained }
    }
    pub fn shows_content(self) -> bool {
        self.retained
            && !matches!(
                self.state,
                ContentState::Denied | ContentState::Unconfigured
            )
    }
    pub fn confirmed_empty(self) -> bool {
        !self.retained && self.state == ContentState::Ready
    }
    fn retry(self) -> bool {
        matches!(
            self.state,
            ContentState::Offline | ContentState::Failed | ContentState::Denied
        )
    }
    fn copy(self) -> Option<(&'static str, &'static str, &'static str)> {
        Some(match self.state {
            ContentState::Ready => return None,
            ContentState::Loading if self.retained => (
                "refresh",
                "Wird aktualisiert …",
                "Der bestätigte Stand bleibt sichtbar.",
            ),
            ContentState::Loading => ("clock", "Wird geladen …", "Bitte warte einen Moment."),
            ContentState::Offline => (
                "cloud",
                "Offline",
                if self.retained {
                    "Der bestätigte Stand bleibt sichtbar. Verbinde dich erneut und lade die Inhalte neu."
                } else {
                    "Inhalte konnten noch nicht geladen werden. Prüfe deine Verbindung und versuche es erneut."
                },
            ),
            ContentState::Failed => (
                "info",
                "Aktualisierung fehlgeschlagen.",
                if self.retained {
                    "Der bestätigte Stand bleibt sichtbar. Bitte erneut versuchen."
                } else {
                    "Inhalte konnten nicht geladen werden. Bitte erneut versuchen."
                },
            ),
            ContentState::Denied => (
                "lock",
                "Kein Zugriff.",
                "Deine Berechtigungen können sich geändert haben. Prüfe den Zugriff erneut.",
            ),
            ContentState::Unconfigured => (
                "info",
                "Verbindung noch nicht verfügbar.",
                "Diese Funktion ist in diesem Client noch nicht verfügbar.",
            ),
        })
    }
}

impl Desktop {
    pub(super) fn collection_feedback(
        &self,
        state: CollectionState,
        id: &'static str,
        retry: LocalAction,
        cx: &Context<Self>,
    ) -> Div {
        let Some((symbol, title, detail)) = state.copy() else {
            return div();
        };
        let t = self.theme;
        let warning = matches!(
            state.state,
            ContentState::Offline | ContentState::Failed | ContentState::Denied
        );
        let fg = if warning { t.warning } else { t.muted };
        div()
            .flex_none()
            .rounded(px(8.))
            .bg(if warning { t.warning_bg } else { t.subtle })
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(self.scaled_icon(symbol, 16., fg))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .text_size(self.sp(12.))
                            .line_height(self.sp(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg)
                            .child(self.l(title)),
                    ),
            )
            .child(self.hint(self.l(detail)))
            .when(state.retry(), |d| {
                d.child(self.button(id, "Erneut laden", ButtonStyle::Secondary, true, retry, cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_and_temporary_failure_keep_content_but_never_claim_empty() {
        for state in [
            ContentState::Loading,
            ContentState::Offline,
            ContentState::Failed,
        ] {
            let retained = CollectionState::new(state, true);
            assert!(retained.shows_content());
            assert!(!retained.confirmed_empty());
            assert!(retained.copy().is_some());
            let initial = CollectionState::new(state, false);
            assert!(!initial.shows_content());
            assert!(!initial.confirmed_empty());
            assert!(initial.copy().is_some());
            assert_eq!(initial.retry(), state != ContentState::Loading);
        }
        assert!(CollectionState::new(ContentState::Ready, false).confirmed_empty());
        assert!(CollectionState::new(ContentState::Ready, true).shows_content());
        assert!(
            CollectionState::new(ContentState::Ready, true)
                .copy()
                .is_none()
        );
    }

    #[test]
    fn confirmed_denial_never_displays_retained_content_and_still_offers_revalidation() {
        for retained in [false, true] {
            let denied = CollectionState::new(ContentState::Denied, retained);
            assert!(!denied.shows_content());
            assert!(!denied.confirmed_empty());
            assert!(denied.retry());
            assert!(denied.copy().is_some());
            let unsupported = CollectionState::new(ContentState::Unconfigured, retained);
            assert!(!unsupported.shows_content());
            assert!(!unsupported.confirmed_empty());
            assert!(!unsupported.retry());
        }
    }
}
