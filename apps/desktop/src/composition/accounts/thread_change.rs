//! One scoped optimistic thread change, reconciled by its correlated receipt.
use talk_core::ui::{
    Text,
    conversation::RemoteState,
    messaging::MessagePanel,
    presentation::{Completion, RequestId},
};

pub(super) enum Change {
    Title { root: u128, title: Text<256> },
    Notifications { root: u128, level: i8 },
}

pub(super) fn begin(panel: &mut MessagePanel, request: RequestId, change: Change) {
    let root = match &change {
        Change::Title { root, .. } | Change::Notifications { root, .. } => *root,
    };
    let Some(details) = panel
        .thread_details
        .as_mut()
        .filter(|details| panel.thread_root == Some(root) && details.root == root)
    else {
        return;
    };
    let following = match &change {
        Change::Notifications { level, .. } => Some(*level != 3),
        Change::Title { .. } => None,
    };
    panel.thread_change_prior = Some((request, details.clone(), following));
    // A detail read started before the write cannot overwrite its projection.
    panel.thread_details_request = None;
    match change {
        Change::Title { title, .. } => details.title = title,
        Change::Notifications { level, .. } => details.notification_level = Some(level),
    }
}

pub(super) fn settle(panel: &mut MessagePanel, request: RequestId, completion: Completion) {
    let Some((_, prior, _)) = panel
        .thread_change_prior
        .take_if(|(pending, _, _)| *pending == request)
    else {
        return;
    };
    if completion != Completion::ServerAccepted && panel.thread_root == Some(prior.root) {
        panel.thread_details = Some(prior);
    }
}

pub(super) fn cancel(panel: &mut MessagePanel) {
    if let Some((request, _, _)) = panel.thread_change_prior.take()
        && panel.state == RemoteState::Saving(request)
    {
        panel.state = RemoteState::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::UnavailableReason;
    use talk_core::ui::messaging::ThreadDetails;

    #[test]
    fn optimistic_change_rolls_back_only_its_own_failure_and_cancels_on_navigation() {
        let context = crate::composition::fixtures::context(1);
        let mut panel = MessagePanel::new(context);
        panel.thread_root = Some(7);
        panel.thread_details = Some(ThreadDetails {
            root: 7,
            title: Text::new("Before").unwrap(),
            notification_level: Some(0),
            rename: talk_core::ui::Decision::Allowed,
        });
        panel.thread_details_request = Some(RequestId(1));
        panel.state = RemoteState::Saving(RequestId(2));
        begin(
            &mut panel,
            RequestId(2),
            Change::Title {
                root: 7,
                title: Text::new("After").unwrap(),
            },
        );
        assert_eq!(
            panel.thread_details.as_ref().unwrap().title.as_str(),
            "After"
        );
        assert!(panel.thread_details_request.is_none());
        settle(
            &mut panel,
            RequestId(1),
            Completion::Failed(UnavailableReason::PolicyStale),
        );
        assert_eq!(
            panel.thread_details.as_ref().unwrap().title.as_str(),
            "After"
        );
        settle(
            &mut panel,
            RequestId(2),
            Completion::Failed(UnavailableReason::PolicyStale),
        );
        assert_eq!(
            panel.thread_details.as_ref().unwrap().title.as_str(),
            "Before"
        );

        begin(
            &mut panel,
            RequestId(3),
            Change::Notifications { root: 7, level: 3 },
        );
        panel.state = RemoteState::Saving(RequestId(3));
        assert_eq!(
            panel.thread_details.as_ref().unwrap().notification_level,
            Some(3)
        );
        cancel(&mut panel);
        assert_eq!(panel.state, RemoteState::Idle);
        assert!(panel.thread_change_prior.is_none());
    }
}
