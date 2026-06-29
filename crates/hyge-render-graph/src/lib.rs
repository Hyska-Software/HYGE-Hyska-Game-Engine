//! Hyge render graph: a DAG of typed `Pass` declarations with automatic
//! `wgpu` barrier inference and an arena allocator for transient (frame-scoped)
//! resources.
//!
//! See `docs/architecture.md` §6.3 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-020..R-022.
