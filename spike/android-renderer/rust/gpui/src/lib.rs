/// GPUI's `Rgba` (crates/gpui/src/color.rs): sRGB channels from 0 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}
