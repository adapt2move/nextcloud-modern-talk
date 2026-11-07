//! One room-scoped, keyboard-accessible person card. Member authority stays in
//! the account owner and is revalidated again before a direct room is created.
#[cfg(test)]
use super::state::tests::draft_snapshot;
use super::{
    ButtonStyle, Desktop, LocalAction,
    controls::{Availability, scoped_id},
};
use gpui::{Context, Div, FocusHandle, KeyDownEvent, Window, div, prelude::*, px};
use std::time::{Duration, Instant};
use talk_core::ui::{
    Decision,
    conversation::{CreateAction, DirectorySource, Member, MemberRole, PersonRef},
    presentation::{ConversationKind, ConversationRef, Presence, RequestId, UiIntent},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Source {
    Member(PersonRef),
    Message(ConversationRef, u128, bool),
}
impl Source {
    fn context(self) -> ConversationRef {
        match self {
            Self::Member(person) => person.context,
            Self::Message(context, _, _) => context,
        }
    }
    fn id(self) -> u128 {
        match self {
            Self::Member(person) => person.local,
            Self::Message(_, message, _) => message,
        }
    }
    fn ordinal(self) -> u128 {
        match self {
            Self::Member(_) => 0,
            Self::Message(_, _, false) => 1,
            Self::Message(_, _, true) => 2,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Target {
    source: Source,
    generation: u64,
}
#[derive(Clone)]
pub(crate) enum Action {
    Open(Source),
    Direct(Target),
    Compose(Target),
    Call(Target),
    Mention(Target),
}
struct PendingMention {
    context: ConversationRef,
    request: RequestId,
    author_key: talk_core::ui::Text<256>,
    edit_generation: u64,
}
struct PendingCall {
    source: ConversationRef,
    request: RequestId,
    target: Option<ConversationRef>,
    started: Instant,
}
#[derive(Debug, PartialEq)]
enum CallProgress {
    Wait,
    Cancel,
    Failed,
    Unavailable,
    Start(talk_core::ui::CallScope),
}
impl PendingCall {
    fn advance(
        &mut self,
        snapshot: &talk_core::ui::presentation::PresentationSnapshot,
        now: Instant,
    ) -> CallProgress {
        use talk_core::ui::presentation::{Completion, ContentState};
        if snapshot.active_account != Some(self.source.account) {
            return CallProgress::Cancel;
        }
        if now.saturating_duration_since(self.started) >= Duration::from_secs(30) {
            return CallProgress::Unavailable;
        }
        if self.target.is_none() {
            let result = snapshot
                .create_panel
                .as_ref()
                .filter(|panel| panel.account == self.source.account)
                .and_then(|panel| panel.result)
                .filter(|result| result.request == self.request);
            if let Some(result) = result {
                if result.completion != Completion::ServerAccepted {
                    return CallProgress::Failed;
                }
                let Some(target) = result
                    .conversation
                    .filter(|target| target.account == self.source.account)
                else {
                    return CallProgress::Cancel;
                };
                self.target = Some(target);
            } else if snapshot.selected != Some(self.source) {
                return CallProgress::Cancel;
            }
        }
        let Some(target) = self.target else {
            return CallProgress::Wait;
        };
        if snapshot.selected != Some(target) {
            return CallProgress::Cancel;
        }
        if snapshot.content == ContentState::Ready
            && let Some(row) = snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|row| row.context == target)
        {
            if row.kind == ConversationKind::Direct
                && let Some(super::CallEntry::Direct(call)) =
                    super::call_entry(row, snapshot.content)
            {
                return CallProgress::Start(call);
            }
            if row.call == Decision::Denied || row.kind != ConversationKind::Direct {
                return CallProgress::Unavailable;
            }
        }
        CallProgress::Wait
    }
}

pub(super) struct Card {
    target: Option<Target>,
    generation: u64,
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    fixture_opened: bool,
    mention: Option<PendingMention>,
    call: Option<PendingCall>,
}
impl Card {
    pub fn new(cx: &Context<Desktop>) -> Self {
        Self {
            target: None,
            generation: 0,
            focus: cx.focus_handle(),
            return_focus: None,
            fixture_opened: false,
            mention: None,
            call: None,
        }
    }
    pub fn show_fixture(&mut self, context: ConversationRef, message: u128) {
        self.generation = 1;
        self.fixture_opened = true;
        self.target = Some(Target {
            source: Source::Message(context, message, false),
            generation: 1,
        });
    }
    fn accepts(&self, target: Target, selected: Option<ConversationRef>) -> bool {
        self.target == Some(target) && selected == Some(target.source.context())
    }
}

fn role(role: MemberRole) -> &'static str {
    match role {
        MemberRole::Owner => "Eigentümer",
        MemberRole::Moderator | MemberRole::GuestModerator => "Moderation",
        MemberRole::Member => "Teilnehmend",
        MemberRole::Guest => "Gast",
        MemberRole::Following => "Folgt",
        MemberRole::Unknown => "Unbekannte Rolle",
    }
}
fn presence(status: Presence) -> &'static str {
    match status {
        Presence::Available => "Verfügbar",
        Presence::Away => "Abwesend",
        Presence::DoNotDisturb => "Nicht stören",
        Presence::Offline => "Offline",
        Presence::Unknown => "Status unbekannt",
    }
}
impl Desktop {
    pub(super) fn cancel_pending_person_call(&mut self) {
        self.people_card.call = None;
    }
    pub(super) fn person_member(&self, source: Source) -> Option<&Member> {
        let panel = self.snapshot.conversation_panel.as_ref().filter(|panel| {
            panel.context == source.context() && panel.members_loaded && !panel.details_stale
        })?;
        match source {
            Source::Member(person) => panel
                .members
                .as_slice()
                .iter()
                .find(|member| member.id == person),
            Source::Message(_, id, _) => {
                let key = self
                    .snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .find(|m| m.local_id == id)?
                    .metadata
                    .author_key
                    .as_ref()?;
                panel
                    .members
                    .as_slice()
                    .iter()
                    .find(|member| member.author_key.as_ref() == Some(key))
            }
        }
    }
    fn person_message(&self, source: Source) -> Option<&talk_core::ui::presentation::MessageView> {
        let Source::Message(context, id, _) = source else {
            return None;
        };
        (self.snapshot.selected == Some(context)).then_some(())?;
        self.snapshot
            .messages
            .as_slice()
            .iter()
            .find(|message| message.local_id == id && !message.metadata.system)
    }
    fn person_can_mention(&self, source: Source) -> bool {
        self.people_card.mention.is_none()
            && self
                .person_member(source)
                .is_some_and(|member| !member.self_user && member.author_key.is_some())
            && self
                .selected()
                .is_some_and(|row| row.context == source.context() && row.send == Decision::Allowed)
            && self.message_feature_enabled(talk_core::ui::messaging::MessageFeature::Mentions)
    }
    pub(super) fn complete_person_mention(
        &mut self,
        request: RequestId,
        completion: talk_core::ui::presentation::Completion,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.people_card.mention.take() else {
            return;
        };
        if pending.request != request {
            self.people_card.mention = Some(pending);
            return;
        }
        if self.snapshot.selected != Some(pending.context)
            || self.composer.read(cx).edit_generation() != pending.edit_generation
            || self.composer.read(cx).composing()
        {
            return;
        }
        let identifier = (completion == talk_core::ui::presentation::Completion::ServerAccepted)
            .then_some(self.snapshot.message_panel.as_ref())
            .flatten()
            .filter(|panel| panel.context == pending.context)
            .and_then(|panel| {
                let mut matches = panel
                    .mentions
                    .as_slice()
                    .iter()
                    .filter(|mention| mention.author_key.as_ref() == Some(&pending.author_key));
                let first = matches.next()?;
                matches.next().is_none().then(|| first.identifier.clone())
            });
        if let Some(identifier) = identifier {
            self.composer.update(cx, |input, cx| {
                input.insert_person_mention(identifier.as_str(), cx)
            });
        } else {
            self.feedback = Some(
                super::feedback::Feedback::error("Die Person kann gerade nicht erwähnt werden.")
                    .in_conversation(pending.context),
            );
        }
    }
    fn person_is_current_direct(&self, source: Source) -> bool {
        let other = self
            .person_member(source)
            .map(|m| !m.self_user)
            .or_else(|| self.person_message(source).map(|m| !m.own))
            .unwrap_or(false);
        other
            && self.selected().is_some_and(|row| {
                row.context == source.context() && row.kind == ConversationKind::Direct
            })
    }
    fn dismiss_person(&mut self, restore: bool, window: &mut Window) {
        self.people_card.target = None;
        if restore && let Some(focus) = self.people_card.return_focus.take() {
            focus.focus(window);
        } else {
            self.people_card.return_focus = None;
        }
    }
    pub(super) fn person_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            Action::Open(source) => {
                if self.snapshot.selected != Some(source.context())
                    || (self.person_member(source).is_none()
                        && self.person_message(source).is_none())
                {
                    return;
                }
                if self
                    .people_card
                    .mention
                    .as_ref()
                    .is_some_and(|pending| self.snapshot.selected != Some(pending.context))
                {
                    self.people_card.mention = None;
                }
                if self
                    .people_card
                    .target
                    .is_some_and(|target| target.source == source)
                {
                    self.dismiss_person(true, window);
                    return;
                }
                let Some(generation) = self.people_card.generation.checked_add(1) else {
                    return;
                };
                self.people_card.generation = generation;
                self.people_card.return_focus = window.focused(cx);
                self.people_card.target = Some(Target { source, generation });
                self.people_card.focus.focus(window);
                if matches!(source, Source::Message(..))
                    && self.person_member(source).is_none()
                    && self.snapshot.conversation_panel.as_ref().is_none_or(|p| {
                        p.context != source.context() || (!p.members_loaded && !p.state.pending())
                    })
                    && let Some(request) = self.room_ui.request.checked_add(1)
                {
                    self.room_ui.request = request;
                    self.submit(UiIntent::Conversation {
                        request: RequestId(request),
                        context: source.context(),
                        action: talk_core::ui::conversation::ConversationAction::Members,
                    });
                }
            }
            Action::Direct(target) => {
                if !self.people_card.accepts(target, self.snapshot.selected) {
                    return;
                }
                let person = self
                    .person_member(target.source)
                    .filter(|m| !m.self_user && m.source == DirectorySource::User)
                    .map(|m| m.id);
                if let Some(person) = person {
                    self.dismiss_person(false, window);
                    self.submit_creation(CreateAction::DirectMember(person));
                }
            }
            Action::Compose(target) => {
                if self.people_card.accepts(target, self.snapshot.selected)
                    && self.person_is_current_direct(target.source)
                    && self.composer_available()
                {
                    self.dismiss_person(false, window);
                    self.overlay = None;
                    self.composer_focus.focus(window);
                }
            }
            Action::Mention(target) => {
                if !self.people_card.accepts(target, self.snapshot.selected)
                    || !self.person_can_mention(target.source)
                {
                    return;
                }
                let Some((name, key)) = self.person_member(target.source).and_then(|member| {
                    member
                        .author_key
                        .as_ref()
                        .map(|key| (member.name.clone(), key.clone()))
                }) else {
                    return;
                };
                let context = target.source.context();
                self.dismiss_person(false, window);
                self.overlay = None;
                self.composer_focus.focus(window);
                let generation = self.composer.read(cx).edit_generation();
                if let Some(request) =
                    self.submit_message(talk_core::ui::messaging::MessageAction::Mentions(name))
                {
                    self.people_card.mention = Some(PendingMention {
                        context,
                        request,
                        author_key: key,
                        edit_generation: generation,
                    });
                }
            }
            Action::Call(target) => {
                if !self.people_card.accepts(target, self.snapshot.selected) {
                    return;
                }
                if self
                    .people_card
                    .call
                    .as_ref()
                    .is_some_and(|pending| pending.started.elapsed() >= Duration::from_secs(30))
                {
                    self.people_card.call = None;
                }
                if self.person_is_current_direct(target.source) {
                    if self.selected_call().is_some() {
                        self.dismiss_person(false, window);
                        self.overlay = None;
                        self.handle(LocalAction::Call, window, cx);
                    }
                } else if self.selected_call().is_some()
                    && self.people_card.call.is_none()
                    && let Some(person) = self
                        .person_member(target.source)
                        .filter(|m| !m.self_user && m.source == DirectorySource::User)
                        .map(|m| m.id)
                    && let Some(number) = self.room_ui.request.checked_add(1)
                {
                    self.room_ui.request = number;
                    self.dismiss_person(false, window);
                    self.overlay = None;
                    let request = RequestId(number);
                    if self.submit(UiIntent::Create {
                        account: person.context.account,
                        request,
                        action: CreateAction::DirectMember(person),
                    }) {
                        self.people_card.call = Some(PendingCall {
                            source: person.context,
                            request,
                            target: None,
                            started: Instant::now(),
                        });
                    }
                }
            }
        }
        cx.notify();
    }
    /// A group-card call first resolves the person's direct room. Only the
    /// matching result may continue, after that room's own call policy loads.
    pub(super) fn refresh_person_call(&mut self) {
        let Some(mut pending) = self.people_card.call.take() else {
            return;
        };
        if self.incoming_visible() {
            return;
        }
        match pending.advance(&self.snapshot, Instant::now()) {
            CallProgress::Wait => self.people_card.call = Some(pending),
            CallProgress::Cancel => {}
            CallProgress::Failed => {
                self.feedback = Some(super::feedback::Feedback::error(
                    "Das Gespräch konnte nicht geöffnet werden. Bitte neu laden und erneut versuchen.",
                ).in_conversation(pending.source));
            }
            CallProgress::Unavailable => {
                let feedback = super::feedback::Feedback::info(
                    "Anrufe sind für dieses Gespräch noch nicht verfügbar.",
                );
                self.feedback = Some(
                    pending
                        .target
                        .map_or(feedback, |target| feedback.in_conversation(target)),
                );
            }
            CallProgress::Start(call) => {
                self.submit(UiIntent::StartDirect {
                    call,
                    media: talk_core::ui::MediaIntent::MicrophoneOnCameraOff,
                });
            }
        }
    }
    pub(super) fn person_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.people_card.target.is_some() && event.keystroke.key == "escape" {
            self.dismiss_person(true, window);
            cx.notify();
            return true;
        }
        false
    }
    pub(super) fn prepare_person_card(&mut self, window: &Window) {
        if self.development
            && self.probe == super::probes::Probe::PeopleCard
            && !self.people_card.fixture_opened
            && window.is_window_active()
            && let Some(context) = self.snapshot.selected
        {
            self.people_card.show_fixture(context, 1);
        }
        if self.people_card.target.is_some_and(|target| {
            self.snapshot.selected != Some(target.source.context())
                || (self.person_member(target.source).is_none()
                    && self.person_message(target.source).is_none())
        }) || !window.is_window_active()
            || self.incoming_visible()
            || self.projection_blocked
        {
            self.people_card.target = None;
            self.people_card.return_focus = None;
        }
    }
    /// The trigger lives in the row; deferred anchoring escapes its clipping area.
    pub(super) fn person_trigger(&self, source: Source, content: Div, cx: &Context<Self>) -> Div {
        let target = self
            .people_card
            .target
            .filter(|target| target.source == source);
        let trigger = div().relative().child(
            self.control_with_geometry(
                scoped_id(
                    "person-trigger",
                    source.context(),
                    source.id(),
                    source.ordinal(),
                ),
                ButtonStyle::Quiet,
                Availability::available(),
                LocalAction::Person(Action::Open(source)),
                (0., 5.),
                cx,
            )
            .min_h(px(0.))
            .p_0()
            .justify_start()
            .child(content),
        );
        trigger.when_some(target, |trigger, target| {
            trigger.child(gpui::deferred(
                gpui::anchored()
                    .anchor(gpui::Corner::TopLeft)
                    .offset(gpui::point(px(0.), px(4.)))
                    .child(self.person_card(target, cx)),
            ))
        })
    }
    fn person_card(&self, target: Target, cx: &Context<Self>) -> Div {
        let source = target.source;
        let member = self.person_member(source);
        let message = self.person_message(source);
        let name = member
            .map(|m| m.name.as_str())
            .or_else(|| message.map(|m| m.author.as_str()))
            .unwrap_or("");
        let direct = self.person_is_current_direct(source);
        let row = self
            .selected()
            .filter(|row| direct && row.context == source.context());
        let state = member
            .map(|m| m.presence)
            .or_else(|| row.map(|r| r.presence))
            .unwrap_or(Presence::Unknown);
        let icon = member
            .and_then(|m| m.status_icon.as_ref())
            .or_else(|| row.and_then(|r| r.status_icon.as_ref()));
        let message_text = member
            .and_then(|m| m.status_message.as_ref())
            .or_else(|| row.and_then(|r| r.status_message.as_ref()));
        let initials: String = name
            .split_whitespace()
            .take(2)
            .filter_map(|s| s.chars().next())
            .flat_map(char::to_uppercase)
            .collect();
        let can_direct = member.is_some_and(|m| !m.self_user && m.source == DirectorySource::User);
        let mut card = self
            .context_menu_surface()
            .id(scoped_id(
                "person-card",
                source.context(),
                source.id(),
                target.generation.into(),
            ))
            .track_focus(&self.people_card.focus)
            .key_context("PersonCard")
            .occlude()
            .w(px(300.))
            .on_mouse_down_out(
                cx.listener(move |this, _: &gpui::MouseDownEvent, window, cx| {
                    if this.people_card.accepts(target, this.snapshot.selected) {
                        this.dismiss_person(false, window);
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .child(self.avatar_with_presence(
                        self.person_avatar(&initials, 48.),
                        state,
                        12.,
                        self.theme.surface,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_ellipsis()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .child(name.to_owned()),
                                    )
                                    .when_some(member, |d, member| {
                                        d.child(
                                            div()
                                                .flex_none()
                                                .rounded(px(5.))
                                                .bg(self.theme.subtle)
                                                .px_1()
                                                .text_size(self.sp(11.))
                                                .text_color(self.theme.muted)
                                                .child(self.l(role(member.role))),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .when(state != Presence::Unknown, |d| {
                                        d.child(
                                            div()
                                                .size(px(8.))
                                                .rounded_full()
                                                .bg(self.theme.presence_dot(state)),
                                        )
                                    })
                                    .child(self.hint(self.l(presence(state)))),
                            ),
                    ),
            )
            .when(icon.is_some() || message_text.is_some(), |d| {
                d.child(
                    div().px_3().pb_2().child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(px(8.))
                            .bg(self.theme.subtle)
                            .px_3()
                            .py_2()
                            .when_some(icon, |d, icon| {
                                let mut value = icon.as_str().to_owned();
                                if value.chars().count() == 1 {
                                    value.push('\u{fe0f}');
                                }
                                d.child(
                                    div()
                                        .flex_none()
                                        .font_family(emoji_font())
                                        .text_size(self.sp(18.))
                                        .child(value),
                                )
                            })
                            .when_some(message_text, |d, message| {
                                d.child(
                                    div()
                                        .min_w_0()
                                        .text_size(self.sp(13.))
                                        .child(message.as_str().to_owned()),
                                )
                            }),
                    ),
                )
            });
        if direct && self.composer_available() {
            card = card.child(
                self.button_with_leading_icon(
                    "person-compose",
                    ("Nachricht senden", "chat"),
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Person(Action::Compose(target)),
                    cx,
                )
                .w_full()
                .justify_start(),
            );
        } else if can_direct {
            card = card.child(
                self.button_with_leading_icon(
                    "person-direct",
                    ("Nachricht senden", "chat"),
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Person(Action::Direct(target)),
                    cx,
                )
                .w_full()
                .justify_start(),
            );
        }
        if (direct && row.is_some_and(|r| r.call == Decision::Allowed))
            || (!direct && can_direct && self.selected_call().is_some())
        {
            card = card.child(
                self.button_with_leading_icon(
                    "person-call",
                    ("Anrufen", "phone"),
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Person(Action::Call(target)),
                    cx,
                )
                .w_full()
                .justify_start(),
            );
        }
        if self.person_can_mention(source) {
            card = card.child(
                self.button_with_leading_icon(
                    "person-mention",
                    ("Erwähnen", "at"),
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Person(Action::Mention(target)),
                    cx,
                )
                .w_full()
                .justify_start(),
            );
        }
        div().child(card)
    }
}

fn emoji_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Apple Color Emoji"
    } else if cfg!(target_os = "windows") {
        "Segoe UI Emoji"
    } else {
        "Noto Color Emoji"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::conversation::{CreatePanel, CreationResult};
    use talk_core::ui::presentation::Completion;

    #[test]
    fn group_card_call_requires_matching_creation_and_current_direct_permission() {
        let mut snapshot = draft_snapshot();
        let source = snapshot.selected.unwrap();
        let mut direct = snapshot.conversations.as_slice()[0].clone();
        direct.context.conversation.local = 2;
        direct.kind = ConversationKind::Direct;
        let target = direct.context;
        direct.call_scope = Some(talk_core::ui::CallScope {
            account: target.account,
            conversation: target.conversation,
            call: talk_core::ui::CallGeneration::from_local(1),
        });
        let mut rooms = talk_core::ui::Page::empty();
        rooms
            .try_push(snapshot.conversations.as_slice()[0].clone())
            .unwrap();
        rooms.try_push(direct).unwrap();
        snapshot.conversations = rooms;
        let now = Instant::now();
        let mut pending = PendingCall {
            source,
            request: RequestId(5),
            target: None,
            started: now,
        };
        let mut panel = CreatePanel::new(source.account);
        panel.result = Some(CreationResult {
            request: RequestId(4),
            completion: Completion::ServerAccepted,
            conversation: Some(target),
        });
        snapshot.create_panel = Some(panel);
        assert_eq!(pending.advance(&snapshot, now), CallProgress::Wait);
        snapshot.selected = Some(target);
        assert_eq!(pending.advance(&snapshot, now), CallProgress::Cancel);
        snapshot
            .create_panel
            .as_mut()
            .unwrap()
            .result
            .as_mut()
            .unwrap()
            .request = RequestId(5);
        snapshot.conversations = {
            let mut rows = snapshot.conversations.as_slice().to_vec();
            rows.iter_mut()
                .find(|row| row.context == target)
                .unwrap()
                .call = Decision::Unknown;
            let mut result = talk_core::ui::Page::empty();
            for row in rows {
                result.try_push(row).unwrap();
            }
            result
        };
        assert_eq!(pending.advance(&snapshot, now), CallProgress::Wait);
        let mut rows = snapshot.conversations.as_slice().to_vec();
        let row = rows.iter_mut().find(|row| row.context == target).unwrap();
        row.call = Decision::Denied;
        let mut result = talk_core::ui::Page::empty();
        for row in rows {
            result.try_push(row).unwrap();
        }
        snapshot.conversations = result;
        assert_eq!(pending.advance(&snapshot, now), CallProgress::Unavailable);
        let mut rows = snapshot.conversations.as_slice().to_vec();
        let row = rows.iter_mut().find(|row| row.context == target).unwrap();
        row.call = Decision::Allowed;
        let expected = row.call_scope.unwrap();
        let mut result = talk_core::ui::Page::empty();
        for row in rows {
            result.try_push(row).unwrap();
        }
        snapshot.conversations = result;
        assert_eq!(
            pending.advance(&snapshot, now),
            CallProgress::Start(expected)
        );
        snapshot.selected = Some(source);
        assert_eq!(pending.advance(&snapshot, now), CallProgress::Cancel);
        snapshot.selected = Some(target);
        assert_eq!(
            pending.advance(&snapshot, now + Duration::from_secs(30)),
            CallProgress::Unavailable
        );
    }
}
