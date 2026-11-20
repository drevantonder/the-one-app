//! How smoothly a window's motion went, frame by frame, as Android's FrameMetrics and Chrome's smoothness metrics
//! tell: each time something on a window moves by itself or follows the person (an animation, a drag, a scroll), its
//! frames are timed from the first to the last, and said once it's still (`Event::Frames`), for Telemetry to keep.

use std::time::Duration;
use web_time::Instant;

/// A frame's interval this much longer than the display's is a long one, one the display showed twice: as Chrome's
/// dropped-frame count has it.
const LONG: f32 = 1.5;

/// After this long with nothing drawn, what moved is still, though its page still asks for frames.
const STILL: Duration = Duration::from_millis(250);

/// What a window's frames did while something on it moved.
#[derive(Debug, Clone)]
pub struct Frames {
    /// What moved it: `drag` (a button or finger held), `scroll` (a wheel or a trackpad), or `motion` (by itself).
    pub kind: &'static str,
    /// How many frames it drew, and over how long.
    pub frames: u32,
    pub took: Duration,
    /// The intervals between its frames: the median, the 95th percentile and the longest.
    pub p50: Duration,
    pub p95: Duration,
    pub longest: Duration,
    /// How many were over 1.5× the display's interval, so it showed a frame twice.
    pub long: u32,
    /// The display's interval: what it says, or else the median of these frames'.
    pub refresh: Duration,
    /// The 95th percentile of the time this thread spent on a frame painting it (laying it out and making its scene),
    /// and drawing it (its GPU commands, waiting for an image to draw on and handing it to the display).
    pub paint: Duration,
    pub draw: Duration,
}

/// A window's frames while something on it moves.
#[derive(Default)]
pub(crate) struct Timing {
    /// What moved it, as the last input said (`input`).
    kind: Option<&'static str>,
    /// When each frame was handed to the display, and how long this thread spent painting and drawing it.
    frames: Vec<(Instant, Duration, Duration)>,
}

impl Timing {
    /// An input that moves what's on the window: `drag` or `scroll`.
    pub(crate) fn input(&mut self, kind: &'static str) {
        // A drag that flings on is a drag.
        if self.kind != Some("drag") {
            self.kind = Some(kind);
        }
    }

    /// A frame was drawn, after `paint` and `draw` on this thread: `moving` says whether something on it still moves.
    /// → how long after the frame before it was, while something moves, and what its frames did, once it's still after
    /// moving.
    pub(crate) fn drawn(&mut self, paint: Duration, draw: Duration, moving: bool, refresh: Option<Duration>) -> (Option<Duration>, Option<Frames>) {
        let now = Instant::now();
        let interval = self.frames.last().map(|(at, ..)| now.duration_since(*at));
        if moving || !self.frames.is_empty() {
            self.frames.push((now, paint, draw));
        }
        (interval, if moving { None } else { self.end(refresh) })
    }

    /// Whether a frame `interval` after the one before it is a long one, which the display showed the one before twice.
    pub(crate) fn long(interval: Duration, refresh: Duration) -> bool {
        interval.as_secs_f32() > refresh.as_secs_f32() * LONG
    }

    /// Whether what moved has been still long enough to be said, though its page still asks for frames.
    pub(crate) fn idle(&self) -> bool {
        self.frames.last().is_some_and(|(at, ..)| at.elapsed() >= STILL)
    }

    /// What its frames did, and it's ready for the next: none, under three frames.
    pub(crate) fn end(&mut self, refresh: Option<Duration>) -> Option<Frames> {
        let kind = self.kind.take().unwrap_or("motion");
        let frames = std::mem::take(&mut self.frames);
        if frames.len() < 3 {
            return None;
        }
        let mut intervals: Vec<Duration> = frames.windows(2).map(|pair| pair[1].0.duration_since(pair[0].0)).collect();
        intervals.sort();
        let mut paint: Vec<Duration> = frames.iter().map(|(_, paint, _)| *paint).collect();
        paint.sort();
        let mut draw: Vec<Duration> = frames.iter().map(|(.., draw)| *draw).collect();
        draw.sort();
        let at = |sorted: &[Duration], quantile: f32| sorted[((sorted.len() - 1) as f32 * quantile).round() as usize];
        let refresh = refresh.unwrap_or_else(|| at(&intervals, 0.5));
        Some(Frames {
            kind,
            frames: frames.len() as u32,
            took: frames[frames.len() - 1].0.duration_since(frames[0].0),
            p50: at(&intervals, 0.5),
            p95: at(&intervals, 0.95),
            longest: intervals[intervals.len() - 1],
            long: intervals.iter().filter(|interval| Timing::long(**interval, refresh)).count() as u32,
            refresh,
            paint: at(&paint, 0.95),
            draw: at(&draw, 0.95),
        })
    }
}
