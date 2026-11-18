//! Deterministic simulation-only analyzer used by offline tests.
//!
//! Poll counts are logical steps, not elapsed time or ATS-2 timing behavior.
//! This module performs no USB, transport, hardware, or wall-clock operations.

use std::{collections::VecDeque, time::Duration};

use thiserror::Error;

use super::{config::Channel, measurement::Measurement};

/// Caller-configured simulated completion for one scenario.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MockScenario {
    pub channel: Channel,
    /// Number of `poll_ready` calls that return `false` before readiness.
    pub pending_polls: usize,
    /// Synthetic duration advanced only through `advance_time`; no sleeping occurs.
    pub delay: Duration,
    pub completion: Result<Measurement, MockFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockFailure {
    Timeout,
    Transport,
    External,
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum MockError {
    #[error("no simulated scenario remains")]
    NoScenario,
    #[error("a simulated scenario is already active")]
    Busy,
    #[error("no simulated scenario was triggered")]
    NotTriggered,
    #[error("simulated reading is not ready")]
    NotReady,
    #[error("next simulated scenario is configured for a different channel")]
    UnexpectedChannel,
    #[error("simulated scenario failed: {0:?}")]
    ScenarioFailure(MockFailure),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ActiveScenario {
    scenario: MockScenario,
    remaining_pending_polls: usize,
    remaining_delay: Duration,
    ready: bool,
}

/// Replayable sequence of explicitly configured synthetic readings.
///
/// Every instance is clearly a simulation and accepts no device handle. A new
/// instance created from the same scenarios produces the same poll/read
/// outcomes.
#[derive(Debug, Clone, PartialEq)]
pub struct MockAnalyzer {
    scenarios: VecDeque<MockScenario>,
    active: Option<ActiveScenario>,
}

impl MockAnalyzer {
    #[must_use]
    pub fn new(scenarios: impl IntoIterator<Item = MockScenario>) -> Self {
        Self {
            scenarios: scenarios.into_iter().collect(),
            active: None,
        }
    }

    /// Start the next configured scenario if its channel matches.
    pub fn trigger(&mut self, channel: Channel) -> Result<(), MockError> {
        if self.active.is_some() {
            return Err(MockError::Busy);
        }
        let Some(scenario) = self.scenarios.front() else {
            return Err(MockError::NoScenario);
        };
        if scenario.channel != channel {
            return Err(MockError::UnexpectedChannel);
        }
        let scenario = self.scenarios.pop_front().ok_or(MockError::NoScenario)?;
        self.active = Some(ActiveScenario {
            scenario,
            remaining_pending_polls: scenario.pending_polls,
            remaining_delay: scenario.delay,
            ready: false,
        });
        Ok(())
    }

    /// Advance synthetic time without sleeping or consulting a wall clock.
    pub fn advance_time(&mut self, elapsed: Duration) -> Result<(), MockError> {
        let active = self.active.as_mut().ok_or(MockError::NotTriggered)?;
        active.remaining_delay = active.remaining_delay.saturating_sub(elapsed);
        Ok(())
    }

    /// Advance one deterministic polling step; no wall-clock delay occurs.
    pub fn poll_ready(&mut self) -> Result<bool, MockError> {
        let active = self.active.as_mut().ok_or(MockError::NotTriggered)?;
        if active.remaining_pending_polls > 0 {
            active.remaining_pending_polls -= 1;
            return Ok(false);
        }
        if !active.remaining_delay.is_zero() {
            return Ok(false);
        }
        active.ready = true;
        Ok(true)
    }

    /// Complete the ready scenario with its configured reading or failure.
    pub fn read(&mut self) -> Result<Measurement, MockError> {
        let active = self.active.as_ref().ok_or(MockError::NotTriggered)?;
        if !active.ready {
            return Err(MockError::NotReady);
        }
        let active = self.active.take().ok_or(MockError::NotTriggered)?;
        active
            .scenario
            .completion
            .map_err(MockError::ScenarioFailure)
    }
}
