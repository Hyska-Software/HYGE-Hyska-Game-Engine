//! Bounded editor data services owned by an editor session.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};

use hyge_asset::{AssetDb, AssetId};
use hyge_render::profiler::FrameStats;
use serde::{Deserialize, Serialize};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// Maximum number of assets exposed in one snapshot.
pub const MAX_ASSET_NODES: usize = 20_000;
/// Maximum number of dependency edges exposed in one snapshot.
pub const MAX_ASSET_EDGES: usize = 40_000;
/// Maximum number of retained console lines.
pub const MAX_CONSOLE_LINES: usize = 1_000;
/// Maximum number of retained profiler samples.
pub const MAX_PROFILER_SAMPLES: usize = 240;
/// Maximum number of pass samples per profiler frame.
pub const MAX_PROFILER_PASSES: usize = 128;
/// Maximum preview jobs in one session.
pub const MAX_PREVIEW_JOBS: usize = 2;

/// A bounded asset tree and dependency graph snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AssetSnapshot {
    /// Monotonic snapshot revision.
    pub snapshot_revision: u64,
    /// Asset tree nodes.
    pub nodes: Vec<AssetNode>,
    /// Directed dependency edges.
    pub edges: Vec<AssetDependencyEdge>,
    /// Non-fatal diagnostics.
    pub diagnostics: Vec<String>,
}

/// One asset tree node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetNode {
    /// Full lowercase BLAKE3 identity.
    pub asset_id: String,
    /// Project-relative cache/source path.
    pub path: String,
}

/// One asset graph edge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetDependencyEdge {
    /// Parent asset identity.
    pub parent: String,
    /// Child asset identity.
    pub child: String,
}

/// A captured tracing line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleLine {
    /// Monotonic line sequence.
    pub sequence: u64,
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Tracing level spelling.
    pub level: String,
    /// Tracing target.
    pub target: String,
    /// Bounded message text.
    pub message: String,
    /// Optional source file.
    pub file: Option<String>,
    /// Optional source line.
    pub line: Option<u32>,
}

/// Console query filter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleFilter {
    /// Minimum level, using tracing order.
    pub min_level: Option<String>,
    /// Optional target prefix.
    pub target_prefix: Option<String>,
}

/// A bounded console snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleSnapshot {
    /// Retained lines after filtering.
    pub lines: Vec<ConsoleLine>,
    /// Applied filter.
    pub filter: ConsoleFilter,
}

/// One render pass timing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfilerPass {
    /// Render graph pass name.
    pub name: String,
    /// GPU duration in milliseconds.
    pub gpu_time_ms: f32,
}

/// One bounded frame telemetry sample.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfilerSample {
    /// Monotonic sample sequence.
    pub frame_id: u64,
    /// Frame duration in milliseconds.
    pub frame_time_ms: f32,
    /// Frames per second.
    pub fps: f32,
    /// Total GPU duration in milliseconds.
    pub gpu_time_ms: f32,
    /// Draw calls.
    pub draw_calls: u32,
    /// Rendered instances.
    pub instance_count: u32,
    /// Resident process memory when available.
    pub process_resident_bytes: Option<u64>,
    /// Bytes occupied by the project asset cache.
    pub asset_cache_bytes: u64,
    /// Per-pass timings.
    pub passes: Vec<ProfilerPass>,
}

/// A bounded profiler history snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfilerSnapshot {
    /// Retained samples, oldest first.
    pub samples: Vec<ProfilerSample>,
}

/// Result of a preview request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewResult {
    /// Client supplied job id.
    pub job_id: String,
    /// Terminal state.
    pub state: String,
    /// Deterministic output path, when completed.
    pub path: Option<String>,
    /// Output dimensions.
    pub width: Option<u32>,
    /// Output dimensions.
    pub height: Option<u32>,
    /// BLAKE3 hash of output bytes.
    pub hash: Option<String>,
}

/// Captures tracing lines in a bounded ring.
#[derive(Clone, Debug)]
pub struct ConsoleBuffer {
    inner: Arc<Mutex<ConsoleState>>,
}

#[derive(Debug, Default)]
struct ConsoleState {
    next_sequence: u64,
    lines: VecDeque<ConsoleLine>,
}

impl ConsoleBuffer {
    /// Creates an empty bounded console.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ConsoleState::default())),
        }
    }

    /// Records one tracing event after applying deterministic bounds.
    pub fn record(
        &self,
        level: &str,
        target: &str,
        message: &str,
        file: Option<&str>,
        line: Option<u32>,
    ) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        state.next_sequence = state.next_sequence.saturating_add(1);
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64);
        let mut text = message.chars().take(16 * 1024).collect::<String>();
        if message.chars().count() > 16 * 1024 {
            text.push('…');
        }
        let mut target = target.chars().take(256).collect::<String>();
        if target.len() > 256 {
            target.push('…');
        }
        let sequence = state.next_sequence;
        state.lines.push_back(ConsoleLine {
            sequence,
            timestamp_ms,
            level: level.to_owned(),
            target,
            message: text,
            file: file.map(str::to_owned),
            line,
        });
        while state.lines.len() > MAX_CONSOLE_LINES {
            state.lines.pop_front();
        }
    }

    /// Returns retained lines matching a filter.
    #[must_use]
    pub fn snapshot(&self, filter: ConsoleFilter) -> ConsoleSnapshot {
        let Ok(state) = self.inner.lock() else {
            return ConsoleSnapshot {
                lines: Vec::new(),
                filter,
            };
        };
        let minimum = filter
            .min_level
            .as_deref()
            .and_then(level_rank)
            .unwrap_or(0);
        let lines = state
            .lines
            .iter()
            .filter(|line| {
                level_rank(&line.level).unwrap_or(0) >= minimum
                    && filter
                        .target_prefix
                        .as_deref()
                        .map_or(true, |prefix| line.target.starts_with(prefix))
            })
            .cloned()
            .collect();
        ConsoleSnapshot { lines, filter }
    }
}

impl Default for ConsoleBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// `tracing-subscriber` layer that forwards events into a [`ConsoleBuffer`].
#[derive(Clone, Debug)]
pub struct ConsoleLayer {
    buffer: ConsoleBuffer,
}

impl ConsoleLayer {
    /// Creates a layer backed by the supplied bounded buffer.
    #[must_use]
    pub fn new(buffer: ConsoleBuffer) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for ConsoleLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);
        let message = visitor.message.unwrap_or_default();
        self.buffer.record(
            metadata.level().as_str(),
            metadata.target(),
            &message,
            metadata.file(),
            metadata.line(),
        );
    }
}

#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
}

impl tracing::field::Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        }
    }
}

/// Captures profiler samples and asset cache size.
#[derive(Clone, Debug, Default)]
pub struct ProfilerStore {
    inner: Arc<Mutex<VecDeque<ProfilerSample>>>,
    next_frame: Arc<AtomicU64>,
    asset_cache_bytes: Arc<AtomicU64>,
}

impl ProfilerStore {
    /// Records a renderer sample.
    pub fn record_frame_stats(&self, stats: &FrameStats, asset_cache_bytes: u64) {
        self.asset_cache_bytes
            .store(asset_cache_bytes, Ordering::Release);
        let frame_id = self
            .next_frame
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        let sample = ProfilerSample {
            frame_id,
            frame_time_ms: stats.frame_time_ms,
            fps: stats.fps,
            gpu_time_ms: stats.total_gpu_time_ms,
            draw_calls: stats.draw_calls,
            instance_count: stats.instance_count,
            process_resident_bytes: None,
            asset_cache_bytes,
            passes: stats
                .passes
                .iter()
                .take(MAX_PROFILER_PASSES)
                .map(|pass| ProfilerPass {
                    name: pass.name.clone(),
                    gpu_time_ms: pass.gpu_time_ms,
                })
                .collect(),
        };
        if let Ok(mut samples) = self.inner.lock() {
            samples.push_back(sample);
            while samples.len() > MAX_PROFILER_SAMPLES {
                samples.pop_front();
            }
        }
    }

    /// Returns the bounded profiler history.
    #[must_use]
    pub fn snapshot(&self) -> ProfilerSnapshot {
        ProfilerSnapshot {
            samples: self
                .inner
                .lock()
                .map_or_else(|_| Vec::new(), |samples| samples.iter().cloned().collect()),
        }
    }
}

/// Deterministic, bounded preview job manager.
#[derive(Clone, Debug, Default)]
pub struct PreviewManager {
    root: Arc<Mutex<Option<PathBuf>>>,
    jobs: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl PreviewManager {
    /// Sets the active project root and cancels old jobs.
    pub fn set_project(&self, root: Option<&Path>) {
        self.cancel_all();
        if let Ok(mut current) = self.root.lock() {
            *current = root.map(Path::to_path_buf);
        }
    }

    /// Cancels one job.
    pub fn cancel(&self, job_id: &str) -> bool {
        self.jobs
            .lock()
            .ok()
            .and_then(|jobs| jobs.get(job_id).cloned())
            .map(|token| {
                token.store(true, Ordering::Release);
                true
            })
            .unwrap_or(false)
    }

    /// Cancels every active job.
    pub fn cancel_all(&self) {
        if let Ok(jobs) = self.jobs.lock() {
            for token in jobs.values() {
                token.store(true, Ordering::Release);
            }
        }
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.clear();
        }
    }

    /// Builds a deterministic 256x256 neutral preview for a registered mesh.
    pub fn request(&self, asset_id: &str, job_id: &str) -> Result<PreviewResult, String> {
        let root = self
            .root
            .lock()
            .map_err(|_| "preview root lock poisoned")?
            .clone()
            .ok_or_else(|| "project is not open".to_owned())?;
        let id = parse_asset_id(asset_id)?;
        let db =
            AssetDb::open(&root.join(".hyge.db")).map_err(|error| format!("asset db: {error}"))?;
        let source = db.lookup(&id).ok_or_else(|| "asset not found".to_owned())?;
        let jobs = self.jobs.lock().map_err(|_| "preview jobs lock poisoned")?;
        if jobs.len() >= MAX_PREVIEW_JOBS {
            return Err("preview job limit reached".to_owned());
        }
        drop(jobs);
        let token = Arc::new(AtomicBool::new(false));
        self.jobs
            .lock()
            .map_err(|_| "preview jobs lock poisoned")?
            .insert(job_id.to_owned(), Arc::clone(&token));
        let result = (|| {
            if token.load(Ordering::Acquire) {
                return Ok(PreviewResult {
                    job_id: job_id.to_owned(),
                    state: "cancelled".to_owned(),
                    path: None,
                    width: None,
                    height: None,
                    hash: None,
                });
            }
            if source.extension().and_then(|extension| extension.to_str()) != Some("hyge-mesh") {
                return Err("asset preview supports .hyge-mesh only".to_owned());
            }
            let bytes = fs::read(&source).map_err(|error| error.to_string())?;
            let mesh = hyge_asset::importer::mesh::from_bytes(&bytes)
                .map_err(|error| format!("mesh preview: {error}"))?;
            let image = render_mesh_preview(&mesh);
            let mut encoded = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut encoded, image::ImageFormat::Png)
                .map_err(|error| error.to_string())?;
            if token.load(Ordering::Acquire) {
                return Ok(PreviewResult {
                    job_id: job_id.to_owned(),
                    state: "cancelled".to_owned(),
                    path: None,
                    width: None,
                    height: None,
                    hash: None,
                });
            }
            let output_dir = root.join(".hyge").join("previews");
            fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
            let output = output_dir.join(format!("{asset_id}.png"));
            let temp = output_dir.join(format!(".{asset_id}.{job_id}.tmp"));
            fs::write(&temp, encoded.get_ref()).map_err(|error| error.to_string())?;
            if token.load(Ordering::Acquire) {
                let _ = fs::remove_file(&temp);
                return Ok(PreviewResult {
                    job_id: job_id.to_owned(),
                    state: "cancelled".to_owned(),
                    path: None,
                    width: None,
                    height: None,
                    hash: None,
                });
            }
            fs::rename(&temp, &output).map_err(|error| error.to_string())?;
            Ok(PreviewResult {
                job_id: job_id.to_owned(),
                state: "completed".to_owned(),
                path: Some(
                    output
                        .strip_prefix(&root)
                        .unwrap_or(&output)
                        .display()
                        .to_string(),
                ),
                width: Some(256),
                height: Some(256),
                hash: Some(blake3::hash(encoded.get_ref()).to_hex().to_string()),
            })
        })();
        self.jobs.lock().ok().map(|mut jobs| jobs.remove(job_id));
        result
    }
}

fn render_mesh_preview(mesh: &hyge_asset::importer::mesh::MeshData) -> image::RgbaImage {
    const SIZE: u32 = 256;
    let mut image = image::RgbaImage::from_pixel(SIZE, SIZE, image::Rgba([20, 24, 30, 255]));
    if mesh.vertices.is_empty() {
        return image;
    }
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
    );
    for vertex in &mesh.vertices {
        let [x, y, z] = vertex.position;
        let projected_x = x - z * 0.35;
        let projected_y = y + z * 0.2;
        min_x = min_x.min(projected_x);
        max_x = max_x.max(projected_x);
        min_y = min_y.min(projected_y);
        max_y = max_y.max(projected_y);
    }
    let span = (max_x - min_x).max(max_y - min_y).max(0.001);
    let point = |vertex: &hyge_asset::importer::mesh::Vertex| {
        let [x, y, z] = vertex.position;
        let x = (x - z * 0.35 - min_x) / span * 216.0 + 20.0;
        let y = (y + z * 0.2 - min_y) / span * 216.0 + 20.0;
        (x as i32, (SIZE as f32 - 1.0 - y) as i32)
    };
    for triangle in mesh.indices.chunks_exact(3) {
        let Some(a) = mesh.vertices.get(triangle[0] as usize) else {
            continue;
        };
        let Some(b) = mesh.vertices.get(triangle[1] as usize) else {
            continue;
        };
        let Some(c) = mesh.vertices.get(triangle[2] as usize) else {
            continue;
        };
        let normal = [
            (a.normal[0] + b.normal[0] + c.normal[0]) / 3.0,
            (a.normal[1] + b.normal[1] + c.normal[1]) / 3.0,
            (a.normal[2] + b.normal[2] + c.normal[2]) / 3.0,
        ];
        let light = (normal[0] * 0.4 + normal[1] * 0.7 + normal[2] * 0.5).max(0.0);
        let color = image::Rgba([
            (70.0 + 130.0 * light) as u8,
            (100.0 + 110.0 * light) as u8,
            (135.0 + 95.0 * light) as u8,
            255,
        ]);
        let (ax, ay) = point(a);
        let (bx, by) = point(b);
        let (cx, cy) = point(c);
        draw_line(&mut image, ax, ay, bx, by, color);
        draw_line(&mut image, bx, by, cx, cy, color);
        draw_line(&mut image, cx, cy, ax, ay, color);
    }
    image
}

fn draw_line(
    image: &mut image::RgbaImage,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: image::Rgba<u8>,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        if x0 >= 0 && y0 >= 0 && x0 < image.width() as i32 && y0 < image.height() as i32 {
            image.put_pixel(x0 as u32, y0 as u32, color);
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice = error * 2;
        if twice >= dy {
            error += dy;
            x0 += sx;
        }
        if twice <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

/// Session-owned collection of bounded data services.
#[derive(Clone, Debug, Default)]
pub struct EditorDataServices {
    /// Session console buffer.
    pub console: ConsoleBuffer,
    /// Session profiler history.
    pub profiler: ProfilerStore,
    /// Session preview manager.
    pub previews: PreviewManager,
    asset_revision: Arc<AtomicU64>,
}

impl EditorDataServices {
    /// Returns a tracing layer connected to this session's console buffer.
    #[must_use]
    pub fn console_layer(&self) -> ConsoleLayer {
        ConsoleLayer::new(self.console.clone())
    }

    /// Creates a project asset snapshot from the authoritative AssetDb.
    pub fn asset_snapshot(&self, root: &Path) -> Result<AssetSnapshot, String> {
        let db = AssetDb::open(&root.join(".hyge.db"))
            .map_err(|error| format!("asset db unavailable: {error}"))?;
        let assets = db.list_assets();
        let edges = db.dependency_edges();
        if assets.len() > MAX_ASSET_NODES || edges.len() > MAX_ASSET_EDGES {
            return Err("asset snapshot limit exceeded".to_owned());
        }
        let nodes = assets
            .into_iter()
            .filter_map(|asset| relative_node(root, asset.asset_id, &asset.path))
            .collect::<Vec<_>>();
        let edges = edges
            .into_iter()
            .map(|edge| AssetDependencyEdge {
                parent: hex_id(edge.parent),
                child: hex_id(edge.child),
            })
            .collect();
        Ok(AssetSnapshot {
            snapshot_revision: self
                .asset_revision
                .fetch_add(1, Ordering::AcqRel)
                .saturating_add(1),
            nodes,
            edges,
            diagnostics: Vec::new(),
        })
    }
}

fn relative_node(root: &Path, id: AssetId, path: &Path) -> Option<AssetNode> {
    let relative = if path.is_absolute() {
        let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let canonical_path = fs::canonicalize(path).ok()?;
        canonical_path
            .strip_prefix(&canonical_root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/")
    } else {
        path.to_string_lossy().replace('\\', "/")
    };
    Some(AssetNode {
        asset_id: hex_id(id),
        path: relative,
    })
}
fn hex_id(id: AssetId) -> String {
    id.as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn parse_asset_id(value: &str) -> Result<AssetId, String> {
    if value.len() != 64 {
        return Err("asset id must be 64 hex characters".to_owned());
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex(pair[0])? << 4) | hex(pair[1])?;
    }
    Ok(AssetId::from(bytes))
}
fn hex(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("asset id is not hexadecimal".to_owned()),
    }
}
fn level_rank(value: &str) -> Option<u8> {
    match value.to_ascii_lowercase().as_str() {
        "trace" => Some(0),
        "debug" => Some(1),
        "info" => Some(2),
        "warn" => Some(3),
        "error" => Some(4),
        _ => None,
    }
}
