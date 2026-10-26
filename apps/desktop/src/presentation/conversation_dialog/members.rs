//! Bounded member navigation and one scoped, generation-checked action menu.
use super::*;
use crate::presentation::{controls::Skeleton, focus::scoped_id};
use gpui::{ElementId, Focusable, KeyDownEvent};
use talk_core::ui::conversation::{
    ConversationPanel, CsvFileError, CsvFileOutcome, Member, PersonRef,
};
use talk_core::ui::presentation::{AccountConnectionState, Completion, ContentState};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Page {
    #[default]
    People,
    Bots,
    Invite,
    Csv,
}
impl Page {
    pub fn key(self) -> usize {
        match self {
            Self::People => 10,
            Self::Bots => 11,
            Self::Invite => 12,
            Self::Csv => 13,
        }
    }
    pub fn navigation_request(
        self,
        members_loaded: bool,
        members_allowed: bool,
    ) -> Option<ConversationAction> {
        (self == Self::People && !members_loaded && members_allowed)
            .then_some(ConversationAction::Members)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MenuTarget {
    person: PersonRef,
    generation: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemberChoice {
    Role(MemberRole),
    Remove,
}
impl MemberChoice {
    fn label(self) -> &'static str {
        match self {
            Self::Role(MemberRole::Moderator) => "Zur Moderation ernennen",
            Self::Role(_) => "Als Mitglied setzen",
            Self::Remove => "Entfernen",
        }
    }
    fn id(self, target: MenuTarget) -> ElementId {
        scoped_id(
            "member-menu-choice",
            target.person.context,
            target.person.local,
            u128::from(target.generation) * 2 + u128::from(self == Self::Remove),
        )
    }
    fn command(self, person: PersonRef) -> ConversationAction {
        match self {
            Self::Role(role) => ConversationAction::SetRole {
                member: person,
                role,
            },
            Self::Remove => ConversationAction::RemoveMember(person),
        }
    }
}

pub(super) enum FocusTarget {
    Control(ElementId),
    Search,
    Csv,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Dismissal {
    Menu,
    Confirmation,
    Invitation,
    Dialog,
}
struct Preview {
    context: ConversationRef,
    request: Option<RequestId>,
    ready: bool,
    retained: bool,
}
#[derive(Clone, Copy)]
struct CsvPick {
    context: ConversationRef,
    request: RequestId,
    session: u64,
    edit_generation: u64,
}
impl CsvPick {
    fn accepts(
        self,
        context: Option<ConversationRef>,
        page: Page,
        session: u64,
        request: RequestId,
        generation: u64,
        result_generation: u64,
    ) -> bool {
        context == Some(self.context)
            && page == Page::Csv
            && session == self.session
            && request == self.request
            && generation == self.edit_generation
            && result_generation == self.edit_generation
    }
}
#[derive(Default)]
pub(super) struct Flow {
    pub page: Page,
    generation: u64,
    menu: Option<MenuTarget>,
    preview: Option<Preview>,
    return_control: Option<ElementId>,
    session: u64,
    csv_pick: Option<CsvPick>,
    csv_cancel: Option<CsvPick>,
    bots_requested: bool,
    bots_details_requested: bool,
}
impl Flow {
    pub fn is_bots(&self) -> bool {
        self.page == Page::Bots
    }
    pub fn show(&mut self, page: Page) {
        let Some(session) = self.session.checked_add(1) else {
            return;
        };
        self.session = session;
        self.page = page;
        self.menu = None;
        self.preview = None;
        self.return_control = None;
        self.bots_requested = false;
        self.bots_details_requested = false;
    }
    pub fn reset(&mut self) {
        // Never reuse a menu generation after closing and reopening a dialog.
        self.show(Page::People);
    }
    fn open_menu(&mut self, person: PersonRef) -> Option<MenuTarget> {
        if self.page != Page::People {
            return None;
        }
        self.generation = self.generation.checked_add(1)?;
        let target = MenuTarget {
            person,
            generation: self.generation,
        };
        self.menu = Some(target);
        self.return_control = Some(menu_id(person));
        Some(target)
    }
    fn accepts(&self, target: MenuTarget, context: Option<ConversationRef>) -> bool {
        self.page == Page::People
            && self.menu == Some(target)
            && context == Some(target.person.context)
    }
    pub fn close_menu(&mut self) {
        self.menu = None;
    }
    pub fn preview(&mut self, context: Option<ConversationRef>, request: Option<RequestId>) {
        // Retention is display-only. A new query invalidates prior authority,
        // including when admission rejects the replacement request.
        let retained = self.preview.as_ref().is_some_and(|preview| {
            Some(preview.context) == context && (preview.ready || preview.retained)
        });
        self.preview = context.map(|context| Preview {
            context,
            request,
            ready: false,
            retained,
        });
    }
    fn observe(&mut self, panel: &ConversationPanel) {
        if let Some(preview) = &mut self.preview
            && preview.context == panel.context
            && let Some(result) = panel.result
            && Some(result.request) == preview.request
        {
            preview.ready = result.completion == Completion::ServerAccepted;
        }
    }
    fn preview_ready(&self) -> bool {
        self.preview.as_ref().is_some_and(|preview| preview.ready)
    }
    fn preview_visible(&self) -> bool {
        self.preview
            .as_ref()
            .is_some_and(|preview| preview.ready || preview.retained)
    }
    fn preview_loading(&self, state: RemoteState) -> bool {
        self.preview.as_ref().is_some_and(|preview| {
            !preview.ready
                && preview
                    .request
                    .is_some_and(|request| state == RemoteState::Loading(request))
        })
    }
    fn dismissal(&self, confirming: bool) -> Dismissal {
        if self.menu.is_some() {
            Dismissal::Menu
        } else if confirming {
            Dismissal::Confirmation
        } else if matches!(self.page, Page::Invite | Page::Csv) {
            Dismissal::Invitation
        } else {
            Dismissal::Dialog
        }
    }
}

fn menu_id(person: PersonRef) -> ElementId {
    scoped_id("member-menu", person.context, 0, person.local)
}
fn choices(member: &Member) -> Vec<MemberChoice> {
    if member.self_user || matches!(member.role, MemberRole::Owner | MemberRole::Unknown) {
        return Vec::new();
    }
    let role = match member.role {
        MemberRole::Member | MemberRole::Guest => Some(MemberRole::Moderator),
        MemberRole::Moderator | MemberRole::GuestModerator => Some(MemberRole::Member),
        _ => None,
    };
    role.map(MemberChoice::Role)
        .into_iter()
        .chain([MemberChoice::Remove])
        .collect()
}
fn member_choice_allowed(
    panel: Option<&ConversationPanel>,
    selected: Option<ConversationRef>,
    person: PersonRef,
    choice: MemberChoice,
    authorized: bool,
) -> bool {
    authorized
        && selected == Some(person.context)
        && panel.is_some_and(|panel| {
            panel.context == person.context
                && panel
                    .members
                    .as_slice()
                    .iter()
                    .any(|member| member.id == person && choices(member).contains(&choice))
        })
}
fn role_label(role: MemberRole) -> &'static str {
    match role {
        MemberRole::Owner => "Eigentümer",
        MemberRole::Moderator | MemberRole::GuestModerator => "Moderation",
        MemberRole::Guest => "Gast",
        MemberRole::Following => "Folgt",
        MemberRole::Member => "Teilnehmend",
        MemberRole::Unknown => "Unbekannte Rolle",
    }
}

impl Desktop {
    pub(super) fn cancel_member_csv_pick(&mut self) {
        if self.room_ui.members.csv_cancel.is_none() {
            self.room_ui.members.csv_cancel = self.room_ui.members.csv_pick.take();
        }
        if let Some(pick) = self.room_ui.members.csv_cancel {
            // Cancellation is scoped to the original ticket, including after
            // selection changes. The worker keeps ownership until cleanup.
            let result = self.port.try_submit(UiIntent::Conversation {
                context: pick.context,
                request: pick.request,
                action: ConversationAction::CancelCsvPick {
                    pick_request: pick.request,
                },
            });
            if !matches!(result, Err(talk_core::ui::SubmitError::Busy)) {
                self.room_ui.members.csv_cancel = None;
            }
        }
    }
    pub(super) fn pick_member_csv(&mut self, cx: &mut Context<Self>) {
        if self.room_ui.members.page != Page::Csv
            || self.room_ui.members.csv_pick.is_some()
            || self.room_ui.members.csv_cancel.is_some()
            || !self.csv_available().is_enabled()
        {
            return;
        }
        let Some(context) = self.room_ui.context else {
            return;
        };
        let edit_generation = self.room_ui.csv.read(cx).edit_generation();
        if let Some(request) =
            self.submit_room_request(ConversationAction::PickCsv { edit_generation })
        {
            self.room_ui.members.csv_pick = Some(CsvPick {
                context,
                request,
                session: self.room_ui.members.session,
                edit_generation,
            });
        }
    }
    pub(super) fn refresh_member_csv_pick(&mut self, cx: &mut Context<Self>) {
        if self.room_ui.members.csv_cancel.is_some() {
            self.cancel_member_csv_pick();
        }
        let Some(pick) = self.room_ui.members.csv_pick else {
            return;
        };
        if self.overlay != Some(Overlay::Participants)
            || self.room_ui.members.page != Page::Csv
            || self.snapshot.selected != Some(pick.context)
            || !self.room_members_applicable()
            || self
                .room_panel()
                .and_then(|panel| panel.details.as_ref())
                .is_none_or(|details| !details.availability(RoomSetting::CsvImport).enabled())
        {
            self.cancel_member_csv_pick();
            return;
        }
        let Some(result) = self
            .room_panel()
            .and_then(|panel| panel.csv_file.as_ref())
            .filter(|result| result.request == pick.request)
        else {
            return;
        };
        let accepted = pick.accepts(
            self.room_ui.context,
            self.room_ui.members.page,
            self.room_ui.members.session,
            result.request,
            self.room_ui.csv.read(cx).edit_generation(),
            result.edit_generation,
        );
        let outcome = accepted.then(|| result.outcome.clone());
        self.cancel_member_csv_pick();
        match outcome {
            Some(CsvFileOutcome::Loaded(csv)) => {
                if self
                    .room_ui
                    .csv
                    .update(cx, |input, _| input.set_context_value(csv.as_str()))
                {
                    self.room_ui.members.preview(self.room_ui.context, None);
                    self.room_ui.pending_member_focus = Some(FocusTarget::Csv);
                }
            }
            Some(CsvFileOutcome::Failed(error)) => {
                self.feedback = Some(
                    crate::presentation::feedback::Feedback::error(csv_error_label(error))
                        .in_conversation(pick.context)
                        .on_surface(self.overlay),
                );
            }
            Some(CsvFileOutcome::Cancelled) | None => {}
        }
    }
    pub(super) fn load_bots_on_entry(&mut self) {
        if self.overlay != Some(Overlay::Participants)
            || !self.room_members_applicable()
            || self.room_ui.members.page != Page::Bots
            || self.room_ui.members.bots_requested
            || self.bots_offline()
        {
            return;
        }
        if !self.room_ui.members.bots_details_requested
            && self
                .room_panel()
                .is_some_and(|panel| panel.details.is_none() && panel.state == RemoteState::Idle)
        {
            if self.submit_room_request(ConversationAction::Read).is_some() {
                self.room_ui.members.bots_details_requested = true;
            }
            return;
        }
        if !self.bots_available().is_enabled()
            || self
                .room_panel()
                .is_none_or(|panel| !bots_need_initial_load(panel))
        {
            return;
        }
        if self.submit_room_request(ConversationAction::Bots).is_some() {
            self.room_ui.members.bots_requested = true;
        }
    }
    pub(super) fn room_panel(&self) -> Option<&ConversationPanel> {
        self.snapshot.conversation_panel.as_ref().filter(|panel| {
            Some(panel.context) == self.room_ui.context
                && Some(panel.context) == self.snapshot.selected
        })
    }
    pub(super) fn refresh_member_preview(&mut self) {
        if let Some(panel) = self.snapshot.conversation_panel.as_ref()
            && Some(panel.context) == self.room_ui.context
            && Some(panel.context) == self.snapshot.selected
        {
            self.room_ui.members.observe(panel);
        }
    }
    fn bots_offline(&self) -> bool {
        let Some(context) = self.room_ui.context else {
            return true;
        };
        self.snapshot.selected != Some(context)
            || self.snapshot.content == ContentState::Offline
            || self.snapshot.conversation_list.content == ContentState::Offline
            || !self
                .snapshot
                .accounts
                .as_slice()
                .iter()
                .find(|account| account.scope == context.account)
                .is_some_and(|account| {
                    account.connection != AccountConnectionState::Disconnected
                        && account.connection_failure.is_none()
                })
    }
    pub(super) fn bots_available(&self) -> Availability {
        let value = self
            .room_panel()
            .and_then(|panel| panel.details.as_ref())
            .map_or(conversation::Availability::UNKNOWN, |details| {
                details.availability(RoomSetting::Bots)
            });
        if self.bots_offline() {
            Availability::when(false, "Bots sind offline nicht verfügbar.")
        } else if !value.implemented
            || value.server == FeatureSupport::Unsupported
            || matches!(value.allowed, Decision::Denied | Decision::Stale)
        {
            Availability::when(false, unavailable(value))
        } else if self.room_panel().is_some_and(|panel| {
            matches!(
                panel.bots_state,
                RemoteState::Loading(_) | RemoteState::Saving(_)
            ) || matches!(
                panel.state,
                RemoteState::Loading(_) | RemoteState::Saving(_)
            )
        }) {
            Availability::pending()
        } else {
            Availability::when(value.enabled(), unavailable(value))
        }
    }
    fn bot_change_available(&self) -> Availability {
        if self.room_panel().is_some_and(|panel| {
            matches!(panel.state, RemoteState::Uncertain(_))
                || matches!(panel.bots_state, RemoteState::Uncertain(_))
        }) {
            Availability::when(false, "Ergebnis unklar. Vor erneutem Ändern neu laden.")
        } else {
            self.bots_available()
        }
    }
    pub(super) fn csv_available(&self) -> Availability {
        self.room_available(RoomSetting::CsvImport)
    }
    fn menu_member(&self, target: MenuTarget) -> Option<&Member> {
        if self.overlay != Some(Overlay::Participants)
            || self.room_ui.confirmation.is_some()
            || !self.room_ui.members.accepts(target, self.snapshot.selected)
            || !self.room_available(RoomSetting::Roles).is_enabled()
        {
            return None;
        }
        self.room_panel()?
            .members
            .as_slice()
            .iter()
            .find(|member| member.id == target.person)
    }
    pub(super) fn member_command_allowed(&self, action: &ConversationAction) -> bool {
        match action {
            ConversationAction::Bots | ConversationAction::SetBotEnabled { .. } => {
                self.overlay == Some(Overlay::Participants)
                    && self.room_members_applicable()
                    && self.room_ui.members.page == Page::Bots
                    && self.bots_available().is_enabled()
                    && match action {
                        ConversationAction::SetBotEnabled { bot, .. } => {
                            self.bot_change_available().is_enabled()
                                && self.room_panel().is_some_and(|panel| {
                                    panel.bots_loaded
                                        && panel.bots.as_slice().iter().any(|value| {
                                            value.id == *bot && value.enabled.is_some()
                                        })
                                })
                        }
                        _ => true,
                    }
            }
            ConversationAction::SetRole { member, .. }
            | ConversationAction::RemoveMember(member) => {
                let choice = match action {
                    ConversationAction::SetRole { role, .. } => MemberChoice::Role(*role),
                    _ => MemberChoice::Remove,
                };
                self.overlay == Some(Overlay::Participants)
                    && self.room_ui.members.page == Page::People
                    && member_choice_allowed(
                        self.room_panel(),
                        self.snapshot.selected,
                        *member,
                        choice,
                        self.room_available(RoomSetting::Roles).is_enabled(),
                    )
            }
            ConversationAction::Invite(person) => {
                self.overlay == Some(Overlay::Participants)
                    && matches!(self.room_ui.members.page, Page::Invite | Page::Csv)
                    && self.room_ui.members.preview_ready()
                    && self.room_available(RoomSetting::Invite).is_enabled()
                    && (self.room_ui.members.page != Page::Csv || self.csv_available().is_enabled())
                    && self.room_panel().is_some_and(|panel| {
                        panel
                            .directory
                            .as_slice()
                            .iter()
                            .any(|value| value.id == *person)
                    })
            }
            _ => true,
        }
    }
    pub(super) fn member_confirmation_available(
        &self,
        action: Option<&ConversationAction>,
    ) -> Availability {
        let Some(action) = action else {
            return Availability::when(false, "Derzeit nicht verfügbar.");
        };
        let availability = match action {
            ConversationAction::SetRole { .. } | ConversationAction::RemoveMember(_) => {
                self.room_available(RoomSetting::Roles)
            }
            ConversationAction::Invite(_) if self.room_ui.members.page == Page::Csv => {
                self.csv_available()
            }
            ConversationAction::Invite(_) => self.room_available(RoomSetting::Invite),
            _ => return Availability::available(),
        };
        if !availability.is_enabled() {
            availability
        } else {
            Availability::when(
                self.member_command_allowed(action),
                "Diese Person ist nicht mehr verfügbar.",
            )
        }
    }
    pub(super) fn open_member_menu(
        &mut self,
        person: PersonRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay != Some(Overlay::Participants)
            || self.room_ui.confirmation.is_some()
            || self.room_ui.context != Some(person.context)
            || self.snapshot.selected != Some(person.context)
            || !self.room_available(RoomSetting::Roles).is_enabled()
            || self.room_panel().is_none_or(|panel| {
                !panel
                    .members
                    .as_slice()
                    .iter()
                    .any(|member| member.id == person && !choices(member).is_empty())
            })
        {
            return;
        }
        if self
            .room_ui
            .members
            .menu
            .is_some_and(|target| target.person == person)
        {
            self.dismiss_member_menu(window, cx);
        } else if self.room_ui.members.open_menu(person).is_some() {
            self.room_ui.member_focus.focus(window);
            self.room_ui.pending_member_focus = self.room_ui.members.menu.and_then(|target| {
                self.menu_member(target)
                    .and_then(|member| choices(member).first().copied())
                    .map(|choice| FocusTarget::Control(choice.id(target)))
            });
        }
    }
    pub(super) fn choose_member_action(
        &mut self,
        target: MenuTarget,
        choice: MemberChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .menu_member(target)
            .is_none_or(|member| !choices(member).contains(&choice))
        {
            return;
        }
        self.room_ui.members.close_menu();
        self.room_action(Action::Confirm(choice.command(target.person)), window, cx);
    }
    fn dismiss_member_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .room_ui
            .members
            .menu
            .map(|target| menu_id(target.person))
            .and_then(|id| self.control_focus.current(&id));
        self.room_ui.members.close_menu();
        self.room_ui.pending_member_focus = None;
        if let Some(focus) = focus {
            focus.focus(window);
        } else {
            self.modal_focus.focus(window);
        }
        cx.notify();
    }
    pub(super) fn restore_member_page_focus(&mut self) {
        if self.overlay != Some(Overlay::Participants) {
            self.room_ui.pending_member_focus = Some(FocusTarget::Control("close-overlay".into()));
            return;
        }
        self.room_ui.pending_member_focus = Some(match self.room_ui.members.page {
            Page::Invite => FocusTarget::Search,
            Page::Csv => FocusTarget::Csv,
            Page::People => FocusTarget::Control(
                self.room_ui
                    .members
                    .return_control
                    .clone()
                    .unwrap_or_else(|| "room-add-members".into()),
            ),
            Page::Bots => FocusTarget::Control("room-bots-tab".into()),
        });
    }
    pub(super) fn back_from_members(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(
            self.overlay,
            Some(Overlay::Participants | Overlay::RoomInfo)
        ) {
            return false;
        }
        match self
            .room_ui
            .members
            .dismissal(self.room_ui.confirmation.is_some())
        {
            Dismissal::Menu => self.dismiss_member_menu(window, cx),
            Dismissal::Confirmation => {
                self.room_ui.confirmation = None;
                self.restore_member_page_focus();
            }
            Dismissal::Invitation => {
                self.cancel_member_csv_pick();
                let csv = self.room_ui.members.page == Page::Csv;
                self.room_ui.members.show(Page::People);
                self.begin_surface_reveal(("room-page", Page::People.key()).into());
                self.room_ui.pending_member_focus = Some(FocusTarget::Control(
                    if csv {
                        "room-csv-toggle"
                    } else {
                        "room-add-members"
                    }
                    .into(),
                ));
            }
            Dismissal::Dialog => return false,
        }
        cx.notify();
        true
    }
    pub(crate) fn close_room_dialog_state(&mut self) {
        self.cancel_member_csv_pick();
        self.room_ui.confirmation = None;
        self.room_ui.members.reset();
        self.room_ui.pending_member_focus = None;
    }
    pub(crate) fn configure_member_inputs(&self, incoming: bool, cx: &mut Context<Self>) {
        let participants = self.overlay == Some(Overlay::Participants);
        let editing = participants && !incoming && self.room_ui.confirmation.is_none();
        if participants {
            for input in [
                &self.room_ui.name,
                &self.room_ui.description,
                &self.room_ui.avatar,
                &self.room_ui.password,
            ] {
                input.update(cx, |input, _| input.interactive = false);
            }
        }
        self.room_ui.search.update(cx, |input, _| {
            input.interactive = editing && self.room_ui.members.page == Page::Invite
        });
        self.room_ui.csv.update(cx, |input, _| {
            input.interactive = editing && self.room_ui.members.page == Page::Csv
        });
    }
    pub(crate) fn prepare_member_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.room_ui.member_anchor.set(None);
        if !matches!(
            self.overlay,
            Some(Overlay::Participants | Overlay::RoomInfo)
        ) {
            self.close_room_dialog_state();
            return;
        }
        if let Some(target) = self.room_ui.members.menu {
            let lost_focus = self.room_ui.pending_member_focus.is_none()
                && !self.room_ui.member_focus.contains_focused(window, cx);
            if self.menu_member(target).is_none()
                || lost_focus
                || !window.is_window_active()
                || self.incoming_visible()
                || self.account_removal.confirming()
                || self.update_restart_pending
                || self.projection_blocked
            {
                self.room_ui.members.close_menu();
                self.room_ui.pending_member_focus = None;
            }
        }
        if self.overlay != Some(Overlay::Participants) {
            self.room_ui.members.reset();
        }
    }
    pub(crate) fn finish_member_dialog_frame(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.incoming_visible()
            || self.account_removal.confirming()
            || self.update_restart_pending
        {
            return;
        }
        let Some(target) = self.room_ui.pending_member_focus.take() else {
            return;
        };
        let focus = match target {
            FocusTarget::Search => Some(self.room_ui.search.focus_handle(cx)),
            FocusTarget::Csv => Some(self.room_ui.csv.focus_handle(cx)),
            FocusTarget::Control(id) => self.control_focus.current(&id),
        };
        if let Some(focus) = focus {
            focus.focus(window);
        } else {
            self.modal_focus.focus(window);
        }
        self.navigation.reveal.request();
    }
    pub(crate) fn member_dialog_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.incoming_visible() || self.account_removal.confirming() {
            return false;
        }
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            return self.back_from_members(window, cx);
        }
        let Some(target) = self.room_ui.members.menu else {
            return false;
        };
        if key == "tab" {
            self.dismiss_member_menu(window, cx);
            return true;
        }
        // A member menu owns shortcuts until dismissed. Modified shortcuts must
        // never open another dialog over its scoped confirmation flow.
        if event.keystroke.modifiers.modified() {
            return true;
        }
        if matches!(key, "up" | "down" | "home" | "end") {
            let handles: Vec<_> = self
                .menu_member(target)
                .into_iter()
                .flat_map(choices)
                .filter_map(|choice| self.control_focus.current(&choice.id(target)))
                .collect();
            let current = handles.iter().position(|focus| focus.is_focused(window));
            if let Some(index) = menu_next(current, handles.len(), key)
                && let Some(focus) = handles.get(index)
            {
                focus.focus(window);
            }
            return true;
        }
        !matches!(key, "enter" | "space")
    }

    pub(super) fn members_page(&self, narrow: bool, cx: &Context<Self>) -> Div {
        let title = match self.room_ui.members.page {
            Page::People => "Personen",
            Page::Bots => "Bots",
            Page::Invite => "Mitglieder hinzufügen",
            Page::Csv => "CSV-Import",
        };
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.room_page_header(title, narrow));
        let Some(panel) = self.room_panel() else {
            return body.child(self.button(
                "room-load-details",
                "Erneut laden",
                ButtonStyle::Secondary,
                true,
                LocalAction::Room(Action::Command(ConversationAction::Read)),
                cx,
            ));
        };
        match self.room_ui.members.page {
            Page::People => self.member_rows(body, panel, cx),
            Page::Bots => self.bot_rows(body, panel, cx),
            Page::Invite | Page::Csv => self.invitation_page(body, panel, cx),
        }
    }
    fn member_rows(&self, mut body: Div, panel: &ConversationPanel, cx: &Context<Self>) -> Div {
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_center()
                .gap_2()
                .py_2()
                .child(self.button(
                    "room-add-members",
                    "Mitglieder hinzufügen",
                    ButtonStyle::Primary,
                    self.room_available(RoomSetting::Invite),
                    LocalAction::Room(Action::AddMembers),
                    cx,
                ))
                .child(self.button(
                    "room-csv-toggle",
                    "CSV-Import",
                    ButtonStyle::Quiet,
                    self.csv_available(),
                    LocalAction::Room(Action::ShowCsv),
                    cx,
                )),
        );
        if panel.details.is_none()
            && panel.state == RemoteState::Idle
            && !panel.members_loaded
            && panel.members.as_slice().is_empty()
        {
            return body.child(self.button(
                "room-load-details",
                "Erneut laden",
                ButtonStyle::Secondary,
                true,
                LocalAction::Room(Action::Command(ConversationAction::Read)),
                cx,
            ));
        }
        if let Some(details) = panel.details.as_ref()
            && !details.availability(RoomSetting::Members).enabled()
        {
            let availability = details.availability(RoomSetting::Members);
            if availability.implemented
                && availability.server != FeatureSupport::Unsupported
                && availability.allowed == Decision::Unknown
                && initial_room_load(
                    Some(panel.state),
                    panel.members_loaded || !panel.members.as_slice().is_empty(),
                )
            {
                return body.child(self.collection_skeleton(Skeleton::Rows));
            }
            return body.child(self.hint(self.l(unavailable(availability))));
        }
        if initial_room_load(
            Some(panel.state),
            panel.members_loaded || !panel.members.as_slice().is_empty(),
        ) {
            return body.child(self.collection_skeleton(Skeleton::Rows));
        }
        if panel.members_loaded && panel.members.as_slice().is_empty() {
            body = body.child(self.hint(self.l("Keine passenden Personen gefunden.")));
        }
        for member in panel.members.as_slice() {
            body = body.child(
                div()
                    .id(scoped_id("member-row", panel.context, 0, member.id.local))
                    .flex()
                    .items_start()
                    .gap_2()
                    .py_1()
                    .child(self.content_avatar(
                        talk_core::ui::content::AssetTarget::ParticipantAvatar {
                            context: panel.context,
                            participant: member.id.local,
                        },
                        &person_initials(member.name.as_str()),
                        30.,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .w_full()
                                    .whitespace_normal()
                                    .child(member.name.as_str().to_owned()),
                            )
                            .child(self.hint(self.l(role_label(member.role)))),
                    )
                    .when(!choices(member).is_empty(), |row| {
                        row.child(self.member_menu_trigger(member, cx))
                    }),
            );
        }
        if !panel.members_loaded && !matches!(panel.state, RemoteState::Loading(_)) {
            body = body.child(self.button(
                "room-load-members",
                "Mitglieder laden",
                ButtonStyle::Secondary,
                self.room_available(RoomSetting::Members),
                LocalAction::Room(Action::Command(ConversationAction::Members)),
                cx,
            ));
        }
        body
    }
    fn member_menu_trigger(&self, member: &Member, cx: &Context<Self>) -> Div {
        let person = member.id;
        let menu = self
            .room_ui
            .members
            .menu
            .filter(|target| target.person == person);
        div()
            .relative()
            .flex_none()
            .on_children_prepainted(cx.processor(
                move |this, bounds: Vec<gpui::Bounds<gpui::Pixels>>, _, _| {
                    if this
                        .room_ui
                        .members
                        .menu
                        .is_some_and(|target| target.person == person)
                    {
                        this.room_ui.member_anchor.set(bounds.first().copied());
                    }
                },
            ))
            .child(self.icon_button(
                menu_id(person),
                ("more", "Mitgliedsaktionen"),
                if menu.is_some() {
                    ButtonStyle::Secondary
                } else {
                    ButtonStyle::Quiet
                },
                self.room_available(RoomSetting::Roles),
                LocalAction::Room(Action::MemberMenu(person)),
                cx,
            ))
            .when_some(menu, |trigger, target| {
                let mut menu = self
                    .context_menu_surface()
                    .id(scoped_id(
                        "member-popup",
                        person.context,
                        person.local,
                        target.generation.into(),
                    ))
                    .track_focus(&self.room_ui.member_focus)
                    .key_context("MemberMenu")
                    .occlude()
                    .w(self.sp(240.).min(px(320.)))
                    .on_mouse_down_out(cx.listener(
                        move |this, event: &gpui::MouseDownEvent, window, cx| {
                            // A click on the invoker toggles through its normal action.
                            if this.room_ui.members.menu == Some(target)
                                && !this
                                    .room_ui
                                    .member_anchor
                                    .get()
                                    .is_some_and(|bounds| bounds.contains(&event.position))
                            {
                                this.dismiss_member_menu(window, cx);
                            }
                        },
                    ));
                for choice in choices(member) {
                    menu = menu.child(
                        self.button(
                            choice.id(target),
                            choice.label(),
                            if choice == MemberChoice::Remove {
                                ButtonStyle::Danger
                            } else {
                                ButtonStyle::Quiet
                            },
                            self.room_available(RoomSetting::Roles),
                            LocalAction::Room(Action::MemberChoice(target, choice)),
                            cx,
                        )
                        .w_full()
                        .justify_start(),
                    );
                }
                // Published GPUI 0.2.2 defers beyond ancestor clipping while
                // retaining the modal focus ancestry. Anchored fits the window.
                trigger.child(gpui::deferred(
                    gpui::anchored()
                        .anchor(gpui::Corner::TopRight)
                        .offset(gpui::point(px(34.), px(36.)))
                        .child(menu),
                ))
            })
    }
    fn bot_rows(&self, mut body: Div, panel: &ConversationPanel, cx: &Context<Self>) -> Div {
        body = body.child(self.hint(self.l("Verfügbare Bots für dieses Gespräch. Registrierung und Zugangsdaten verwaltet die Administration.")));
        if self.bots_offline() {
            body = body.child(self.hint(self.l("Bots sind offline nicht verfügbar.")));
        }
        if initial_room_load(
            Some(panel.bots_state),
            panel.bots_loaded || !panel.bots.as_slice().is_empty(),
        ) {
            return body.child(self.collection_skeleton(Skeleton::Rows));
        }
        for bot in panel.bots.as_slice() {
            let availability = if bot.enabled.is_some() {
                self.bot_change_available()
            } else {
                Availability::when(
                    false,
                    "Die Verfügbarkeit ist nicht bekannt. Bitte neu laden.",
                )
            };
            body = body.child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .py_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .whitespace_normal()
                                    .child(bot.name.as_str().to_owned()),
                            )
                            .child(self.hint(bot.description.as_str())),
                    )
                    .child(self.switch(
                        scoped_id("room-bot", panel.context, 0, bot.id.local),
                        "Bot aktivieren",
                        bot.enabled,
                        availability,
                        LocalAction::Room(Action::Command(ConversationAction::SetBotEnabled {
                            bot: bot.id,
                            enabled: bot.enabled != Some(true),
                        })),
                        cx,
                    )),
            );
        }
        if panel.bots_loaded && panel.bots.as_slice().is_empty() {
            body = body.child(self.hint(self.l("Keine verfügbaren Bots.")));
        }
        body
    }
    fn invitation_page(&self, mut body: Div, panel: &ConversationPanel, cx: &Context<Self>) -> Div {
        let csv = self.room_ui.members.page == Page::Csv;
        body = body.child(self.button(
            "room-members-back",
            "Zurück zu Mitgliedern",
            ButtonStyle::Quiet,
            true,
            LocalAction::Room(Action::MembersBack),
            cx,
        ));
        if csv {
            body = body
                .child(self.hint(self.l(
                    "CSV-Datei auswählen oder Adressen einfügen. Maximal 100 Adressen und 64 KiB.",
                )))
                .child(self.dialog_actions().child(self.button_with_leading_icon(
                    "room-csv-file",
                    ("CSV-Datei auswählen", "file"),
                    ButtonStyle::Secondary,
                    if self.room_ui.members.csv_pick.is_some() {
                        Availability::pending()
                    } else {
                        self.csv_available()
                    },
                    LocalAction::Room(Action::PickCsv),
                    cx,
                )))
                .child(self.room_input(&self.room_ui.csv, cx))
                .child(self.dialog_actions().child(self.button(
                    "room-csv-preview",
                    "Importvorschau erstellen",
                    ButtonStyle::Secondary,
                    self.csv_available(),
                    LocalAction::Room(Action::CsvPreview),
                    cx,
                )));
        } else {
            body =
                body.child(self.hint(
                    self.l("Suchergebnisse werden erst nach deiner Bestätigung eingeladen."),
                ))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .child(self.room_input(&self.room_ui.search, cx))
                        .child(self.button(
                            "room-directory-search",
                            "Verzeichnis durchsuchen",
                            ButtonStyle::Secondary,
                            self.room_available(RoomSetting::Invite),
                            LocalAction::Room(Action::Search),
                            cx,
                        )),
                );
        }
        let loading = self.room_ui.members.preview_loading(panel.state);
        if loading && !self.room_ui.members.preview_visible() {
            return body.child(self.collection_skeleton(Skeleton::Rows));
        }
        if !self.room_ui.members.preview_visible() {
            return body;
        }
        if csv {
            body = body.child(self.hint(&self.lf(
                "{duplicates} doppelte und {invalid} ungültige Zeilen.",
                &[
                    ("duplicates", &panel.csv_duplicates.to_string()),
                    ("invalid", &panel.csv_invalid.to_string()),
                ],
            )));
        }
        for person in panel.directory.as_slice() {
            body = body.child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .py_2()
                    .child(self.avatar(&person_initials(person.name.as_str()), false))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .whitespace_normal()
                                    .child(person.name.as_str().to_owned()),
                            )
                            .child(self.hint(person.label.as_str())),
                    )
                    .child(self.button(
                        scoped_id("room-invite", panel.context, 0, person.id.local),
                        "Einladen",
                        ButtonStyle::Secondary,
                        if !self.room_ui.members.preview_ready() {
                            if loading {
                                Availability::pending()
                            } else {
                                Availability::when(
                                    false,
                                    "Die Vorschau ist nicht bestätigt. Bitte erneut suchen.",
                                )
                            }
                        } else if csv && !self.csv_available().is_enabled() {
                            self.csv_available()
                        } else {
                            self.room_available(RoomSetting::Invite)
                        },
                        LocalAction::Room(Action::Confirm(ConversationAction::Invite(person.id))),
                        cx,
                    )),
            );
        }
        if panel.directory_loaded && panel.directory.as_slice().is_empty() {
            body = body.child(self.hint(self.l("Keine passenden Personen gefunden.")));
        }
        body
    }
}

fn bots_need_initial_load(panel: &ConversationPanel) -> bool {
    !panel.bots_loaded
        && panel.bots_state == RemoteState::Idle
        && !matches!(
            panel.state,
            RemoteState::Loading(_) | RemoteState::Saving(_) | RemoteState::Uncertain(_)
        )
}

fn csv_error_label(error: CsvFileError) -> &'static str {
    match error {
        CsvFileError::TooLarge => "Die CSV-Datei ist zu groß. Maximal 64 KiB sind erlaubt.",
        CsvFileError::InvalidUtf8 => "Die CSV-Datei muss als UTF-8 gespeichert sein.",
        CsvFileError::NotRegularFile => "Bitte wähle eine reguläre CSV-Datei aus.",
        CsvFileError::PickerUnavailable => {
            "Die Dateiauswahl konnte nicht geöffnet werden. Bitte erneut versuchen."
        }
        CsvFileError::TimedOut => {
            "Die CSV-Datei konnte nicht rechtzeitig gelesen werden. Bitte erneut versuchen."
        }
        CsvFileError::Unavailable(_) => "Der CSV-Import ist für dieses Gespräch nicht verfügbar.",
        CsvFileError::ReadFailed => {
            "Die CSV-Datei konnte nicht gelesen werden. Prüfe den Dateizugriff und versuche es erneut."
        }
    }
}

fn menu_next(current: Option<usize>, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match key {
        "home" => 0,
        "end" => count - 1,
        "up" => current.map_or(count - 1, |index| (index + count - 1) % count),
        _ => current.map_or(0, |index| (index + 1) % count),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::conversation::RoomResult;
    use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};

    fn context(account: u128, runtime: u128, host: u128, room: u128) -> ConversationRef {
        ConversationRef {
            account: AccountScope {
                account: AccountId::from_local(account),
                runtime: RuntimeGeneration::from_local(runtime),
            },
            conversation: ConversationId {
                account: AccountId::from_local(account),
                host: HostId::from_local(host),
                local: room,
            },
        }
    }
    fn person() -> PersonRef {
        PersonRef {
            context: context(1, 1, 1, 1),
            local: 42,
        }
    }
    fn member(role: MemberRole) -> Member {
        Member {
            id: person(),
            name: Text::new("Synthetic participant").unwrap(),
            role,
            source: DirectorySource::User,
            permissions: None,
            self_user: false,
        }
    }
    #[test]
    fn member_menu_generation_and_full_scope_reject_old_callbacks() {
        let mut flow = Flow::default();
        let first = flow.open_menu(person()).unwrap();
        assert!(flow.accepts(first, Some(person().context)));
        for scope in [
            context(2, 1, 1, 1),
            context(1, 2, 1, 1),
            context(1, 1, 2, 1),
            context(1, 1, 1, 2),
        ] {
            assert!(!flow.accepts(first, Some(scope)));
            assert_ne!(
                menu_id(person()),
                menu_id(PersonRef {
                    context: scope,
                    ..person()
                })
            );
        }
        flow.close_menu();
        assert!(!flow.accepts(first, Some(person().context)));
        let second = flow.open_menu(person()).unwrap();
        assert_ne!(first, second);
        assert!(!flow.accepts(first, Some(person().context)));
        flow.reset();
        let reopened = flow.open_menu(person()).unwrap();
        assert_ne!(second, reopened);
        assert_ne!(
            MemberChoice::Remove.id(second),
            MemberChoice::Remove.id(reopened)
        );
        flow.show(Page::Invite);
        assert!(!flow.accepts(reopened, Some(person().context)));
        assert!(flow.open_menu(person()).is_none());
    }
    #[test]
    fn menu_handles_retain_all_person_id_bits() {
        let low = person();
        let high = PersonRef {
            local: low.local | (1 << 100),
            ..low
        };
        assert_ne!(menu_id(low), menu_id(high));
        let mut flow = Flow {
            generation: u64::MAX,
            ..Default::default()
        };
        assert!(flow.open_menu(low).is_none());
        assert!(flow.menu.is_none());
    }
    #[test]
    fn roles_and_removal_revalidate_current_member_authority() {
        let mut panel = ConversationPanel::new(person().context);
        panel.members.try_push(member(MemberRole::Member)).unwrap();
        let promote = MemberChoice::Role(MemberRole::Moderator);
        assert!(member_choice_allowed(
            Some(&panel),
            Some(panel.context),
            person(),
            promote,
            true
        ));
        assert!(!member_choice_allowed(
            Some(&panel),
            Some(panel.context),
            person(),
            promote,
            false
        ));
        assert!(!member_choice_allowed(
            Some(&panel),
            Some(context(1, 2, 1, 1)),
            person(),
            promote,
            true
        ));
        panel.members = talk_core::ui::Page::empty();
        panel
            .members
            .try_push(member(MemberRole::Moderator))
            .unwrap();
        assert!(!member_choice_allowed(
            Some(&panel),
            Some(panel.context),
            person(),
            promote,
            true
        ));
        assert!(member_choice_allowed(
            Some(&panel),
            Some(panel.context),
            person(),
            MemberChoice::Role(MemberRole::Member),
            true
        ));
        panel.members = talk_core::ui::Page::empty();
        assert!(!member_choice_allowed(
            Some(&panel),
            Some(panel.context),
            person(),
            MemberChoice::Remove,
            true
        ));
        for role in [MemberRole::Owner, MemberRole::Unknown] {
            assert!(choices(&member(role)).is_empty());
        }
        let mut self_member = member(MemberRole::Member);
        self_member.self_user = true;
        assert!(choices(&self_member).is_empty());
        assert_eq!(
            choices(&member(MemberRole::Following)),
            [MemberChoice::Remove]
        );
        assert!(choices(&member(MemberRole::Guest)).contains(&promote));
        assert!(
            choices(&member(MemberRole::GuestModerator))
                .contains(&MemberChoice::Role(MemberRole::Member))
        );
    }
    #[test]
    fn invitation_preview_accepts_only_its_request_and_scope_and_does_not_cross_pages() {
        let mut flow = Flow::default();
        flow.show(Page::Invite);
        flow.preview(Some(person().context), Some(RequestId(7)));
        let mut panel = ConversationPanel::new(context(2, 1, 1, 1));
        panel.result = Some(RoomResult {
            request: RequestId(7),
            completion: Completion::ServerAccepted,
        });
        flow.observe(&panel);
        assert!(!flow.preview_ready());
        panel.context = person().context;
        panel.result.as_mut().unwrap().request = RequestId(6);
        flow.observe(&panel);
        assert!(!flow.preview_ready());
        assert!(!flow.preview_loading(RemoteState::Loading(RequestId(6))));
        assert!(flow.preview_loading(RemoteState::Loading(RequestId(7))));
        panel.result.as_mut().unwrap().request = RequestId(7);
        flow.observe(&panel);
        assert!(flow.preview_ready());
        flow.show(Page::Csv);
        flow.observe(&panel);
        assert!(!flow.preview_ready());
        flow.preview(Some(person().context), Some(RequestId(8)));
        panel.result = Some(RoomResult {
            request: RequestId(8),
            completion: Completion::Failed(talk_core::ui::UnavailableReason::PermissionDenied),
        });
        flow.observe(&panel);
        assert!(!flow.preview_ready());
        flow.reset();
        assert_eq!(flow.page, Page::People);
        assert!(flow.preview.is_none());
    }
    #[test]
    fn invitation_refresh_retains_rows_without_granting_unconfirmed_invites() {
        let current = person().context;
        let mut flow = Flow::default();
        flow.show(Page::Invite);
        flow.preview(Some(current), None);
        assert!(!flow.preview_visible());
        assert!(!flow.preview_loading(RemoteState::Loading(RequestId(7))));
        flow.preview(Some(current), Some(RequestId(7)));
        let mut panel = ConversationPanel::new(current);
        panel.result = Some(RoomResult {
            request: RequestId(7),
            completion: Completion::ServerAccepted,
        });
        flow.observe(&panel);
        assert!(flow.preview_ready());
        flow.preview(Some(current), None);
        assert!(
            !flow.preview_ready(),
            "rejected admission cannot reuse confirmed preview authority"
        );
        assert!(flow.preview_visible());
        assert!(!flow.preview_loading(RemoteState::Loading(RequestId(7))));
        flow.preview(Some(current), Some(RequestId(8)));
        assert!(flow.preview_visible());
        assert!(!flow.preview_ready());
        assert!(flow.preview_loading(RemoteState::Loading(RequestId(8))));
        panel.result = Some(RoomResult {
            request: RequestId(8),
            completion: Completion::Failed(talk_core::ui::UnavailableReason::PermissionDenied),
        });
        flow.observe(&panel);
        assert!(flow.preview_visible());
        assert!(!flow.preview_ready());
        assert!(!flow.preview_loading(RemoteState::Failed(
            RequestId(8),
            talk_core::ui::UnavailableReason::PermissionDenied,
        )));
        panel.result.as_mut().unwrap().completion = Completion::Uncertain;
        flow.observe(&panel);
        assert!(flow.preview_visible());
        assert!(!flow.preview_ready());
        flow.preview(Some(current), Some(RequestId(9)));
        assert!(flow.preview_visible());
        flow.preview(Some(context(2, 1, 1, 1)), Some(RequestId(10)));
        assert!(
            !flow.preview_visible(),
            "retained content cannot cross scopes"
        );
        flow.show(Page::Csv);
        assert!(!flow.preview_visible());
    }
    #[test]
    fn dismissal_unwinds_menu_then_confirmation_then_invitation_then_dialog() {
        let mut flow = Flow::default();
        flow.open_menu(person()).unwrap();
        assert_eq!(flow.dismissal(true), Dismissal::Menu);
        flow.close_menu();
        assert_eq!(flow.dismissal(true), Dismissal::Confirmation);
        assert_eq!(flow.dismissal(false), Dismissal::Dialog);
        for page in [Page::Invite, Page::Csv] {
            flow.show(page);
            assert_eq!(flow.dismissal(true), Dismissal::Confirmation);
            assert_eq!(flow.dismissal(false), Dismissal::Invitation);
            flow.reset();
            assert_eq!(flow.dismissal(false), Dismissal::Dialog);
        }
        flow.show(Page::Bots);
        assert_eq!(flow.dismissal(false), Dismissal::Dialog);
    }
    #[test]
    fn member_navigation_reuses_loaded_rows_and_bot_loading_has_its_own_owner() {
        for loaded in [false, true] {
            for allowed in [false, true] {
                assert_eq!(Page::Bots.navigation_request(loaded, allowed), None);
                assert_eq!(Page::Invite.navigation_request(loaded, allowed), None);
                assert_eq!(Page::Csv.navigation_request(loaded, allowed), None);
            }
        }
        assert_eq!(
            Page::People.navigation_request(false, true),
            Some(ConversationAction::Members)
        );
        assert_eq!(Page::People.navigation_request(true, true), None);
        assert_eq!(Page::People.navigation_request(false, false), None);
    }
    #[test]
    fn bot_initial_load_waits_for_admitted_room_work_and_does_not_repeat_failures() {
        let mut panel = ConversationPanel::new(person().context);
        assert!(bots_need_initial_load(&panel));
        for state in [
            RemoteState::Loading(RequestId(1)),
            RemoteState::Saving(RequestId(1)),
            RemoteState::Uncertain(RequestId(1)),
        ] {
            panel.state = state;
            assert!(!bots_need_initial_load(&panel));
        }
        panel.state = RemoteState::Idle;
        for state in [
            RemoteState::Loading(RequestId(1)),
            RemoteState::Saving(RequestId(1)),
            RemoteState::Failed(
                RequestId(1),
                talk_core::ui::UnavailableReason::PermissionDenied,
            ),
            RemoteState::Uncertain(RequestId(1)),
        ] {
            panel.bots_state = state;
            assert!(!bots_need_initial_load(&panel));
        }
        panel.bots_state = RemoteState::Idle;
        panel.bots_loaded = true;
        assert!(
            !bots_need_initial_load(&panel),
            "a confirmed empty list is loaded too"
        );
        let mut flow = Flow::default();
        flow.show(Page::Bots);
        flow.bots_requested = true;
        flow.show(Page::People);
        assert!(!flow.bots_requested);
    }
    #[test]
    fn csv_completion_requires_unchanged_editor_page_session_request_and_scope() {
        let current = person().context;
        let pick = CsvPick {
            context: current,
            request: RequestId(8),
            session: 2,
            edit_generation: 10,
        };
        assert!(pick.accepts(Some(current), Page::Csv, 2, RequestId(8), 10, 10));
        for scope in [
            context(2, 1, 1, 1),
            context(1, 2, 1, 1),
            context(1, 1, 2, 1),
            context(1, 1, 1, 2),
        ] {
            assert!(!pick.accepts(Some(scope), Page::Csv, 2, RequestId(8), 10, 10));
        }
        assert!(!pick.accepts(Some(current), Page::People, 2, RequestId(8), 10, 10));
        assert!(!pick.accepts(Some(current), Page::Csv, 3, RequestId(8), 10, 10));
        assert!(!pick.accepts(Some(current), Page::Csv, 2, RequestId(9), 10, 10));
        assert!(!pick.accepts(Some(current), Page::Csv, 2, RequestId(8), 11, 10));
        assert!(!pick.accepts(Some(current), Page::Csv, 2, RequestId(8), 10, 11));
        let mut flow = Flow::default();
        flow.show(Page::Csv);
        let old = flow.session;
        flow.reset();
        flow.show(Page::Csv);
        assert!(
            flow.session > old,
            "reopening cannot revive a previous picker"
        );
    }
    #[test]
    fn menu_keyboard_navigation_wraps_and_handles_empty_actions() {
        assert_eq!(menu_next(None, 0, "down"), None);
        assert_eq!(menu_next(None, 2, "down"), Some(0));
        assert_eq!(menu_next(Some(0), 2, "up"), Some(1));
        assert_eq!(menu_next(Some(1), 2, "down"), Some(0));
        assert_eq!(menu_next(Some(1), 2, "home"), Some(0));
        assert_eq!(menu_next(Some(0), 2, "end"), Some(1));
        assert_eq!(menu_next(Some(0), 1, "down"), Some(0));
    }
}
