//! Frame-time telemetry (SPEC §8 "Perf gates", P4).
//!
//! Average FPS is close to useless on its own: a run that averages 120 fps
//! while dropping to 40 for one frame in fifty *feels* broken, and at 90 Hz in
//! a headset it is broken. So the numbers we keep are the ones that describe
//! the bad frames — the 1% low and the single worst frame in the window.
//!
//! Pure and clock-free: the caller supplies frame durations, so all of this is
//! unit-testable without a GPU or a window.

use std::collections::VecDeque;

/// Statistics over one logging window, in both frame-time and FPS terms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub frames: usize,
    pub avg_fps: f64,
    /// FPS implied by the *mean of the worst 1%* of frame times. Note this is
    /// not the 99th percentile: with 100 samples the 99th percentile lands on
    /// the second-worst frame and hides the very spike you are hunting.
    pub low_1pct_fps: f64,
    pub avg_ms: f64,
    /// The single worst frame in the window.
    pub worst_ms: f64,
    pub best_ms: f64,
}

pub struct FrameStats {
    /// Short rolling buffer for the live on-screen readout.
    recent: VecDeque<f64>,
    recent_cap: usize,
    /// Every frame since the last summary was taken.
    window: Vec<f64>,
    /// The first frame after startup or a resize includes device setup and
    /// swapchain reconfiguration; counting it would slander the renderer.
    skip_next: bool,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::new(120)
    }
}

impl FrameStats {
    pub fn new(recent_cap: usize) -> Self {
        Self {
            recent: VecDeque::with_capacity(recent_cap),
            recent_cap,
            window: Vec::new(),
            skip_next: true,
        }
    }

    /// Discard the next frame's timing (startup, resize, or anything else that
    /// stalls for reasons the renderer is not responsible for).
    pub fn skip_next_frame(&mut self) {
        self.skip_next = true;
    }

    pub fn record(&mut self, dt_s: f64) {
        if self.skip_next {
            self.skip_next = false;
            return;
        }
        // A non-positive or absurd dt means the clock, not the renderer.
        if !(dt_s.is_finite() && dt_s > 0.0) {
            return;
        }
        if self.recent.len() == self.recent_cap {
            self.recent.pop_front();
        }
        self.recent.push_back(dt_s);
        self.window.push(dt_s);
    }

    /// Smoothed instantaneous rate for the live readout. Averaging the frame
    /// *times* and inverting is the honest way — averaging per-frame FPS values
    /// overweights the fast frames and flatters the result.
    pub fn smoothed_fps(&self) -> f64 {
        if self.recent.is_empty() {
            return 0.0;
        }
        let mean = self.recent.iter().sum::<f64>() / self.recent.len() as f64;
        if mean > 0.0 {
            1.0 / mean
        } else {
            0.0
        }
    }

    /// 1% low over the recent buffer, for the live readout.
    pub fn recent_low_1pct_fps(&self) -> f64 {
        let mut v: Vec<f64> = self.recent.iter().copied().collect();
        worst_fraction_fps(&mut v, 0.01)
    }

    /// Frames accumulated since the last summary. Used by the tests to pin
    /// down which timings are counted and which are rejected.
    #[cfg(test)]
    pub fn window_frames(&self) -> usize {
        self.window.len()
    }

    /// Summarise and clear the logging window.
    pub fn take_summary(&mut self) -> Option<Summary> {
        if self.window.is_empty() {
            return None;
        }
        let mut v = std::mem::take(&mut self.window);
        let frames = v.len();
        let total: f64 = v.iter().sum();
        let avg = total / frames as f64;
        v.sort_by(f64::total_cmp);
        Some(Summary {
            frames,
            avg_fps: 1.0 / avg,
            low_1pct_fps: worst_fraction_fps(&mut v, 0.01),
            avg_ms: avg * 1000.0,
            worst_ms: v[frames - 1] * 1000.0,
            best_ms: v[0] * 1000.0,
        })
    }
}

/// FPS implied by the mean of the worst `frac` of frame times (the "1% low"
/// convention when `frac` is 0.01). Always includes at least one frame, so a
/// short window still reports its worst. Sorts in place.
fn worst_fraction_fps(v: &mut [f64], frac: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let take = (((v.len() as f64) * frac).ceil() as usize).clamp(1, v.len());
    let worst = &v[v.len() - take..];
    let mean = worst.iter().sum::<f64>() / take as f64;
    if mean > 0.0 {
        1.0 / mean
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats_of(frames: &[f64]) -> FrameStats {
        let mut s = FrameStats::new(1024);
        s.skip_next = false;
        for f in frames {
            s.record(*f);
        }
        s
    }

    #[test]
    fn empty_is_zero_not_nan() {
        let mut s = FrameStats::new(8);
        assert_eq!(s.smoothed_fps(), 0.0);
        assert_eq!(s.recent_low_1pct_fps(), 0.0);
        assert!(s.take_summary().is_none());
    }

    #[test]
    fn steady_frames_report_their_rate() {
        let s = stats_of(&[1.0 / 120.0; 100]);
        assert!((s.smoothed_fps() - 120.0).abs() < 1e-6);
    }

    /// The headline number must be the mean of frame *times*, not the mean of
    /// per-frame FPS — the latter flatters a stuttering run.
    #[test]
    fn average_is_over_frame_times_not_rates() {
        // Half the frames at 10 ms, half at 30 ms: mean time 20 ms => 50 fps.
        // Mean of rates would be (100 + 33.3)/2 = 66.7 fps, which is a lie.
        let mut frames = vec![0.010; 50];
        frames.extend(vec![0.030; 50]);
        let s = stats_of(&frames);
        assert!(
            (s.smoothed_fps() - 50.0).abs() < 1e-6,
            "got {}",
            s.smoothed_fps()
        );
    }

    /// One bad frame in a hundred must move the 1% low but barely dent the
    /// average — that difference is the entire point of tracking it.
    #[test]
    fn one_stutter_shows_in_the_low_not_the_average() {
        let mut frames = vec![1.0 / 120.0; 99];
        frames.push(0.050); // one 50 ms hitch
        let mut s = stats_of(&frames);
        let sum = s.take_summary().unwrap();
        assert!(sum.avg_fps > 90.0, "average masked the hitch: {sum:?}");
        assert!(
            (sum.low_1pct_fps - 20.0).abs() < 0.5,
            "1% low should expose the 50 ms frame: {sum:?}"
        );
        // The distinction that motivates the metric: the average says 'fine',
        // the 1% low says 'this stutters'.
        assert!(sum.avg_fps > 4.0 * sum.low_1pct_fps);
        assert!((sum.worst_ms - 50.0).abs() < 1e-6);
        assert!((sum.best_ms - 1000.0 / 120.0).abs() < 1e-6);
        assert_eq!(sum.frames, 100);
    }

    #[test]
    fn taking_a_summary_clears_the_window() {
        let mut s = stats_of(&[0.01; 10]);
        assert_eq!(s.window_frames(), 10);
        assert!(s.take_summary().is_some());
        assert_eq!(s.window_frames(), 0);
        assert!(s.take_summary().is_none());
        // The live readout survives: it is a separate rolling buffer.
        assert!(s.smoothed_fps() > 0.0);
    }

    #[test]
    fn recent_buffer_is_bounded() {
        let mut s = FrameStats::new(8);
        s.skip_next = false;
        for _ in 0..100 {
            s.record(0.01);
        }
        assert_eq!(s.recent.len(), 8);
    }

    #[test]
    fn first_frame_is_skipped() {
        let mut s = FrameStats::new(8); // starts with skip_next = true
        s.record(2.0); // pretend startup stall
        assert_eq!(s.window_frames(), 0);
        s.record(0.01);
        assert_eq!(s.window_frames(), 1);
    }

    #[test]
    fn garbage_timings_are_ignored() {
        let mut s = stats_of(&[0.01]);
        s.record(0.0);
        s.record(-1.0);
        s.record(f64::NAN);
        s.record(f64::INFINITY);
        assert_eq!(s.window_frames(), 1);
        assert!(s.smoothed_fps().is_finite());
    }
}
