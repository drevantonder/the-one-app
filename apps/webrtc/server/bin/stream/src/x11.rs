//! A Linux display's screen, X11's (`$DISPLAY`): its pictures, with its pointer drawn on them (XFixes), 30 a second
//! at most and only when one changes, encoded
//! as H.264 with OpenH264, constrained baseline, which every browser decodes; what the viewer does is played on the
//! display through XTest. A display that's resized is a stream of another size, so the stream ends and the viewer
//! connects again for it.
//!
//! Buttons are X's own; a key or a button pressed says the modifiers held with it, and one let go the same.
use crate::{Event, Source, input::Input};
use anyhow::{Context, anyhow};
use openh264::OpenH264API;
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, UsageType};
use openh264::formats::{BgraSliceU8, YUVBuffer};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xfixes::{ConnectionExt as _, GetCursorImageReply};
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, Keycode, Keysym, Window};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

#[derive(Deserialize)]
pub struct Config {}

const FRAME: Duration = Duration::from_millis(33);

pub struct X11 {
    conn: RustConnection,
    root: Window,
    /// Each keysym the keyboard has, with its key and whether it takes Shift.
    keys: HashMap<Keysym, (Keycode, bool)>,
    /// The keys with nothing on them, which a character the keyboard has no key for is put on, each in turn, and
    /// left on: a program reads what a key types as it takes its press, which may be after the next is typed.
    spare: Vec<Keycode>,
    next: usize,
    keyframe: Arc<AtomicBool>,
}

impl X11 {
    pub fn start(_config: Config, events: Sender<Event>) -> anyhow::Result<Self> {
        let (conn, screen) = x11rb::connect(None).context("couldn't reach the display")?;
        let root = conn.setup().roots[screen].root;
        let (keys, spare) = keyboard(&conn)?;
        let keyframe = Arc::new(AtomicBool::new(true));
        let wanted = keyframe.clone();
        std::thread::spawn(move || {
            let why = match pictures(&events, &wanted) {
                Ok(why) => why,
                Err(why) => format!("the display's pictures ended: {why:#}"),
            };
            let _ = events.send(Event::Ended(why));
        });
        Ok(Self { conn, root, keys, spare, next: 0, keyframe })
    }

    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) {
        let _ = self.conn.xtest_fake_input(kind, detail, x11rb::CURRENT_TIME, self.root, x, y, 0);
    }

    fn hold(&self, mods: &[String], down: bool) {
        let codes: Vec<_> = mods.iter().filter_map(|m| self.keys.get(&keysym(m)?)).map(|(code, _)| *code).collect();
        let kind = if down { x11rb::protocol::xproto::KEY_PRESS_EVENT } else { x11rb::protocol::xproto::KEY_RELEASE_EVENT };
        let ordered: Vec<_> = if down { codes } else { codes.into_iter().rev().collect() };
        for code in ordered {
            self.fake(kind, code, 0, 0);
        }
    }

    fn tap(&self, code: Keycode) {
        self.fake(x11rb::protocol::xproto::KEY_PRESS_EVENT, code, 0, 0);
        self.fake(x11rb::protocol::xproto::KEY_RELEASE_EVENT, code, 0, 0);
    }

    /// Presses the key `name` types, with the modifiers `held`, and Shift where the key's character takes it.
    fn press(&mut self, name: &str, held: &[String]) {
        let Some(sym) = keysym(name) else { return };
        match self.keys.get(&sym) {
            Some(&(code, shifted)) => {
                let mut held = held.to_vec();
                if shifted && !held.iter().any(|m| m == "Shift_L") {
                    held.push("Shift_L".into());
                }
                self.hold(&held, true);
                self.tap(code);
                self.hold(&held, false);
            }
            None if !self.spare.is_empty() => {
                let spare = self.spare[self.next % self.spare.len()];
                self.next += 1;
                let _ = self.conn.change_keyboard_mapping(1, spare, 1, &[sym]);
                let _ = self.conn.sync();
                self.keys.retain(|_, (code, _)| *code != spare);
                self.keys.insert(sym, (spare, false));
                self.hold(held, true);
                self.tap(spare);
                self.hold(held, false);
            }
            None => {}
        }
    }
}

impl Source for X11 {
    fn input(&mut self, input: Input) {
        use x11rb::protocol::xproto::{BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, MOTION_NOTIFY_EVENT};
        match input {
            Input::Move { x, y } => self.fake(MOTION_NOTIFY_EVENT, 0, x as i16, y as i16),
            Input::Button { b, down: true, mods } => {
                self.hold(&mods, true);
                self.fake(BUTTON_PRESS_EVENT, b, 0, 0);
            }
            Input::Button { b, down: false, mods } => {
                self.fake(BUTTON_RELEASE_EVENT, b, 0, 0);
                self.hold(&mods, false);
            }
            Input::Key { key, mods } => self.press(&key, &mods),
            Input::Text { text } => {
                for c in text.chars() {
                    self.press(&if c == '\n' { "Return".into() } else { c.to_string() }, &[]);
                }
            }
        }
        let _ = self.conn.flush();
    }

    fn keyframe(&mut self) {
        self.keyframe.store(true, Ordering::Relaxed);
    }
}

/// The keysym of a key X names, or of the character a one-character name is.
fn keysym(name: &str) -> Option<Keysym> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        let c = c as u32;
        return Some(if c < 0x100 { c } else { 0x0100_0000 | c });
    }
    Some(match name {
        "Return" => 0xff0d,
        "BackSpace" => 0xff08,
        "Tab" => 0xff09,
        "Escape" => 0xff1b,
        "Delete" => 0xffff,
        "Home" => 0xff50,
        "Left" => 0xff51,
        "Up" => 0xff52,
        "Right" => 0xff53,
        "Down" => 0xff54,
        "Prior" => 0xff55,
        "Next" => 0xff56,
        "End" => 0xff57,
        "space" => 0x20,
        "Shift_L" => 0xffe1,
        "Control_L" => 0xffe3,
        "Alt_L" => 0xffe9,
        "Super_L" => 0xffeb,
        _ => return None,
    })
}

/// The keyboard's keysyms, each with the first key that types it and whether that takes Shift, and the keys with none.
fn keyboard(conn: &RustConnection) -> anyhow::Result<(HashMap<Keysym, (Keycode, bool)>, Vec<Keycode>)> {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let mapping = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
    let per = mapping.keysyms_per_keycode as usize;
    let mut keys = HashMap::new();
    let mut spare = vec![];
    for (i, syms) in mapping.keysyms.chunks(per).enumerate() {
        let code = min + i as u8;
        if syms.iter().all(|&s| s == 0) {
            spare.push(code);
        }
        for (level, &sym) in syms.iter().take(2).enumerate() {
            if sym != 0 {
                keys.entry(sym).or_insert((code, level == 1));
            }
        }
    }
    Ok((keys, spare))
}

/// The display's pictures, each that differs from the one before encoded and sent, until it's resized: gives why
/// it ended.
fn pictures(events: &Sender<Event>, keyframe: &AtomicBool) -> anyhow::Result<String> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    conn.randr_select_input(root, randr::NotifyMask::SCREEN_CHANGE)?;
    conn.xfixes_query_version(5, 0)?.reply()?;
    let geometry = conn.get_geometry(root)?.reply()?;
    // H.264 takes only an even width and height, so a display of an odd one leaves out its last column or row.
    let (width, height) = (geometry.width & !1, geometry.height & !1);
    let config = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .max_frame_rate(FrameRate::from_hz(30.0))
        .bitrate(BitRate::from_bps(6_000_000))
        .skip_frames(false);
    let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config).map_err(|why| anyhow!("{why}"))?;
    let mut yuv = YUVBuffer::new(width as usize, height as usize);
    let mut last = vec![];
    loop {
        let started = Instant::now();
        while let Some(event) = conn.poll_for_event()? {
            if let XEvent::RandrScreenChangeNotify(_) = event {
                return Ok("the display was resized".into());
            }
        }
        let mut image = conn.get_image(ImageFormat::Z_PIXMAP, root, 0, 0, width, height, !0)?.reply()?;
        if let Ok(pointer) = conn.xfixes_get_cursor_image()?.reply() {
            draw(&mut image.data, width as usize, height as usize, &pointer);
        }
        let whole = keyframe.swap(false, Ordering::Relaxed);
        if whole || image.data != last {
            yuv.read_rgb(BgraSliceU8::new(&image.data, (width as usize, height as usize)));
            if whole {
                encoder.force_intra_frame();
            }
            let frame = encoder.encode(&yuv).map_err(|why| anyhow!("{why}"))?.to_vec();
            if !frame.is_empty() && events.send(Event::Frame(frame)).is_err() {
                return Ok("the stream ended".into());
            }
            last = image.data;
        }
        std::thread::sleep(FRAME.saturating_sub(started.elapsed()));
    }
}

/// The pointer drawn on a picture of BGRX pixels: its pixels are ARGB, their colour already multiplied by its alpha.
fn draw(picture: &mut [u8], width: usize, height: usize, pointer: &GetCursorImageReply) {
    let left = pointer.x as isize - pointer.xhot as isize;
    let top = pointer.y as isize - pointer.yhot as isize;
    for (i, &argb) in pointer.cursor_image.iter().enumerate() {
        let (x, y) = (left + (i % pointer.width as usize) as isize, top + (i / pointer.width as usize) as isize);
        let alpha = argb >> 24;
        if alpha == 0 || x < 0 || y < 0 || x as usize >= width || y as usize >= height {
            continue;
        }
        let at = (y as usize * width + x as usize) * 4;
        for (channel, shift) in [0, 8, 16].into_iter().enumerate() {
            let under = picture[at + channel] as u32;
            picture[at + channel] = (((argb >> shift) & 0xFF) + under * (255 - alpha) / 255).min(255) as u8;
        }
    }
}
