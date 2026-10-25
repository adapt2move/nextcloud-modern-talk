//! Notice timing only. Raw reachability still governs admission and queued work.
use super::{ButtonStyle, Desktop, LocalAction, controls::Availability};
use gpui::{Context, Div, div, prelude::*, px};
use std::time::{Duration, Instant};
use talk_core::ui::{
    AccountScope,
    presentation::{ConnectionFailure, ContentState, ConversationRef, UiIntent},
};

const OFFLINE_GRACE: Duration = Duration::from_secs(4);

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
            cx.notify();
        }
    }

    pub(super) fn offline_notice_visible(&self) -> bool {
        self.offline_notice.visible(self.snapshot.active_account)
    }

    pub(super) fn offline_footer(
        &self,
        context: ConversationRef,
        cx: &Context<Self>,
    ) -> Option<Div> {
        self.offline_notice_visible()
            .then(|| self.connection_line(context, None, cx))
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
        let (label, detail) = failure
            .map(super::state::connection_failure_copy)
            .unwrap_or(("Offline", ""));
        let tint = if failure.is_some() {
            self.theme.danger
        } else {
            self.theme.muted
        };
        let availability = Availability::available().help(detail);
        let id = super::controls::scoped_id("connection-retry", context, 0, 0);
        div()
            .min_h(px(28.))
            .flex()
            .items_center()
            .gap_2()
            .text_size(self.sp(12.))
            .line_height(self.sp(18.))
            .text_color(tint)
            .child(self.scaled_icon(if failure.is_some() { "info" } else { "cloud" }, 14., tint))
            .child(div().flex_1().min_w_0().child(self.l(label)))
            .child(
                self.icon_button(
                    id,
                    ("refresh", "Erneut laden"),
                    ButtonStyle::Quiet,
                    availability,
                    LocalAction::Intent(UiIntent::Refresh(context)),
                    cx,
                )
                .size(px(28.))
                .min_h(px(28.)),
            )
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
