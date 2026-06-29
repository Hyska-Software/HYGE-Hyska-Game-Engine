//! Hyge render: the runtime renderer.
//!
//! Owns the `wgpu::Device` / `wgpu::Queue` (typically on a dedicated render
//! thread), manages the `BindlessTable`, runs the clustered forward pipeline
//! (PBR + IBL + cascaded shadows + post-process), and produces the final
//! surface present per frame.
//!
//! See `docs/architecture.md` §6.4 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-023..R-025
//! and R-040..R-058.
