//! Bounded local-clock requests. Platform timezone discovery belongs to the host.
use crate::bounded::Text;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepeatedTime {
    Ask,
    First,
    Second,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeadlineChoice {
    InThirtyMinutes,
    InOneHour,
    TomorrowMorning,
    NextMondayMorning,
    Custom {
        local: Text<32>,
        repeated: RepeatedTime,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeadlineMoment {
    pub timestamp: u64,
    /// Local system date and clock time, suitable for the editable field.
    pub local: Text<32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalTimeError {
    Unavailable,
    Invalid,
    Past,
    Skipped,
    Repeated,
}
