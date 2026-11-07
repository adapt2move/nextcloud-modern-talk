//! Bounded, account-scoped quick-switcher projection and directory debounce.
use super::{ButtonStyle, Desktop, LocalAction, Overlay, SearchKind, controls::BannerRole};
use gpui::{Context, Div, FontWeight, Task, div, prelude::*};
use std::time::Duration;
use talk_core::ui::{
    conversation::AccountPersonRef,
    presentation::{ConversationKind, ConversationRef},
};

const MAX_RESULTS: usize = 20;
const DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Target {
    Room(ConversationRef),
    Person(AccountPersonRef),
}

#[derive(Default)]
pub(super) struct Switcher {
    query: String,
    selected: Option<Target>,
    debounce: Option<Task<()>>,
    scheduled: Option<(talk_core::ui::AccountScope, String)>,
    failures: u8,
    last_failure: Option<talk_core::ui::presentation::RequestId>,
    failure_reason: Option<talk_core::ui::UnavailableReason>,
}
impl Switcher {
    fn record_failure(
        &mut self,
        request: talk_core::ui::presentation::RequestId,
        reason: talk_core::ui::UnavailableReason,
    ) -> bool {
        if self.last_failure == Some(request) {
            return false;
        }
        self.last_failure = Some(request);
        self.failure_reason = Some(reason);
        if retryable(reason) && self.failures < 2 {
            self.failures += 1;
            true
        } else {
            false
        }
    }
}

// Match subsequences, favoring contiguous prefixes while retaining source order
// for ties. The source is the bounded, recency-ordered conversation snapshot.
fn score(name: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let name = name.to_lowercase();
    if name.starts_with(query) {
        return Some(0);
    }
    if let Some(index) = name.find(query) {
        return Some(1 + index);
    }
    let mut chars = name.chars();
    let mut skipped = 0;
    for wanted in query.chars() {
        loop {
            match chars.next() {
                Some(c) if c == wanted => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
    }
    Some(256 + skipped)
}

fn room_score(title: &str, names: &[talk_core::ui::Text<256>], query: &str) -> Option<usize> {
    score(title, query)
        .into_iter()
        .chain(
            names
                .iter()
                .filter_map(|name| score(name.as_str(), query).map(|value| value + 8)),
        )
        .min()
}

fn retryable(reason: talk_core::ui::UnavailableReason) -> bool {
    use talk_core::ui::{UnavailableReason, presentation::ConnectionFailure};
    matches!(
        reason,
        UnavailableReason::Busy
            | UnavailableReason::Connection(
                ConnectionFailure::Unreachable
                    | ConnectionFailure::TimedOut
                    | ConnectionFailure::ServerUnavailable
                    | ConnectionFailure::RateLimited
            )
    )
}

fn directory_failure_message(reason: talk_core::ui::UnavailableReason) -> Option<&'static str> {
    use talk_core::ui::{UnavailableReason, presentation::ConnectionFailure};
    match reason {
        UnavailableReason::Unsupported => {
            Some("Personensuche wird auf diesem Server nicht unterstützt.")
        }
        UnavailableReason::PermissionDenied
        | UnavailableReason::Connection(ConnectionFailure::AccessDenied) => {
            Some("Du darfst das Personenverzeichnis nicht durchsuchen.")
        }
        UnavailableReason::ReauthenticationRequired => {
            Some("Verbinde dein Konto erneut, um Personen zu suchen.")
        }
        _ => None,
    }
}

impl Desktop {
    pub(super) fn switcher_input_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).content.trim().to_lowercase();
        if self.switcher.query != query {
            self.switcher.query = query;
            self.switcher.selected = None;
            self.switcher.debounce = None;
            self.switcher.scheduled = None;
            self.switcher.failures = 0;
            self.switcher.last_failure = None;
            self.switcher.failure_reason = None;
            self.schedule_switcher_directory(cx);
        }
        cx.notify();
    }

    pub(super) fn schedule_switcher_directory(&mut self, cx: &mut Context<Self>) {
        let query = self.switcher.query.clone();
        let Some(account) = self.snapshot.active_account else {
            return;
        };
        if self.overlay != Some(Overlay::Search)
            || query.is_empty()
            || !self.active_account_connected()
            || !matches!(self.search_kind, SearchKind::All | SearchKind::People)
            || self.directory_query.as_ref() == Some(&(account, query.clone()))
            || self.switcher.scheduled.as_ref() == Some(&(account, query.clone()))
        {
            return;
        }
        self.switcher.scheduled = Some((account, query.clone()));
        let delay = if self.switcher.failures == 0 {
            DEBOUNCE
        } else {
            Duration::from_secs(1 << (self.switcher.failures - 1))
        };
        self.switcher.debounce = Some(cx.spawn(async move |desktop, cx| {
            cx.background_executor().timer(delay).await;
            let _ = desktop.update(cx, |desktop, cx| {
                desktop.switcher.scheduled = None;
                if desktop.overlay == Some(Overlay::Search)
                    && desktop.snapshot.active_account == Some(account)
                    && desktop.switcher.query == query
                    && matches!(desktop.search_kind, SearchKind::All | SearchKind::People)
                {
                    desktop.find_search_people(cx);
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn switcher_directory_failed(
        &mut self,
        request: talk_core::ui::presentation::RequestId,
        reason: talk_core::ui::UnavailableReason,
    ) {
        if self.overlay == Some(Overlay::Search)
            && self.directory_query.as_ref().is_some_and(|(account, q)| {
                Some(*account) == self.snapshot.active_account && q == &self.switcher.query
            })
            && self.switcher.record_failure(request, reason)
        {
            self.directory_query = None;
        }
    }

    fn switcher_targets(&self) -> Vec<Target> {
        let query = &self.switcher.query;
        let mut targets = Vec::new();
        if matches!(
            self.search_kind,
            SearchKind::All | SearchKind::Conversations
        ) && super::collection::CollectionState::new(
            self.snapshot.conversation_list.content,
            !self.snapshot.conversations.as_slice().is_empty(),
        )
        .shows_content()
        {
            let mut rooms: Vec<_> = self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .enumerate()
                .filter(|(_, r)| Some(r.context.account) == self.snapshot.active_account)
                .filter_map(|(index, r)| {
                    room_score(r.title.as_str(), r.participant_names.as_slice(), query)
                        .map(|score| (score, std::cmp::Reverse(r.last_activity), index, r.context))
                })
                .collect();
            rooms.sort_by_key(|(score, recent, index, _)| (*score, *recent, *index));
            let room_limit = if !query.is_empty() && self.search_kind == SearchKind::All {
                12
            } else {
                MAX_RESULTS
            };
            targets.extend(
                rooms
                    .into_iter()
                    .take(room_limit)
                    .map(|(_, _, _, id)| Target::Room(id)),
            );
        }
        if !query.is_empty()
            && self.active_account_connected()
            && matches!(self.search_kind, SearchKind::All | SearchKind::People)
            && self.directory_query.as_ref().is_some_and(|(account, q)| {
                Some(*account) == self.snapshot.active_account && q == query
            })
            && let Some(panel) = self
                .snapshot
                .create_panel
                .as_ref()
                .filter(|p| Some(p.account) == self.snapshot.active_account)
            && self
                .room_ui
                .create_people_request
                .as_ref()
                .is_some_and(|request| request.ready(panel))
        {
            targets.extend(
                panel
                    .people
                    .as_slice()
                    .iter()
                    .take(MAX_RESULTS.saturating_sub(targets.len()))
                    .map(|p| Target::Person(p.id)),
            );
        }
        targets
    }

    pub(super) fn switcher_move(&mut self, delta: i8) {
        let targets = self.switcher_targets();
        if targets.is_empty() {
            self.switcher.selected = None;
            return;
        }
        let current = self
            .switcher
            .selected
            .and_then(|target| targets.iter().position(|item| *item == target))
            .unwrap_or(0);
        let index = if delta < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(targets.len() - 1)
        };
        self.switcher.selected = targets.get(index).copied();
    }

    pub(super) fn switcher_action(&self) -> Option<LocalAction> {
        let targets = self.switcher_targets();
        let target = match self.switcher.selected {
            Some(target) => targets.contains(&target).then_some(target),
            None => targets.first().copied(),
        }?;
        match target {
            Target::Room(id) => Some(LocalAction::Navigate(id)),
            Target::Person(id) => Some(LocalAction::CreateDirect(id)),
        }
    }

    pub(super) fn switcher_request_admitted(&mut self) {
        self.switcher.failure_reason = None;
    }

    pub(super) fn switcher_results(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_1();
        let targets = self.switcher_targets();
        let selected = match self.switcher.selected {
            Some(target) => targets.iter().position(|item| *item == target),
            None => (!targets.is_empty()).then_some(0),
        };
        for (index, target) in targets.iter().enumerate() {
            match target {
                Target::Room(id) => {
                    if let Some(room) = self
                        .snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .find(|r| r.context == *id)
                    {
                        let kind = match room.kind {
                            ConversationKind::Direct => "Direktnachricht",
                            ConversationKind::NoteToSelf => "Notiz an mich",
                            ConversationKind::Group => "Gruppengespräch",
                            ConversationKind::GuestAccessible => "Öffentliches Gespräch",
                            _ => "Gespräch",
                        };
                        body = body.child(
                            self.style_list_row(
                                self.button(
                                    super::controls::scoped_id("switcher-room", *id, 0, 0),
                                    "",
                                    ButtonStyle::Quiet,
                                    true,
                                    LocalAction::Navigate(*id),
                                    cx,
                                ),
                                super::controls::RowSize::Detailed,
                            )
                            .w_full()
                            .justify_start()
                            .gap_3()
                            .px_3()
                            .py_2()
                            .when(Some(index) == selected, |d| d.bg(self.theme.selected))
                            .child(self.content_avatar(
                                talk_core::ui::content::AssetTarget::ConversationAvatar(*id),
                                room.initials.as_str(),
                                32.,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(room.title.as_str().to_owned()),
                            )
                            .child(self.hint(self.l(kind)))
                            .when(room.unread_mention == Some(true), |d| {
                                d.child(self.count_badge("@"))
                            })
                            .when(
                                room.unread > 0 && room.unread_mention != Some(true),
                                |d| {
                                    d.child(self.count_badge(if room.unread > 99 {
                                        "99+".to_owned()
                                    } else {
                                        room.unread.to_string()
                                    }))
                                },
                            ),
                        );
                    }
                }
                Target::Person(id) => {
                    if let Some(person) = self
                        .snapshot
                        .create_panel
                        .as_ref()
                        .and_then(|p| p.people.as_slice().iter().find(|p| p.id == *id))
                    {
                        body = body.child(
                            self.button(
                                gpui::SharedString::from(format!("switcher-person-{}", id.local)),
                                "",
                                ButtonStyle::Quiet,
                                true,
                                LocalAction::CreateDirect(*id),
                                cx,
                            )
                            .w_full()
                            .justify_start()
                            .gap_3()
                            .px_3()
                            .py_2()
                            .when(Some(index) == selected, |d| d.bg(self.theme.selected))
                            .child(self.content_avatar(
                                talk_core::ui::content::AssetTarget::PersonAvatar(*id),
                                "?",
                                32.,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .child(person.name.as_str().to_owned()),
                            )
                            .child(self.hint(self.l("Person"))),
                        );
                    }
                }
            }
        }
        if self.active_account_connected()
            && self
                .directory_query
                .as_ref()
                .is_some_and(|(account, query)| {
                    Some(*account) == self.snapshot.active_account && query == &self.switcher.query
                })
            && let Some(reason) = self.switcher.failure_reason
            && let Some(message) = directory_failure_message(reason)
        {
            body = body.child(self.banner(BannerRole::Error, self.l(message)));
        }
        if !self.active_account_connected() && !self.switcher.query.is_empty() {
            body = body.child(self.banner(
                BannerRole::Warning,
                self.l("Personensuche benötigt eine Verbindung."),
            ));
        }
        if targets.is_empty()
            && (self.search_kind == SearchKind::Conversations
                || self.switcher.failure_reason.is_none())
        {
            body = body.child(self.empty_row(self.l("Keine passenden Gespräche oder Personen.")));
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::{Switcher, directory_failure_message, room_score, score};
    use talk_core::ui::{
        UnavailableReason,
        presentation::{ConnectionFailure, RequestId},
    };
    #[test]
    fn rejected_directory_admission_retries_twice_and_deduplicates_completions() {
        let mut state = Switcher::default();
        assert!(state.record_failure(RequestId(1), UnavailableReason::Busy));
        assert!(!state.record_failure(RequestId(1), UnavailableReason::Busy));
        assert!(state.record_failure(RequestId(2), UnavailableReason::Busy));
        assert!(!state.record_failure(RequestId(3), UnavailableReason::Busy));
        assert_eq!(state.failures, 2);
        let mut unsupported = Switcher::default();
        assert!(!unsupported.record_failure(RequestId(1), UnavailableReason::Unsupported));
        assert_eq!(unsupported.failures, 0);
        assert!(directory_failure_message(UnavailableReason::Unsupported).is_some());
        assert_eq!(directory_failure_message(UnavailableReason::Busy), None);
        assert_eq!(
            directory_failure_message(UnavailableReason::Connection(ConnectionFailure::TimedOut)),
            None
        );
    }
    #[test]
    fn fuzzy_matching_prefers_prefix_and_preserves_unicode() {
        assert!(score("Navigation Team", "nav") < score("Team Navigation", "nav"));
        assert!(score("Málaga", "mlg").is_some());
        assert_eq!(score("Alpha", "xyz"), None);
        assert_eq!(score("Alpha", ""), Some(0));
    }
    #[test]
    fn group_matches_observed_participant_names_without_a_title_match() {
        let names = [talk_core::ui::Text::new("Mara Fischer").unwrap()];
        assert_eq!(score("Produktteam", "fischer"), None);
        assert!(room_score("Produktteam", &names, "fischer").is_some());
        assert_eq!(room_score("Produktteam", &names, "unknown person"), None);
    }
}
