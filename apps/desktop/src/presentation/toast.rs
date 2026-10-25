//! One transient action failure above the chat, without changing its layout.
use super::{
    ButtonStyle, Desktop, LocalAction, Overlay,
    controls::{Availability, scoped_id},
};
use gpui::{Context, Div, Focusable, KeyDownEvent, Window, div, prelude::*, px};
use std::time::{Duration, Instant};
use talk_core::ui::{
    AccountScope, UnavailableReason, messaging::MessageActionError, presentation::ConversationRef,
};

const LIFETIME: Duration = Duration::from_secs(8);
const ACCOUNTS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Key {
    context: ConversationRef,
    id: u64,
}
#[derive(Clone, Copy)]
struct Toast {
    key: Key,
    reason: MessageActionError,
    expires: Instant,
}

#[derive(Default)]
pub(super) struct Toasts {
    // Error IDs increase across all rooms of one account/runtime. This bounded
    // watermark prevents a projection replay from showing a dismissed failure.
    seen: Vec<(AccountScope, u64)>,
    current: Option<Toast>,
}
impl Toasts {
    fn reconcile(
        &mut self,
        selected: Option<ConversationRef>,
        incoming: Option<(Key, MessageActionError)>,
        admitted: &[AccountScope],
        now: Instant,
    ) -> bool {
        let before = self.current.map(|toast| toast.key);
        self.seen.retain(|(scope, _)| admitted.contains(scope));
        if self.current.is_some_and(|toast| {
            selected != Some(toast.key.context)
                || !admitted.contains(&toast.key.context.account)
                || now >= toast.expires
        }) {
            self.current = None;
        }
        if let Some((key, reason)) = incoming.filter(|(key, _)| {
            selected == Some(key.context) && admitted.contains(&key.context.account)
        }) {
            let seen = self
                .seen
                .iter_mut()
                .find(|(scope, _)| *scope == key.context.account);
            let fresh = if let Some((_, id)) = seen {
                if key.id > *id {
                    *id = key.id;
                    true
                } else {
                    false
                }
            } else if self.seen.len() < ACCOUNTS {
                self.seen.push((key.context.account, key.id));
                true
            } else {
                false
            };
            if fresh {
                self.current = Some(Toast {
                    key,
                    reason,
                    expires: now + LIFETIME,
                });
            }
        }
        before != self.current.map(|toast| toast.key)
    }
    fn dismiss(&mut self, key: Key) -> bool {
        if self.current.is_some_and(|toast| toast.key == key) {
            self.current = None;
            true
        } else {
            false
        }
    }
}

fn copy(error: MessageActionError) -> &'static str {
    let MessageActionError::Failed(reason) = error else {
        return "Ergebnis unklar. Vor erneutem Ändern neu laden.";
    };
    match reason {
        UnavailableReason::Connection(failure) => super::state::connection_failure_copy(failure).1,
        UnavailableReason::PermissionDenied => "Keine Berechtigung für diese Aktion.",
        UnavailableReason::ReauthenticationRequired => "Bitte erneut anmelden.",
        UnavailableReason::Unsupported | UnavailableReason::Unconfigured => {
            "Diese Aktion ist in diesem Client noch nicht verfügbar."
        }
        UnavailableReason::Busy => {
            "Die App ist gerade ausgelastet. Bitte kurz warten und erneut versuchen."
        }
        UnavailableReason::PolicyUnknown | UnavailableReason::PolicyStale => {
            "Die Änderung konnte nicht gespeichert werden. Bitte erneut versuchen."
        }
    }
}

impl Desktop {
    pub(super) fn refresh_action_toast(&mut self, cx: &mut Context<Self>) {
        let admitted: Vec<_> = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .map(|account| account.scope)
            .collect();
        let incoming = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| Some(panel.context) == self.snapshot.selected)
            .and_then(|panel| {
                panel.action_error.map(|(id, reason)| {
                    (
                        Key {
                            context: panel.context,
                            id,
                        },
                        reason,
                    )
                })
            });
        if self
            .action_toasts
            .reconcile(self.snapshot.selected, incoming, &admitted, Instant::now())
        {
            cx.notify();
        }
    }

    pub(super) fn dismiss_action_toast(
        &mut self,
        context: ConversationRef,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot.selected != Some(context) {
            return;
        }
        let key = Key { context, id };
        let focused = self
            .control_focus
            .current(&scoped_id(
                "dismiss-action-toast",
                context,
                u128::from(id),
                0,
            ))
            .is_some_and(|handle| handle.is_focused(window));
        if self.action_toasts.dismiss(key) {
            if focused {
                if self.composer_available() {
                    self.composer.focus_handle(cx).focus(window);
                } else {
                    self.navigation.history_focus().focus(window);
                }
            }
            cx.notify();
        }
    }

    pub(super) fn action_toast_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(toast) = self.action_toasts.current else {
            return false;
        };
        if event.keystroke.key != "escape" || event.keystroke.modifiers.modified() {
            return false;
        }
        if self
            .control_focus
            .current(&scoped_id(
                "dismiss-action-toast",
                toast.key.context,
                u128::from(toast.key.id),
                0,
            ))
            .is_none_or(|handle| !handle.is_focused(window))
        {
            return false;
        }
        self.dismiss_action_toast(toast.key.context, toast.key.id, window, cx);
        true
    }

    pub(super) fn action_toast_view(&self, window: &Window, cx: &Context<Self>) -> Option<Div> {
        let toast = self.action_toasts.current.filter(|toast| {
            Some(toast.key.context) == self.snapshot.selected
                && !self.update_restart_pending
                && !self.projection_blocked
                && !self.onboarding_open
                && self.prejoin.is_none()
                && !self.navigation_open
                && !self.account_removal.confirming()
                && !self.incoming_visible()
                && matches!(self.overlay, None | Some(Overlay::CallChat))
                && Instant::now() < toast.expires
        })?;
        Some(
            div()
                .absolute()
                .right_4()
                .bottom_4()
                .occlude()
                .w(self
                    .sp(360.)
                    .min((window.viewport_size().width - px(32.)).max(px(1.))))
                .rounded(px(10.))
                .border_1()
                .border_color(self.theme.border)
                .bg(self.theme.surface)
                .shadow_lg()
                .p_3()
                .flex()
                .items_start()
                .gap_2()
                .child(self.scaled_icon(
                    "info",
                    17.,
                    if toast.reason == MessageActionError::Uncertain {
                        self.theme.warning
                    } else {
                        self.theme.danger
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(self.sp(13.))
                        .line_height(self.sp(19.))
                        .text_color(self.theme.text)
                        .child(self.l(copy(toast.reason))),
                )
                .child(
                    self.icon_button(
                        scoped_id(
                            "dismiss-action-toast",
                            toast.key.context,
                            u128::from(toast.key.id),
                            0,
                        ),
                        ("x", "Meldung schließen"),
                        ButtonStyle::Quiet,
                        Availability::available(),
                        LocalAction::DismissActionToast(toast.key.context, toast.key.id),
                        cx,
                    )
                    .size(px(28.))
                    .min_h(px(28.)),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, ConversationId, HostId, RuntimeGeneration};
    #[test]
    fn uncertainty_never_instructs_the_user_to_repeat_a_possibly_applied_action() {
        assert_eq!(
            copy(MessageActionError::Uncertain),
            "Ergebnis unklar. Vor erneutem Ändern neu laden."
        );
        assert_eq!(
            copy(MessageActionError::Failed(
                UnavailableReason::PermissionDenied
            )),
            "Keine Berechtigung für diese Aktion."
        );
    }
    fn context(room: u128, runtime: u128) -> ConversationRef {
        let account = AccountId::from_local(1);
        ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(runtime),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: room,
            },
        }
    }
    #[test]
    fn replay_cannot_extend_expiry_or_redisplay_a_dismissed_error() {
        let now = Instant::now();
        let context = context(1, 1);
        let key = Key { context, id: 7 };
        let incoming = Some((
            key,
            MessageActionError::Failed(UnavailableReason::PermissionDenied),
        ));
        let mut toasts = Toasts::default();
        assert!(toasts.reconcile(Some(context), incoming, &[context.account], now));
        assert!(!toasts.reconcile(
            Some(context),
            incoming,
            &[context.account],
            now + Duration::from_secs(7)
        ));
        assert!(toasts.reconcile(Some(context), incoming, &[context.account], now + LIFETIME));
        assert!(toasts.current.is_none());
        assert!(!toasts.reconcile(
            Some(context),
            incoming,
            &[context.account],
            now + LIFETIME * 2
        ));
        let new = Key { id: 8, ..key };
        toasts.reconcile(
            Some(context),
            Some((
                new,
                MessageActionError::Failed(UnavailableReason::PermissionDenied),
            )),
            &[context.account],
            now + LIFETIME * 3,
        );
        assert!(!toasts.dismiss(key));
        assert!(toasts.current.is_some());
        assert!(toasts.dismiss(new));
        assert!(!toasts.reconcile(
            Some(context),
            Some((
                new,
                MessageActionError::Failed(UnavailableReason::PermissionDenied)
            )),
            &[context.account],
            now + LIFETIME * 4
        ));
    }
    #[test]
    fn room_and_runtime_replacement_never_inherit_an_obsolete_failure() {
        let now = Instant::now();
        let first = context(1, 1);
        let second = context(2, 1);
        let replacement = context(2, 2);
        let old = Key {
            context: first,
            id: 3,
        };
        let mut toasts = Toasts::default();
        toasts.reconcile(
            Some(first),
            Some((
                old,
                MessageActionError::Failed(UnavailableReason::PermissionDenied),
            )),
            &[first.account],
            now,
        );
        assert!(toasts.reconcile(
            Some(second),
            Some((
                old,
                MessageActionError::Failed(UnavailableReason::PermissionDenied)
            )),
            &[first.account],
            now
        ));
        assert!(toasts.current.is_none());
        assert!(!toasts.reconcile(
            Some(first),
            Some((
                old,
                MessageActionError::Failed(UnavailableReason::PermissionDenied)
            )),
            &[first.account],
            now
        ));
        let fresh = Key {
            context: replacement,
            id: 1,
        };
        assert!(toasts.reconcile(
            Some(replacement),
            Some((
                fresh,
                MessageActionError::Failed(UnavailableReason::PermissionDenied)
            )),
            &[replacement.account],
            now
        ));
        assert_eq!(toasts.current.map(|toast| toast.key), Some(fresh));
        assert_eq!(toasts.seen.len(), 1);
        toasts.reconcile(None, None, &[], now);
        assert!(toasts.current.is_none() && toasts.seen.is_empty());
    }
}
