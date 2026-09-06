//! Pure adaptive sampling policy (T-308). Platform input-idle measurement is
//! intentionally outside this module; the capture owner supplies a duration.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplingPolicy {
    pub active_interval: Duration,
    pub idle_interval: Duration,
    pub idle_after: Duration,
    pub deep_idle_after: Duration,
    pub forced_capture_after: Duration,
}

impl Default for SamplingPolicy {
    fn default() -> Self {
        Self {
            active_interval: Duration::from_secs(2),
            idle_interval: Duration::from_secs(15),
            idle_after: Duration::from_secs(60),
            deep_idle_after: Duration::from_secs(5 * 60),
            forced_capture_after: Duration::from_secs(2 * 60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingDecision {
    CaptureNow,
    Wait(Duration),
    DeepIdle,
}

impl SamplingPolicy {
    /// Choose a deterministic cadence from an input-idle observation and time
    /// since the last successful capture. A forced interval retains sparse
    /// continuity while an operator is active but visually unchanged.
    pub fn decide(&self, input_idle: Duration, since_capture: Duration) -> SamplingDecision {
        if input_idle >= self.deep_idle_after {
            return SamplingDecision::DeepIdle;
        }
        if since_capture >= self.forced_capture_after {
            return SamplingDecision::CaptureNow;
        }
        let interval = if input_idle >= self.idle_after {
            self.idle_interval
        } else {
            self.active_interval
        };
        match interval.checked_sub(since_capture) {
            Some(wait) if !wait.is_zero() => SamplingDecision::Wait(wait),
            _ => SamplingDecision::CaptureNow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_idle_timeline_has_active_idle_forced_and_deep_idle_states() {
        let policy = SamplingPolicy::default();
        assert_eq!(
            policy.decide(Duration::from_secs(0), Duration::from_secs(1)),
            SamplingDecision::Wait(Duration::from_secs(1))
        );
        assert_eq!(
            policy.decide(Duration::from_secs(61), Duration::from_secs(1)),
            SamplingDecision::Wait(Duration::from_secs(14))
        );
        assert_eq!(
            policy.decide(Duration::from_secs(61), Duration::from_secs(120)),
            SamplingDecision::CaptureNow
        );
        assert_eq!(
            policy.decide(Duration::from_secs(300), Duration::from_secs(120)),
            SamplingDecision::DeepIdle
        );
    }
}
