//! Account-wide search form and bounded grouped results; all I/O is an intent.
use super::{ButtonStyle, Desktop, LocalAction, controls::BannerRole, input::TextInput};
use gpui::{Context, Div, Entity, FontWeight, HighlightStyle, StyledText, div, prelude::*};
use talk_core::ui::{
    AccountScope, Text, UnavailableReason,
    conversation::{AccountPersonRef, RemoteState},
    presentation::{ConversationRef, RequestId, UiIntent},
    search::{Action, Filters, Hit, Panel},
};

const UNSUPPORTED_SEARCH: &str =
    "Die Nachrichtensuche oder der gewählte Filter wird auf diesem Server nicht unterstützt.";

#[derive(Clone, Copy, Eq, PartialEq)]
enum FilterError {
    Date,
    Person,
    Timezone,
}
fn boundary_error(error: talk_core::preferences::time::LocalTimeError) -> FilterError {
    if error == talk_core::preferences::time::LocalTimeError::Unavailable {
        FilterError::Timezone
    } else {
        FilterError::Date
    }
}

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
            since: cx.new(|cx| TextInput::new("TT.MM.JJJJ", 10, theme, cx)),
            until: cx.new(|cx| TextInput::new("TT.MM.JJJJ", 10, theme, cx)),
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
    pub fn reformat_dates(
        &self,
        old: talk_core::preferences::Locale,
        new: talk_core::preferences::Locale,
        cx: &mut Context<Desktop>,
    ) {
        if old == new {
            return;
        }
        for input in [&self.since, &self.until] {
            let value = input.read(cx).content.trim();
            if let Ok(Some(day)) = date(value, old) {
                let formatted = format_date(day.as_str(), new);
                input.update(cx, |input, _| {
                    input.set_value(&formatted);
                });
            }
        }
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
// Parse locale input into a validated canonical civil date. Timezone resolution
// belongs to SettingsPort, never to the renderer or the server's permissive parser.
fn date(value: &str, locale: talk_core::preferences::Locale) -> Result<Option<Text<32>>, ()> {
    use talk_core::preferences::Locale;
    if value.is_empty() {
        return Ok(None);
    }
    let b = value.as_bytes();
    if b.len() != 10 || !value.is_ascii() {
        return Err(());
    }
    let (year, month, day) = if b[4] == b'-' && b[7] == b'-' {
        (&value[0..4], &value[5..7], &value[8..10])
    } else {
        let separator = if locale == Locale::German { b'.' } else { b'/' };
        if b[2] != separator || b[5] != separator {
            return Err(());
        }
        if locale == Locale::English {
            (&value[6..10], &value[0..2], &value[3..5])
        } else {
            (&value[6..10], &value[3..5], &value[0..2])
        }
    };
    if !year
        .bytes()
        .chain(month.bytes())
        .chain(day.bytes())
        .all(|b| b.is_ascii_digit())
    {
        return Err(());
    }
    let year: u32 = year.parse().map_err(|_| ())?;
    let month: u32 = month.parse().map_err(|_| ())?;
    let day: u32 = day.parse().map_err(|_| ())?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
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
    Text::new(&format!("{year:04}-{month:02}-{day:02}"))
        .map(Some)
        .map_err(|_| ())
}
fn format_date(day: &str, locale: talk_core::preferences::Locale) -> String {
    use talk_core::preferences::Locale;
    // `day` is the canonical ASCII output of date(), never unvalidated input.
    let (year, month, day) = (&day[..4], &day[5..7], &day[8..10]);
    match locale {
        Locale::German => format!("{day}.{month}.{year}"),
        Locale::English => format!("{month}/{day}/{year}"),
        Locale::Spanish => format!("{day}/{month}/{year}"),
    }
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

fn unsupported_search(
    panel: Option<&Panel>,
    account: Option<AccountScope>,
    filters: &Filters,
) -> bool {
    panel.is_some_and(|panel| {
        Some(panel.account) == account
            && panel.filters == *filters
            && matches!(
                panel.state,
                RemoteState::Failed(_, UnavailableReason::Unsupported)
            )
    })
}

fn search_failure_message(reason: UnavailableReason) -> Option<&'static str> {
    use talk_core::ui::presentation::ConnectionFailure;
    match reason {
        UnavailableReason::Unsupported => Some(UNSUPPORTED_SEARCH),
        UnavailableReason::PermissionDenied
        | UnavailableReason::Connection(ConnectionFailure::AccessDenied) => {
            Some("Du darfst diese Nachrichtensuche nicht verwenden.")
        }
        UnavailableReason::ReauthenticationRequired => {
            Some("Verbinde dein Konto erneut, um Nachrichten zu suchen.")
        }
        _ => None,
    }
}

impl Desktop {
    fn account_search_filters(&self, cx: &Context<Self>) -> Result<Filters, FilterError> {
        let start = self.account_search_ui.since.read(cx).content.trim();
        let end = self.account_search_ui.until.read(cx).content.trim();
        let start = date(start, self.locale).map_err(|_| FilterError::Date)?;
        let end = date(end, self.locale).map_err(|_| FilterError::Date)?;
        if start
            .as_ref()
            .zip(end.as_ref())
            .is_some_and(|(start, end)| start.as_str() > end.as_str())
        {
            return Err(FilterError::Date);
        }
        let since = start
            .map(|day| self.settings_port.search_date_boundary(day, false))
            .transpose()
            .map_err(boundary_error)?;
        let until = end
            .map(|day| self.settings_port.search_date_boundary(day, true))
            .transpose()
            .map_err(boundary_error)?;
        let current_person = self.account_search_ui.person.read(cx).content.trim();
        let person = self
            .account_search_ui
            .selected_person
            .as_ref()
            .filter(|(_, name)| name == current_person)
            .map(|(id, _)| *id);
        if !current_person.is_empty() && person.is_none() {
            return Err(FilterError::Person);
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
            return Err(FilterError::Person);
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
            Err(error) => {
                self.account_search_ui.invalid_date = error == FilterError::Date;
                return;
            }
        };
        if unsupported_search(self.snapshot.search_panel.as_ref(), Some(account), &filters) {
            return;
        }
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
        if !self.account_search_current(cx)
            || self.snapshot.search_panel.as_ref().is_some_and(|panel| {
                matches!(
                    panel.state,
                    RemoteState::Failed(_, UnavailableReason::Unsupported)
                )
            })
        {
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
        let filters = self.account_search_filters(cx);
        let unsupported = filters.as_ref().is_ok_and(|filters| {
            unsupported_search(
                self.snapshot.search_panel.as_ref(),
                self.snapshot.active_account,
                filters,
            )
        });
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
        body = body.child(self.hint(self.l("Zeitraum: von / bis, jeweils TT.MM.JJJJ (Ortszeit).")));
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
            || self
                .account_search_filters(cx)
                .is_err_and(|error| error == FilterError::Date)
        {
            body = body.child(self.banner(
                BannerRole::Error,
                self.l("Bitte gib einen gültigen Zeitraum ein."),
            ));
        }
        if self
            .account_search_filters(cx)
            .is_err_and(|error| error == FilterError::Person)
        {
            body = body.child(self.banner(
                BannerRole::Warning,
                self.l("Suche und wähle eine Person für den Filter."),
            ));
        }
        if self
            .account_search_filters(cx)
            .is_err_and(|error| error == FilterError::Timezone)
        {
            body = body.child(self.banner(
                BannerRole::Warning,
                self.l("Die lokale Zeitzone ist noch nicht verfügbar."),
            ));
        }
        if !self.active_account_connected() {
            body = body.child(self.banner(
                BannerRole::Warning,
                self.l("Nachrichtensuche benötigt eine Verbindung."),
            ));
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
        if unsupported {
            body = body.child(self.banner(BannerRole::Warning, self.l(UNSUPPORTED_SEARCH)));
        }
        body = body.child(
            self.button(
                "account-search-submit",
                "Nachrichten suchen",
                ButtonStyle::Primary,
                self.active_account_connected()
                    && !self.search.read(cx).content.trim().is_empty()
                    && filters.is_ok()
                    && !unsupported
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
            if let RemoteState::Failed(_, reason) = panel.state
                && reason != UnavailableReason::Unsupported
                && let Some(message) = search_failure_message(reason)
            {
                body = body.child(self.banner(BannerRole::Error, self.l(message)));
            }
            if panel.loaded && panel.hits.as_slice().is_empty() {
                body = body.child(self.empty_row(self.l("Keine Nachrichten gefunden.")));
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
            if panel.more && !unsupported {
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
    use super::{date, format_date, search_failure_message, unsupported_search};
    use talk_core::ui::{
        AccountId, AccountScope, Page, RuntimeGeneration, Text, UnavailableReason,
        conversation::RemoteState,
        presentation::{ConnectionFailure, RequestId},
        search::{Filters, Panel},
    };

    #[test]
    fn unsupported_filters_are_gated_per_account_and_transient_failures_stay_quiet() {
        let account = AccountScope {
            account: AccountId::from_local(1),
            runtime: RuntimeGeneration::from_local(1),
        };
        let filters = Filters {
            conversation: None,
            person: None,
            from_self: false,
            since: None,
            until: None,
        };
        let panel = Panel {
            account,
            request: RequestId(1),
            term: Text::new("map").unwrap(),
            filters: filters.clone(),
            state: RemoteState::Failed(RequestId(1), UnavailableReason::Unsupported),
            hits: Page::empty(),
            loaded: false,
            more: false,
        };
        assert!(unsupported_search(Some(&panel), Some(account), &filters));
        let other_account = AccountScope {
            runtime: RuntimeGeneration::from_local(2),
            ..account
        };
        assert!(!unsupported_search(
            Some(&panel),
            Some(other_account),
            &filters
        ));
        let other_filters = Filters {
            from_self: true,
            ..filters
        };
        assert!(!unsupported_search(
            Some(&panel),
            Some(account),
            &other_filters
        ));
        assert!(search_failure_message(UnavailableReason::Unsupported).is_some());
        assert_eq!(search_failure_message(UnavailableReason::Busy), None);
        assert_eq!(
            search_failure_message(UnavailableReason::Connection(ConnectionFailure::TimedOut)),
            None
        );
    }
    #[test]
    fn calendar_validation_uses_each_locale_and_canonical_order() {
        use talk_core::preferences::Locale::{English, German, Spanish};
        for (locale, good) in [
            (German, "29.02.2024"),
            (English, "02/29/2024"),
            (Spanish, "29/02/2024"),
        ] {
            let canonical = date(good, locale).unwrap().unwrap();
            assert_eq!(canonical.as_str(), "2024-02-29");
            assert_eq!(format_date(canonical.as_str(), locale), good);
            assert!(date("2023-02-29", locale).is_err());
            assert!(date("2024-13-01", locale).is_err());
            assert_eq!(date("", locale), Ok(None));
        }
        assert!(date("01/31/2024", Spanish).is_err());
        let start = date("31.01.2024", German).unwrap().unwrap();
        let end = date("01.02.2024", German).unwrap().unwrap();
        assert!(start.as_str() < end.as_str());
        for bad in ["xx.xx.xxxx", "日.01.2024", "01.é1.2024"] {
            assert!(date(bad, German).is_err());
        }
    }
}
