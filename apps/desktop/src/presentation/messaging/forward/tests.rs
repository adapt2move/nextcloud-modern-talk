use super::*;
use crate::presentation::feedback;
use std::collections::HashMap;
use talk_core::ui::presentation::{DeliveryState, NativeDesktopPort, PresentationSnapshot};
use talk_core::ui::{
    AccountId, AccountScope, ConversationId, HostId, Page, RuntimeGeneration, SubmitError,
};

struct Port {
    rejection: Option<SubmitError>,
    attempts: usize,
    last_intent: Option<UiIntent>,
}

#[test]
fn return_focus_resolves_remounted_invoker_and_rejects_newer_context_generation() {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Mount(u64);

    let account = AccountId::from_local(1);
    let context = ConversationRef {
        account: AccountScope {
            account,
            runtime: RuntimeGeneration::from_local(1),
        },
        conversation: ConversationId {
            account,
            host: HostId::from_local(1),
            local: 1,
        },
    };
    let control = scoped_id("message-menu", context, 7, 0);
    let prior = ReturnFocus {
        context,
        surface: ReturnSurface::CallChat,
        control: control.clone(),
    };
    let mut mounted = HashMap::from([(control.clone(), Mount(1))]);
    assert_eq!(
        prior.resolve(Some(context), Some(Overlay::CallChat), |id| mounted
            .get(id)
            .copied()),
        FocusReturn::Invoker(Mount(1))
    );

    // One Forward frame retires the unmounted CallChat control. Returning to
    // CallChat remounts the same scoped ID with a different handle generation.
    mounted.clear();
    assert_eq!(
        prior.resolve(Some(context), Some(Overlay::Forward), |id| mounted
            .get(id)
            .copied()),
        FocusReturn::Skip
    );
    mounted.insert(control, Mount(2));
    assert_eq!(
        prior.resolve(Some(context), Some(Overlay::CallChat), |id| mounted
            .get(id)
            .copied()),
        FocusReturn::Invoker(Mount(2))
    );

    let newer = ConversationRef {
        account: AccountScope {
            runtime: RuntimeGeneration::from_local(2),
            ..context.account
        },
        ..context
    };
    assert_eq!(
        prior.resolve::<Mount>(Some(newer), Some(Overlay::CallChat), |_| panic!(
            "A retired context cannot resolve focus"
        )),
        FocusReturn::Skip
    );
    assert_eq!(
        prior.resolve::<Mount>(Some(context), Some(Overlay::Thread), |_| panic!(
            "A different surface cannot resolve focus"
        )),
        FocusReturn::Skip
    );

    // An available different message must not become the missing invoker.
    mounted.clear();
    mounted.insert(scoped_id("message-menu", context, 8, 0), Mount(3));
    assert_eq!(
        prior.resolve(Some(context), Some(Overlay::CallChat), |id| mounted
            .get(id)
            .copied()),
        FocusReturn::Surface(ReturnSurface::CallChat)
    );
}
impl NativeDesktopPort for Port {
    fn try_submit(&mut self, intent: UiIntent) -> Result<(), SubmitError> {
        self.attempts += 1;
        self.last_intent = Some(intent);
        self.rejection.map_or(Ok(()), Err)
    }
    fn take_snapshot(&mut self) -> Option<PresentationSnapshot> {
        None
    }
}

#[test]
fn quiet_busy_admission_keeps_choice_and_real_forward_failures_stay_visible() {
    let account = AccountId::from_local(1);
    let context = ConversationRef {
        account: AccountScope {
            account,
            runtime: RuntimeGeneration::from_local(1),
        },
        conversation: ConversationId {
            account,
            host: HostId::from_local(1),
            local: 1,
        },
    };
    let destination = ConversationRef {
        conversation: ConversationId {
            local: 2,
            ..context.conversation
        },
        ..context
    };
    let original = MessageView {
        local_id: 7,
        author: Text::new("Synthetic sender").unwrap(),
        initials: Text::new("S").unwrap(),
        time_label: Text::new("10:00").unwrap(),
        body: Text::new("Approved source").unwrap(),
        own: false,
        delivery: DeliveryState::Received,
        attachments: Page::empty(),
        reply_count: 0,
        metadata: Default::default(),
    };
    let mut session = Session::default();
    let target = session.open(context, &original).unwrap();
    let mut port = Port {
        rejection: Some(SubmitError::Busy),
        attempts: 0,
        last_intent: None,
    };
    assert!(session.select(target, destination));
    assert_eq!(port.attempts, 0, "Selection does not admit a send");

    let intent = |request| UiIntent::Message {
        request: RequestId(request),
        context,
        action: MessageAction::Forward {
            message: target.message,
            destination,
        },
    };
    let mut notice = None;
    assert!(!feedback::submit(
        &mut port,
        &mut notice,
        intent(1),
        Some(Overlay::Forward)
    ));
    assert!(
        notice.is_none(),
        "Non-admission is quiet and retains the choice"
    );
    port.rejection = Some(SubmitError::Unavailable(
        talk_core::ui::UnavailableReason::PermissionDenied,
    ));
    assert!(!feedback::submit(
        &mut port,
        &mut notice,
        intent(2),
        Some(Overlay::Forward)
    ));
    let exact_failure = notice.unwrap();
    assert_eq!(exact_failure.context, Some(context));
    assert!(exact_failure == exact_failure.on_surface(Some(Overlay::Forward)));
    assert!(exact_failure != exact_failure.on_surface(None));
    assert!(exact_failure != exact_failure.on_surface(Some(Overlay::Thread)));
    assert_eq!(session.target(), Some(target));
    assert_eq!(
        session.active.as_ref().unwrap().destination,
        Some(destination)
    );
    assert_eq!(session.active.as_ref().unwrap().original, original);

    port.rejection = None;
    assert!(feedback::submit(
        &mut port,
        &mut notice,
        intent(3),
        Some(Overlay::Forward)
    ));
    assert!(
        notice == Some(exact_failure),
        "Admission alone cannot erase a genuine failure"
    );
    assert_eq!(port.attempts, 3);
    assert!(port.last_intent == Some(intent(3)));
}
