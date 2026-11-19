//! What the viewer does, as the UI library's LiveScreen sends it down the data channel (core/ui's
//! `drawing-primitives/input.ts`): positions in the video's pixels, buttons and keys as X names them.
use serde::Deserialize;

#[derive(Deserialize, Debug)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Input {
    Move { x: f64, y: f64 },
    /// X's buttons: 1 left, 2 middle, 3 right, 4 to 7 the wheel (up, down, left, right).
    Button {
        b: u8,
        down: bool,
        #[serde(default)]
        mods: Vec<String>,
    },
    /// A key, pressed and let go.
    Key {
        key: String,
        #[serde(default)]
        mods: Vec<String>,
    },
    Text { text: String },
}
