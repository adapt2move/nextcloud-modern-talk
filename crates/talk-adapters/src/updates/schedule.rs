//! Persist-before-request daily scheduling. One missed day produces at most one check.
use super::UpdateError;

pub(super) const DAY: u64 = 86_400;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Schedule {
    pub next: u64,
    pub last_day: Option<u64>,
}
impl Schedule {
    pub fn new(now: u64, random_second: u64) -> Self {
        let today = now / DAY;
        let slot = today
            .saturating_mul(DAY)
            .saturating_add(random_second % DAY);
        Self {
            next: if slot > now {
                slot
            } else {
                slot.saturating_add(DAY)
            },
            last_day: None,
        }
    }
    pub fn due(self, now: u64) -> bool {
        now >= self.next && self.last_day.is_none_or(|day| now / DAY > day)
    }
    pub fn claim(self, now: u64, random_second: u64) -> Self {
        let day = now / DAY;
        Self {
            next: day
                .saturating_add(1)
                .saturating_mul(DAY)
                .saturating_add(random_second % DAY),
            last_day: Some(day),
        }
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, UpdateError> {
        if bytes.len() > 256 {
            return Err(UpdateError::Storage);
        }
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| UpdateError::Storage)?;
        if value["schema"].as_u64() != Some(1) {
            return Err(UpdateError::Storage);
        }
        let next = value["next"]
            .as_u64()
            .filter(|v| *v > 0 && *v <= 4_102_444_799)
            .ok_or(UpdateError::Storage)?;
        let last_day = if value["last_day"].is_null() {
            None
        } else {
            Some(value["last_day"].as_u64().ok_or(UpdateError::Storage)?)
        };
        if last_day.is_some_and(|d| next / DAY <= d) {
            return Err(UpdateError::Storage);
        }
        Ok(Self { next, last_day })
    }
    pub fn encode(self) -> Result<Vec<u8>, UpdateError> {
        serde_json::to_vec(
            &serde_json::json!({"schema":1,"next":self.next,"last_day":self.last_day}),
        )
        .map_err(|_| UpdateError::Storage)
    }
}
pub(super) fn random_second() -> Result<u64, UpdateError> {
    // Rejection sampling keeps all seconds equiprobable (no modulo bias).
    for _ in 0..8 {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).map_err(|_| UpdateError::Unavailable)?;
        let value = u64::from_le_bytes(bytes);
        if value < u64::MAX - u64::MAX % DAY {
            return Ok(value % DAY);
        }
    }
    Err(UpdateError::Unavailable)
}
pub(super) fn now() -> Result<u64, UpdateError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| UpdateError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_manual_check_and_missed_days_do_not_reset_daily_claim() {
        let schedule = Schedule::new(20 * DAY + 40, 100);
        assert_eq!(schedule.next, 20 * DAY + 100);
        let restored = Schedule::decode(&schedule.encode().unwrap()).unwrap();
        assert!(!restored.due(20 * DAY + 99));
        assert!(restored.due(20 * DAY + 100));
        let claimed = restored.claim(24 * DAY + 500, 600);
        assert_eq!(claimed.next, 25 * DAY + 600);
        assert!(!claimed.due(24 * DAY + 900));
        assert!(!claimed.due(19 * DAY)); // clock rollback
        assert_eq!(
            Schedule::decode(&claimed.encode().unwrap()).unwrap(),
            claimed
        );
        // Manual work never calls claim, so its completion leaves the schedule intact.
        assert_eq!(restored, schedule);
    }
    #[test]
    fn first_past_slot_moves_to_tomorrow_and_corrupt_schedule_fails_closed() {
        assert_eq!(Schedule::new(40, 20).next, DAY + 20);
        for value in [
            b"{}".as_slice(),
            b"{\"schema\":1,\"next\":1,\"last_day\":1}",
            b"null",
        ] {
            assert_eq!(Schedule::decode(value), Err(UpdateError::Storage));
        }
    }
}
