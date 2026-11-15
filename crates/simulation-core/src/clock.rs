use serde::{Deserialize, Serialize};

/// Operating status of the simulation clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClockStatus {
    /// Normal operation, all systems running at full rate.
    Normal,
    /// Overrun threshold exceeded; low priority work reduced.
    Degraded,
    /// Data invariant or critical error occurred; simulation paused.
    Paused,
}

/// Fixed-rate simulation clock maintaining logical ticks and overrun telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationClock {
    pub tick_rate_hz: u32,
    pub fixed_delta_ms: u32,
    pub current_tick: u64,
    pub overrun_count: u64,
    pub consecutive_overruns: u32,
    pub status: ClockStatus,
}

impl SimulationClock {
    /// Constructs a standard 10Hz clock (100ms per logical tick).
    #[must_use]
    pub const fn new(tick_rate_hz: u32) -> Self {
        let fixed_delta_ms = if tick_rate_hz > 0 {
            1000 / tick_rate_hz
        } else {
            100
        };
        Self {
            tick_rate_hz,
            fixed_delta_ms,
            current_tick: 0,
            overrun_count: 0,
            consecutive_overruns: 0,
            status: ClockStatus::Normal,
        }
    }

    /// Advances the logical tick counter by 1.
    ///
    /// Even when overruns occur, the tick number is monotonically incremented
    /// without skipping any tick number.
    pub fn advance_tick(&mut self) -> u64 {
        self.current_tick += 1;
        self.current_tick
    }

    /// Records an execution overrun (processing took longer than fixed_delta_ms).
    pub fn record_overrun(&mut self) {
        self.overrun_count += 1;
        self.consecutive_overruns += 1;
        if self.consecutive_overruns >= 5 && self.status == ClockStatus::Normal {
            self.status = ClockStatus::Degraded;
        }
    }

    /// Records a normal on-time execution tick.
    pub fn record_on_time(&mut self) {
        self.consecutive_overruns = 0;
        if self.status == ClockStatus::Degraded {
            self.status = ClockStatus::Normal;
        }
    }

    /// Transitions clock to paused state.
    pub fn pause(&mut self) {
        self.status = ClockStatus::Paused;
    }
}

impl Default for SimulationClock {
    fn default() -> Self {
        Self::new(10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_advances_monotonically_without_skips() {
        let mut clock = SimulationClock::default();
        for i in 1..=100 {
            assert_eq!(clock.advance_tick(), i);
        }
        assert_eq!(clock.current_tick, 100);
    }

    #[test]
    fn test_overrun_degradation_and_recovery() {
        let mut clock = SimulationClock::default();
        for _ in 0..4 {
            clock.record_overrun();
            assert_eq!(clock.status, ClockStatus::Normal);
        }
        clock.record_overrun();
        assert_eq!(clock.status, ClockStatus::Degraded);

        clock.record_on_time();
        assert_eq!(clock.status, ClockStatus::Normal);
    }
}
