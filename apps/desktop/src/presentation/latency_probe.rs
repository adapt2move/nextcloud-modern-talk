//! Typed native UI entry points used only by the explicit developer flag.
use super::*;
use talk_core::ui::Text;
use talk_core::ui::messaging::{ComposeTarget, MessageAction};
pub(super) struct PerfRun {
    mode: &'static str,
    remaining: u16,
    last: std::time::Instant,
    started: std::time::Instant,
}
impl Desktop {
    pub(super) fn observe_perf_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.perf_probe.is_none() {
            return;
        }
        let started = std::time::Instant::now();
        let view = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |view, cx| {
                let Some(run) = view.perf_probe.as_mut() else {
                    return;
                };
                println!(
                    "DEV_PERF {} {} {} {} {}",
                    run.mode,
                    run.last.elapsed().as_micros(),
                    started.elapsed().as_micros(),
                    view.snapshot.messages.as_slice().len(),
                    view.snapshot.conversations.as_slice().len()
                );
                run.remaining -= 1;
                run.last = std::time::Instant::now();
                let mode = run.mode;
                let remaining = run.remaining;
                if remaining == 0 || run.started.elapsed() >= std::time::Duration::from_secs(60) {
                    view.perf_probe = None;
                    println!("DEV_PERF_DONE {mode}");
                    return;
                }
                match mode {
                    "scroll" => view.message_ui.list.scroll_by(px(
                        if view.perf_probe.as_ref().is_some_and(|r| {
                            r.started.elapsed() < std::time::Duration::from_secs(30)
                        }) {
                            -24.
                        } else {
                            24.
                        },
                    )),
                    "switcher" => view.handle(
                        if remaining % 2 == 0 {
                            LocalAction::Open(Overlay::Search)
                        } else {
                            LocalAction::Close
                        },
                        window,
                        cx,
                    ),
                    _ => (),
                }
                cx.notify();
            });
        });
    }

    pub(crate) fn enable_latency_probe(&mut self) {
        #[cfg(debug_assertions)]
        {
            self.latency_probe = true;
            self.port.enable_latency_probe();
        }
    }
    pub(crate) fn latency_probe_action(
        &mut self,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.enable_latency_probe();
        if action == "diagnose" {
            println!(
                "DEV_HEALTH {:?} {:?} {:?} {:?}",
                self.snapshot
                    .accounts
                    .as_slice()
                    .first()
                    .map(|a| (a.connection, a.connection_failure)),
                self.snapshot
                    .saved_accounts
                    .as_slice()
                    .first()
                    .map(|a| a.phase),
                self.snapshot.conversation_list.content,
                self.snapshot.login.phase,
            );
        }
        if matches!(action, "perf-scroll" | "perf-switcher" | "perf-sidebar") {
            self.perf_probe = Some(PerfRun {
                mode: match action {
                    "perf-scroll" => "scroll",
                    "perf-switcher" => "switcher",
                    _ => "sidebar",
                },
                remaining: 18_000,
                last: std::time::Instant::now(),
                started: std::time::Instant::now(),
            });
            cx.notify();
        }
        if action == "status" {
            let canonical: Vec<_> = self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .filter(|m| m.local_id <= u128::from(u64::MAX))
                .collect();
            let unique = canonical
                .iter()
                .map(|m| m.local_id)
                .collect::<std::collections::HashSet<_>>()
                .len();
            let sentinels = canonical
                .iter()
                .filter(|m| m.body.as_str().starts_with("Native recovery sentinel"))
                .count();
            let outgoing = self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .filter(|m| m.own && m.local_id > u128::from(u64::MAX))
                .count();
            println!(
                "DEV_STATUS {} {} {} {} {} {}",
                self.offline_notice_visible(),
                self.snapshot.pending_intents,
                canonical.len(),
                unique,
                sentinels,
                outgoing
            );
        }
        let selected = self.snapshot.selected;
        let intent = match action {
            "open0" | "open1" => self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .filter(|r| r.title.as_str().starts_with("A native latency"))
                .nth(usize::from(action == "open1"))
                .map(|r| UiIntent::OpenConversation(r.context)),
            "send" => selected.map(|context| UiIntent::Message {
                request: RequestId(10_001),
                context,
                action: MessageAction::Send {
                    body: Text::new("Native latency fixture message")
                        .expect("static bounded fixture"),
                    target: ComposeTarget::default(),
                },
            }),
            "react" => selected
                .zip(
                    self.snapshot
                        .messages
                        .as_slice()
                        .iter()
                        .find(|m| !m.own && !m.metadata.system),
                )
                .map(|(context, message)| UiIntent::Message {
                    request: RequestId(10_002),
                    context,
                    action: MessageAction::React {
                        message: message.local_id,
                        emoji: Text::new("👍").expect("static emoji"),
                        add: !message
                            .metadata
                            .reactions
                            .as_slice()
                            .iter()
                            .any(|r| r.emoji.as_str() == "👍" && r.own),
                    },
                }),
            "mark-read" => selected.and_then(|context| {
                self.snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .filter(|m| m.local_id <= u128::from(u64::MAX))
                    .map(|m| m.local_id)
                    .max()
                    .map(|message| UiIntent::Message {
                        request: RequestId(10_005),
                        context,
                        action: MessageAction::MarkRead(message),
                    })
            }),
            "read" => selected.map(|context| UiIntent::Message {
                request: RequestId(10_003),
                context,
                action: MessageAction::MarkUnread,
            }),
            "favorite" => selected.and_then(|context| {
                self.snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .find(|r| r.context == context)
                    .map(|r| UiIntent::SetFavorite {
                        request: RequestId(10_004),
                        context,
                        favorite: r.favorite != Some(true),
                    })
            }),
            _ => None,
        };
        if let Some(intent) = intent {
            match intent {
                UiIntent::Message {
                    context,
                    action: MessageAction::Send { body, .. },
                    ..
                } => {
                    self.composer.update(cx, |input, cx| {
                        input.set_value(body.as_str());
                        cx.notify();
                    });
                    let before = self.drafts.probe_request_sequence();
                    self.handle(LocalAction::Send, window, cx);
                    let after = self.drafts.probe_request_sequence();
                    println!(
                        "DEV_INPUT {action} {}",
                        if after > before { after } else { 0 }
                    );
                    let _ = context;
                }
                UiIntent::Message {
                    action: message, ..
                } if !matches!(message, MessageAction::Send { .. }) => {
                    let request = self.submit_message(message);
                    println!("DEV_INPUT {action} {}", request.map_or(0, |r| r.0));
                    if action == "mark-read"
                        && request.is_some()
                        && let Some(context) = selected
                    {
                        self.release_unread_hold(context);
                    }
                }
                intent => {
                    let request = match &intent {
                        UiIntent::Message { request, .. }
                        | UiIntent::SetFavorite { request, .. } => request.0,
                        _ => 0,
                    };
                    self.handle(LocalAction::Intent(intent), window, cx);
                    println!("DEV_INPUT {action} {request}");
                }
            }
        }
        self.refresh(cx);
        cx.notify();
        format!(
            "{} {} {}",
            self.snapshot.conversations.as_slice().len(),
            self.snapshot.messages.as_slice().len(),
            if self.snapshot.content == ContentState::Ready {
                "ready"
            } else {
                "pending"
            }
        )
    }
}
