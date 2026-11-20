use super::super::scene::Shadow;
use super::Renderer;

/// A shadow's tile's format: one channel, how much of each point its box covers.
pub(super) const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The widest a shadow's tile is, in device pixels; a wider one's shadow is worked out where it's drawn.
const TILE_MOST: f32 = 2048.0;

/// What tells a shadow's tile from another's: its blur, its corners and its box's size in the tile, to a quarter of a
/// device pixel.
pub(super) type TileKey = [u32; 7];

/// A shadow drawn from a tile, as Skia and Chromium draw a blurred rounded box: its box blurred once, exactly, into a
/// texture of its own, and drawn from that. Along an axis longer than two corners (each its radius and three blurs),
/// the tile's box is two corners long, and drawn stretched: its corners as they are and between them its middle, as
/// along an edge the blur no longer depends on how far the other edge is; along a shorter one, the box is as long as it
/// is. One with no blur, or inside its box (`inset`), is worked out where it's drawn. → its tile's key, and its box in
/// the tile.
pub(super) fn tiled(shadow: &Shadow) -> Option<(TileKey, [f32; 2])> {
    let blur = shadow.blur_radius.0;
    let radii = &shadow.corner_radii;
    let corner = radii.top_left.0.max(radii.top_right.0).max(radii.bottom_right.0).max(radii.bottom_left.0) + 3.0 * blur;
    let tile = [shadow.bounds.size.width.0.min(2.0 * corner), shadow.bounds.size.height.0.min(2.0 * corner)];
    if shadow.inset != 0 || blur <= 0.0 || tile[0].max(tile[1]) + 6.0 * blur > TILE_MOST {
        return None;
    }
    let quarter = |value: f32| (value * 4.0).round() as u32;
    let key = [quarter(blur), quarter(radii.top_left.0), quarter(radii.top_right.0), quarter(radii.bottom_right.0), quarter(radii.bottom_left.0), quarter(tile[0]), quarter(tile[1])];
    Some((key, tile))
}

impl Renderer {
    /// Draws a batch of shadows, each from its tile where it has one, else worked out where it's drawn, in runs.
    pub(super) fn draw_shadows(&self, pass: &mut wgpu::RenderPass<'_>, shadows: &[Shadow], instances: &wgpu::BindGroup, offset: u32) {
        let tiles = self.pipelines.tiles.get();
        pass.set_bind_group(1, instances, &[offset]);
        let mut at = 0;
        while at < shadows.len() {
            let tile = tiles.zip(tiled(&shadows[at]).and_then(|(key, _)| self.tiles.get(&key)));
            match tile {
                Some(((_, pipeline), (_, group))) => {
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(2, group, &[]);
                    pass.draw(0..10, at as u32..at as u32 + 1);
                    self.drew();
                    at += 1;
                }
                None => {
                    // The run of those worked out where they're drawn.
                    let end = (at..shadows.len()).find(|one| tiles.is_some() && tiled(&shadows[*one]).is_some_and(|(key, _)| self.tiles.contains_key(&key))).unwrap_or(shadows.len());
                    pass.set_pipeline(&self.pipelines.shadows);
                    pass.draw(0..10, at as u32..end as u32);
                    self.drew();
                    at = end;
                }
            }
        }
    }

    /// Draws a shadow's tile, `size`, from its box alone (`instance`, with the frame's `globals` and `gamma` of its
    /// own), and keeps it, as `key`'s. A window's shadows are few and keep their tiles; past 64 they're made again.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_tile(&mut self, encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer, instances: &wgpu::BindGroup, key: TileKey, (width, height): (u32, u32), globals: (u64, u64), gamma: (u64, u64), instance: (u64, u64)) {
        if self.tiles.len() >= 64 {
            self.tiles.clear();
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TILE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let globals = self.pipelines.layouts.globals(buffer, globals, gamma);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                // No timestamps: on an iPhone's GPU a tile drawn in a pass that took them was kept as if wholly covered,
                // its shadow a dark square round what casts it, for as long as the tile was kept; it's drawn once a
                // shadow's shape, no frame's cost.
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines.tiles().0);
            pass.set_bind_group(0, &globals, &[]);
            pass.set_bind_group(1, instances, &[instance.0 as u32]);
            pass.draw(0..4, 0..1);
            self.drew();
        }
        let group = self.texture(&view);
        self.tiles.insert(key, (texture, group));
    }
}
