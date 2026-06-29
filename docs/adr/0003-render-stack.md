# ADR-0003: Bindless + Render Graph + Clustered Forward Renderer

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

Hyge targets AAA-grade rendering on PC. The renderer must:

- Scale to 1000+ materials, 10k+ instances, 64+ dynamic lights without per-draw descriptor updates.
- Allow feature authors to add passes (shadows, post-process, etc.) without hand-managing resource transitions.
- Provide high-quality PBR lighting with both directional and dynamic point/spot lights.
- Support modern features (cascaded shadows, IBL, post-process chain, meshlet culling).

## Decision

Adopt a **bindless** descriptor-heap pattern, driven by a **render graph** that infers barriers automatically, with a **clustered forward** shading pipeline:

- One global bindless heap, indexed by `uint` IDs (`mesh_id`, `material_id`, `texture_id`, `light_id`).
- Render graph as a DAG of `Pass<TIn, TOut>` declarations; the compiler infers `wgpu` barriers and transient lifetimes.
- Z-cluster × XY-tile light culling (16 Z × 16×16 XY by default), `LightGrid` SSBO consumed in the PBR fragment.

## Consequences

### Positive

- **No descriptor thrash** at draw time: one bindless set is bound once per pass; the shader reads `mesh_id` / `material_id` as `u32` indices.
- **Render graph** centralizes resource lifetime: transient textures are recycled within a frame, persistent ones (shadow atlas, bindless heap) span frames.
- **Clustered forward** gives forward-shading material variety (transparency, custom shading) with deferred-shading light count.
- PBR + IBL + cascaded shadows are all first-class in the same pipeline.
- GPU compute culling (meshlet) coexists with CPU frustum culling (static instances).

### Negative

- Bindless requires careful slot-layout planning and a long-lived `BindlessTable` resource (see `docs/architecture.md` §8.1).
- Clustered forward is more expensive per light than forward+ without clusters; the win is light count, not single-light cost.
- Render graph compile cost is non-trivial; we mitigate by caching the compiled graph and re-compiling only on pass-graph changes.
- Meshlet baking is an import-time cost (paid by `hyge-tools import`, not by the runtime).

## Alternatives Considered

### Traditional per-draw descriptor sets

- **Pros:** Simple mental model; no bindless bookkeeping.
- **Cons:** Per-draw bind updates dominate the frame at scale; doesn't reach 1000+ instances.
- **Rejected because:** does not meet the scalability target.

### Deferred shading

- **Pros:** Trivial per-light cost; very high light counts; well-known pattern.
- **Cons:** Costs material variety (transparency, complex BSDFs), high bandwidth (g-buffer), more complex MSAA.
- **Rejected because:** the loss of forward material flexibility outweighs the light-count benefit at our target scale (64 dynamic lights).

### Mesh-shader-driven GPU culling (Nanite-style)

- **Pros:** State of the art; minimal CPU involvement; very high geometric density.
- **Cons:** `wgpu` does not yet expose mesh shaders stably on all backends; raises the implementation cost dramatically.
- **Rejected for v0.1:** revisit after `wgpu` lands stable mesh-shader support (see `docs/architecture.md` §18 Open Questions #9).

### Software-rasterized forward (MVP)

- **Pros:** Minimum code; fast to prototype.
- **Cons:** Does not scale; locks the design away from production features before v1.
- **Rejected because:** fails the scalability target.

## References

- `docs/architecture.md` §6.4 (hyge-render), §8 (renderer architecture)
- ADR-0001 (graphics API) — `wgpu` exposes the descriptor-heap primitives we rely on
- ADR-0007 (scene/prefab) — instancing extracts `DrawCommand` lists consumed by the bindless path
