//! A Windows PC's screen: its primary display through Windows.Graphics.Capture, with the pointer, 30 pictures a
//! second at most and only as it changes, encoded as `h264` says; what the viewer does is the PC's own mouse and
//! keyboard input (`SendInput`), as the person signed in to it would give it. It runs in their session, as the
//! server that starts it does.
//!
//! Keys as the viewer names them are the PC's virtual keys, and text is typed as itself, whatever the keyboard.
use crate::h264::{Picture, encoded};
use crate::{Event, Source, input::Input};
use anyhow::anyhow;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Deserialize)]
pub struct Config {}

type Failure = Box<dyn std::error::Error + Send + Sync>;

struct Pictures(SyncSender<Picture>);

impl GraphicsCaptureApiHandler for Pictures {
    type Flags = SyncSender<Picture>;
    type Error = Failure;

    fn new(context: Context<Self::Flags>) -> Result<Self, Failure> {
        Ok(Self(context.flags))
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Failure> {
        let (width, height) = (frame.width() as usize, frame.height() as usize);
        let mut buffer = frame.buffer()?;
        let stride = buffer.row_pitch() as usize;
        // One the encoder hasn't taken the last of yet is left: the next is newer.
        let _ = self.0.try_send(Picture { bgra: buffer.as_raw_buffer().to_vec(), width, height, stride });
        Ok(())
    }
}

pub struct GraphicsCapture {
    _capture: CaptureControl<Pictures, Failure>,
    /// The size of the pictures sent, which the viewer's positions are in.
    size: Arc<Mutex<(usize, usize)>>,
    at: (i32, i32),
    keyframe: Arc<AtomicBool>,
}

impl GraphicsCapture {
    pub fn start(_config: Config, events: Sender<Event>) -> anyhow::Result<Self> {
        let (taken, pictures) = sync_channel(1);
        let settings = Settings::new(
            Monitor::primary().map_err(|why| anyhow!("Windows has no primary display: {why}"))?,
            CursorCaptureSettings::WithCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(Duration::from_millis(33)),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            taken,
        );
        let capture = Pictures::start_free_threaded(settings).map_err(|why| anyhow!("Windows didn't capture the screen: {why}"))?;
        let keyframe = Arc::new(AtomicBool::new(true));
        let size = Arc::new(Mutex::new((0, 0)));
        let (wanted, sized) = (keyframe.clone(), size.clone());
        std::thread::spawn(move || {
            let why = encoded(pictures, wanted, &sized, &events).map_or_else(|why| format!("the screen's pictures ended: {why:#}"), |why| why);
            let _ = events.send(Event::Ended(why));
        });
        Ok(Self { _capture: capture, size, at: (0, 0), keyframe })
    }

    fn mouse(&self, flags: MOUSE_EVENT_FLAGS, data: i32) {
        send(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx: self.at.0, dy: self.at.1, mouseData: data as _, dwFlags: flags | MOUSEEVENTF_ABSOLUTE, time: 0, dwExtraInfo: 0 },
            },
        });
    }

    fn keys(&self, codes: &[VIRTUAL_KEY]) {
        for (code, up) in codes.iter().map(|c| (*c, false)).chain(codes.iter().rev().map(|c| (*c, true))) {
            let extended = if matches!(code, VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_DELETE) { KEYEVENTF_EXTENDEDKEY } else { 0 };
            send(key(code, 0, extended | if up { KEYEVENTF_KEYUP } else { 0 }));
        }
    }
}

impl Source for GraphicsCapture {
    fn input(&mut self, input: Input) {
        match input {
            Input::Move { x, y } => {
                // Absolute positions are of 65536 steps across the primary display.
                let (width, height) = *self.size.lock().unwrap();
                let step = |at: f64, of: usize| (at * 65535.0 / (of.max(2) - 1) as f64).round() as i32;
                self.at = (step(x, width), step(y, height));
                self.mouse(MOUSEEVENTF_MOVE, 0);
            }
            Input::Button { b: b @ 1..=3, down, mods } => {
                let codes: Vec<_> = mods.iter().filter_map(|m| modifier(m)).collect();
                if down {
                    for &code in &codes {
                        send(key(code, 0, 0));
                    }
                }
                let flags = match (b, down) {
                    (1, true) => MOUSEEVENTF_LEFTDOWN,
                    (1, false) => MOUSEEVENTF_LEFTUP,
                    (3, true) => MOUSEEVENTF_RIGHTDOWN,
                    (3, false) => MOUSEEVENTF_RIGHTUP,
                    (_, true) => MOUSEEVENTF_MIDDLEDOWN,
                    (_, false) => MOUSEEVENTF_MIDDLEUP,
                };
                self.mouse(flags, 0);
                if !down {
                    for &code in codes.iter().rev() {
                        send(key(code, 0, KEYEVENTF_KEYUP));
                    }
                }
            }
            Input::Button { b: b @ 4..=7, down: true, .. } => {
                // A step of the wheel, as a wheel's notch is: up and right positive.
                let (flags, data) = [(MOUSEEVENTF_WHEEL, 120), (MOUSEEVENTF_WHEEL, -120), (MOUSEEVENTF_HWHEEL, -120), (MOUSEEVENTF_HWHEEL, 120)][b as usize - 4];
                self.mouse(flags, data);
            }
            Input::Button { .. } => {}
            Input::Key { key, mods } => {
                if let Some(code) = keycode(&key).or_else(|| keycode(&key.to_lowercase())) {
                    let mut codes: Vec<_> = mods.iter().filter_map(|m| modifier(m)).collect();
                    codes.push(code);
                    self.keys(&codes);
                }
            }
            Input::Text { text } => {
                for unit in text.replace('\n', "\r").encode_utf16() {
                    if unit == '\r' as u16 {
                        self.keys(&[VK_RETURN]);
                        continue;
                    }
                    send(key(0, unit, KEYEVENTF_UNICODE));
                    send(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
            }
        }
    }

    fn keyframe(&mut self) {
        self.keyframe.store(true, Ordering::Relaxed);
    }
}

fn key(code: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: code, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn send(input: INPUT) {
    if unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) } != 1 {
        eprintln!("Windows took no input: {}", std::io::Error::last_os_error());
    }
}

/// X's modifiers as a PC's keys.
fn modifier(name: &str) -> Option<VIRTUAL_KEY> {
    Some(match name {
        "Control_L" => VK_CONTROL,
        "Shift_L" => VK_SHIFT,
        "Alt_L" => VK_MENU,
        "Super_L" => VK_LWIN,
        _ => return None,
    })
}

/// A PC's virtual key for a key X names.
fn keycode(name: &str) -> Option<VIRTUAL_KEY> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        return match c {
            'a'..='z' => Some(c.to_ascii_uppercase() as VIRTUAL_KEY),
            '0'..='9' => Some(c as VIRTUAL_KEY),
            _ => None,
        };
    }
    Some(match name {
        "Return" => VK_RETURN,
        "BackSpace" => VK_BACK,
        "Tab" => VK_TAB,
        "Escape" => VK_ESCAPE,
        "Delete" => VK_DELETE,
        "Left" => VK_LEFT,
        "Right" => VK_RIGHT,
        "Up" => VK_UP,
        "Down" => VK_DOWN,
        "Home" => VK_HOME,
        "End" => VK_END,
        "Prior" => VK_PRIOR,
        "Next" => VK_NEXT,
        "space" => VK_SPACE,
        _ => return None,
    })
}
