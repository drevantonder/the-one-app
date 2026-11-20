//! A Mac's screen: its main display through ScreenCaptureKit, with the pointer, 30 pictures a second at most and only
//! as it changes, encoded as `h264` says; what the viewer does is posted as the Mac's own mouse and keyboard events
//! (Quartz Event Services). macOS allows both only to the program it holds responsible, Screen Recording and
//! Accessibility: for Computer Use, its helper app, which starts this.
//!
//! Its pictures are at most as wide as `h264` sends, in pixels, and the viewer's positions in them are the display's
//! points scaled. Keys as the viewer names them go to the Mac as a Mac's: Control is ⌘, as a person on another
//! computer means it, Super is Control, and text is typed as itself, whatever the keyboard.
use crate::h264::{Picture, encoded};
use crate::{Event, Source, input::Input};
use anyhow::{Context, anyhow};
use core_graphics::display::CGDisplay;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField, ScrollEventUnit};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use screencapturekit::cm::CMSampleBufferExt;
use screencapturekit::cv::CVPixelBufferLockFlags;
use screencapturekit::prelude::*;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Deserialize)]
pub struct Config {}

struct Pictures(SyncSender<Picture>);

impl SCStreamOutputTrait for Pictures {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, kind: SCStreamOutputType) {
        if !matches!(kind, SCStreamOutputType::Screen) {
            return;
        }
        let Some(buffer) = sample.pixel_buffer() else { return };
        let Ok(locked) = buffer.lock(CVPixelBufferLockFlags::READ_ONLY) else { return };
        let Some(bgra) = (unsafe { locked.as_slice() }) else { return };
        // One the encoder hasn't taken the last of yet is left: the next is newer.
        let _ = self.0.try_send(Picture { bgra: bgra.to_vec(), width: locked.width(), height: locked.height(), stride: locked.bytes_per_row() });
    }
}

pub struct ScreenCaptureKit {
    _stream: SCStream,
    source: CGEventSource,
    /// The display's points for each of the pictures' pixels.
    points: f64,
    at: CGPoint,
    held: Option<CGMouseButton>,
    /// The last click: where, when, and how many in a row, which a double click is told by.
    clicked: (CGPoint, Instant, i64),
    keyframe: Arc<AtomicBool>,
}

impl ScreenCaptureKit {
    pub fn start(_config: Config, events: Sender<Event>) -> anyhow::Result<Self> {
        let main = CGDisplay::main().id;
        let content = SCShareableContent::get().map_err(|why| anyhow!("macOS doesn't allow seeing the screen (Screen Recording): {why:?}"))?;
        let display = content.displays().into_iter().find(|d| d.display_id() == main).context("there's no main display")?;
        let bounds = CGDisplay::new(main).bounds();
        // Its pictures as wide as its points, or as `h264` sends, whichever is less, the height to match, both even.
        let width = (bounds.size.width as usize).min(1920) & !1;
        let height = ((width as f64 * bounds.size.height / bounds.size.width) as usize) & !1;
        let filter = SCContentFilter::create().with_display(&display).with_excluding_windows(&[]).build().map_err(|why| anyhow!("{why:?}"))?;
        let config = SCStreamConfiguration::new()
            .with_width(width as u32)
            .with_height(height as u32)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(true)
            .with_minimum_frame_interval(&CMTime::new(1, 30));
        let (taken, pictures) = sync_channel(1);
        let mut stream = SCStream::new(&filter, &config).map_err(|why| anyhow!("{why:?}"))?;
        stream.add_output_handler(Pictures(taken), SCStreamOutputType::Screen).map_err(|why| anyhow!("{why:?}"))?;
        stream.start_capture().map_err(|why| anyhow!("macOS doesn't allow seeing the screen (Screen Recording): {why:?}"))?;

        let keyframe = Arc::new(AtomicBool::new(true));
        let wanted = keyframe.clone();
        std::thread::spawn(move || {
            let why = encoded(pictures, wanted, &Mutex::new((0, 0)), &events).map_or_else(|why| format!("the screen's pictures ended: {why:#}"), |why| why);
            let _ = events.send(Event::Ended(why));
        });
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|()| anyhow!("macOS gave no event source"))?;
        Ok(Self {
            _stream: stream,
            source,
            points: bounds.size.width / width as f64,
            at: CGPoint::new(0.0, 0.0),
            held: None,
            clicked: (CGPoint::new(-1.0, -1.0), Instant::now(), 0),
            keyframe,
        })
    }

    fn post(&self, event: Result<CGEvent, ()>) {
        match event {
            Ok(event) => event.post(CGEventTapLocation::HID),
            Err(()) => eprintln!("macOS made no event"),
        }
    }

    fn mouse(&self, kind: CGEventType, button: CGMouseButton, clicks: i64, flags: CGEventFlags) {
        if let Ok(event) = CGEvent::new_mouse_event(self.source.clone(), kind, self.at, button) {
            event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, clicks);
            event.set_flags(flags);
            event.post(CGEventTapLocation::HID);
        }
    }

    fn key(&self, code: u16, down: bool, flags: CGEventFlags) {
        if let Ok(event) = CGEvent::new_keyboard_event(self.source.clone(), code, down) {
            event.set_flags(flags);
            event.post(CGEventTapLocation::HID);
        }
    }
}

impl Source for ScreenCaptureKit {
    fn input(&mut self, input: Input) {
        match input {
            Input::Move { x, y } => {
                self.at = CGPoint::new(x * self.points, y * self.points);
                let kind = match self.held {
                    Some(CGMouseButton::Left) => CGEventType::LeftMouseDragged,
                    Some(CGMouseButton::Right) => CGEventType::RightMouseDragged,
                    Some(CGMouseButton::Center) => CGEventType::OtherMouseDragged,
                    None => CGEventType::MouseMoved,
                };
                self.mouse(kind, self.held.unwrap_or(CGMouseButton::Left), 0, CGEventFlags::empty());
            }
            Input::Button { b: b @ 1..=3, down, mods } => {
                let (button, pressed, released) = match b {
                    1 => (CGMouseButton::Left, CGEventType::LeftMouseDown, CGEventType::LeftMouseUp),
                    3 => (CGMouseButton::Right, CGEventType::RightMouseDown, CGEventType::RightMouseUp),
                    _ => (CGMouseButton::Center, CGEventType::OtherMouseDown, CGEventType::OtherMouseUp),
                };
                if down {
                    let (at, when, count) = self.clicked;
                    let again = when.elapsed() < Duration::from_millis(500) && (at.x - self.at.x).abs() + (at.y - self.at.y).abs() < 8.0;
                    self.clicked = (self.at, Instant::now(), if again { count + 1 } else { 1 });
                    self.held = Some(button);
                }
                self.mouse(if down { pressed } else { released }, button, self.clicked.2, flags(&mods));
                if !down {
                    self.held = None;
                }
            }
            Input::Button { b: b @ 4..=7, down: true, .. } => {
                // A step of the wheel, three lines, up and left positive.
                let (vertical, horizontal) = [(3, 0), (-3, 0), (0, 3), (0, -3)][b as usize - 4];
                self.post(CGEvent::new_scroll_event(self.source.clone(), ScrollEventUnit::LINE, 2, vertical, horizontal, 0));
            }
            Input::Button { .. } => {}
            Input::Key { key, mods } => {
                let Some(code) = keycode(&key).or_else(|| keycode(&key.to_lowercase())) else { return };
                let flags = flags(&mods);
                self.key(code, true, flags);
                self.key(code, false, flags);
            }
            Input::Text { text } => {
                for c in text.chars() {
                    if c == '\n' {
                        self.key(36, true, CGEventFlags::empty());
                        self.key(36, false, CGEventFlags::empty());
                        continue;
                    }
                    for down in [true, false] {
                        if let Ok(event) = CGEvent::new_keyboard_event(self.source.clone(), 0, down) {
                            event.set_string(&c.to_string());
                            event.post(CGEventTapLocation::HID);
                        }
                    }
                }
            }
        }
    }

    fn keyframe(&mut self) {
        self.keyframe.store(true, Ordering::Relaxed);
    }
}

/// X's modifiers as a Mac's.
fn flags(mods: &[String]) -> CGEventFlags {
    mods.iter().fold(CGEventFlags::empty(), |flags, m| {
        flags
            | match m.as_str() {
                "Control_L" => CGEventFlags::CGEventFlagCommand,
                "Shift_L" => CGEventFlags::CGEventFlagShift,
                "Alt_L" => CGEventFlags::CGEventFlagAlternate,
                "Super_L" => CGEventFlags::CGEventFlagControl,
                _ => CGEventFlags::empty(),
            }
    })
}

/// A Mac's virtual key code for a key X names, on its US layout.
fn keycode(name: &str) -> Option<u16> {
    const LAYOUT: &[(char, u16)] = &[
        ('a', 0), ('s', 1), ('d', 2), ('f', 3), ('h', 4), ('g', 5), ('z', 6), ('x', 7), ('c', 8), ('v', 9), ('b', 11),
        ('q', 12), ('w', 13), ('e', 14), ('r', 15), ('y', 16), ('t', 17), ('1', 18), ('2', 19), ('3', 20), ('4', 21),
        ('6', 22), ('5', 23), ('=', 24), ('9', 25), ('7', 26), ('-', 27), ('8', 28), ('0', 29), (']', 30), ('o', 31),
        ('u', 32), ('[', 33), ('i', 34), ('p', 35), ('l', 37), ('j', 38), ('\'', 39), ('k', 40), (';', 41), ('\\', 42),
        (',', 43), ('/', 44), ('n', 45), ('m', 46), ('.', 47), ('`', 50),
    ];
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        return LAYOUT.iter().find(|(k, _)| *k == c).map(|(_, code)| *code);
    }
    Some(match name {
        "Return" => 36,
        "Tab" => 48,
        "space" => 49,
        "BackSpace" => 51,
        "Escape" => 53,
        "Delete" => 117,
        "Home" => 115,
        "End" => 119,
        "Prior" => 116,
        "Next" => 121,
        "Left" => 123,
        "Right" => 124,
        "Down" => 125,
        "Up" => 126,
        _ => return None,
    })
}
