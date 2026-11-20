use std::sync::Arc;

/// How many passes a frame's GPU time is taken of, each a timestamp as it begins and one as it ends.
const STAMPED: u32 = 32;

/// Timestamps of a frame's passes, as WebGPU's `timestamp-query` takes them (at a pass's start and end: a tiled GPU's
/// stage boundaries), resolved into one of a few buffers that are read back once the GPU's done, never waited for:
/// the GPU time of the frame drawn a frame or two before.
pub(super) struct Stamps {
    set: wgpu::QuerySet,
    resolved: wgpu::Buffer,
    /// Each buffer read back, whether it's mapped, and while it's in use, which of its passes blurred.
    read: Vec<(wgpu::Buffer, Arc<std::sync::atomic::AtomicBool>, Option<Vec<bool>>)>,
    /// The buffer this frame's go to, and which of its passes so far blur (a backdrop's glass).
    writing: Option<usize>,
    passes: std::cell::RefCell<Vec<bool>>,
    /// Nanoseconds a tick.
    period: f64,
    /// The last frame read back's GPU time, all of it and its blurs'.
    pub(super) gpu: Option<(web_time::Duration, web_time::Duration)>,
}

/// A pass longer than this on the GPU is a timestamp it didn't take, and left as an earlier frame's.
const LONGEST_PASS: u64 = 100_000_000;

impl Stamps {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Stamps> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let bytes = u64::from(STAMPED * 2) * 8;
        let set = device.create_query_set(&wgpu::QuerySetDescriptor { label: None, ty: wgpu::QueryType::Timestamp, count: STAMPED * 2 });
        let resolved = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = (0..3)
            .map(|_| (device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false }), Arc::default(), None))
            .collect();
        Some(Stamps { set, resolved, read, writing: None, passes: Default::default(), period: f64::from(queue.get_timestamp_period()), gpu: None })
    }

    /// As a frame starts: what the GPU has finished of the frames before is read, and a buffer free for this one's
    /// chosen, if one is.
    pub(super) fn begin(&mut self, device: &wgpu::Device) {
        let _ = device.poll(wgpu::PollType::Poll);
        self.read();
        self.writing = self.read.iter().position(|(_, _, held)| held.is_none());
        self.passes.borrow_mut().clear();
    }

    /// What's been read back of the frames the GPU has finished: the latest's time.
    pub(super) fn read(&mut self) {
        for (buffer, mapped, held) in &mut self.read {
            let Some(passes) = held.as_ref() else { continue };
            if !mapped.swap(false, std::sync::atomic::Ordering::Acquire) {
                continue;
            }
            if let Ok(view) = buffer.get_mapped_range(..passes.len() as u64 * 16) {
                let ticks: &[u64] = bytemuck::cast_slice(&view);
                let (mut all, mut blurs) = (0, 0);
                for (pass, blurs_it) in ticks.chunks_exact(2).zip(passes) {
                    let took = pass[1].wrapping_sub(pass[0]);
                    if pass[1] >= pass[0] && took < LONGEST_PASS {
                        all += took;
                        blurs += if *blurs_it { took } else { 0 };
                    }
                }
                let time = |ticks: u64| web_time::Duration::from_nanos((ticks as f64 * self.period) as u64);
                self.gpu = Some((time(all), time(blurs)));
            }
            buffer.unmap();
            *held = None;
        }
    }

    /// The timestamps of a pass about to begin, one that draws, and `blurs` (a backdrop's glass), while there's room
    /// for them this frame. A pass that draws nothing is never given them: a tiled GPU takes its timestamps where its
    /// stages begin and end, and one with no vertex stage would keep an earlier frame's.
    pub(super) fn pass(&self, blurs: bool) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let mut passes = self.passes.borrow_mut();
        let at = passes.len() as u32 * 2;
        (self.writing.is_some() && at + 2 <= STAMPED * 2).then(|| {
            passes.push(blurs);
            wgpu::RenderPassTimestampWrites { query_set: &self.set, beginning_of_pass_write_index: Some(at), end_of_pass_write_index: Some(at + 1) }
        })
    }

    /// As the frame's commands are finished: its timestamps resolved and copied out to be read back.
    pub(super) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let count = self.passes.borrow().len() as u32 * 2;
        let (Some(writing), 1..) = (self.writing, count) else { return };
        encoder.resolve_query_set(&self.set, 0..count, &self.resolved, 0);
        encoder.copy_buffer_to_buffer(&self.resolved, 0, &self.read[writing].0, 0, u64::from(count) * 8);
    }

    /// Once they're submitted: read back when the GPU's done with them.
    pub(super) fn submitted(&mut self) {
        let passes = std::mem::take(&mut *self.passes.borrow_mut());
        let (Some(writing), false) = (self.writing.take(), passes.is_empty()) else { return };
        let (buffer, mapped, held) = &mut self.read[writing];
        let mapped = mapped.clone();
        buffer.map_async(wgpu::MapMode::Read, ..passes.len() as u64 * 16, move |done| mapped.store(done.is_ok(), std::sync::atomic::Ordering::Release));
        *held = Some(passes);
    }
}
