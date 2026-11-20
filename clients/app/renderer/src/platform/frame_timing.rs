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
    /// How many were over 1.5× the display's interval, so it showed a frame twice, and where each was among them (0
    /// the interval after its first frame).
    pub long: u32,
    pub long_at: Vec<u32>,
    /// The display's interval: what it says, or else the median of these frames'.
    pub refresh: Duration,
    /// The 95th percentile of each part of a frame (`Frame`).
    pub paint: Duration,
    pub draw: Duration,
    pub acquire: Duration,
    pub gpu: Option<Duration>,
    /// Of `gpu`, its blurs': what's behind a backdrop's glass.
    pub blur: Option<Duration>,
    /// The first long frame, what it took, and the frame after the longest interval, where that's long.
    pub first_long: Option<Frame>,
    pub longest_gap: Option<(Duration, Frame)>,
}

/// One frame: when it was handed to the display, and what it took. `paint` is this thread laying it out and making
/// its scene; `draw` its GPU commands, waiting for an image to draw on (`acquire`, which a display that hasn't shown
/// the frames before keeps it waiting for) and handing it to the display; `gpu` the GPU's time on its passes, where
/// the GPU says, a frame or two behind.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub at: Instant,
    pub paint: Duration,
    pub draw: Duration,
    pub acquire: Duration,
    pub gpu: Option<Duration>,
    /// Of `gpu`, its blurs': what's behind a backdrop's glass.
    pub blur: Option<Duration>,
    /// Since the frame before, this thread's time applying what its page changed, and in the loop's turns, the
    /// frame's own among them: where a long gap's time went on this thread, or that it went elsewhere.
    pub applied: Duration,
    pub turned: Duration,
}

/// A window's frames while something on it moves.
#[derive(Default)]
pub(crate) struct Timing {
    /// What moved it, as the last input said (`input`).
    kind: Option<&'static str>,
    frames: Vec<Frame>,
    /// This thread's time since the last frame (`Frame::applied`, `Frame::turned`).
    applied: Duration,
    turned: Duration,
}

impl Timing {
    /// An input that moves what's on the window: `drag` or `scroll`.
    pub(crate) fn input(&mut self, kind: &'static str) {
        // A drag that flings on is a drag.
        if self.kind != Some("drag") {
            self.kind = Some(kind);
        }
    }

    /// This thread spent `time` applying what the page changed.
    pub(crate) fn applied(&mut self, time: Duration) {
        self.applied += time;
    }

    /// This thread spent `time` on a turn of the loop.
    pub(crate) fn turned(&mut self, time: Duration) {
        self.turned += time;
    }

    /// `frame` was drawn: `moving` says whether something on it still moves. → how long after the frame before it was,
    /// while something moves, and what its frames did, once it's still after moving.
    pub(crate) fn drawn(&mut self, mut frame: Frame, moving: bool, refresh: Option<Duration>) -> (Option<Duration>, Option<Frames>) {
        frame.applied = std::mem::take(&mut self.applied);
        frame.turned = std::mem::take(&mut self.turned);
        let interval = self.frames.last().map(|last| frame.at.duration_since(last.at));
        if moving || !self.frames.is_empty() {
            self.frames.push(frame);
        }
        (interval, if moving { None } else { self.end(refresh) })
    }

    /// Whether a frame `interval` after the one before it is a long one, which the display showed the one before twice.
    pub(crate) fn long(interval: Duration, refresh: Duration) -> bool {
        interval.as_secs_f32() > refresh.as_secs_f32() * LONG
    }

    /// Whether what moved has been still long enough to be said, though its page still asks for frames.
    pub(crate) fn idle(&self) -> bool {
        self.frames.last().is_some_and(|last| super::since(last.at) >= STILL)
    }

    /// What its frames did, and it's ready for the next: none, under three frames.
    pub(crate) fn end(&mut self, refresh: Option<Duration>) -> Option<Frames> {
        let kind = self.kind.take().unwrap_or("motion");
        let frames = std::mem::take(&mut self.frames);
        if frames.len() < 3 {
            return None;
        }
        let between: Vec<Duration> = frames.windows(2).map(|pair| pair[1].at.duration_since(pair[0].at)).collect();
        let at = |mut all: Vec<Duration>, quantile: f32| {
            all.sort();
            all[((all.len() - 1) as f32 * quantile).round() as usize]
        };
        let refresh = refresh.unwrap_or_else(|| at(between.clone(), 0.5));
        let long_at: Vec<u32> = (0..between.len() as u32).filter(|index| Timing::long(between[*index as usize], refresh)).collect();
        let gpu: Vec<Duration> = frames.iter().filter_map(|frame| frame.gpu).collect();
        let blur: Vec<Duration> = frames.iter().filter_map(|frame| frame.blur).collect();
        Some(Frames {
            kind,
            frames: frames.len() as u32,
            took: frames[frames.len() - 1].at.duration_since(frames[0].at),
            p50: at(between.clone(), 0.5),
            p95: at(between.clone(), 0.95),
            longest: at(between.clone(), 1.0),
            long: long_at.len() as u32,
            first_long: long_at.first().map(|index| frames[*index as usize + 1]),
            longest_gap: (0..between.len()).max_by_key(|index| between[*index]).filter(|index| long_at.contains(&(*index as u32))).map(|index| (between[index], frames[index + 1])),
            long_at,
            refresh,
            paint: at(frames.iter().map(|frame| frame.paint).collect(), 0.95),
            draw: at(frames.iter().map(|frame| frame.draw).collect(), 0.95),
            acquire: at(frames.iter().map(|frame| frame.acquire).collect(), 0.95),
            gpu: (!gpu.is_empty()).then(|| at(gpu, 0.95)),
            blur: (!blur.is_empty()).then(|| at(blur, 0.95)),
        })
    }
}
