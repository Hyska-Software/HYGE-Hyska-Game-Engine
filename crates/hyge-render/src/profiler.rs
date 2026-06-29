//! GPU and frame profiling support.
//!
//! The profiler owns a `wgpu::QuerySet::Timestamp` when the selected
//! adapter supports `TIMESTAMP_QUERY`. Each render-graph pass receives
//! two timestamp writes: one immediately before recording and one
//! immediately after recording. The resolved timestamps are converted
//! to milliseconds and copied into [`FrameStats`], which is both an ECS
//! resource and the data source for the egui debug overlay.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use hyge_ecs::prelude::*;

/// Per-pass GPU timing captured for the latest frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PassStats {
    /// Render-graph pass name.
    pub name: String,
    /// GPU time spent in this pass, in milliseconds.
    pub gpu_time_ms: f32,
}

/// Frame-level render profiling data.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameStats {
    /// CPU wall-clock frame time in milliseconds.
    pub frame_time_ms: f32,
    /// Frames per second derived from [`FrameStats::frame_time_ms`].
    pub fps: f32,
    /// Total measured GPU time in milliseconds.
    pub total_gpu_time_ms: f32,
    /// Per-pass GPU timing in graph execution order.
    pub passes: Vec<PassStats>,
    /// Draw call count recorded by the render path.
    pub draw_calls: u32,
    /// Rendered instance count recorded by the render path.
    pub instance_count: u32,
}

impl Resource for FrameStats {}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            frame_time_ms: 0.0,
            fps: 0.0,
            total_gpu_time_ms: 0.0,
            passes: Vec::new(),
            draw_calls: 0,
            instance_count: 0,
        }
    }
}

/// GPU timestamp profiler for render-graph passes.
#[derive(Debug)]
pub struct GpuProfiler {
    query_set: Option<wgpu::QuerySet>,
    resolve_buffer: Option<wgpu::Buffer>,
    readback_buffer: Option<wgpu::Buffer>,
    timestamp_period_ns: f32,
    max_passes: u32,
    frame_start: Instant,
    stats: FrameStats,
}

impl GpuProfiler {
    /// Number of graph passes supported by the R-025 profiler query set.
    pub const DEFAULT_MAX_PASSES: u32 = 128;

    /// Creates a new profiler. If `timestamp_queries_enabled` is false,
    /// GPU timings are reported as zero while CPU frame time and counters
    /// are still populated.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        timestamp_queries_enabled: bool,
    ) -> Self {
        let max_passes = Self::DEFAULT_MAX_PASSES;
        let query_count = max_passes * 2;
        let buffer_size = u64::from(query_count) * std::mem::size_of::<u64>() as u64;
        let (query_set, resolve_buffer, readback_buffer) = if timestamp_queries_enabled {
            let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("hyge-render/profiler-timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: query_count,
            });
            let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hyge-render/profiler-resolve"),
                size: buffer_size,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hyge-render/profiler-readback"),
                size: buffer_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            (Some(query_set), Some(resolve_buffer), Some(readback_buffer))
        } else {
            (None, None, None)
        };

        Self {
            query_set,
            resolve_buffer,
            readback_buffer,
            timestamp_period_ns: queue.get_timestamp_period(),
            max_passes,
            frame_start: Instant::now(),
            stats: FrameStats::default(),
        }
    }

    /// Marks the beginning of CPU frame timing.
    pub fn begin_frame(&mut self) {
        self.frame_start = Instant::now();
    }

    /// Writes the start timestamp for a pass, if timestamp queries are active.
    pub fn write_pass_start(&self, encoder: &mut wgpu::CommandEncoder, pass_index: u32) {
        if pass_index >= self.max_passes {
            return;
        }
        if let Some(query_set) = &self.query_set {
            encoder.write_timestamp(query_set, pass_index * 2);
        }
    }

    /// Writes the end timestamp for a pass, if timestamp queries are active.
    pub fn write_pass_end(&self, encoder: &mut wgpu::CommandEncoder, pass_index: u32) {
        if pass_index >= self.max_passes {
            return;
        }
        if let Some(query_set) = &self.query_set {
            encoder.write_timestamp(query_set, pass_index * 2 + 1);
        }
    }

    /// Appends timestamp resolve/copy commands for the current frame.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder, pass_count: u32) {
        let query_count = pass_count.saturating_mul(2).min(self.max_passes * 2);
        if query_count == 0 {
            return;
        }
        let (Some(query_set), Some(resolve_buffer), Some(readback_buffer)) = (
            self.query_set.as_ref(),
            self.resolve_buffer.as_ref(),
            self.readback_buffer.as_ref(),
        ) else {
            return;
        };
        let size = u64::from(query_count) * std::mem::size_of::<u64>() as u64;
        encoder.resolve_query_set(query_set, 0..query_count, resolve_buffer, 0);
        encoder.copy_buffer_to_buffer(resolve_buffer, 0, readback_buffer, 0, size);
    }

    /// Finalizes frame stats after queue submission.
    pub fn finish_frame(
        &mut self,
        device: &wgpu::Device,
        pass_names: &[String],
        draw_calls: u32,
        instance_count: u32,
    ) {
        let frame_time_ms = self.frame_start.elapsed().as_secs_f32() * 1_000.0;
        let mut stats = FrameStats {
            frame_time_ms,
            fps: fps_from_duration(Duration::from_secs_f32(frame_time_ms / 1_000.0)),
            total_gpu_time_ms: 0.0,
            passes: pass_names
                .iter()
                .map(|name| PassStats {
                    name: name.clone(),
                    gpu_time_ms: 0.0,
                })
                .collect(),
            draw_calls,
            instance_count,
        };

        if let Some(timestamps) = self.read_timestamps(device, pass_names.len() as u32) {
            for (idx, pass) in stats.passes.iter_mut().enumerate() {
                let start = timestamps.get(idx * 2).copied().unwrap_or_default();
                let end = timestamps.get(idx * 2 + 1).copied().unwrap_or(start);
                let delta = end.saturating_sub(start) as f32;
                pass.gpu_time_ms = delta * self.timestamp_period_ns / 1_000_000.0;
                stats.total_gpu_time_ms += pass.gpu_time_ms;
            }
        }

        self.stats = stats;
    }

    /// Returns the latest frame stats.
    #[must_use]
    pub fn stats(&self) -> &FrameStats {
        &self.stats
    }

    fn read_timestamps(&self, device: &wgpu::Device, pass_count: u32) -> Option<Vec<u64>> {
        let readback_buffer = self.readback_buffer.as_ref()?;
        let query_count = pass_count.saturating_mul(2).min(self.max_passes * 2);
        if query_count == 0 {
            return Some(Vec::new());
        }
        let size = u64::from(query_count) * std::mem::size_of::<u64>() as u64;
        let slice = readback_buffer.slice(0..size);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        device.poll(wgpu::Maintain::Wait);
        let Ok(Ok(())) = rx.recv() else {
            return None;
        };
        let mapped = slice.get_mapped_range();
        let timestamps = bytemuck::cast_slice(&mapped).to_vec();
        drop(mapped);
        readback_buffer.unmap();
        Some(timestamps)
    }
}

/// Computes FPS from a frame duration.
#[must_use]
pub fn fps_from_duration(duration: Duration) -> f32 {
    let seconds = duration.as_secs_f32();
    if seconds <= f32::EPSILON {
        0.0
    } else {
        1.0 / seconds
    }
}

/// Draws the top-left debug profiler overlay using egui.
pub fn debug_overlay(ctx: &egui::Context, stats: &FrameStats) {
    egui::Area::new(egui::Id::new("hyge-profiler-overlay"))
        .fixed_pos(egui::pos2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_min_size(egui::vec2(200.0, 80.0));
            ui.label(format!("Frame: {:.2} ms", stats.frame_time_ms));
            ui.label(format!("FPS: {:.1}", stats.fps));
            ui.label(format!("GPU: {:.2} ms", stats.total_gpu_time_ms));
            ui.label(format!("Draws: {}", stats.draw_calls));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_from_duration_handles_zero() {
        assert_eq!(fps_from_duration(Duration::ZERO), 0.0);
    }

    #[test]
    fn fps_from_duration_computes_sixty_fps() {
        let fps = fps_from_duration(Duration::from_secs_f32(1.0 / 60.0));
        assert!((fps - 60.0).abs() < 0.01);
    }

    #[test]
    fn default_frame_stats_are_empty() {
        let stats = FrameStats::default();
        assert_eq!(stats.draw_calls, 0);
        assert_eq!(stats.instance_count, 0);
        assert!(stats.passes.is_empty());
    }
}
