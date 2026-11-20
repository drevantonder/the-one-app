//! How pressed the device is, as the W3C Compute Pressure API says it (`PressureObserver`'s states, which are Apple's
//! `ProcessInfo.ThermalState`'s): the worse of what its system says (`Display::pressure`: heat, a battery saver) and
//! what its windows' own frames say, by how many of the last missed the display's refresh. Under serious pressure or
//! worse, glass is drawn as reduced transparency has it (`Screen::pressure`), and once the frames are on time and the
//! system calm again, as it was.

use std::collections::VecDeque;
use std::time::Duration;
use web_time::Instant;

/// Compute Pressure's `PressureState`, in its order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pressure {
    #[default]
    Nominal,
    Fair,
    Serious,
    Critical,
}

impl Pressure {
    /// Its name, as `PressureRecord.state` says it.
    pub fn name(self) -> &'static str {
        match self {
            Pressure::Nominal => "nominal",
            Pressure::Fair => "fair",
            Pressure::Serious => "serious",
            Pressure::Critical => "critical",
        }
    }
}

/// How many of the last frames are weighed, and for how long a late one counts.
const WEIGHED: usize = 60;
const REMEMBERED: Duration = Duration::from_secs(5);

/// Whether each of the last frames, across every window, met the display's refresh.
#[derive(Default)]
pub(crate) struct Deadlines {
    frames: VecDeque<(Instant, bool)>,
}

impl Deadlines {
    /// A frame was shown, `late` or on time.
    pub(crate) fn frame(&mut self, late: bool) {
        self.frame_at(Instant::now(), late);
    }

    /// A frame was shown at `at`, `late` or on time: a window's frames now, the inspector's `replay` on its clock.
    pub(crate) fn frame_at(&mut self, at: Instant, late: bool) {
        self.frames.push_back((at, late));
        if self.frames.len() > WEIGHED {
            self.frames.pop_front();
        }
    }

    /// What the frames say: fair with one late, serious with a tenth of them late (at least three), critical with a
    /// quarter (at least six); a late frame counts for five seconds, so with nothing moving it eases by itself.
    pub(crate) fn pressure(&mut self) -> Pressure {
        self.pressure_at(Instant::now())
    }

    /// What the frames say at `now`, as `pressure`.
    pub(crate) fn pressure_at(&mut self, now: Instant) -> Pressure {
        while self.frames.front().is_some_and(|(at, _)| now.duration_since(*at) > REMEMBERED) {
            self.frames.pop_front();
        }
        let late = self.frames.iter().filter(|(_, late)| *late).count();
        let share = late as f32 / self.frames.len().max(1) as f32;
        match late {
            0 => Pressure::Nominal,
            6.. if share >= 0.25 => Pressure::Critical,
            3.. if share >= 0.1 => Pressure::Serious,
            _ => Pressure::Fair,
        }
    }
}

/// What an Apple device says: its thermal state, which Compute Pressure's states are, and Low Power Mode as serious, as
/// Apple has apps do less in it.
#[cfg(target_vendor = "apple")]
pub(crate) fn apple() -> Pressure {
    use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};
    let info = NSProcessInfo::processInfo();
    let thermal = match info.thermalState() {
        NSProcessInfoThermalState::Critical => Pressure::Critical,
        NSProcessInfoThermalState::Serious => Pressure::Serious,
        NSProcessInfoThermalState::Fair => Pressure::Fair,
        _ => Pressure::Nominal,
    };
    let saving = if info.isLowPowerModeEnabled() { Pressure::Serious } else { Pressure::Nominal };
    thermal.max(saving)
}
