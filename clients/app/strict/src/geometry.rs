// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/geometry.rs and crates/gpui/src/color.rs) at
// commit 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the strict host: the
// units, points, sizes, bounds, corners, edges and colours its scene and renderer use, with the layout the shaders
// read (`#[repr(C)]`).
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use std::fmt::Debug;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// A length in logical pixels, as a page's CSS pixels are.
#[derive(Clone, Copy, Default, Debug, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Pixels(pub f32);

/// A length in pixels on the screen: logical pixels times the window's scale.
#[derive(Clone, Copy, Default, Debug, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ScaledPixels(pub f32);

/// A whole number of the device's pixels, as a texture is measured.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct DevicePixels(pub i32);

pub const fn px(pixels: f32) -> Pixels {
    Pixels(pixels)
}

impl Pixels {
    pub const ZERO: Pixels = Pixels(0.0);
    pub fn scale(self, factor: f32) -> ScaledPixels {
        ScaledPixels(self.0 * factor)
    }
    pub fn max(self, other: Pixels) -> Pixels {
        Pixels(self.0.max(other.0))
    }
    pub fn min(self, other: Pixels) -> Pixels {
        Pixels(self.0.min(other.0))
    }
}

impl From<Pixels> for f32 {
    fn from(pixels: Pixels) -> f32 {
        pixels.0
    }
}
impl From<f32> for Pixels {
    fn from(pixels: f32) -> Pixels {
        Pixels(pixels)
    }
}
impl From<i32> for DevicePixels {
    fn from(pixels: i32) -> DevicePixels {
        DevicePixels(pixels)
    }
}
impl From<DevicePixels> for i32 {
    fn from(pixels: DevicePixels) -> i32 {
        pixels.0
    }
}
impl From<DevicePixels> for u32 {
    fn from(pixels: DevicePixels) -> u32 {
        pixels.0 as u32
    }
}
impl From<u32> for DevicePixels {
    fn from(pixels: u32) -> DevicePixels {
        DevicePixels(pixels as i32)
    }
}
impl From<DevicePixels> for ScaledPixels {
    fn from(pixels: DevicePixels) -> ScaledPixels {
        ScaledPixels(pixels.0 as f32)
    }
}

macro_rules! arithmetic {
    ($unit:ident, $inner:ty) => {
        impl Add for $unit {
            type Output = $unit;
            fn add(self, other: $unit) -> $unit {
                $unit(self.0 + other.0)
            }
        }
        impl Sub for $unit {
            type Output = $unit;
            fn sub(self, other: $unit) -> $unit {
                $unit(self.0 - other.0)
            }
        }
        impl AddAssign for $unit {
            fn add_assign(&mut self, other: $unit) {
                self.0 = self.0 + other.0;
            }
        }
        impl SubAssign for $unit {
            fn sub_assign(&mut self, other: $unit) {
                self.0 = self.0 - other.0;
            }
        }
        impl Mul<$inner> for $unit {
            type Output = $unit;
            fn mul(self, factor: $inner) -> $unit {
                $unit(self.0 * factor)
            }
        }
        impl Div<$inner> for $unit {
            type Output = $unit;
            fn div(self, factor: $inner) -> $unit {
                $unit(self.0 / factor)
            }
        }
        impl Neg for $unit {
            type Output = $unit;
            fn neg(self) -> $unit {
                $unit(-self.0)
            }
        }
        impl Half for $unit {
            fn half(&self) -> $unit {
                $unit(self.0 / (2 as $inner))
            }
        }
    };
}
arithmetic!(Pixels, f32);
arithmetic!(ScaledPixels, f32);
arithmetic!(DevicePixels, i32);

/// Half of a value, as the bounds tree measures a box's perimeter.
pub trait Half {
    fn half(&self) -> Self;
}
impl Half for f32 {
    fn half(&self) -> f32 {
        self / 2.0
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Point<T> {
    pub x: T,
    pub y: T,
}

pub const fn point<T>(x: T, y: T) -> Point<T> {
    Point { x, y }
}

impl<T> Point<T> {
    pub const fn new(x: T, y: T) -> Point<T> {
        Point { x, y }
    }
    pub fn map<U>(self, f: impl Fn(T) -> U) -> Point<U> {
        Point { x: f(self.x), y: f(self.y) }
    }
}
impl Point<Pixels> {
    pub fn scale(&self, factor: f32) -> Point<ScaledPixels> {
        Point { x: self.x.scale(factor), y: self.y.scale(factor) }
    }
}
impl<T: Add<Output = T>> Add for Point<T> {
    type Output = Point<T>;
    fn add(self, other: Point<T>) -> Point<T> {
        Point { x: self.x + other.x, y: self.y + other.y }
    }
}
impl<T: Sub<Output = T>> Sub for Point<T> {
    type Output = Point<T>;
    fn sub(self, other: Point<T>) -> Point<T> {
        Point { x: self.x - other.x, y: self.y - other.y }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Size<T> {
    pub width: T,
    pub height: T,
}

pub const fn size<T>(width: T, height: T) -> Size<T> {
    Size { width, height }
}

impl<T> Size<T> {
    pub fn map<U>(self, f: impl Fn(T) -> U) -> Size<U> {
        Size { width: f(self.width), height: f(self.height) }
    }
}
impl<T: PartialOrd + Clone> Size<T> {
    pub fn max(&self, other: &Size<T>) -> Size<T> {
        let pick = |a: &T, b: &T| if a >= b { a.clone() } else { b.clone() };
        Size { width: pick(&self.width, &other.width), height: pick(&self.height, &other.height) }
    }
}
impl Size<Pixels> {
    pub fn scale(&self, factor: f32) -> Size<ScaledPixels> {
        Size { width: self.width.scale(factor), height: self.height.scale(factor) }
    }
}

/// A rectangle: where its top left corner is, and its size.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Bounds<T> {
    pub origin: Point<T>,
    pub size: Size<T>,
}

impl<T> Bounds<T>
where
    T: Clone + Copy + Default + Debug + PartialOrd + Add<Output = T> + Sub<Output = T>,
{
    pub fn new(origin: Point<T>, size: Size<T>) -> Bounds<T> {
        Bounds { origin, size }
    }

    pub fn from_corners(top_left: Point<T>, bottom_right: Point<T>) -> Bounds<T> {
        Bounds { origin: top_left, size: Size { width: bottom_right.x - top_left.x, height: bottom_right.y - top_left.y } }
    }

    pub fn bottom_right(&self) -> Point<T> {
        Point { x: self.origin.x + self.size.width, y: self.origin.y + self.size.height }
    }

    pub fn intersects(&self, other: &Bounds<T>) -> bool {
        let (a, b) = (self.bottom_right(), other.bottom_right());
        self.origin.x < b.x && other.origin.x < a.x && self.origin.y < b.y && other.origin.y < a.y
    }

    pub fn intersect(&self, other: &Bounds<T>) -> Bounds<T> {
        let max = |a: T, b: T| if a > b { a } else { b };
        let min = |a: T, b: T| if a < b { a } else { b };
        let (a, b) = (self.bottom_right(), other.bottom_right());
        let origin = Point { x: max(self.origin.x, other.origin.x), y: max(self.origin.y, other.origin.y) };
        let corner = Point { x: min(a.x, b.x), y: min(a.y, b.y) };
        let corner = Point { x: max(corner.x, origin.x), y: max(corner.y, origin.y) };
        Bounds::from_corners(origin, corner)
    }

    pub fn union(&self, other: &Bounds<T>) -> Bounds<T> {
        let max = |a: T, b: T| if a > b { a } else { b };
        let min = |a: T, b: T| if a < b { a } else { b };
        let (a, b) = (self.bottom_right(), other.bottom_right());
        Bounds::from_corners(
            Point { x: min(self.origin.x, other.origin.x), y: min(self.origin.y, other.origin.y) },
            Point { x: max(a.x, b.x), y: max(a.y, b.y) },
        )
    }

    pub fn contains(&self, point: &Point<T>) -> bool {
        let corner = self.bottom_right();
        point.x >= self.origin.x && point.x < corner.x && point.y >= self.origin.y && point.y < corner.y
    }

    pub fn is_empty(&self) -> bool {
        !(self.size.width > T::default()) || !(self.size.height > T::default())
    }
}

impl<T: Half + Add<Output = T> + Copy> Bounds<T> {
    pub fn half_perimeter(&self) -> T {
        self.size.width + self.size.height
    }
}

impl Bounds<DevicePixels> {
    pub fn is_zero(&self) -> bool {
        self.size.width.0 == 0 || self.size.height.0 == 0
    }
}

impl Bounds<Pixels> {
    pub fn scale(&self, factor: f32) -> Bounds<ScaledPixels> {
        Bounds { origin: self.origin.scale(factor), size: self.size.scale(factor) }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
#[repr(C)]
pub struct Corners<T> {
    pub top_left: T,
    pub top_right: T,
    pub bottom_right: T,
    pub bottom_left: T,
}

impl<T: Clone> Corners<T> {
    pub fn all(value: T) -> Corners<T> {
        Corners { top_left: value.clone(), top_right: value.clone(), bottom_right: value.clone(), bottom_left: value }
    }
}
impl Corners<Pixels> {
    pub fn scale(&self, factor: f32) -> Corners<ScaledPixels> {
        Corners {
            top_left: self.top_left.scale(factor),
            top_right: self.top_right.scale(factor),
            bottom_right: self.bottom_right.scale(factor),
            bottom_left: self.bottom_left.scale(factor),
        }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
#[repr(C)]
pub struct Edges<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}
impl Edges<Pixels> {
    pub fn scale(&self, factor: f32) -> Edges<ScaledPixels> {
        Edges { top: self.top.scale(factor), right: self.right.scale(factor), bottom: self.bottom.scale(factor), left: self.left.scale(factor) }
    }
}

/// What a primitive is clipped to.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
#[repr(C)]
pub struct ContentMask<T> {
    pub bounds: Bounds<T>,
}

/// A colour in sRGB, each channel 0 to 1.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// A colour by hue, saturation and lightness, as the shaders take it.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
#[repr(C)]
pub struct Hsla {
    pub h: f32,
    pub s: f32,
    pub l: f32,
    pub a: f32,
}

impl Hsla {
    pub fn transparent_black() -> Hsla {
        Hsla::default()
    }
    pub fn opacity(&self, factor: f32) -> Hsla {
        Hsla { a: self.a * factor.clamp(0.0, 1.0), ..*self }
    }
    pub fn is_transparent(&self) -> bool {
        self.a == 0.0
    }
}

impl From<Rgba> for Hsla {
    fn from(color: Rgba) -> Hsla {
        let r = color.r;
        let g = color.g;
        let b = color.b;
        let max = r.max(g.max(b));
        let min = r.min(g.min(b));
        let delta = max - min;
        let l = (max + min) / 2.0;
        let s = if l == 0.0 || l == 1.0 {
            0.0
        } else if l < 0.5 {
            delta / (2.0 * l)
        } else {
            delta / (2.0 - 2.0 * l)
        };
        let h = if delta == 0.0 {
            0.0
        } else if max == r {
            ((g - b) / delta).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / delta + 2.0) / 6.0
        } else {
            ((r - g) / delta + 4.0) / 6.0
        };
        Hsla { h, s, l, a: color.a }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
#[repr(C)]
pub struct LinearColorStop {
    pub color: Hsla,
    pub percentage: f32,
}

/// What fills a quad: here always a solid colour (`tag` 0), in the layout the shaders read for gradients too.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
#[repr(C)]
pub struct Background {
    pub(crate) tag: u32,
    pub(crate) color_space: u32,
    pub(crate) solid: Hsla,
    pub(crate) gradient_angle_or_pattern_height: f32,
    pub(crate) colors: [LinearColorStop; 2],
    pad: u32,
}

impl From<Hsla> for Background {
    fn from(color: Hsla) -> Background {
        Background { solid: color, ..Default::default() }
    }
}

impl Background {
    pub fn opacity(&self, factor: f32) -> Background {
        Background { solid: self.solid.opacity(factor), ..*self }
    }
    pub fn is_transparent(&self) -> bool {
        self.solid.is_transparent()
    }
}
