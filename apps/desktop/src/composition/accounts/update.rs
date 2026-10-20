//! A restart is admitted by the same serialized owner as calls, including native
//! notification actions. Rendering a hidden button is not an admission check.
use super::*;
use talk_core::preferences::updates::UpdateError;
use talk_core::ui::presentation::CallStage;

impl Accounts {
    pub(in crate::composition) fn seal_for_update(&mut self) -> Result<(), UpdateError> {
        if self
            .snapshot
            .call
            .as_ref()
            .is_some_and(|call| !matches!(call.stage, CallStage::Ended | CallStage::Failed))
            || self
                .pending
                .iter()
                .any(|pending| matches!(pending.purpose, Purpose::Call(_)))
            || self.media.capture().is_some()
            || self
                .media
                .snapshot()
                .is_some_and(|call| call.phase != talk_core::calls::Phase::Ended)
        {
            return Err(UpdateError::ActiveCall);
        }
        self.call_actions.seal_for_update()
    }

    /// Only used before the drain begins. A drained account cannot be reopened
    /// by releasing this seal; a new application lifetime restores it normally.
    pub(in crate::composition) fn abort_update_admission(&mut self) {
        self.call_actions.release_update_seal();
    }
}
