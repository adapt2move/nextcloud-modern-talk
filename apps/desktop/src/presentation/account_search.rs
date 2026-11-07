//! Account-wide search form and bounded grouped results; all I/O is an intent.
use super::{ButtonStyle, Desktop, LocalAction, input::TextInput};
use gpui::{Context, Div, Entity, FontWeight, HighlightStyle, StyledText, div, prelude::*};
use talk_core::ui::{
    Text,
    conversation::{AccountPersonRef, RemoteState},
    presentation::{ConversationRef, RequestId, UiIntent},
    search::{Action, Filters, Hit},
};

pub(super) struct AccountSearchUi {
    pub person: Entity<TextInput>,
    pub since: Entity<TextInput>,
    pub until: Entity<TextInput>,
    room: Option<ConversationRef>,
    room_picker: bool,
    person_query: Option<String>,
    selected_person: Option<(AccountPersonRef, String)>,
    from_self: bool,
    selected: Option<(ConversationRef, u128)>,
    request: u64,
    pub jump: Option<(
        ConversationRef,
        u128,
        Option<ConversationRef>,
        std::time::Instant,
    )>,
    invalid_date: bool,
}
impl AccountSearchUi {
    pub fn new(theme: super::theme::Theme, cx: &mut Context<Desktop>) -> Self {
        Self {
            person: cx.new(|cx| TextInput::new("Person suchen …", 256, theme, cx)),
            since: cx.new(|cx| TextInput::new("JJJJ-MM-TT", 10, theme, cx)),
            until: cx.new(|cx| TextInput::new("JJJJ-MM-TT", 10, theme, cx)),
            room: None,
            room_picker: false,
            person_query: None,
            selected_person: None,
            from_self: false,
            selected: None,
            request: 0,
            jump: None,
            invalid_date: false,
        }
    }
    pub fn inputs(&self) -> [Entity<TextInput>; 3] {
        [self.person.clone(), self.since.clone(), self.until.clone()]
    }
    pub fn reset(&mut self, cx: &mut Context<Desktop>) {
        self.room = None;
        self.room_picker = false;
        self.person_query = None;
        self.selected_person = None;
        self.from_self = false;
        self.selected = None;
        self.jump = None;
        self.invalid_date = false;
        for input in self.inputs() {
            input.update(cx, |input, _| input.reset());
        }
    }
}
// Calendar dates are explicit UTC boundaries. Reject invalid dates rather than
// asking PHP's permissive date parser to silently reinterpret user input.
fn date(value: &str, end: bool) -> Result<Option<Text<32>>, ()> {
    if value.is_empty() {
        return Ok(None);
    }
    let b = value.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return Err(());
    }
    let year: u32 = value[0..4].parse().map_err(|_| ())?;
    let month: u32 = value[5..7].parse().map_err(|_| ())?;
    let day: u32 = value[8..10].parse().map_err(|_| ())?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0 || day == 0 || day > max {
        return Err(());
    }
    Text::new(&format!(
        "{value}T{}Z",
        if end { "23:59:59" } else { "00:00:00" }
    ))
    .map(Some)
    .map_err(|_| ())
}
fn highlight(text: &str, query: &str, color: gpui::Hsla) -> StyledText {
    let lower = text.to_lowercase();
    let term = query.to_lowercase();
    let range = if term.is_empty() || lower.len() != text.len() {
        None
    } else {
        lower.find(&term).and_then(|start| {
            text.get(start..start + term.len())
                .map(|_| start..start + term.len())
        })
    };
    StyledText::new(text.to_owned()).with_highlights(range.into_iter().map(|range| {
        (
            range,
            HighlightStyle {
                color: Some(color),
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
        )
    }))
}
impl Desktop {
    fn account_search_filters(&self, cx: &Context<Self>) -> Result<Filters, bool> {
        let start = self.account_search_ui.since.read(cx).content.trim();
        let end = self.account_search_ui.until.read(cx).content.trim();
        let since = date(start, false).map_err(|_| true)?;
        let until = date(end, true).map_err(|_| true)?;
        if !start.is_empty() && !end.is_empty() && start > end {
            return Err(true);
        }
        let current_person = self.account_search_ui.person.read(cx).content.trim();
        let person = self
            .account_search_ui
            .selected_person
            .as_ref()
            .filter(|(_, name)| name == current_person)
            .map(|(id, _)| *id);
        if !current_person.is_empty() && person.is_none() {
            return Err(false);
        }
        if let Some(id) = person
            && !self.snapshot.create_panel.as_ref().is_some_and(|panel| {
                Some(panel.account) == self.snapshot.active_account
                    && self
                        .room_ui
                        .create_people_request
                        .as_ref()
                        .is_some_and(|request| request.ready(panel))
                    && panel.people.as_slice().iter().any(|person| person.id == id)
            })
        {
            return Err(false);
        }
        Ok(Filters {
            conversation: self.account_search_ui.room,
            person,
            from_self: self.account_search_ui.from_self,
            since,
            until,
        })
    }
    pub(super) fn start_account_search(&mut self, cx: &Context<Self>) {
        let Some(account) = self.snapshot.active_account else {
            return;
        };
        let query = self.search.read(cx).content.trim();
        let Ok(term) = Text::new(query) else {
            return;
        };
        if query.is_empty() {
            return;
        }
        let filters = match self.account_search_filters(cx) {
            Ok(filters) => filters,
            Err(is_date) => {
                self.account_search_ui.invalid_date = is_date;
                return;
            }
        };
        let Some(next) = self.account_search_ui.request.checked_add(1) else {
            return;
        };
        self.account_search_ui.request = next;
        self.account_search_ui.invalid_date = false;
        self.account_search_ui.selected = None;
        self.submit(UiIntent::AccountSearch {
            account,
            request: RequestId(next),
            action: Action::Search { term, filters },
        });
    }
    pub(super) fn next_account_search(&mut self, cx: &Context<Self>) {
        if !self.account_search_current(cx) {
            return;
        }
        let Some(account) = self.snapshot.active_account else {
            return;
        };
        let Some(next) = self.account_search_ui.request.checked_add(1) else {
            return;
        };
        self.account_search_ui.request = next;
        self.submit(UiIntent::AccountSearch {
            account,
            request: RequestId(next),
            action: Action::More,
        });
    }
    pub(super) fn find_account_search_person(&mut self, cx: &Context<Self>) {
        let query = self.account_search_ui.person.read(cx).content.trim();
        let Ok(query) = Text::new(query) else {
            return;
        };
        if query.as_str().is_empty() {
            return;
        }
        self.account_search_ui.selected_person = None;
        self.account_search_ui.from_self = false;
        self.account_search_ui.person_query = Some(query.as_str().to_lowercase());
        self.submit_creation(talk_core::ui::conversation::CreateAction::FindPeople(query));
    }
    pub(super) fn select_account_search_person(
        &mut self,
        id: AccountPersonRef,
        cx: &mut Context<Self>,
    ) {
        let Some(person) = self
            .snapshot
            .create_panel
            .as_ref()
            .filter(|p| {
                Some(p.account) == self.snapshot.active_account
                    && self
                        .room_ui
                        .create_people_request
                        .as_ref()
                        .is_some_and(|r| r.ready(p))
            })
            .and_then(|p| p.people.as_slice().iter().find(|p| p.id == id))
        else {
            return;
        };
        let name = person.name.as_str().to_owned();
        self.account_search_ui.person.update(cx, |input, _| {
            input.set_value(&name);
        });
        self.account_search_ui.selected_person = Some((id, name));
        self.account_search_ui.from_self = false;
        self.account_search_ui.person_query = None;
    }
    pub(super) fn account_search_toggle_self(&mut self, cx: &mut Context<Self>) {
        self.account_search_ui.from_self = !self.account_search_ui.from_self;
        self.account_search_ui.selected_person = None;
        self.account_search_ui.person_query = None;
        self.account_search_ui
            .person
            .update(cx, |input, _| input.reset());
    }
    pub(super) fn account_search_room_picker(&mut self) {
        self.account_search_ui.room_picker = !self.account_search_ui.room_picker;
    }
    pub(super) fn select_account_search_room(&mut self, room: Option<ConversationRef>) {
        if room.is_none_or(|context| {
            Some(context.account) == self.snapshot.active_account
                && self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .any(|r| r.context == context)
        }) {
            self.account_search_ui.room = room;
            self.account_search_ui.room_picker = false;
        }
    }
    fn account_search_hits(&self) -> Vec<&Hit> {
        let Some(panel) = self
            .snapshot
            .search_panel
            .as_ref()
            .filter(|p| Some(p.account) == self.snapshot.active_account)
        else {
            return Vec::new();
        };
        self.snapshot
            .conversations
            .as_slice()
            .iter()
            .flat_map(|room| {
                panel
                    .hits
                    .as_slice()
                    .iter()
                    .filter(move |hit| hit.context == room.context)
            })
            .collect()
    }
    pub(super) fn account_search_move(&mut self, direction: i8) {
        let hits = self.account_search_hits();
        let selected = self
            .account_search_ui
            .selected
            .and_then(|(context, id)| {
                hits.iter()
                    .position(|hit| hit.context == context && hit.message == id)
            })
            .unwrap_or(0);
        let next = if direction < 0 {
            selected.saturating_sub(1)
        } else {
            (selected + 1).min(hits.len().saturating_sub(1))
        };
        let target = hits.get(next).map(|hit| (hit.context, hit.message));
        self.account_search_ui.selected = target;
    }
    pub(super) fn account_search_current(&self, cx: &Context<Self>) -> bool {
        self.snapshot.search_panel.as_ref().is_some_and(|panel| {
            panel.loaded
                && !panel.state.pending()
                && Some(panel.account) == self.snapshot.active_account
                && panel.term.as_str() == self.search.read(cx).content.trim()
                && self
                    .account_search_filters(cx)
                    .is_ok_and(|filters| filters == panel.filters)
        })
    }
    pub(super) fn refresh_account_search_jump(&mut self) {
        let Some((context, message, origin, started)) = self.account_search_ui.jump else {
            return;
        };
        if self.snapshot.active_account != Some(context.account)
            || started.elapsed() > std::time::Duration::from_secs(15)
        {
            self.account_search_ui.jump = None;
            return;
        }
        if self.snapshot.selected != Some(context) {
            if self.snapshot.selected != origin {
                self.account_search_ui.jump = None;
            }
            return;
        }
        if self.snapshot.content == talk_core::ui::presentation::ContentState::Denied {
            self.account_search_ui.jump = None;
            return;
        }
        if self.snapshot.message_panel.as_ref().is_some_and(|panel| {
            panel.context == context
                && !panel.history.pending()
                && self.snapshot.content == talk_core::ui::presentation::ContentState::Ready
        }) && self
            .submit_message(talk_core::ui::messaging::MessageAction::Context(message))
            .is_some()
        {
            self.account_search_ui.jump = None;
        }
    }
    pub(super) fn account_search_selected(&self) -> Option<(ConversationRef, u128)> {
        let hits = self.account_search_hits();
        match self.account_search_ui.selected {
            Some((context, id)) => hits
                .iter()
                .find(|hit| hit.context == context && hit.message == id)
                .map(|hit| (hit.context, hit.message)),
            None => hits.first().map(|hit| (hit.context, hit.message)),
        }
    }
    pub(super) fn account_search_body(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_2();
        let room_label = self
            .account_search_ui
            .room
            .and_then(|c| {
                self.snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .find(|r| r.context == c)
            })
            .map_or(self.l("Alle Gespräche"), |r| r.title.as_str());
        body = body.child(
            div()
                .flex()
                .gap_2()
                .child(
                    self.button(
                        "account-search-room",
                        "",
                        ButtonStyle::Secondary,
                        true,
                        LocalAction::AccountSearchRooms,
                        cx,
                    )
                    .child(room_label.to_owned()),
                )
                .child(
                    self.input_surface(&self.account_search_ui.since, cx)
                        .flex_1()
                        .min_w_0()
                        .child(self.account_search_ui.since.clone()),
                )
                .child(
                    self.input_surface(&self.account_search_ui.until, cx)
                        .flex_1()
                        .min_w_0()
                        .child(self.account_search_ui.until.clone()),
                ),
        );
        body = body.child(self.hint(self.l("Zeitraum: von / bis, jeweils JJJJ-MM-TT (UTC).")));
        if self.account_search_ui.room_picker {
            body = body.child(self.button(
                "account-search-room-all",
                "Alle Gespräche",
                ButtonStyle::Quiet,
                true,
                LocalAction::AccountSearchRoom(None),
                cx,
            ));
            for room in self.snapshot.conversations.as_slice() {
                body = body.child(
                    self.button(
                        super::controls::scoped_id(
                            "account-search-room-choice",
                            room.context,
                            0,
                            0,
                        ),
                        "",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::AccountSearchRoom(Some(room.context)),
                        cx,
                    )
                    .child(room.title.as_str().to_owned()),
                );
            }
        }
        body = body.child(
            div()
                .flex()
                .gap_2()
                .child(
                    self.input_surface(&self.account_search_ui.person, cx)
                        .flex_1()
                        .min_w_0()
                        .child(self.account_search_ui.person.clone()),
                )
                .child(self.button(
                    "account-search-self",
                    "Von mir",
                    if self.account_search_ui.from_self {
                        ButtonStyle::Primary
                    } else {
                        ButtonStyle::Secondary
                    },
                    true,
                    LocalAction::AccountSearchSelf,
                    cx,
                ))
                .child(
                    self.button(
                        "account-search-person-find",
                        "Person suchen",
                        ButtonStyle::Secondary,
                        self.active_account_connected()
                            && !self
                                .account_search_ui
                                .person
                                .read(cx)
                                .content
                                .trim()
                                .is_empty()
                            && self
                                .snapshot
                                .create_panel
                                .as_ref()
                                .is_none_or(|panel| !panel.state.pending()),
                        LocalAction::AccountSearchFindPerson,
                        cx,
                    ),
                ),
        );
        let person_query = self
            .account_search_ui
            .person
            .read(cx)
            .content
            .trim()
            .to_lowercase();
        if self.account_search_ui.person_query.as_ref() == Some(&person_query)
            && let Some(panel) = self.snapshot.create_panel.as_ref().filter(|p| {
                Some(p.account) == self.snapshot.active_account
                    && self
                        .room_ui
                        .create_people_request
                        .as_ref()
                        .is_some_and(|r| r.ready(p))
            })
        {
            for person in panel.people.as_slice().iter().take(10) {
                body = body.child(
                    self.button(
                        gpui::SharedString::from(format!(
                            "account-search-person-{}",
                            person.id.local
                        )),
                        "",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::AccountSearchPerson(person.id),
                        cx,
                    )
                    .child(person.name.as_str().to_owned()),
                );
            }
        }
        if self.account_search_ui.invalid_date
            || self.account_search_filters(cx).is_err_and(|date| date)
        {
            body = body.child(self.hint(self.l("Bitte gib einen gültigen Zeitraum ein.")));
        }
        if self
            .account_search_filters(cx)
            .is_err_and(|is_date| !is_date)
        {
            body = body.child(self.hint(self.l("Suche und wähle eine Person für den Filter.")));
        }
        if !self.active_account_connected() {
            body = body.child(self.hint(self.l("Nachrichtensuche benötigt eine Verbindung.")));
            let query = self.search.read(cx).content.trim().to_lowercase();
            if !query.is_empty()
                && self.account_search_filters(cx).is_ok_and(|filters| {
                    filters.person.is_none()
                        && !filters.from_self
                        && filters.since.is_none()
                        && filters.until.is_none()
                        && filters
                            .conversation
                            .is_none_or(|room| Some(room) == self.snapshot.selected)
                })
                && let Some(context) = self.snapshot.selected
            {
                let local: Vec<_> = self
                    .snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .filter(|message| message.body.as_str().to_lowercase().contains(&query))
                    .take(20)
                    .collect();
                if !local.is_empty() {
                    body = body
                        .child(self.hint(self.l("Zuletzt geladene Treffer in diesem Gespräch")));
                }
                for message in local {
                    body = body.child(
                        self.button(
                            super::controls::scoped_id(
                                "account-search-local",
                                context,
                                message.local_id,
                                0,
                            ),
                            "",
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::LocateMessage(context, message.local_id),
                            cx,
                        )
                        .w_full()
                        .justify_start()
                        .child(highlight(
                            message.body.as_str(),
                            &query,
                            self.theme.accent,
                        )),
                    );
                }
            }
        }
        body = body.child(
            self.button(
                "account-search-submit",
                "Nachrichten suchen",
                ButtonStyle::Primary,
                self.active_account_connected()
                    && !self.search.read(cx).content.trim().is_empty()
                    && self.account_search_filters(cx).is_ok()
                    && self
                        .snapshot
                        .search_panel
                        .as_ref()
                        .is_none_or(|panel| !panel.state.pending()),
                LocalAction::AccountSearchSubmit,
                cx,
            ),
        );
        if let Some(panel) = self.snapshot.search_panel.as_ref().filter(|p| {
            Some(p.account) == self.snapshot.active_account
                && p.term.as_str() == self.search.read(cx).content.trim()
                && self
                    .account_search_filters(cx)
                    .is_ok_and(|filters| filters == p.filters)
        }) {
            if panel.state.pending() {
                body = body.child(self.collection_skeleton(super::controls::Skeleton::Rows));
            }
            if matches!(panel.state, RemoteState::Failed(..)) {
                body = body
                    .child(self.hint(self.l("Die Nachrichtensuche ist derzeit nicht verfügbar.")));
            }
            if panel.loaded && panel.hits.as_slice().is_empty() {
                body = body.child(self.hint(self.l("Keine Nachrichten gefunden.")));
            }
            let hits = self.account_search_hits();
            let selected = self
                .account_search_ui
                .selected
                .or_else(|| hits.first().map(|h| (h.context, h.message)));
            let mut last = None;
            for hit in hits {
                if last != Some(hit.context) {
                    if let Some(room) = self
                        .snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .find(|r| r.context == hit.context)
                    {
                        body = body.child(
                            div()
                                .pt_2()
                                .text_size(self.sp(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(room.title.as_str().to_owned()),
                        );
                    }
                    last = Some(hit.context);
                }
                let target = (hit.context, hit.message);
                body = body.child(
                    self.button(
                        super::controls::scoped_id(
                            "account-search-hit",
                            hit.context,
                            hit.message,
                            0,
                        ),
                        "",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::AccountSearchHit(hit.context, hit.message),
                        cx,
                    )
                    .w_full()
                    .justify_start()
                    .flex_col()
                    .items_start()
                    .px_3()
                    .py_1()
                    .when(selected == Some(target), |d| d.bg(self.theme.selected))
                    .child(self.hint(hit.title.as_str()))
                    .child(div().text_size(self.sp(14.)).child(highlight(
                        hit.excerpt.as_str(),
                        panel.term.as_str(),
                        self.theme.accent,
                    ))),
                );
            }
            if panel.more {
                body = body.child(self.button(
                    "account-search-more",
                    "Weitere Ergebnisse",
                    ButtonStyle::Secondary,
                    !panel.state.pending(),
                    LocalAction::AccountSearchMore,
                    cx,
                ));
            }
        }
        body
    }
}
#[cfg(test)]
mod tests {
    use super::date;
    #[test]
    fn calendar_validation_rejects_impossible_dates_and_reversed_values_can_be_compared() {
        assert!(date("2024-02-29", false).is_ok());
        assert!(date("2023-02-29", false).is_err());
        assert!(date("2024-13-01", true).is_err());
        assert!(
            date("2024-01-01", true)
                .unwrap()
                .unwrap()
                .as_str()
                .ends_with("23:59:59Z")
        );
    }
}
