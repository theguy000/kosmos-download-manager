use std::collections::VecDeque;
use std::time::Duration;
use tokio::time::Instant;

/// Displayed speed is the average over this trailing window.
const WINDOW: Duration = Duration::from_secs(6);
/// Ticks run every 200 ms; a longer gap means paused or stalled, so old samples are stale.
const GAP: Duration = Duration::from_secs(1);
/// Spans shorter than this are too noisy to divide by.
const MIN_SPAN_SECS: f64 = 0.4;

/// Trailing-window speed, so one slow or bursty tick moves it gradually instead of jumping.
#[derive(Default)]
pub(super) struct SpeedMeter {
    /// (time, cumulative bytes) samples covering roughly `WINDOW`.
    samples: VecDeque<(Instant, u64)>,
    total: u64,
}

impl SpeedMeter {
    pub(super) fn record(&mut self, now: Instant, bytes: u64) {
        self.total = self.total.saturating_add(bytes);
        let previous = self.samples.back().map(|&(time, _)| time);
        if previous.is_none_or(|time| now.duration_since(time) > GAP) {
            // Baseline only; the bytes of this one tick are not measurable without a span.
            self.samples.clear();
            self.samples.push_back((now, self.total));
            return;
        }
        self.samples.push_back((now, self.total));
        // Keep one baseline sample at or just beyond the window edge.
        while self.samples.len() > 2 && now.duration_since(self.samples[1].0) >= WINDOW {
            self.samples.pop_front();
        }
    }

    /// Bytes per second over the trailing window.
    pub(super) fn speed(&self) -> u64 {
        let (Some(&(start, first)), Some(&(end, last))) =
            (self.samples.front(), self.samples.back())
        else {
            return 0;
        };
        let span = end.duration_since(start).as_secs_f64();
        if span < MIN_SPAN_SECS {
            return 0;
        }
        ((last - first) as f64 / span) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(meter: &mut SpeedMeter, start: Instant, ticks: std::ops::Range<u32>, per_tick: u64) {
        for tick in ticks {
            meter.record(start + Duration::from_millis(200) * tick, per_tick);
        }
    }

    #[test]
    fn steady_rate_reads_exactly() {
        let (mut meter, start) = (SpeedMeter::default(), Instant::now());
        feed(&mut meter, start, 0..50, 200_000);
        assert_eq!(meter.speed(), 1_000_000);
    }

    #[test]
    fn one_slow_tick_barely_moves_speed() {
        let (mut meter, start) = (SpeedMeter::default(), Instant::now());
        feed(&mut meter, start, 0..50, 200_000);
        let before = meter.speed();
        meter.record(start + Duration::from_millis(200) * 50, 0);
        let drop = before - meter.speed();
        assert!(drop < before / 20, "dropped {drop} of {before}");
    }

    #[test]
    fn stall_decays_gradually_instead_of_dropping_to_zero() {
        let (mut meter, start) = (SpeedMeter::default(), Instant::now());
        feed(&mut meter, start, 0..50, 200_000);
        feed(&mut meter, start, 50..55, 0);
        let speed = meter.speed();
        assert!(speed > 700_000 && speed < 1_000_000, "speed {speed}");
    }

    #[test]
    fn pause_gap_discards_stale_samples() {
        let (mut meter, start) = (SpeedMeter::default(), Instant::now());
        feed(&mut meter, start, 0..50, 200_000);
        let resume = start + Duration::from_secs(60);
        meter.record(resume, 0);
        assert_eq!(meter.speed(), 0);
        meter.record(resume + Duration::from_millis(200), 100_000);
        meter.record(resume + Duration::from_millis(400), 100_000);
        meter.record(resume + Duration::from_millis(600), 100_000);
        assert_eq!(meter.speed(), 500_000);
    }
}
