//! Versioned viewport-frame ring ABI and revisioned input validation.

#![allow(dead_code, missing_docs)]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use hyge_editor_shm::SharedMapping;

/// Fixed ring magic encoded in the shared-memory header.
pub const RING_MAGIC: [u8; 8] = *b"HYGEVPR1";
/// Shared-memory ABI version.
pub const RING_ABI_VERSION: u32 = 1;
/// Frame slots retained by the producer.
pub const RING_SLOT_COUNT: usize = 3;
/// Frame bytes use RGBA8 sRGB.
pub const PIXEL_FORMAT_RGBA8_SRGB: u32 = 1;
/// Largest supported transport dimension.
pub const MAX_VIEWPORT_DIMENSION: u32 = 4096;
const GLOBAL_HEADER_BYTES: usize = 64;
const SLOT_HEADER_BYTES: usize = 64;
const PRODUCER_PID_OFFSET: usize = 24;
const PRODUCER_HEARTBEAT_OFFSET: usize = 32;
const CONSUMER_PID_OFFSET: usize = 40;
const CONSUMER_HEARTBEAT_OFFSET: usize = 48;
const TRANSPORT_STATE_OFFSET: usize = 56;

/// State published in the shared-memory header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportState {
    ProducerReady,
    Closing,
    Closed,
}

/// Metadata for one committed frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    /// Monotonic producer frame id.
    pub frame_id: u64,
    /// Mapping generation.
    pub generation: u64,
    /// Slot index in the ring.
    pub slot: u32,
    /// Frame dimensions.
    pub width: u32,
    pub height: u32,
    /// Pixel format discriminator.
    pub pixel_format: u32,
    /// ECS snapshot revision.
    pub scene_revision: u64,
    /// Editor camera revision.
    pub camera_revision: u64,
    /// Number of committed bytes.
    pub byte_len: u32,
    /// Even commit sequence for stable reads.
    pub sequence: u64,
}

#[derive(Clone)]
struct Slot {
    header: Option<FrameHeader>,
    pixels: Vec<u8>,
}

/// In-process representation of the byte-stable ring contract.
pub struct ViewportRing {
    generation: u64,
    state: TransportState,
    next_frame: u64,
    slots: [Slot; RING_SLOT_COUNT],
}

impl ViewportRing {
    /// Creates an empty ring for a negotiated mapping generation.
    #[must_use]
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            state: TransportState::ProducerReady,
            next_frame: 1,
            slots: std::array::from_fn(|_| Slot {
                header: None,
                pixels: Vec::new(),
            }),
        }
    }

    /// Publishes one complete RGBA8 frame and returns its stable header.
    pub fn publish(
        &mut self,
        width: u32,
        height: u32,
        scene_revision: u64,
        camera_revision: u64,
        pixels: &[u8],
    ) -> Result<FrameHeader, String> {
        if self.state != TransportState::ProducerReady {
            return Err("viewport transport is closing".into());
        }
        if width == 0
            || height == 0
            || width > MAX_VIEWPORT_DIMENSION
            || height > MAX_VIEWPORT_DIMENSION
        {
            return Err("viewport transport dimensions exceed 4096x4096".into());
        }
        let expected = width as usize * height as usize * 4;
        if pixels.len() != expected {
            return Err("viewport frame is not RGBA8-sized".into());
        }
        let frame_id = self.next_frame;
        self.next_frame = self.next_frame.saturating_add(1);
        let slot = (frame_id as usize % RING_SLOT_COUNT) as u32;
        let sequence = frame_id.saturating_mul(2);
        let header = FrameHeader {
            frame_id,
            generation: self.generation,
            slot,
            width,
            height,
            pixel_format: PIXEL_FORMAT_RGBA8_SRGB,
            scene_revision,
            camera_revision,
            byte_len: pixels.len() as u32,
            sequence,
        };
        let destination = &mut self.slots[slot as usize];
        destination.header = None;
        destination.pixels.clear();
        destination.pixels.extend_from_slice(pixels);
        destination.header = Some(header.clone());
        Ok(header)
    }

    /// Copies a frame only if its current slot still has the requested commit.
    #[must_use]
    pub fn consume(&self, expected: &FrameHeader) -> Option<Vec<u8>> {
        if expected.generation != self.generation || self.state == TransportState::Closed {
            return None;
        }
        let slot = self.slots.get(expected.slot as usize)?;
        (slot.header.as_ref()? == expected).then(|| slot.pixels.clone())
    }

    /// Marks the ring as closing and prevents further writes.
    pub fn close(&mut self) {
        self.state = TransportState::Closed;
    }
}

/// A Windows named mapping paired with the portable ring verifier.
pub struct SharedViewportTransport {
    pub name: String,
    pub ring: ViewportRing,
    mapping: Option<SharedMapping>,
}

impl SharedViewportTransport {
    /// Creates a transport. Non-Windows platforms retain the verifier only.
    pub fn create(name: String, generation: u64, bytes: usize) -> Result<Self, String> {
        let mapping = match SharedMapping::create(&name, bytes) {
            Ok(mapping) => Some(mapping),
            Err(hyge_editor_shm::SharedMemoryError::UnsupportedPlatform) => None,
            Err(error) => return Err(error.to_string()),
        };
        let mut transport = Self {
            name,
            ring: ViewportRing::new(generation),
            mapping,
        };
        if let Some(mapping) = &mut transport.mapping {
            let pid = std::process::id() as u64;
            let heartbeat = unix_millis();
            mapping
                .with_bytes_mut(|bytes| {
                    bytes[PRODUCER_PID_OFFSET..PRODUCER_PID_OFFSET + 8]
                        .copy_from_slice(&pid.to_le_bytes());
                    bytes[PRODUCER_HEARTBEAT_OFFSET..PRODUCER_HEARTBEAT_OFFSET + 8]
                        .copy_from_slice(&heartbeat.to_le_bytes());
                    bytes[CONSUMER_PID_OFFSET..CONSUMER_PID_OFFSET + 8].fill(0);
                    bytes[CONSUMER_HEARTBEAT_OFFSET..CONSUMER_HEARTBEAT_OFFSET + 8].fill(0);
                    bytes[TRANSPORT_STATE_OFFSET..TRANSPORT_STATE_OFFSET + 8]
                        .copy_from_slice(&1_u64.to_le_bytes());
                })
                .map_err(|error| error.to_string())?;
        }
        Ok(transport)
    }
    /// Indicates whether an OS-backed mapping is active.
    #[must_use]
    pub const fn is_mapped(&self) -> bool {
        self.mapping.is_some()
    }

    /// Closes the producer side of the transport exactly once.
    pub fn close(&mut self) {
        if let Some(mapping) = &mut self.mapping {
            mapping
                .with_bytes_mut(|bytes| {
                    bytes[TRANSPORT_STATE_OFFSET..TRANSPORT_STATE_OFFSET + 8]
                        .copy_from_slice(&2_u64.to_le_bytes());
                })
                .ok();
        }
        self.ring.close();
        self.mapping.take();
    }

    /// Returns whether the consumer heartbeat is older than `timeout`.
    #[must_use]
    pub fn consumer_is_stale(&self, timeout: std::time::Duration) -> bool {
        let Some(mapping) = &self.mapping else {
            return false;
        };
        let now = unix_millis();
        mapping
            .with_bytes(|bytes| {
                let heartbeat = u64::from_le_bytes(
                    bytes[CONSUMER_HEARTBEAT_OFFSET..CONSUMER_HEARTBEAT_OFFSET + 8]
                        .try_into()
                        .unwrap_or([0; 8]),
                );
                heartbeat != 0 && now.saturating_sub(heartbeat) > timeout.as_millis() as u64
            })
            .unwrap_or(false)
    }

    /// Publishes a frame into both the verifier and the named mapping.
    pub fn publish(
        &mut self,
        width: u32,
        height: u32,
        scene_revision: u64,
        camera_revision: u64,
        pixels: &[u8],
    ) -> Result<FrameHeader, String> {
        let header = self
            .ring
            .publish(width, height, scene_revision, camera_revision, pixels)?;
        if let Some(mapping) = &mut self.mapping {
            let heartbeat = unix_millis();
            let stride = SLOT_HEADER_BYTES + pixels.len();
            let offset = GLOBAL_HEADER_BYTES + header.slot as usize * stride;
            if offset + stride > mapping.len() {
                return Err("shared-memory mapping is too small for viewport frame".into());
            }
            mapping
                .with_bytes_mut(|bytes| {
                    bytes[..8].copy_from_slice(&RING_MAGIC);
                    bytes[8..12].copy_from_slice(&RING_ABI_VERSION.to_le_bytes());
                    bytes[12..16].copy_from_slice(&(RING_SLOT_COUNT as u32).to_le_bytes());
                    bytes[16..24].copy_from_slice(&header.generation.to_le_bytes());
                    bytes[PRODUCER_HEARTBEAT_OFFSET..PRODUCER_HEARTBEAT_OFFSET + 8]
                        .copy_from_slice(&heartbeat.to_le_bytes());
                    bytes[offset..offset + SLOT_HEADER_BYTES].fill(0);
                    bytes[offset..offset + 8].copy_from_slice(&header.frame_id.to_le_bytes());
                    bytes[offset + 8..offset + 12].copy_from_slice(&header.width.to_le_bytes());
                    bytes[offset + 12..offset + 16].copy_from_slice(&header.height.to_le_bytes());
                    bytes[offset + 16..offset + 20]
                        .copy_from_slice(&header.pixel_format.to_le_bytes());
                    bytes[offset + 20..offset + 24].copy_from_slice(&header.byte_len.to_le_bytes());
                    bytes[offset + 24..offset + 32]
                        .copy_from_slice(&header.scene_revision.to_le_bytes());
                    bytes[offset + 32..offset + 40]
                        .copy_from_slice(&header.camera_revision.to_le_bytes());
                    bytes[offset + 40..offset + 48].copy_from_slice(&header.sequence.to_le_bytes());
                    bytes[offset + SLOT_HEADER_BYTES..offset + stride].copy_from_slice(pixels);
                    bytes[offset + 48..offset + 56].copy_from_slice(&header.sequence.to_le_bytes());
                })
                .map_err(|error| error.to_string())?;
        }
        Ok(header)
    }
}

impl Drop for SharedViewportTransport {
    fn drop(&mut self) {
        self.close();
    }
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

/// Normalized editor input accepted by the viewport.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewportInput {
    Mouse {
        dx: f32,
        dy: f32,
    },
    Wheel {
        dx: f32,
        dy: f32,
    },
    Key {
        code: String,
        pressed: bool,
    },
    Camera {
        command: CameraCommand,
        x: f32,
        y: f32,
    },
}
/// Camera operation requested by the frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraCommand {
    Orbit,
    Pan,
    Dolly,
    Focus,
}
/// Revisioned input batch.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ViewportInputBatch {
    pub generation: u64,
    pub expected_input_revision: u64,
    pub input_revision: u64,
    pub events: Vec<ViewportInput>,
}

/// Applies ordered input batches and rate-limits the control path.
#[derive(Default)]
pub struct InputBridge {
    revision: u64,
    recent: VecDeque<Instant>,
}
impl InputBridge {
    /// Returns the latest accepted input revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Validates ordering and a 120 batches-per-second limit.
    pub fn accept(
        &mut self,
        batch: &ViewportInputBatch,
        generation: u64,
    ) -> Result<(), &'static str> {
        if batch.generation != generation
            || batch.expected_input_revision != self.revision
            || batch.input_revision != self.revision.saturating_add(1)
        {
            return Err("stale_input_revision");
        }
        let now = Instant::now();
        while self
            .recent
            .front()
            .is_some_and(|time| now.duration_since(*time) > Duration::from_secs(1))
        {
            self.recent.pop_front();
        }
        if self.recent.len() >= 120 {
            return Err("input_rate_limited");
        }
        if batch.events.len() > 64 {
            return Err("invalid_viewport_input");
        }
        self.recent.push_back(now);
        self.revision = batch.input_revision;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stress_preserves_committed_headers_and_detects_overwrite() {
        let mut ring = ViewportRing::new(7);
        let mut stale = None;
        for frame in 0..1_000_u64 {
            let pixels = vec![frame as u8; 4 * 4 * 4];
            let header = ring
                .publish(4, 4, frame, frame + 1, &pixels)
                .expect("publish");
            assert_eq!(ring.consume(&header), Some(pixels));
            stale.get_or_insert(header);
        }
        assert!(ring.consume(&stale.expect("first header")).is_none());
        ring.close();
        assert!(ring.publish(1, 1, 1, 1, &[0; 4]).is_err());
    }

    #[test]
    fn input_requires_consecutive_revision_and_rate_limits() {
        let mut bridge = InputBridge::default();
        let first = ViewportInputBatch {
            generation: 2,
            expected_input_revision: 0,
            input_revision: 1,
            events: vec![],
        };
        assert!(bridge.accept(&first, 2).is_ok());
        assert_eq!(bridge.accept(&first, 2), Err("stale_input_revision"));
        for revision in 2..=120 {
            let batch = ViewportInputBatch {
                generation: 2,
                expected_input_revision: revision - 1,
                input_revision: revision,
                events: vec![],
            };
            assert!(bridge.accept(&batch, 2).is_ok());
        }
        let limited = ViewportInputBatch {
            generation: 2,
            expected_input_revision: 120,
            input_revision: 121,
            events: vec![],
        };
        assert_eq!(bridge.accept(&limited, 2), Err("input_rate_limited"));
    }
}
