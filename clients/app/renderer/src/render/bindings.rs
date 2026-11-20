//! What each pipeline binds and which shaders it runs, as plain data, which the renderer makes its bind group layouts
//! and pipelines from, and `build.rs` its Direct3D 12 shaders from, ahead of time: a binding's place in its group is
//! its `@binding`.

/// A binding of a group, seen from every stage.
#[derive(Clone, Copy)]
pub enum Binding {
    Uniform,
    /// A read-only storage buffer, bound at an offset of its own (`has_dynamic_offset`).
    Storage,
    Texture { filterable: bool },
    Sampler,
}

/// Group 0: the frame's globals, its gamma and the blue noise it dithers with.
pub const GLOBALS: &[Binding] = &[Binding::Uniform, Binding::Uniform, Binding::Texture { filterable: false }];
/// Group 2, where a pipeline samples a texture: the texture and its sampler.
pub const TEXTURE: &[Binding] = &[Binding::Texture { filterable: true }, Binding::Sampler];

/// Group 1: what a pipeline draws from.
#[derive(Clone, Copy)]
pub enum Data {
    /// A batch's instances, at their offset in the frame's data.
    Instances,
    /// A blur pass's parameters.
    BlurParams,
}

impl Data {
    pub const fn bindings(self) -> &'static [Binding] {
        match self {
            Data::Instances => &[Binding::Storage],
            Data::BlurParams => &[Binding::Uniform],
        }
    }
}

/// A pipeline's shaders, what it draws from, and whether it samples a texture.
pub struct Pipeline {
    pub vertex: &'static str,
    pub fragment: &'static str,
    pub data: Data,
    pub samples: bool,
}

impl Pipeline {
    /// Its bind groups, in order, which `build.rs` binds its shaders by.
    #[allow(dead_code)]
    pub fn groups(&self) -> Vec<&'static [Binding]> {
        let mut groups = vec![GLOBALS, self.data.bindings()];
        if self.samples {
            groups.push(TEXTURE);
        }
        groups
    }
}

const fn drawing(vertex: &'static str, fragment: &'static str, samples: bool) -> Pipeline {
    Pipeline { vertex, fragment, data: Data::Instances, samples }
}

pub const QUADS: Pipeline = drawing("vs_quad", "fs_quad", false);
pub const SHADOWS: Pipeline = drawing("vs_shadow", "fs_shadow", false);
pub const UNDERLINES: Pipeline = drawing("vs_underline", "fs_underline", false);
pub const MONO_SPRITES: Pipeline = drawing("vs_mono_sprite", "fs_mono_sprite", true);
pub const POLY_SPRITES: Pipeline = drawing("vs_poly_sprite", "fs_poly_sprite", true);
pub const RASTERS: Pipeline = drawing("vs_raster", "fs_raster", true);
pub const SHADOW_COVERAGE: Pipeline = drawing("vs_shadow_whole", "fs_shadow_coverage", false);
pub const SHADOW_TILES: Pipeline = drawing("vs_shadow", "fs_shadow_tile", true);
pub const BACKDROPS: Pipeline = drawing("vs_backdrop", "fs_backdrop", true);
pub const BLUR: Pipeline = Pipeline { vertex: "vs_blur", fragment: "fs_blur", data: Data::BlurParams, samples: true };

/// Every pipeline the renderer draws with, which `build.rs` compiles Windows' shaders for.
#[allow(dead_code)]
pub const PIPELINES: &[Pipeline] = &[QUADS, SHADOWS, UNDERLINES, MONO_SPRITES, POLY_SPRITES, RASTERS, SHADOW_COVERAGE, SHADOW_TILES, BACKDROPS, BLUR];
