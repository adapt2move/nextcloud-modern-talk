//! Notice timing only. Raw reachability still governs admission and queued work.
use super::{
    ButtonStyle, Desktop, LocalAction,
    controls::{Availability, BannerRole},
};
use gpui::{Context, Div, div, prelude::*, px};
use std::time::{Duration, Instant};
use talk_core::ui::{
    AccountScope,
    presentation::{ConnectionFailure, ContentState, ConversationRef, UiIntent},
};

const OFFLINE_GRACE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
enum Observation {
    Healthy,
    Pending,
    TransientFailure,
    ActionableFailure,
}

pub(super) fn transient(failure: ConnectionFailure) -> bool {
    matches!(
        failure,
        ConnectionFailure::Unreachable
            | ConnectionFailure::TimedOut
            | ConnectionFailure::ServerUnavailable
    )
}

fn observation(
    content: ContentState,
    rooms: ContentState,
    failure: Option<ConnectionFailure>,
) -> Observation {
    if failure.is_some_and(|failure| !transient(failure)) || content == ContentState::Denied {
        Observation::ActionableFailure
    } else if failure.is_some()
        || content == ContentState::Offline
        || rooms == ContentState::Offline
    {
        Observation::TransientFailure
    } else if content == ContentState::Ready || rooms == ContentState::Ready {
        Observation::Healthy
    } else if content == ContentState::Loading || rooms == ContentState::Loading {
        Observation::Pending
    } else {
        Observation::ActionableFailure
    }
}

/// One active account/runtime episode, with no timer, executor or retained task.
/// The existing refresh tick advances the deadline even without a new snapshot.
#[derive(Default)]
pub(super) struct Notice {
    scope: Option<AccountScope>,
    failed_since: Option<Instant>,
    visible: bool,
}
impl Notice {
    fn observe(
        &mut self,
        scope: Option<AccountScope>,
        observed: Observation,
        now: Instant,
    ) -> bool {
        let before = self.visible;
        if self.scope != scope {
            self.scope = scope;
            self.failed_since = None;
            self.visible = false;
        }
        match observed {
            Observation::TransientFailure if scope.is_some() => {
                self.failed_since.get_or_insert(now);
            }
            Observation::Pending if scope.is_some() => {}
            _ => {
                self.failed_since = None;
                self.visible = false;
            }
        }
        self.visible = self
            .failed_since
            .is_some_and(|at| now.saturating_duration_since(at) >= OFFLINE_GRACE);
        before != self.visible
    }
    fn visible(&self, scope: Option<AccountScope>) -> bool {
        scope.is_some() && self.scope == scope && self.visible
    }
    /// Start of the current unhealthy episode, for "disconnected since" copy.
    fn since(&self, scope: Option<AccountScope>) -> Option<Instant> {
        self.visible(scope).then_some(self.failed_since).flatten()
    }
}

/// The two calm states a title-bar status can show. A healthy account shows nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Link {
    /// The server is slow or briefly unreachable and retries are running.
    Reconnecting,
    /// The device cannot reach the server, or accepted changes are waiting for it.
    Offline,
}

/// The waiting dot. On a light wash the shared away-presence amber is too pale
/// to reach 3:1 as a graphic, so light themes use a deeper amber.
pub(super) fn status_dot(theme: super::Theme) -> gpui::Hsla {
    if theme.dark {
        theme.presence_dot(talk_core::ui::presentation::Presence::Away)
    } else {
        gpui::rgb(0xb37400).into()
    }
}

/// Classify a sustained transient failure. Queued changes always mean the user
/// should read "Offline": the queue only drains once the server is back.
fn link(queued: u32, failure: Option<ConnectionFailure>) -> Link {
    if queued > 0 || matches!(failure, Some(ConnectionFailure::Unreachable)) {
        Link::Offline
    } else {
        Link::Reconnecting
    }
}

impl Desktop {
    pub(super) fn refresh_offline_notice(&mut self, cx: &mut Context<Self>) {
        let scope = self.snapshot.active_account;
        let failure = scope.and_then(|scope| self.account_connection_failure(scope));
        let observed = observation(
            self.snapshot.content,
            self.snapshot.conversation_list.content,
            failure,
        );
        if self.offline_notice.observe(scope, observed, Instant::now()) {
            if !self.offline_notice_visible() {
                self.connection_details_open = false;
            }
            cx.notify();
        }
    }

    pub(super) fn offline_notice_visible(&self) -> bool {
        self.offline_notice.visible(self.snapshot.active_account)
    }

    /// Accepted changes that wait for the server. `pending_intents` is the
    /// durable per-account count; the open room's queue is only a fallback for
    /// projections that do not carry it.
    fn queued_changes(&self) -> u32 {
        let room = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| Some(panel.context) == self.snapshot.selected)
            .map_or(0, |panel| panel.queued_actions);
        u32::from(self.snapshot.pending_intents).max(room)
    }

    /// `None` while the account is healthy, and during the five-second grace
    /// period that hides routine blips.
    pub(super) fn link_state(&self) -> Option<(Link, u32)> {
        if !self.offline_notice_visible() {
            return None;
        }
        let failure = self
            .snapshot
            .active_account
            .and_then(|scope| self.account_connection_failure(scope));
        let queued = self.queued_changes();
        Some((link(queued, failure), queued))
    }

    /// Compact, calm status for the application title bar: a dot and one word,
    /// with the details one click away. Nothing is drawn while healthy.
    pub(super) fn connection_status(&self, cx: &Context<Self>) -> Option<Div> {
        let (state, queued) = self.link_state()?;
        let t = self.theme;
        let word = self.l(match state {
            Link::Reconnecting => "Verbinde …",
            Link::Offline => "Offline",
        });
        let open = self.connection_details_open;
        let pill = div()
            .h(px(24.))
            .px(px(10.))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .bg(BannerRole::Warning.wash(t))
            .when(open, |d| d.bg(BannerRole::Warning.wash(t).opacity(1.)))
            .text_color(t.text)
            .text_size(px(12.))
            .line_height(px(16.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(
                div()
                    .flex_none()
                    .size(px(7.))
                    .rounded_full()
                    .bg(status_dot(t)),
            )
            .child(word.to_owned())
            .when(queued > 0, |d| {
                d.child(
                    div()
                        .text_color(t.muted)
                        .font_weight(gpui::FontWeight::NORMAL)
                        .child(format!("· {queued}")),
                )
            });
        let trigger = self
            .control(
                "connection-status",
                ButtonStyle::Quiet,
                Availability::available(),
                LocalAction::ToggleConnectionDetails,
                cx,
            )
            .h(px(32.))
            .min_h(px(32.))
            .px_0()
            .py_0()
            .rounded_full()
            .child(pill);
        Some(div().relative().flex_none().child(trigger).when(open, |d| {
            d.child(gpui::deferred(
                gpui::anchored()
                    .anchor(gpui::Corner::TopLeft)
                    .offset(gpui::point(px(0.), px(4.)))
                    .child(self.connection_details(state, queued, cx)),
            ))
        }))
    }

    fn connection_details(
        &self,
        state: Link,
        queued: u32,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let t = self.theme;
        let (title, detail) = match (state, queued > 0) {
            (Link::Offline, true) => (
                "Offline",
                "Änderungen werden gesendet, sobald du wieder online bist.",
            ),
            (Link::Offline, false) => (
                "Offline",
                "Der bestätigte Stand bleibt sichtbar. Neue Inhalte erscheinen nach der Verbindung.",
            ),
            (Link::Reconnecting, _) => (
                "Verbindung wird wiederhergestellt",
                "Der bestätigte Stand bleibt sichtbar. Talk versucht es automatisch erneut.",
            ),
        };
        let since = self
            .offline_notice
            .since(self.snapshot.active_account)
            .map(|at| Instant::now().saturating_duration_since(at).as_secs() / 60);
        let row = |label: String, value: String| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .text_size(self.sp(12.))
                .line_height(self.sp(18.))
                .child(div().text_color(t.muted).child(label))
                .child(div().font_weight(gpui::FontWeight::MEDIUM).child(value))
        };
        self.context_menu_surface()
            .id("connection-details")
            .w(px(300.))
            .occlude()
            .p_3()
            .gap_2()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.connection_details_open {
                    this.connection_details_open = false;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(self.l(title).to_owned()),
                    )
                    .child(
                        div()
                            .text_size(self.sp(12.))
                            .line_height(self.sp(18.))
                            .text_color(t.muted)
                            .child(self.l(detail).to_owned()),
                    ),
            )
            .when(queued > 0 || since.is_some(), |d| {
                d.child(div().h(px(1.)).bg(t.border)).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when(queued > 0, |d| {
                            d.child(row(
                                self.l("Wartet auf Senden").to_owned(),
                                self.lf(
                                    if queued == 1 {
                                        "{n} Änderung"
                                    } else {
                                        "{n} Änderungen"
                                    },
                                    &[("n", &queued.to_string())],
                                ),
                            ))
                        })
                        .when_some(since, |d, minutes| {
                            d.child(row(
                                self.l("Getrennt seit").to_owned(),
                                if minutes == 0 {
                                    self.l("weniger als einer Minute").to_owned()
                                } else {
                                    self.lf(
                                        if minutes == 1 {
                                            "{n} Minute"
                                        } else {
                                            "{n} Minuten"
                                        },
                                        &[("n", &minutes.to_string())],
                                    )
                                },
                            ))
                        }),
                )
            })
            .child(
                div().flex().justify_end().child(
                    self.button(
                        "connection-retry",
                        "Jetzt erneut verbinden",
                        ButtonStyle::Secondary,
                        true,
                        LocalAction::RetryConnection,
                        cx,
                    )
                    .min_h(px(32.)),
                ),
            )
    }

    /// Retry both the account's conversation list and the open room.
    pub(super) fn retry_connection(&mut self) {
        let Some(scope) = self.snapshot.active_account else {
            return;
        };
        self.submit(UiIntent::ReloadConversations(scope));
        if let Some(context) = self.snapshot.selected {
            self.submit(UiIntent::Refresh(context));
        }
    }

    pub(super) fn connection_failure_footer(
        &self,
        context: ConversationRef,
        failure: ConnectionFailure,
        cx: &Context<Self>,
    ) -> Option<Div> {
        (!transient(failure)).then(|| self.connection_line(context, Some(failure), cx))
    }

    fn connection_line(
        &self,
        context: ConversationRef,
        failure: Option<ConnectionFailure>,
        cx: &Context<Self>,
    ) -> Div {
        let (title, detail) = failure.map_or(
            (
                "Verbindung wird wiederhergestellt",
                "Talk versucht es automatisch erneut.",
            ),
            super::state::connection_failure_copy,
        );
        let id = super::controls::scoped_id("connection-retry", context, 0, 0);
        // Words and icon name the cause; the wash only says "needs a look".
        let (role, icon) = match failure {
            Some(ConnectionFailure::AccessDenied) => (BannerRole::Error, "lock"),
            Some(ConnectionFailure::RateLimited) => (BannerRole::Warning, "clock"),
            Some(ConnectionFailure::InvalidResponse) => (BannerRole::Error, "info"),
            Some(_) => (BannerRole::Error, "cloud-off"),
            None => (BannerRole::Warning, "cloud-off"),
        };
        self.notice(role, self.l(title), Some(self.l(detail)), Some(icon))
            .child(self.notice_action(
                id,
                "Erneut versuchen",
                LocalAction::Intent(UiIntent::Refresh(context)),
                cx,
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, RuntimeGeneration};
    fn account(runtime: u128) -> AccountScope {
        AccountScope {
            account: AccountId::from_local(1),
            runtime: RuntimeGeneration::from_local(runtime),
        }
    }

    #[test]
    fn transient_blip_never_shows_a_notice_and_recovery_cancels_the_deadline() {
        let now = Instant::now();
        let scope = Some(account(1));
        let mut notice = Notice::default();
        assert!(!notice.observe(scope, Observation::TransientFailure, now));
        assert!(!notice.observe(
            scope,
            Observation::TransientFailure,
            now + OFFLINE_GRACE - Duration::from_millis(1)
        ));
        assert!(!notice.visible(scope));
        assert!(!notice.observe(scope, Observation::Healthy, now + OFFLINE_GRACE));
        assert!(!notice.observe(scope, Observation::Pending, now + OFFLINE_GRACE * 2));
        assert!(!notice.visible(scope));
    }

    #[test]
    fn sustained_failure_uses_one_deadline_and_a_retry_does_not_flash_it_away() {
        let now = Instant::now();
        let scope = Some(account(1));
        let mut notice = Notice::default();
        notice.observe(scope, Observation::TransientFailure, now);
        notice.observe(scope, Observation::Pending, now + Duration::from_secs(2));
        assert!(notice.observe(scope, Observation::TransientFailure, now + OFFLINE_GRACE));
        assert!(notice.visible(scope));
        assert!(!notice.observe(
            scope,
            Observation::TransientFailure,
            now + OFFLINE_GRACE * 4
        ));
        assert!(!notice.observe(scope, Observation::Pending, now + OFFLINE_GRACE * 5));
        assert!(notice.visible(scope));
        assert!(notice.observe(scope, Observation::Healthy, now + OFFLINE_GRACE * 6));
        assert!(!notice.visible(scope));
    }

    #[test]
    fn runtime_replacement_and_actionable_failures_retire_old_notice_authority() {
        let now = Instant::now();
        let mut notice = Notice::default();
        notice.observe(Some(account(1)), Observation::TransientFailure, now);
        notice.observe(
            Some(account(1)),
            Observation::TransientFailure,
            now + OFFLINE_GRACE,
        );
        assert!(!notice.visible(Some(account(2))));
        assert!(notice.observe(
            Some(account(2)),
            Observation::TransientFailure,
            now + OFFLINE_GRACE
        ));
        assert!(!notice.visible(Some(account(2))));
        notice.observe(
            Some(account(2)),
            Observation::TransientFailure,
            now + OFFLINE_GRACE * 2,
        );
        assert!(notice.visible(Some(account(2))));
        assert!(notice.observe(
            Some(account(2)),
            Observation::ActionableFailure,
            now + OFFLINE_GRACE * 3
        ));
        assert!(!notice.visible(Some(account(2))));
        assert!(!notice.observe(None, Observation::TransientFailure, now + OFFLINE_GRACE * 4));
    }

    #[test]
    fn queued_changes_or_an_unreachable_server_read_offline_otherwise_reconnecting() {
        assert_eq!(link(0, None), Link::Reconnecting);
        assert_eq!(
            link(0, Some(ConnectionFailure::TimedOut)),
            Link::Reconnecting
        );
        assert_eq!(
            link(0, Some(ConnectionFailure::ServerUnavailable)),
            Link::Reconnecting
        );
        assert_eq!(link(0, Some(ConnectionFailure::Unreachable)), Link::Offline);
        assert_eq!(link(1, None), Link::Offline);
        assert_eq!(link(7, Some(ConnectionFailure::TimedOut)), Link::Offline);
    }

    #[test]
    fn disconnected_since_reports_only_the_visible_episode_of_the_current_scope() {
        let now = Instant::now();
        let scope = Some(account(1));
        let mut notice = Notice::default();
        notice.observe(scope, Observation::TransientFailure, now);
        assert!(
            notice.since(scope).is_none(),
            "grace period hides the episode"
        );
        notice.observe(scope, Observation::TransientFailure, now + OFFLINE_GRACE);
        assert_eq!(notice.since(scope), Some(now));
        assert!(notice.since(Some(account(2))).is_none());
        notice.observe(scope, Observation::Healthy, now + OFFLINE_GRACE * 2);
        assert!(notice.since(scope).is_none());
    }

    #[test]
    fn denial_and_rate_limits_are_actionable_immediately() {
        for failure in [
            ConnectionFailure::AccessDenied,
            ConnectionFailure::RateLimited,
            ConnectionFailure::InvalidResponse,
        ] {
            assert!(matches!(
                observation(ContentState::Offline, ContentState::Offline, Some(failure)),
                Observation::ActionableFailure
            ));
        }
        for failure in [
            ConnectionFailure::Unreachable,
            ConnectionFailure::TimedOut,
            ConnectionFailure::ServerUnavailable,
        ] {
            assert!(matches!(
                observation(ContentState::Ready, ContentState::Ready, Some(failure)),
                Observation::TransientFailure
            ));
        }
    }
}
