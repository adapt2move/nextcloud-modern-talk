//! One viewport-driven history request. Transport and page bounds belong to composition.
use super::*;
use talk_core::ui::conversation::RemoteState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Status {
    Quiet,
    Loading,
    Retry,
    End,
}

#[derive(Default)]
pub(super) struct History {
    visible: bool,
    at_top: bool,
    demand: bool,
    attempted: Option<u128>,
    pending: Option<(RequestId, u64)>,
    failed: bool,
    attempts: u64,
}

impl History {
    pub fn visibility(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            self.demand = false;
            self.at_top = false;
        }
    }
    pub fn upward(&mut self) {
        self.demand = true;
    }
    pub fn layout(&mut self, at_top: bool) {
        self.at_top = at_top;
    }
    pub fn reconcile(&mut self, revision: u64, state: RemoteState, older: Option<u128>) {
        let Some((request, admitted_revision)) = self.pending else {
            return;
        };
        if revision <= admitted_revision || state == RemoteState::Loading(request) {
            return;
        }
        // The synchronous composition port has admitted this request before it
        // publishes its next revision. Loading may be coalesced with completion.
        self.pending = None;
        self.failed = matches!(state, RemoteState::Failed(_, _) | RemoteState::Uncertain(_))
            || older == self.attempted;
    }
    fn admit(&mut self, older: Option<u128>, state: RemoteState, retry: bool) -> Option<u128> {
        let cursor = older?;
        if !self.visible
            || !self.at_top
            || self.pending.is_some()
            || matches!(state, RemoteState::Loading(_) | RemoteState::Saving(_))
            || (!retry
                && (!self.demand
                    || self.attempted == Some(cursor)
                    || matches!(state, RemoteState::Failed(_, _) | RemoteState::Uncertain(_))))
        {
            return None;
        }
        self.demand = false;
        self.attempts = self.attempts.saturating_add(1);
        self.attempted = Some(cursor);
        self.failed = false;
        Some(cursor)
    }
    fn submitted(&mut self, request: Option<RequestId>, revision: u64) {
        self.pending = request.map(|request| (request, revision));
        self.failed = request.is_none();
    }
    pub fn diagnostic(&self) -> (bool, bool, bool, bool, u64, bool) {
        (
            self.visible,
            self.at_top,
            self.demand,
            self.pending.is_some(),
            self.attempts,
            self.failed,
        )
    }
    fn status(&self, older: Option<u128>, state: RemoteState) -> Status {
        if self.pending.is_some()
            || matches!(state, RemoteState::Loading(_) | RemoteState::Saving(_))
        {
            Status::Loading
        } else if self.failed
            || matches!(state, RemoteState::Failed(_, _) | RemoteState::Uncertain(_))
        {
            Status::Retry
        } else if older.is_none() {
            Status::End
        } else {
            Status::Quiet
        }
    }
}

impl Desktop {
    /// Called by the existing composition refresh tick or an explicit retry.
    /// Rendering records geometry only; it never starts I/O or a task.
    pub(super) fn request_older_history(&mut self, retry: bool) -> bool {
        if self.update_restart_pending
            || self.projection_blocked
            || self.onboarding_open
            || self.navigation_open
            || self.prejoin.is_some()
            || self.incoming_visible()
            || !matches!(self.overlay, None | Some(Overlay::CallChat))
        {
            return false;
        }
        let Some(panel) = self.snapshot.message_panel.as_ref().filter(|panel| {
            Some(panel.context) == self.snapshot.selected
                && self.snapshot.content == ContentState::Ready
        }) else {
            return false;
        };
        let context = panel.context;
        // With a fully filtered page there is no row to paint a top boundary.
        // Its explicit load/retry button is a deliberate paging demand instead.
        if retry && self.snapshot.messages.as_slice().is_empty() {
            self.message_ui.history.borrow_mut().layout(true);
        }
        if self
            .message_ui
            .history
            .borrow_mut()
            .admit(panel.older, panel.history, retry)
            .is_none()
        {
            return false;
        }
        let request = self.message_ui.request.checked_add(1).map(RequestId);
        let admitted = request.filter(|request| {
            self.message_ui.request = request.0;
            self.port
                .try_submit(UiIntent::Message {
                    context,
                    request: *request,
                    action: MessageAction::Older,
                })
                .is_ok()
        });
        self.message_ui
            .history
            .borrow_mut()
            .submitted(admitted, self.snapshot.revision);
        self.message_ui.arrival.borrow_mut().manual_scroll();
        true
    }

    pub(super) fn history_boundary(&self, cx: &Context<Self>) -> Div {
        let mut boundary = div()
            .h(self.sp(32.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .text_size(self.sp(12.))
            .text_color(self.theme.muted);
        let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| Some(panel.context) == self.snapshot.selected)
        else {
            return boundary;
        };
        match self
            .message_ui
            .history
            .borrow()
            .status(panel.older, panel.history)
        {
            Status::Quiet if self.snapshot.messages.as_slice().is_empty() => {
                boundary = boundary.child(self.button(
                    "load-older-hidden-page",
                    "Ältere Nachrichten laden",
                    ButtonStyle::Quiet,
                    self.snapshot.content == ContentState::Ready && panel.older.is_some(),
                    LocalAction::MessageIn(panel.context, Action::RetryHistory),
                    cx,
                ));
            }
            Status::Quiet => {}
            Status::Loading => {
                boundary = boundary.child(self.pending_dots(self.theme.muted));
            }
            Status::Retry => {
                boundary = boundary.child(
                    self.button(
                        "retry-older-history",
                        "Ältere Nachrichten erneut laden",
                        ButtonStyle::Quiet,
                        self.snapshot.content == ContentState::Ready && panel.older.is_some(),
                        LocalAction::MessageIn(panel.context, Action::RetryHistory),
                        cx,
                    )
                    .min_h(self.sp(28.))
                    .py_0()
                    .text_size(self.sp(12.)),
                );
            }
            Status::End => boundary = boundary.child(self.l("Beginn des Gesprächs")),
        }
        boundary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_top() -> History {
        let mut history = History::default();
        history.visibility(true);
        history.layout(true);
        history.upward();
        history
    }

    #[test]
    fn only_actual_top_and_user_demand_admit_one_page() {
        let mut history = at_top();
        history.layout(false);
        assert_eq!(history.admit(Some(100), RemoteState::Idle, false), None);
        history.layout(true);
        assert_eq!(
            history.admit(Some(100), RemoteState::Idle, false),
            Some(100)
        );
        history.submitted(Some(RequestId(7)), 10);
        history.upward();
        assert_eq!(history.admit(Some(100), RemoteState::Idle, false), None);
        history.reconcile(11, RemoteState::Loading(RequestId(7)), Some(100));
        assert_eq!(
            history.status(Some(100), RemoteState::Idle),
            Status::Loading
        );
        history.reconcile(12, RemoteState::Idle, Some(50));
        history.layout(false); // Prepended rows moved the real boundary offscreen.
        assert_eq!(history.admit(Some(50), RemoteState::Idle, false), None);
        history.layout(true);
        assert_eq!(history.admit(Some(50), RemoteState::Idle, false), Some(50));
    }

    #[test]
    fn failure_or_nonadvancing_cursor_requires_deliberate_retry() {
        let mut history = at_top();
        assert_eq!(
            history.admit(Some(100), RemoteState::Idle, false),
            Some(100)
        );
        history.submitted(Some(RequestId(7)), 10);
        history.reconcile(11, RemoteState::Idle, Some(100));
        assert_eq!(history.status(Some(100), RemoteState::Idle), Status::Retry);
        history.upward();
        assert_eq!(history.admit(Some(100), RemoteState::Idle, false), None);
        assert_eq!(history.admit(Some(100), RemoteState::Idle, true), Some(100));
        history.submitted(None, 11);
        assert_eq!(history.admit(Some(100), RemoteState::Idle, false), None);
        history.visibility(false);
        assert_eq!(history.admit(Some(100), RemoteState::Idle, true), None);
        history = History::default(); // A different room inherits no demand or attempt.
        history.visibility(true);
        history.layout(true);
        assert_eq!(history.admit(Some(100), RemoteState::Idle, false), None);
        assert_eq!(history.status(None, RemoteState::Idle), Status::End);
    }
}
