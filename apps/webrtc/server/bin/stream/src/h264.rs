//! H.264 for a screen whose pictures the sender encodes itself (a display's, not a phone's or a simulator's, which
//! encode their own): Cisco's OpenH264, constrained baseline, which every browser decodes. A picture is encoded only
//! when it differs from the one before, and the last again, whole, when the viewer asks for one, so a still screen
//! sends nothing and a viewer that lost a frame isn't left waiting for the screen to change.
use anyhow::anyhow;
use openh264::OpenH264API;
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, UsageType};
use openh264::formats::{BgraSliceU8, YUVBuffer, YUVSource};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(not(target_os = "linux"))]
use std::sync::{
    Mutex,
    mpsc::{Receiver, RecvTimeoutError, Sender},
};
#[cfg(not(target_os = "linux"))]
use std::time::Duration;

/// The widest picture sent: a wider screen's are halved until they fit, as more costs the encoder and the viewer
/// more than it shows them.
const WIDEST: usize = 1920;

pub struct H264 {
    encoder: Option<(Encoder, YUVBuffer)>,
    /// The last picture, as encoded: BGRA, its size.
    last: (Vec<u8>, usize, usize),
    /// How many times wider the screen is than the pictures sent.
    pub scale: usize,
    keyframe: Arc<AtomicBool>,
}

impl H264 {
    /// `keyframe` is set when the viewer asks for a whole frame.
    pub fn new(keyframe: Arc<AtomicBool>) -> Self {
        Self { encoder: None, last: (vec![], 0, 0), scale: 1, keyframe }
    }

    /// A picture of BGRA rows `stride` bytes apart: its H.264, if it's to be sent.
    pub fn picture(&mut self, bgra: &[u8], width: usize, height: usize, stride: usize) -> anyhow::Result<Option<Vec<u8>>> {
        let (mut picture, mut width, mut height) = (packed(bgra, width, height, stride), width, height);
        self.scale = 1;
        while width > WIDEST {
            (picture, width, height) = halved(&picture, width, height);
            self.scale *= 2;
        }
        // H.264 takes only an even width and height, so a screen of an odd one leaves out its last column or row.
        if width % 2 == 1 || height % 2 == 1 {
            picture = packed(&picture, width & !1, height & !1, width * 4);
            (width, height) = (width & !1, height & !1);
        }
        let whole = self.keyframe.swap(false, Ordering::Relaxed);
        if !whole && (&picture, width, height) == (&self.last.0, self.last.1, self.last.2) {
            return Ok(None);
        }
        self.last = (picture, width, height);
        self.encoded(whole)
    }

    /// The last picture again, whole, if the viewer asked for one since: for a screen that sends a picture only as
    /// it changes.
    #[cfg(not(target_os = "linux"))]
    pub fn again(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        if self.last.0.is_empty() || !self.keyframe.swap(false, Ordering::Relaxed) {
            return Ok(None);
        }
        self.encoded(true)
    }

    fn encoded(&mut self, whole: bool) -> anyhow::Result<Option<Vec<u8>>> {
        let (picture, width, height) = (&self.last.0, self.last.1, self.last.2);
        if self.encoder.as_ref().is_none_or(|(_, yuv)| yuv.dimensions() != (width, height)) {
            let config = EncoderConfig::new()
                .usage_type(UsageType::ScreenContentRealTime)
                .max_frame_rate(FrameRate::from_hz(30.0))
                .bitrate(BitRate::from_bps(6_000_000))
                .skip_frames(false);
            let encoder = Encoder::with_api_config(OpenH264API::from_source(), config).map_err(|why| anyhow!("{why}"))?;
            self.encoder = Some((encoder, YUVBuffer::new(width, height)));
        }
        let (encoder, yuv) = self.encoder.as_mut().unwrap();
        yuv.read_rgb(BgraSliceU8::new(picture, (width, height)));
        if whole {
            encoder.force_intra_frame();
        }
        let frame = encoder.encode(yuv).map_err(|why| anyhow!("{why}"))?.to_vec();
        Ok((!frame.is_empty()).then_some(frame))
    }
}

/// `width` × `height` BGRA pixels whose rows are `stride` bytes apart, with no bytes between rows.
fn packed(bgra: &[u8], width: usize, height: usize, stride: usize) -> Vec<u8> {
    if stride == width * 4 && bgra.len() == width * height * 4 {
        return bgra.to_vec();
    }
    (0..height).flat_map(|row| &bgra[row * stride..row * stride + width * 4]).copied().collect()
}

/// A picture half as wide and as tall, each pixel the mean of four.
fn halved(bgra: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = (width / 2, height / 2);
    let mut out = vec![0; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let at = |dx: usize, dy: usize| bgra[((2 * y + dy) * width + 2 * x + dx) * 4 + c] as u16;
                out[(y * w + x) * 4 + c] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4) as u8;
            }
        }
    }
    (out, w, h)
}

/// A picture as a screen gave it: BGRA rows `stride` bytes apart.
#[cfg(not(target_os = "linux"))]
pub struct Picture {
    pub bgra: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
}

/// Each picture of a screen that gives them only as it changes, encoded as it comes, and the last again when the
/// viewer asks for a whole one; `size` is kept the size of those sent, which the viewer's positions are in. Gives
/// why it ended.
#[cfg(not(target_os = "linux"))]
pub fn encoded(pictures: Receiver<Picture>, keyframe: Arc<AtomicBool>, size: &Mutex<(usize, usize)>, events: &Sender<crate::Event>) -> anyhow::Result<String> {
    let mut h264 = H264::new(keyframe);
    loop {
        let frame = match pictures.recv_timeout(Duration::from_millis(100)) {
            Ok(p) => h264.picture(&p.bgra, p.width, p.height, p.stride)?,
            Err(RecvTimeoutError::Timeout) => h264.again()?,
            Err(RecvTimeoutError::Disconnected) => return Ok("the screen's pictures stopped".into()),
        };
        *size.lock().unwrap() = (h264.last.1, h264.last.2);
        if let Some(frame) = frame
            && events.send(crate::Event::Frame(frame)).is_err()
        {
            return Ok("the stream ended".into());
        }
    }
}
