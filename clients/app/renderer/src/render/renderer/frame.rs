use super::super::scene::{Recording, TransformationMatrix};
use super::*;

/// What a target draws of a frame: the batches that reach its box of the scene, by where they are among the scene's,
/// what it drew (`Drawn`), and the blurs behind its backdrops.
struct Planned {
    globals: (u64, u64),
    batches: Vec<usize>,
    drawn: Drawn,
    blurs: Vec<Vec<Blur>>,
}

impl Renderer {
    pub(super) fn draw_frame(&mut self, scene: &Scene, origins: &[[f32; 2]]) -> bool {
        self.drew = web_time::Instant::now();
        self.draws.set(0);
        self.passes.set(0);
        self.work.set(Work::default());
        if let Some(stamps) = &mut self.stamps {
            stamps.begin(&self.device);
        }
        let acquiring = web_time::Instant::now();
        let images: Vec<Option<Image>> = self.targets.iter_mut().map(|target| target.acquire(&self.device)).collect();
        self.acquired = acquiring.elapsed();
        // None to draw on now (a window hidden or asleep, a surface lost): it's drawn on the next.
        if images.iter().all(Option::is_none) {
            return false;
        }

        // The frame's data: the gamma, each batch's instances, then each target's globals and its backdrops' blurs'
        // parameters, each shadow tile's and each raster's.
        self.data.clear();
        let gamma = self.gamma;
        let gamma = self.push(bytemuck::bytes_of(&gamma));
        let batches: Vec<PrimitiveBatch<'_>> = scene.batches().collect();
        let instances: Vec<(u64, u64)> = batches.iter().map(|batch| self.push_batch(batch)).collect();
        // What each batch is: its kind, texture and instances.
        let hash = |hashing: &dyn Fn(&mut std::collections::hash_map::DefaultHasher)| {
            use std::hash::Hasher;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            hashing(&mut hasher);
            hasher.finish()
        };
        let hashes: Vec<u64> = batches
            .iter()
            .zip(&instances)
            .map(|(batch, (offset, length))| {
                use std::hash::Hash;
                let texture = match batch {
                    PrimitiveBatch::MonochromeSprites { texture_id, .. } | PrimitiveBatch::PolychromeSprites { texture_id, .. } => Some(*texture_id),
                    _ => None,
                };
                let bytes = &self.data[*offset as usize..(*offset + *length) as usize];
                hash(&|hasher| (std::mem::discriminant(batch), texture, bytes).hash(hasher))
            })
            .collect();
        let generation = self.atlas.generation();
        let planned: Vec<Option<Planned>> = images
            .iter()
            .zip(origins)
            .map(|(image, origin)| {
                let image = image.as_ref()?;
                let size = (image.texture.width(), image.texture.height());
                let globals = GlobalParams { viewport_size: [size.0 as f32, size.1 as f32], origin: *origin, premultiplied_alpha: u32::from(self.premultiplied), pad: [0; 3] };
                let globals = self.push(bytemuck::bytes_of(&globals));
                // Only what reaches its box of the scene: a panel's pane of glass (`platform::Window::arrange`) draws a
                // strip of its page, not all of it again.
                let px = crate::geometry::ScaledPixels;
                let area = Bounds { origin: crate::geometry::Point { x: px(origin[0]), y: px(origin[1]) }, size: crate::geometry::Size { width: px(size.0 as f32), height: px(size.1 as f32) } };
                let reaching: Vec<usize> = (0..batches.len()).filter(|at| batches[*at].reaches(&area)).collect();
                let drawn = Drawn {
                    over: hash(&|hasher| std::hash::Hash::hash(&(size, origin.map(f32::to_bits), generation), hasher)),
                    batches: reaching.iter().map(|at| hashes[*at]).collect(),
                };
                // A backdrop's blur is of what's drawn before it in what it blurs (`Blur::area`): the same while that
                // is, wherever it is and whatever else moves, as the first of its batch moves (a sheet carried up) or
                // a switch turns beside a bar of glass, it's kept. What's drawn there is the batches before that reach
                // it, a backdrop among them as what's behind it is, and the backdrops of its own batch before it, not
                // it, so a backdrop fading (a veil coming in) blurs what's behind it once, not each frame.
                let drawing: Vec<PrimitiveBatch<'_>> = reaching.iter().map(|at| batches[*at]).collect();
                let mut blurs: Vec<Vec<Blur>> = Vec::with_capacity(drawing.len());
                for (at, batch) in drawing.iter().enumerate() {
                    let PrimitiveBatch::Backdrops(backdrops) = batch else {
                        blurs.push(Vec::new());
                        continue;
                    };
                    let mut these: Vec<Blur> = Vec::with_capacity(backdrops.len());
                    for (one, backdrop) in backdrops.iter().enumerate() {
                        // Safety: the scene's primitives are `#[repr(C)]` structs of numbers, as `push_instances` reads.
                        let earlier = unsafe { std::slice::from_raw_parts(backdrops.as_ptr().cast::<u8>(), std::mem::size_of_val(&backdrops[..one])) };
                        let shown = backdrop.bounds.intersect(&backdrop.content_mask.bounds);
                        let (x, y) = (shown.origin.x.0 - origin[0], shown.origin.y.0 - origin[1]);
                        let region = [x, y, x + shown.size.width.0, y + shown.size.height.0];
                        let mut blur = self.plan_blur(size, backdrop.blur.0, region);
                        let area = blur.area(*origin);
                        let behind: Vec<u64> = (0..at)
                            .filter(|before| drawing[*before].reaches(&area))
                            .flat_map(|before| std::iter::once(drawn.batches[before]).chain(blurs[before].iter().filter_map(|blur| blur.behind)))
                            .chain(these.iter().filter_map(|blur| blur.behind))
                            .collect();
                        blur.behind = Some(hash(&|hasher| std::hash::Hash::hash(&(drawn.over, &behind, earlier, backdrop.blur.0.to_bits(), region.map(f32::to_bits)), hasher)));
                        blur.ready = blur.ready(&drawing, at, one, *origin);
                        these.push(blur);
                    }
                    blurs.push(these);
                }
                Some(Planned { globals, batches: reaching, drawn, blurs })
            })
            .collect();
        // The tiles of the frame's shadows that haven't one yet, each its box alone, blurred, in a frame of its own.
        let mut tiles: Vec<(TileKey, (u32, u32), (u64, u64), (u64, u64))> = Vec::new();
        if self.pipelines.ready_tiles().is_some() {
            let drawn: std::collections::BTreeSet<usize> = planned.iter().flatten().flat_map(|planned| planned.batches.iter().copied()).collect();
            for at in drawn {
                let PrimitiveBatch::Shadows(shadows) = &batches[at] else { continue };
                for shadow in shadows.iter() {
                    let Some((key, tile)) = tiled(shadow) else { continue };
                    if self.tiles.contains_key(&key) || tiles.iter().any(|(planned, ..)| *planned == key) {
                        continue;
                    }
                    let blur = shadow.blur_radius.0;
                    let side = [(tile[0] + 6.0 * blur).ceil() as u32, (tile[1] + 6.0 * blur).ceil() as u32];
                    let globals = GlobalParams { viewport_size: [side[0] as f32, side[1] as f32], origin: [0.0; 2], premultiplied_alpha: 0, pad: [0; 3] };
                    let globals = self.push(bytemuck::bytes_of(&globals));
                    let mut alone = shadow.clone();
                    alone.bounds = crate::geometry::Bounds {
                        origin: crate::geometry::Point { x: crate::geometry::ScaledPixels(3.0 * blur), y: crate::geometry::ScaledPixels(3.0 * blur) },
                        size: crate::geometry::Size { width: crate::geometry::ScaledPixels(tile[0]), height: crate::geometry::ScaledPixels(tile[1]) },
                    };
                    alone.content_mask.bounds = crate::geometry::Bounds {
                        origin: crate::geometry::Point { x: crate::geometry::ScaledPixels(0.0), y: crate::geometry::ScaledPixels(0.0) },
                        size: crate::geometry::Size { width: crate::geometry::ScaledPixels(side[0] as f32), height: crate::geometry::ScaledPixels(side[1] as f32) },
                    };
                    alone.transformation = TransformationMatrix::unit();
                    let instance = self.push_instances(std::slice::from_ref(&alone));
                    tiles.push((key, (side[0], side[1]), globals, instance));
                }
            }
        }
        // The frame's rasters' scenes that haven't a texture yet, each drawn into one of its own, from its origin.
        let recordings: Vec<Recording> = scene.recordings.iter().filter(|recording| !self.rasters.contains_key(&recording.id)).cloned().collect();
        let mut rastering: Vec<(Recording, (u64, u64), Vec<(u64, u64)>)> = Vec::new();
        for recording in recordings {
            let globals = GlobalParams {
                viewport_size: [recording.size[0] as f32, recording.size[1] as f32],
                origin: recording.origin,
                premultiplied_alpha: u32::from(self.premultiplied),
                pad: [0; 3],
            };
            let globals = self.push(bytemuck::bytes_of(&globals));
            let scene = recording.scene.clone();
            let instances = scene.batches().map(|batch| self.push_batch(&batch)).collect();
            rastering.push((recording, globals, instances));
        }
        // What a batch's instances bind is as much as the largest needs, from its offset, so one bind group serves all.
        let largest = instances
            .iter()
            .chain(tiles.iter().map(|(.., instance)| instance))
            .chain(rastering.iter().flat_map(|(.., instances)| instances))
            .map(|(_, length)| *length)
            .max()
            .unwrap_or(4);
        let window = self.instances.as_ref().map_or(0, |(_, window, _)| *window).max(largest.next_power_of_two().max(256));
        self.data.resize(self.data.len() + window as usize, 0);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.write_data(&mut encoder);
        let buffer = self.buffer.clone().expect("written above");
        let binding = |(offset, size): (u64, u64)| wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &buffer, offset, size: wgpu::BufferSize::new(size) });
        if self.instances.as_ref().is_none_or(|(bound, bound_window, _)| *bound != buffer || *bound_window != window) {
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipelines.layouts.instances,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: binding((0, window)) }],
            });
            self.instances = Some((buffer.clone(), window, group));
        }
        let group = self.instances.as_ref().expect("made above").2.clone();

        // Shadows' tiles and rasters first, once for every target that draws them.
        for (key, (width, height), globals, instance) in tiles {
            self.draw_tile(&mut encoder, &buffer, &group, key, (width, height), globals, gamma, instance);
        }
        for (recording, globals, offsets) in rastering {
            self.draw_raster(&mut encoder, &buffer, &group, &recording, globals, gamma, &offsets);
        }
        let gone: Vec<wgpu::Texture> = self.rasters.extract_if(|id, _| !scene.recordings.iter().any(|recording| recording.id == *id) && !scene.rasters.iter().any(|raster| raster.raster == *id)).map(|(_, (texture, _))| texture).collect();
        for texture in gone {
            self.let_go(texture);
        }
        // Then each target, in runs of batches, one pass each, broken at each backdrop that blurs what's drawn before
        // it afresh.
        let mut targets = std::mem::take(&mut self.targets);
        for ((target, image), planned) in targets.iter_mut().zip(&images).zip(planned) {
            let (Some(image), Some(planned)) = (image, planned) else { continue };
            let globals = self.pipelines.layouts.globals(&buffer, planned.globals, gamma);
            let batches: Vec<PrimitiveBatch<'_>> = planned.batches.iter().map(|at| batches[*at]).collect();
            let instances = Instances { group: group.clone(), offsets: planned.batches.iter().map(|at| instances[*at].0 as u32).collect() };
            self.draw_target(&mut encoder, target, image, &globals, &buffer, &batches, &instances, &planned.blurs, planned.drawn);
        }
        self.targets = targets;

        if let Some(stamps) = &self.stamps {
            stamps.resolve(&mut encoder);
        }
        self.belt.finish_and_recall_on_submit(&encoder);
        let submitting = web_time::Instant::now();
        self.queue.submit([encoder.finish()]);
        // The browser says when (a promise), and its windows' loop is woken; natively the surface's next image waits for
        // it, and wgpu would say only as it's polled.
        #[cfg(target_arch = "wasm32")]
        {
            self.frames += 1;
            let (finished, frame) = (self.finished.clone(), self.frames);
            self.queue.on_submitted_work_done(move || {
                finished.fetch_max(frame, std::sync::atomic::Ordering::Release);
                crate::platform::web::soon();
            });
        }
        self.submitted = submitting.elapsed();
        if let Some(stamps) = &mut self.stamps {
            stamps.submitted();
        }
        let presenting = web_time::Instant::now();
        let mut drawn = true;
        for image in images {
            match image {
                Some(Image { frame: Some(frame), .. }) => self.queue.present(frame),
                Some(_) => {}
                None => drawn = false,
            }
        }
        self.presented = presenting.elapsed();
        drawn
    }

    /// Draws `target`'s batches into its `image`: from what it kept of its frames before, where its frame starts as
    /// that did, or what's the same as its last frame's start, two frames running, kept now, where it's worth more
    /// than its copy; then the rest.
    #[allow(clippy::too_many_arguments)]
    fn draw_target(&mut self, encoder: &mut wgpu::CommandEncoder, target: &mut Target, image: &Image, globals: &wgpu::BindGroup, buffer: &wgpu::Buffer, batches: &[PrimitiveBatch<'_>], instances: &Instances, blurs: &[Vec<Blur>], drawn: Drawn) {
        let (texture, view) = (&image.texture, &image.view);
        let size = (texture.width(), texture.height());
        let copies = texture.usage().contains(wgpu::TextureUsages::COPY_DST);
        let same = |kept: &Drawn| kept.over == drawn.over && drawn.batches.starts_with(&kept.batches);
        if (!copies || target.kept.as_ref().is_some_and(|kept| !same(&kept.drawn)))
            && let Some(kept) = target.kept.take()
        {
            self.let_go(kept.texture);
        }
        if copies && target.kept.is_none() && target.last.over == drawn.over {
            let start = drawn.batches.iter().zip(&target.last.batches).take_while(|(now, last)| now == last).count();
            // A shadow's or a blur's pixels cost more than copying the frame; anything else is drawn again.
            let worth = batches[..start].iter().any(|batch| matches!(batch, PrimitiveBatch::Shadows(_) | PrimitiveBatch::Backdrops(_)));
            if worth {
                let made = self.made(size, LAYER);
                let made_view = made.create_view(&wgpu::TextureViewDescriptor::default());
                self.draw_batches(encoder, target, &made, &made_view, globals, buffer, batches, instances, blurs, 0..start);
                target.kept = Some(Kept { texture: made, drawn: Drawn { over: drawn.over, batches: drawn.batches[..start].to_vec() } });
            }
        }
        let from = match &target.kept {
            Some(kept) => {
                encoder.copy_texture_to_texture(kept.texture.as_image_copy(), texture.as_image_copy(), wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 });
                let mut work = self.work.get();
                work.copied += f64::from(size.0) * f64::from(size.1);
                self.work.set(work);
                kept.drawn.batches.len()
            }
            None => 0,
        };
        self.draw_batches(encoder, target, texture, view, globals, buffer, batches, instances, blurs, from..batches.len());
        target.last = drawn;
        // What's behind no backdrop this frame isn't kept; what's still drawn starts the next frame unused.
        target.blurred.retain(|_, (_, used)| std::mem::take(used));
    }

    /// Writes the frame's data into its buffer, grown to hold it, and what's been added to the atlas into its
    /// textures, in the frame's own commands: `Queue::write_buffer` and `Queue::write_texture` copy in commands of the
    /// queue's own, a command encoder more each frame they're used, which a Surface's Intel Direct3D 12 driver makes
    /// and frees as they go: twice the CPU to draw a frame (p95 10 ms, 6.7 with one).
    fn write_data(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let needed = (self.data.len() as u64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        self.data.resize(needed as usize, 0);
        if self.buffer.as_ref().is_none_or(|buffer| buffer.size() < needed) {
            self.buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: needed.next_power_of_two().max(0x10000),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let size = wgpu::BufferSize::new(needed).expect("a frame has its gamma");
        self.belt.write_buffer(encoder, self.buffer.as_ref().expect("made above"), 0, size).copy_from_slice(&self.data);
        let mut work = self.work.get();
        work.uploaded = self.atlas.upload(encoder, &mut self.belt);
        self.work.set(work);
    }

    /// Draws the batches `run` names into `view` in one pass; a backdrop as one of its batch's, with what's behind it
    /// blurred.
    #[allow(clippy::too_many_arguments)]
    fn draw_run(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
        globals: &wgpu::BindGroup,
        batches: &[PrimitiveBatch<'_>],
        instances: &Instances,
        run: &[(usize, Option<(wgpu::TextureView, usize)>)],
    ) {
        if run.is_empty() && !matches!(load, wgpu::LoadOp::Clear(_)) {
            return;
        }
        let mut pass = self.pass(encoder, view, load, self.stamps.as_ref().filter(|_| !run.is_empty()).and_then(|stamps| stamps.pass(false)));
        pass.set_bind_group(0, globals, &[]);
        for (at, blurred) in run {
            let at = *at;
            self.covered(&batches[at]);
            if let PrimitiveBatch::Shadows(shadows) = &batches[at] {
                self.draw_shadows(&mut pass, shadows, &instances.group, instances.offsets[at]);
                continue;
            }
            if let PrimitiveBatch::Rasters(rasters) = &batches[at] {
                if let Some((_, group)) = self.rasters.get(&rasters[0].raster) {
                    pass.set_pipeline(&self.pipelines.rasters);
                    pass.set_bind_group(1, &instances.group, &[instances.offsets[at]]);
                    pass.set_bind_group(2, group, &[]);
                    pass.draw(0..4, 0..1);
                    self.drew();
                }
                continue;
            }
            let (pipeline, texture, count) = match &batches[at] {
                PrimitiveBatch::Quads(quads) => (&self.pipelines.quads, None, quads.len()),
                PrimitiveBatch::Shadows(_) | PrimitiveBatch::Rasters(_) => unreachable!("drawn above"),
                PrimitiveBatch::Underlines(underlines) => (&self.pipelines.underlines, None, underlines.len()),
                PrimitiveBatch::MonochromeSprites { texture_id, sprites } => (&self.pipelines.mono_sprites, Some(*texture_id), sprites.len()),
                PrimitiveBatch::PolychromeSprites { texture_id, sprites } => (&self.pipelines.poly_sprites, Some(*texture_id), sprites.len()),
                PrimitiveBatch::Backdrops(_) => {
                    let Some((blurred, one)) = blurred else { continue };
                    pass.set_pipeline(&self.pipelines.backdrops().0);
                    pass.set_bind_group(1, &instances.group, &[instances.offsets[at]]);
                    pass.set_bind_group(2, &self.texture(blurred), &[]);
                    pass.draw(0..4, *one as u32..*one as u32 + 1);
                    self.drew();
                    continue;
                }
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &instances.group, &[instances.offsets[at]]);
            if let Some(id) = texture {
                pass.set_bind_group(2, &self.atlas_group(id), &[]);
            }
            pass.draw(0..4, 0..count as u32);
            self.drew();
        }
    }

    /// Draws `range` of the frame's batches into `texture` (`view` of it), in runs, one pass each, broken only where a
    /// backdrop blurs what's drawn before it afresh, as a pass must end for what it drew to be read: a backdrop whose
    /// blur is kept from the frames before, and the batches either side, are one pass. Each pass costs its driver a
    /// command list (two, as wgpu-core puts each pass's barriers in one of their own), which Intel's Direct3D 12
    /// driver took 1 to 2 ms each to make ready on a Surface's frames that drew something new. From the first batch,
    /// it's cleared first; from a later one, it already has what's drawn before.
    #[allow(clippy::too_many_arguments)]
    fn draw_batches(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &mut Target,
        texture: &wgpu::Texture,
        view: &wgpu::TextureView,
        globals: &wgpu::BindGroup,
        buffer: &wgpu::Buffer,
        batches: &[PrimitiveBatch<'_>],
        instances: &Instances,
        blurs: &[Vec<Blur>],
        range: std::ops::Range<usize>,
    ) {
        let blurrable = texture.usage().intersects(wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let mut load = if range.start == 0 { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load };
        let mut run: Vec<(usize, Option<(wgpu::TextureView, usize)>)> = Vec::new();
        // Each backdrop's blur kept from the frames before, else taken as the first that needs it is drawn, in one go
        // with each after it that's ready (`Blur::ready`, `Blur::together`): one break in the frame and one blur's three
        // passes for all of them, where each broke it and took three of its own (a launcher's backdrops over its turning
        // pages made 13 passes a frame).
        let mut blurred: std::collections::HashMap<(usize, usize), wgpu::TextureView> = Default::default();
        let mut fresh: Vec<(usize, usize)> = Vec::new();
        if blurrable {
            for at in range.clone() {
                for (one, blur) in blurs[at].iter().enumerate() {
                    match target.kept_blur(blur) {
                        Some(kept) => drop(blurred.insert((at, one), kept)),
                        None => fresh.push((at, one)),
                    }
                }
            }
        }
        for at in range {
            let PrimitiveBatch::Backdrops(backdrops) = &batches[at] else {
                run.push((at, None));
                continue;
            };
            // Where it can't be read, a backdrop draws nothing.
            if !blurrable {
                continue;
            }
            for one in 0..backdrops.len().min(blurs[at].len()) {
                if !blurred.contains_key(&(at, one)) {
                    let mut group: Vec<(usize, usize)> = Vec::new();
                    for &(later, of) in fresh.iter().filter(|place| **place >= (at, one)) {
                        let blur = &blurs[later][of];
                        let together: Vec<&Blur> = group.iter().map(|(at, one)| &blurs[*at][*one]).collect();
                        if (group.is_empty() || blur.ready <= (at, one)) && !blurred.contains_key(&(later, of)) && Blur::together(&together, blur) {
                            group.push((later, of));
                        }
                    }
                    self.draw_run(encoder, view, load, globals, batches, instances, &run);
                    run.clear();
                    load = wgpu::LoadOp::Load;
                    let taking: Vec<&Blur> = group.iter().map(|(at, one)| &blurs[*at][*one]).collect();
                    let views = self.blur_behind(encoder, target, texture, view, globals, buffer, &taking);
                    blurred.extend(group.into_iter().zip(views));
                }
                run.push((at, Some((blurred[&(at, one)].clone(), one))));
            }
        }
        self.draw_run(encoder, view, load, globals, batches, instances, &run);
    }

    /// Adds a batch's instances to the frame's data, and says where.
    fn push_batch(&mut self, batch: &PrimitiveBatch<'_>) -> (u64, u64) {
        match batch {
            PrimitiveBatch::Quads(quads) => self.push_instances(quads),
            PrimitiveBatch::Shadows(shadows) => self.push_instances(shadows),
            PrimitiveBatch::Underlines(underlines) => self.push_instances(underlines),
            PrimitiveBatch::MonochromeSprites { sprites, .. } => self.push_instances(sprites),
            PrimitiveBatch::PolychromeSprites { sprites, .. } => self.push_instances(sprites),
            PrimitiveBatch::Backdrops(backdrops) => self.push_instances(backdrops),
            PrimitiveBatch::Rasters(rasters) => self.push_instances(rasters),
        }
    }

    /// Draws a raster's scene (`recording`) into a texture of its own, over nothing, from its origin (with its own
    /// `globals`, the frame's `gamma`, and its batches' instances at `offsets`), and keeps it, by its id: in one pass,
    /// as a raster's backdrops draw nothing.
    #[allow(clippy::too_many_arguments)]
    fn draw_raster(&mut self, encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer, group: &wgpu::BindGroup, recording: &Recording, globals: (u64, u64), gamma: (u64, u64), offsets: &[(u64, u64)]) {
        let texture = self.made((recording.size[0].max(1), recording.size[1].max(1)), LAYER);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let globals = self.pipelines.layouts.globals(buffer, globals, gamma);
        let batches: Vec<PrimitiveBatch<'_>> = recording.scene.batches().collect();
        let instances = Instances { group: group.clone(), offsets: offsets.iter().map(|(offset, _)| *offset as u32).collect() };
        let run: Vec<(usize, Option<(wgpu::TextureView, usize)>)> = (0..batches.len()).filter(|at| !matches!(batches[*at], PrimitiveBatch::Backdrops(_))).map(|at| (at, None)).collect();
        self.draw_run(encoder, &view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), &globals, &batches, &instances, &run);
        let bound = self.texture(&view);
        self.rasters.insert(recording.id, (texture, bound));
    }
}
