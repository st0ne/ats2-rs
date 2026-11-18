//! Pure software planner for the documented RMS LevelTrig/LevelReady/LevelRdg flow.
//!
//! The planner emits logical action intents only. It performs no I/O and does
//! not assign durations, autorange cycles, overload semantics, or physical units.

use thiserror::Error;

use super::config::Channel;

/// ATS-1.5 software invalid/error sentinel from the statically analyzed read path.
pub const INVALID_READING_SENTINEL: f64 = -1.0e34;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionAction {
    Trigger(Channel),
    PollReady(Channel),
    Read(Channel),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionState {
    NotStarted,
    AwaitingTriggerReturn,
    PollingReady,
    AwaitingRead,
    Finished,
    Failed(AcquisitionFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionFailure {
    CallerReportedTimeout,
    ExternalFailure,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeterReading {
    /// Numeric API result that did not equal the known software error sentinel.
    /// It has no inferred unit, calibration, overload, or quality guarantee.
    Numeric(f64),
    /// The exact software error sentinel; not an overload classification.
    InvalidSentinel,
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum AcquisitionError {
    #[error("event is not valid in the current acquisition state")]
    UnexpectedEvent,
    #[error("ready count must be zero or positive, got {0}")]
    InvalidReadyCount(i32),
}

/// Stateful, deterministic software-only plan for one level reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RmsAcquisitionPlan {
    channel: Channel,
    state: AcquisitionState,
}

impl RmsAcquisitionPlan {
    #[must_use]
    pub const fn new(channel: Channel) -> Self {
        Self {
            channel,
            state: AcquisitionState::NotStarted,
        }
    }

    #[must_use]
    pub const fn state(&self) -> AcquisitionState {
        self.state
    }

    pub fn begin(&mut self) -> Result<AcquisitionAction, AcquisitionError> {
        if self.state != AcquisitionState::NotStarted {
            return Err(AcquisitionError::UnexpectedEvent);
        }
        self.state = AcquisitionState::AwaitingTriggerReturn;
        Ok(AcquisitionAction::Trigger(self.channel))
    }

    /// Call when the external LevelTrig API operation has returned.
    pub fn trigger_returned(&mut self) -> Result<AcquisitionAction, AcquisitionError> {
        if self.state != AcquisitionState::AwaitingTriggerReturn {
            return Err(AcquisitionError::UnexpectedEvent);
        }
        self.state = AcquisitionState::PollingReady;
        Ok(AcquisitionAction::PollReady(self.channel))
    }

    /// Interpret the documented Ready count: zero repeats polling; positive
    /// advances to read. Negative counts are not assigned invented semantics.
    pub fn ready_count(&mut self, count: i32) -> Result<AcquisitionAction, AcquisitionError> {
        if self.state != AcquisitionState::PollingReady {
            return Err(AcquisitionError::UnexpectedEvent);
        }
        if count < 0 {
            return Err(AcquisitionError::InvalidReadyCount(count));
        }
        if count == 0 {
            Ok(AcquisitionAction::PollReady(self.channel))
        } else {
            self.state = AcquisitionState::AwaitingRead;
            Ok(AcquisitionAction::Read(self.channel))
        }
    }

    /// Classify the returned numeric API value only against the exact known
    /// error sentinel. Overload and physical reading validity remain unknown.
    pub fn reading(&mut self, value: f64) -> Result<MeterReading, AcquisitionError> {
        if self.state != AcquisitionState::AwaitingRead {
            return Err(AcquisitionError::UnexpectedEvent);
        }
        self.state = AcquisitionState::Finished;
        if value == INVALID_READING_SENTINEL {
            Ok(MeterReading::InvalidSentinel)
        } else {
            Ok(MeterReading::Numeric(value))
        }
    }

    /// Record a timeout reported by the caller's own clock. This planner does
    /// not choose or measure a timeout duration.
    pub fn timeout(&mut self) -> Result<(), AcquisitionError> {
        if !matches!(
            self.state,
            AcquisitionState::PollingReady | AcquisitionState::AwaitingRead
        ) {
            return Err(AcquisitionError::UnexpectedEvent);
        }
        self.state = AcquisitionState::Failed(AcquisitionFailure::CallerReportedTimeout);
        Ok(())
    }

    /// Record an external failure without interpreting its device semantics.
    pub fn fail(&mut self) -> Result<(), AcquisitionError> {
        if !matches!(
            self.state,
            AcquisitionState::AwaitingTriggerReturn
                | AcquisitionState::PollingReady
                | AcquisitionState::AwaitingRead
        ) {
            return Err(AcquisitionError::UnexpectedEvent);
        }
        self.state = AcquisitionState::Failed(AcquisitionFailure::ExternalFailure);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AcquisitionAction, AcquisitionError, AcquisitionFailure, AcquisitionState,
        INVALID_READING_SENTINEL, MeterReading, RmsAcquisitionPlan,
    };
    use crate::analyzer::config::Channel;

    #[test]
    fn documented_trigger_poll_read_order_repeats_until_ready() {
        let mut plan = RmsAcquisitionPlan::new(Channel::A);

        assert_eq!(plan.begin(), Ok(AcquisitionAction::Trigger(Channel::A)));
        assert_eq!(
            plan.trigger_returned(),
            Ok(AcquisitionAction::PollReady(Channel::A))
        );
        assert_eq!(
            plan.ready_count(0),
            Ok(AcquisitionAction::PollReady(Channel::A))
        );
        assert_eq!(plan.ready_count(3), Ok(AcquisitionAction::Read(Channel::A)));
    }

    #[test]
    fn successful_read_is_numeric_but_does_not_claim_physical_units() {
        let mut plan = ready_to_read();

        assert_eq!(plan.reading(0.75), Ok(MeterReading::Numeric(0.75)));
        assert_eq!(plan.state(), AcquisitionState::Finished);
    }

    #[test]
    fn exact_software_error_sentinel_is_invalid_not_overload() {
        let mut plan = ready_to_read();

        assert_eq!(
            plan.reading(INVALID_READING_SENTINEL),
            Ok(MeterReading::InvalidSentinel)
        );
        assert_eq!(plan.state(), AcquisitionState::Finished);
    }

    #[test]
    fn negative_ready_counts_and_out_of_order_events_fail_closed() {
        let mut plan = RmsAcquisitionPlan::new(Channel::B);

        assert_eq!(plan.ready_count(1), Err(AcquisitionError::UnexpectedEvent));
        plan.begin().unwrap();
        plan.trigger_returned().unwrap();
        assert_eq!(
            plan.ready_count(-1),
            Err(AcquisitionError::InvalidReadyCount(-1))
        );
        assert_eq!(plan.state(), AcquisitionState::PollingReady);
    }

    #[test]
    fn caller_reported_timeout_ends_an_active_poll_without_inventing_duration() {
        let mut plan = RmsAcquisitionPlan::new(Channel::A);
        plan.begin().unwrap();
        plan.trigger_returned().unwrap();

        assert_eq!(plan.timeout(), Ok(()));
        assert_eq!(
            plan.state(),
            AcquisitionState::Failed(AcquisitionFailure::CallerReportedTimeout)
        );
    }

    #[test]
    fn external_failure_is_preserved_as_a_generic_failure() {
        let mut plan = ready_to_read();

        assert_eq!(plan.fail(), Ok(()));
        assert_eq!(
            plan.state(),
            AcquisitionState::Failed(AcquisitionFailure::ExternalFailure)
        );
    }

    fn ready_to_read() -> RmsAcquisitionPlan {
        let mut plan = RmsAcquisitionPlan::new(Channel::A);
        plan.begin().unwrap();
        plan.trigger_returned().unwrap();
        plan.ready_count(1).unwrap();
        plan
    }
}
