//! High precision waiting.
//!
//! Strategy: a coarse OS sleep that wakes up shortly *before* the deadline,
//! then a short busy-wait on the monotonic clock. The coarse margin adapts to
//! the measured oversleep of the OS timer, so the busy part stays as short as
//! possible while still never waking late.
//!
//! * Windows: high resolution waitable timer
//!   (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`, ~0.5 ms granularity).
//! * Linux: `clock_nanosleep` through `std::thread::sleep` (hrtimers, ~50 µs).

use std::time::{Duration, Instant};

const MIN_MARGIN: Duration = Duration::from_micros(150);
const MAX_MARGIN: Duration = Duration::from_millis(3);
/// Long waits are sliced so that cancellation stays responsive.
const SLICE: Duration = Duration::from_millis(8);

pub struct PreciseSleeper {
    margin: Duration,
    /// Exponentially decaying estimate of the OS oversleep.
    oversleep_ns: f64,
    #[cfg(windows)]
    timer: Option<win::Timer>,
}

impl Default for PreciseSleeper {
    fn default() -> Self {
        Self::new()
    }
}

impl PreciseSleeper {
    pub fn new() -> Self {
        Self {
            margin: Duration::from_micros(700),
            oversleep_ns: 300_000.0,
            #[cfg(windows)]
            timer: win::Timer::new(),
        }
    }

    pub fn margin(&self) -> Duration {
        self.margin
    }

    fn os_sleep(&mut self, d: Duration) {
        #[cfg(windows)]
        if let Some(t) = &self.timer {
            t.sleep(d);
            return;
        }
        std::thread::sleep(d);
    }

    /// Waits until `deadline`. Returns `false` if `keep_going` returned false
    /// before the deadline (cancellation).
    pub fn sleep_until(&mut self, deadline: Instant, mut keep_going: impl FnMut() -> bool) -> bool {
        loop {
            if !keep_going() {
                return false;
            }
            let now = Instant::now();
            if now >= deadline {
                return true;
            }
            let remaining = deadline - now;
            if remaining > self.margin {
                let coarse = (remaining - self.margin).min(SLICE);
                let target = now + coarse;
                self.os_sleep(coarse);
                let woke = Instant::now();
                let over = woke.saturating_duration_since(target).as_nanos() as f64;
                // Rise fast, decay slowly: we care about the bad cases.
                self.oversleep_ns = if over > self.oversleep_ns { over } else { self.oversleep_ns * 0.98 + over * 0.02 };
                self.margin = Duration::from_nanos((self.oversleep_ns * 1.25) as u64 + 100_000).clamp(MIN_MARGIN, MAX_MARGIN);
                continue;
            }
            // Final approach: spin.
            while Instant::now() < deadline {
                std::hint::spin_loop();
            }
            return true;
        }
    }
}

#[cfg(windows)]
mod win {
    use std::time::Duration;

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Threading::{
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE, SetWaitableTimer,
        TIMER_ALL_ACCESS, WaitForSingleObject,
    };

    pub struct Timer(HANDLE);
    // The handle is only used by the thread owning the sleeper.
    unsafe impl Send for Timer {}

    impl Timer {
        pub fn new() -> Option<Self> {
            unsafe {
                CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)
                    .ok()
                    .map(Timer)
            }
        }

        pub fn sleep(&self, d: Duration) {
            // Relative due time in 100 ns units (negative = relative).
            let due = -((d.as_nanos() / 100).max(1) as i64);
            unsafe {
                if SetWaitableTimer(self.0, &due, 0, None, None, false).is_ok() {
                    WaitForSingleObject(self.0, INFINITE);
                } else {
                    std::thread::sleep(d);
                }
            }
        }
    }

    impl Drop for Timer {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Statistics of a timing measurement, in microseconds.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct JitterStats {
    pub samples: usize,
    pub mean_us: f64,
    pub median_us: f64,
    pub p95_us: f64,
    pub p99_us: f64,
    pub max_us: f64,
    pub histogram: Vec<(f64, usize)>,
}

impl JitterStats {
    pub fn from_errors(mut errors_us: Vec<f64>) -> Self {
        if errors_us.is_empty() {
            return Self::default();
        }
        errors_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = errors_us.len();
        let pct = |p: f64| errors_us[((n as f64 - 1.0) * p).round() as usize];
        let edges = [5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, f64::INFINITY];
        let mut histogram: Vec<(f64, usize)> = edges.iter().map(|&e| (e, 0)).collect();
        for &e in &errors_us {
            if let Some(b) = histogram.iter_mut().find(|(edge, _)| e <= *edge) {
                b.1 += 1;
            }
        }
        Self {
            samples: n,
            mean_us: errors_us.iter().sum::<f64>() / n as f64,
            median_us: pct(0.5),
            p95_us: pct(0.95),
            p99_us: pct(0.99),
            max_us: errors_us[n - 1],
            histogram,
        }
    }
}

/// Measures how precisely this machine can wake up at given deadlines.
pub fn measure_sleep_jitter(samples: usize, interval: Duration) -> JitterStats {
    let mut s = PreciseSleeper::new();
    let start = Instant::now() + Duration::from_millis(5);
    let mut errors = Vec::with_capacity(samples);
    for i in 0..samples {
        let deadline = start + interval * i as u32;
        s.sleep_until(deadline, || true);
        let late = Instant::now().saturating_duration_since(deadline);
        errors.push(late.as_nanos() as f64 / 1_000.0);
    }
    JitterStats::from_errors(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_wakes_early_and_is_cancellable() {
        let mut s = PreciseSleeper::new();
        for ms in [0u64, 1, 3] {
            let d = Instant::now() + Duration::from_millis(ms);
            assert!(s.sleep_until(d, || true));
            assert!(Instant::now() >= d);
        }
        let far = Instant::now() + Duration::from_secs(10);
        let begin = Instant::now();
        let mut calls = 0;
        assert!(!s.sleep_until(far, || {
            calls += 1;
            calls < 3
        }));
        assert!(begin.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn jitter_is_small() {
        let stats = measure_sleep_jitter(200, Duration::from_micros(1_000));
        // Generous bound for shared CI machines; real numbers are in Diagnostics.
        assert!(stats.median_us < 200.0, "{stats:?}");
    }
}
