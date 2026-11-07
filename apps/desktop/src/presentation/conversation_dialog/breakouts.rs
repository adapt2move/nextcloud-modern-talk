//! Bounded, drag-free breakout controls. The room owner revalidates all roles.
use super::*;
use talk_core::ui::conversation::{BreakoutAssignment, BreakoutMode, DirectorySource, PersonRef};
use talk_core::ui::presentation::CallStage;

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Count(i8),
    ToggleMode,
    Mode(BreakoutMode),
    ToggleMember(PersonRef),
    Assign(PersonRef, u8),
    LoadMembers,
    LoadRooms,
    Configure,
    Apply,
    Start,
    Stop,
    Remove,
    Broadcast,
    Switch(ConversationRef),
    OpenAssigned(ConversationRef),
    Assistance,
}
pub(super) struct State {
    count: u8,
    mode: BreakoutMode,
    initialized: bool,
    mode_open: bool,
    member_open: Option<PersonRef>,
    assignments: Vec<BreakoutAssignment>,
    rooms_attempted: bool,
    count_observed: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            count: 2,
            mode: BreakoutMode::Automatic,
            initialized: false,
            mode_open: false,
            member_open: None,
            assignments: Vec::new(),
            rooms_attempted: false,
            count_observed: false,
        }
    }
}
fn mode_label(mode: BreakoutMode) -> &'static str {
    match mode {
        BreakoutMode::Automatic => "Automatisch zuweisen",
        BreakoutMode::Manual => "Manuell zuweisen",
        BreakoutMode::Free => "Freie Auswahl",
    }
}
fn assignments(
    items: impl IntoIterator<Item = BreakoutAssignment>,
) -> talk_core::ui::Page<BreakoutAssignment, 100> {
    let mut page = talk_core::ui::Page::empty();
    for item in items.into_iter().take(100) {
        let _ = page.try_push(item);
    }
    page
}
impl Desktop {
    pub(super) fn refresh_breakouts(&mut self) {
        if self.overlay != Some(Overlay::RoomInfo) || self.room_ui.tab != Tab::Breakouts {
            return;
        }
        let Some(panel) = self.room_panel() else {
            return;
        };
        let Some(details) = panel.details.as_ref().filter(|_| !panel.details_stale) else {
            return;
        };
        if !details.breakout_supported {
            return;
        }
        let mode = details.breakout_mode;
        let room_count = panel
            .breakout_loaded
            .then_some(panel.breakout_rooms.as_slice().len());
        let should_load = mode.is_some()
            && !details.breakout_child
            && !panel.breakout_loaded
            && !panel.state.pending()
            && !self.room_ui.breakouts.rooms_attempted;
        if !self.room_ui.breakouts.initialized {
            self.room_ui.breakouts.mode = mode.unwrap_or(BreakoutMode::Automatic);
            self.room_ui.breakouts.initialized = true;
        }
        if !self.room_ui.breakouts.count_observed
            && let Some(count) = room_count.filter(|count| (1..=20).contains(count))
        {
            self.room_ui.breakouts.count = count as u8;
            self.room_ui.breakouts.count_observed = true;
        }
        if should_load {
            self.room_ui.breakouts.rooms_attempted = true;
            self.submit_room(ConversationAction::BreakoutRooms);
        }
    }

    pub(super) fn breakout_action(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.overlay != Some(Overlay::RoomInfo) || self.room_ui.tab != Tab::Breakouts {
            return;
        }
        match action {
            Action::Count(delta) => {
                self.room_ui.breakouts.count = self
                    .room_ui
                    .breakouts
                    .count
                    .saturating_add_signed(delta)
                    .clamp(1, 20);
                self.room_ui
                    .breakouts
                    .assignments
                    .retain(|assignment| assignment.room < self.room_ui.breakouts.count);
            }
            Action::ToggleMode => {
                self.room_ui.breakouts.mode_open = !self.room_ui.breakouts.mode_open
            }
            Action::Mode(mode) => {
                self.room_ui.breakouts.mode = mode;
                self.room_ui.breakouts.mode_open = false;
            }
            Action::ToggleMember(member) => {
                self.room_ui.breakouts.member_open =
                    (self.room_ui.breakouts.member_open != Some(member)).then_some(member);
            }
            Action::Assign(member, room) => {
                if room < self.room_ui.breakouts.count {
                    self.room_ui
                        .breakouts
                        .assignments
                        .retain(|entry| entry.member != member);
                    if self.room_ui.breakouts.assignments.len() < 100 {
                        self.room_ui
                            .breakouts
                            .assignments
                            .push(BreakoutAssignment { member, room });
                    }
                }
                self.room_ui.breakouts.member_open = None;
            }
            Action::LoadMembers => self.submit_room(ConversationAction::Members),
            Action::LoadRooms => {
                self.room_ui.breakouts.rooms_attempted = true;
                self.submit_room(ConversationAction::BreakoutRooms);
            }
            Action::Configure => {
                let state = &self.room_ui.breakouts;
                let assignments = if state.mode == BreakoutMode::Manual {
                    assignments(state.assignments.iter().copied())
                } else {
                    talk_core::ui::Page::empty()
                };
                self.submit_room(ConversationAction::ConfigureBreakouts {
                    amount: state.count,
                    mode: state.mode,
                    assignments,
                });
            }
            Action::Apply => {
                let assignments = assignments(self.room_ui.breakouts.assignments.iter().copied());
                self.submit_room(ConversationAction::AssignBreakouts(assignments));
            }
            Action::Start => self.submit_room(ConversationAction::StartBreakouts),
            Action::Stop => self.submit_room(ConversationAction::StopBreakouts),
            Action::Remove => self.submit_room(ConversationAction::RemoveBreakouts),
            Action::Broadcast => {
                if let Some(message) = self
                    .room_ui
                    .breakout_broadcast
                    .read(cx)
                    .submitted_value()
                    .filter(|message| !message.trim().is_empty())
                    .and_then(|message| Text::new(message).ok())
                {
                    self.submit_room(ConversationAction::BroadcastBreakouts(message));
                }
            }
            Action::Switch(target) => self.submit_room(ConversationAction::SwitchBreakout(target)),
            Action::OpenAssigned(target) => {
                self.submit_room(ConversationAction::OpenAssignedBreakout(target))
            }
            Action::Assistance => self.submit_room(ConversationAction::RequestBreakoutAssistance),
        }
        cx.notify();
    }

    pub(super) fn breakout_section(&self, cx: &Context<Self>) -> Option<Div> {
        let panel = self.room_panel()?;
        let details = panel.details.as_ref().filter(|_| !panel.details_stale)?;
        let policy = details.availability(RoomSetting::Breakouts);
        if !details.breakout_supported
            || policy.server != FeatureSupport::Supported
            || !policy.implemented
        {
            return None;
        }
        let moderator = policy.enabled();
        let available = self.room_available(RoomSetting::Breakouts);
        let state = &self.room_ui.breakouts;
        let mut section = self.dialog_section(Some("Gruppenräume"));
        if details.breakout_child {
            if !details.classified && !panel.state.pending() {
                section = section.child(self.button(
                    "breakout-assistance",
                    "Hilfe anfordern",
                    ButtonStyle::Secondary,
                    true,
                    LocalAction::Room(super::Action::Breakouts(Action::Assistance)),
                    cx,
                ));
            }
            return Some(section);
        }
        if moderator {
            let mut mode = div()
                .flex_none()
                .w(self.sp(220.))
                .max_w(gpui::relative(0.55))
                .flex()
                .flex_col()
                .gap_1()
                .child(self.select_control(
                    "breakout-mode",
                    self.l(mode_label(state.mode)).to_owned(),
                    state.mode_open,
                    available,
                    LocalAction::Room(super::Action::Breakouts(Action::ToggleMode)),
                    cx,
                ));
            if state.mode_open {
                for item in [
                    BreakoutMode::Automatic,
                    BreakoutMode::Manual,
                    BreakoutMode::Free,
                ] {
                    mode = mode.child(self.button(
                        ("breakout-mode-option", item as usize),
                        mode_label(item),
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Room(super::Action::Breakouts(Action::Mode(item))),
                        cx,
                    ));
                }
            }
            section = section
                .child(self.dialog_row("Zuweisung", None, mode))
                .child(
                    self.dialog_row(
                        "Anzahl der Gruppenräume",
                        None,
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.icon_button(
                                "breakout-less",
                                ("minus", "Weniger Gruppenräume"),
                                ButtonStyle::Quiet,
                                Availability::when(
                                    state.count > 1 && available.is_enabled(),
                                    "Weniger Gruppenräume",
                                ),
                                LocalAction::Room(super::Action::Breakouts(Action::Count(-1))),
                                cx,
                            ))
                            .child(div().child(state.count.to_string()))
                            .child(self.icon_button(
                                "breakout-more",
                                ("plus", "Mehr Gruppenräume"),
                                ButtonStyle::Quiet,
                                Availability::when(
                                    state.count < 20 && available.is_enabled(),
                                    "Mehr Gruppenräume",
                                ),
                                LocalAction::Room(super::Action::Breakouts(Action::Count(1))),
                                cx,
                            )),
                    ),
                );
            if state.mode == BreakoutMode::Manual {
                if !panel.members_loaded {
                    section = section.child(self.button(
                        "breakout-load-members",
                        "Personen für die Zuweisung laden",
                        ButtonStyle::Secondary,
                        available,
                        LocalAction::Room(super::Action::Breakouts(Action::LoadMembers)),
                        cx,
                    ));
                } else {
                    for member in panel.members.as_slice().iter().filter(|member| {
                        member.source == DirectorySource::User && member.role == MemberRole::Member
                    }) {
                        let chosen = state
                            .assignments
                            .iter()
                            .find(|entry| entry.member == member.id)
                            .map(|entry| entry.room);
                        let expanded = state.member_open == Some(member.id);
                        let label = chosen
                            .map(|index| self.lf("Gruppe {n}", &[("n", &(index + 1).to_string())]))
                            .unwrap_or_else(|| self.l("Keine Auswahl").to_owned());
                        let mut selector = div()
                            .flex_none()
                            .w(self.sp(220.))
                            .max_w(gpui::relative(0.55))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(self.select_control(
                                gpui::ElementId::Name(
                                    format!("breakout-member-{:032x}", member.id.local).into(),
                                ),
                                label,
                                expanded,
                                available,
                                LocalAction::Room(super::Action::Breakouts(Action::ToggleMember(
                                    member.id,
                                ))),
                                cx,
                            ));
                        if expanded {
                            for index in 0..state.count {
                                selector = selector.child(
                                    self.button(
                                        gpui::ElementId::Name(
                                            format!(
                                                "breakout-option-{:032x}-{index}",
                                                member.id.local
                                            )
                                            .into(),
                                        ),
                                        self.lf("Gruppe {n}", &[("n", &(index + 1).to_string())]),
                                        ButtonStyle::Quiet,
                                        true,
                                        LocalAction::Room(super::Action::Breakouts(
                                            Action::Assign(member.id, index),
                                        )),
                                        cx,
                                    ),
                                );
                            }
                        }
                        section =
                            section.child(self.dialog_row(member.name.as_str(), None, selector));
                    }
                }
            }
            section = section.child(self.dialog_actions().child(self.button(
                "breakout-configure",
                "Gruppenräume vorbereiten",
                ButtonStyle::Secondary,
                available,
                LocalAction::Room(super::Action::Breakouts(Action::Configure)),
                cx,
            )));
            if details.breakout_mode.is_some() {
                section = section.child(
                    self.dialog_actions()
                        .child(self.button(
                            "breakout-start-stop",
                            if details.breakout_started == Some(true) {
                                "Gruppenräume beenden"
                            } else {
                                "Gruppenräume starten"
                            },
                            ButtonStyle::Secondary,
                            available,
                            LocalAction::Room(super::Action::Breakouts(
                                if details.breakout_started == Some(true) {
                                    Action::Stop
                                } else {
                                    Action::Start
                                },
                            )),
                            cx,
                        ))
                        .child(self.button(
                            "breakout-remove",
                            "Gruppenräume entfernen",
                            ButtonStyle::Quiet,
                            available,
                            LocalAction::Room(super::Action::Breakouts(Action::Remove)),
                            cx,
                        )),
                );
                if state.mode == BreakoutMode::Manual
                    && panel.breakout_loaded
                    && !state.assignments.is_empty()
                {
                    section = section.child(self.button(
                        "breakout-apply",
                        "Zuweisungen übernehmen",
                        ButtonStyle::Secondary,
                        available,
                        LocalAction::Room(super::Action::Breakouts(Action::Apply)),
                        cx,
                    ));
                }
                section = section.child(self.room_field(
                    "Nachricht an alle Gruppenräume",
                    &self.room_ui.breakout_broadcast,
                    Some(self.button(
                        "breakout-broadcast",
                        "An alle senden",
                        ButtonStyle::Secondary,
                        available,
                        LocalAction::Room(super::Action::Breakouts(Action::Broadcast)),
                        cx,
                    )),
                    cx,
                ));
            }
        }
        if details.breakout_mode.is_some() && (moderator || details.breakout_started == Some(true))
        {
            if !panel.breakout_loaded {
                section = section.child(self.button(
                    "breakout-load-rooms",
                    "Gruppenräume laden",
                    ButtonStyle::Quiet,
                    !panel.state.pending(),
                    LocalAction::Room(super::Action::Breakouts(Action::LoadRooms)),
                    cx,
                ));
            } else {
                for room in panel.breakout_rooms.as_slice() {
                    let can_switch = !moderator
                        && details.breakout_mode == Some(BreakoutMode::Free)
                        && details.breakout_started == Some(true);
                    let call_open = self.snapshot.call.as_ref().is_some_and(|call| {
                        !matches!(call.stage, CallStage::Ended | CallStage::Failed)
                    });
                    let mut row = div().flex().items_center().gap_2().child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(room.name.as_str().to_owned()),
                    );
                    if can_switch {
                        row = row.child(
                            self.button(
                                gpui::ElementId::Name(
                                    format!(
                                        "breakout-switch-{:032x}",
                                        room.context.conversation.local
                                    )
                                    .into(),
                                ),
                                if call_open {
                                    "Anruf verlassen und wechseln"
                                } else {
                                    "In Gruppenraum wechseln"
                                },
                                ButtonStyle::Quiet,
                                Availability::when(
                                    !panel.state.pending(),
                                    "Bitte warte, bis die Aktion abgeschlossen ist.",
                                ),
                                LocalAction::Room(super::Action::Breakouts(Action::Switch(
                                    room.context,
                                ))),
                                cx,
                            ),
                        );
                    } else if !moderator && room.assigned && details.breakout_started == Some(true)
                    {
                        row = row.child(
                            self.button(
                                gpui::ElementId::Name(
                                    format!(
                                        "breakout-assigned-{:032x}",
                                        room.context.conversation.local
                                    )
                                    .into(),
                                ),
                                if call_open {
                                    "Anruf verlassen und öffnen"
                                } else {
                                    "Zugewiesenen Gruppenraum öffnen"
                                },
                                ButtonStyle::Quiet,
                                !panel.state.pending(),
                                LocalAction::Room(super::Action::Breakouts(Action::OpenAssigned(
                                    room.context,
                                ))),
                                cx,
                            ),
                        );
                    }
                    section = section.child(row);
                }
            }
        }
        Some(section)
    }
}
