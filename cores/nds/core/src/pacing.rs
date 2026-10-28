// SPDX-License-Identifier: GPL-3.0-or-later
use std::time::Duration;

/// Absolute deadlines retain small scheduler overruns without accumulating drift.
/// A large overrun rebases at completion, without adding an idle frame to a hitch.
#[derive(Default)]
pub(crate) struct FramePacer {
    deadline: Option<f64>,
}

impl FramePacer {
    pub fn reset(&mut self) {
        self.deadline = None;
    }

    pub fn begin_frame(&mut self, now: f64) {
        self.deadline.get_or_insert(now);
    }

    pub fn finish_frame(&mut self, now: f64, period: f64) -> Duration {
        let target = self.deadline.unwrap_or(now) + period;
        let target = if target < now - period { now } else { target };
        self.deadline = Some(target);
        Duration::from_secs_f64((target - now).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_work_counts_toward_its_budget() {
        let mut pacer = FramePacer::default();
        pacer.begin_frame(1.0);
        let sleep = pacer.finish_frame(1.010, 0.020);
        assert!((sleep.as_secs_f64() - 0.010).abs() < 1e-9);
    }

    #[test]
    fn small_overruns_retain_the_original_cadence() {
        let mut pacer = FramePacer::default();
        pacer.begin_frame(1.0);
        assert_eq!(pacer.finish_frame(1.022, 0.020), Duration::ZERO);
        pacer.begin_frame(1.022);
        let sleep = pacer.finish_frame(1.030, 0.020);
        assert!((sleep.as_secs_f64() - 0.010).abs() < 1e-9);
    }

    #[test]
    fn long_stall_does_not_add_an_extra_sleep_or_catchup_burst() {
        let mut pacer = FramePacer::default();
        pacer.begin_frame(1.0);
        assert_eq!(pacer.finish_frame(1.100, 0.020), Duration::ZERO);
        pacer.begin_frame(1.100);
        let sleep = pacer.finish_frame(1.105, 0.020);
        assert!((sleep.as_secs_f64() - 0.015).abs() < 1e-9);
    }

    #[test]
    fn resume_starts_a_fresh_budget() {
        let mut pacer = FramePacer::default();
        pacer.begin_frame(1.0);
        pacer.reset();
        pacer.begin_frame(5.0);
        let sleep = pacer.finish_frame(5.005, 0.020);
        assert!((sleep.as_secs_f64() - 0.015).abs() < 1e-9);
    }
}
