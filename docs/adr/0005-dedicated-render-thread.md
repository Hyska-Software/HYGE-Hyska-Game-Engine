# ADR-0005: Dedicated Render Thread + FrameSnapshot

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

A 3D engine runs into a critical problem when the render pipeline scales: the render thread and the main/game thread compete for the same data (transforms, lights, asset handles) and the same locks. This causes:

- Frame-time spikes when game-side systems block the render thread.
- Hard-to-reason-about bugs (race conditions, data races) at the ECS/Render boundary.
- Limited parallel scaling: per-frame data must be defensively copied anyway.

## Decision

Adopt a **dedicated render thread** that owns the `wgpu::Device` and `wgpu::Queue` long-lived. The render thread:

- Consumes an immutable `Arc<FrameSnapshot>` produced in the `RenderExtract` schedule on the main thread.
- Never holds a `&mut World`; the render side is read-only on ECS state.
- The main thread schedules: `First` → `PreUpdate` → `FixedUpdate` (N×) → `Update` → `RenderExtract` → `Last`. The render thread runs the `Render` schedule, present, then waits for the next snapshot.

## Consequences

### Positive

- **No lock contention** between render and game threads; the snapshot is owned by the render thread for the entire frame.
- **Clean mental model:** main thread mutates the World; render thread observes.
- **Bounded frame time:** the render thread can present at its own pace; if it falls behind, the main thread blocks at `RenderExtract` (back-pressure).
- **Asset upload parallelism:** `AsyncComputeTaskPool` can do GPU uploads while the main thread runs the next frame's logic.

### Negative

- One frame of latency between World mutation and render (acceptable; matches industry practice).
- The `FrameSnapshot` must be designed to be cheap to extract. We mitigate by reading only dirty archetypes and using `Arc` for shared sub-allocations.
- Two threads to debug instead of one. Mitigated by the snapshot boundary being a hard contract: any access to World from the render thread is a bug.

## Alternatives Considered

### Single-threaded

- **Pros:** No concurrency at all; trivial to reason about.
- **Cons:** Does not meet the v0.1 perf target (144 FPS at 1080p on RTX 3060) at the scene scale we want (10k instances, 64 lights, full PBR + post).
- **Rejected because:** fails the performance target.

### Render thread with shared `&World` (read-only borrows)

- **Pros:** No snapshot copy; latest data always available.
- **Cons:** Requires `unsafe` lifetimes or `Arc<RwLock<...>>` on the World; the borrow checker fights us; subtle races on archetypes.
- **Rejected because:** snapshot is cheaper and safer; the latency cost is invisible at 60+ FPS.

### Fully parallel: render threads per-pass with cross-pass sync

- **Pros:** Maximum GPU pipeline overlap.
- **Cons:** Massive complexity; `wgpu` does not natively support cross-queue synchronization on all backends; correctness is hard to verify.
- **Rejected for v0.1:** revisit in v0.3+ if profiling shows we are CPU-bound on render command recording.

## References

- `docs/architecture.md` §7 (runtime: schedules, threading, FrameSnapshot), §8.5 (asset→GPU flow)
- `bevy` render architecture (precedent) — note: `bevy` uses a similar but not identical snapshot pattern
- ADR-0001 (graphics API) — `wgpu` Device/Queue ownership is a thread-affinity constraint
