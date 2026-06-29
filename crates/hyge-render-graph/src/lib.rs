//! Hyge render graph: a DAG of typed `Pass` declarations with automatic
//! `wgpu` barrier inference and an arena allocator for transient
//! (frame-scoped) resources.
//!
//! # Overview
//!
//! - [`RenderGraph`](graph::RenderGraph) is the user-facing builder.
//!   You declare resources with
//!   [`RenderGraph::add_resource`](graph::RenderGraph::add_resource),
//!   then add passes with
//!   [`RenderGraph::add_pass`](graph::RenderGraph::add_pass). Each
//!   pass implements the [`Pass`](pass::Pass) trait, which says
//!   which resources it reads and writes.
//! - [`RenderGraph::compile`](graph::RenderGraph::compile) topsorts
//!   the inferred DAG, infers [`Barrier`](barrier::Barrier)s
//!   between passes, and returns a
//!   [`CompiledGraph`](compile::CompiledGraph).
//! - [`CompiledGraph::execute`](compile::CompiledGraph::execute)
//!   records each pass into a `wgpu::CommandEncoder`, with the
//!   transient allocator in the
//!   [`TransientAllocator`](allocator::TransientAllocator) handling
//!   first-touch allocation for `ResourceLifetime::Transient`
//!   resources.
//!
//! # Module map
//!
//! - [`graph`] — the [`RenderGraph`](graph::RenderGraph) builder.
//! - [`pass`] — the [`Pass`](pass::Pass) trait and
//!   [`PassContext`](pass::PassContext).
//! - [`resource`] — [`ResourceHandle`](resource::ResourceHandle),
//!   [`ResourceKind`](resource::ResourceKind), and
//!   [`ResourceLifetime`](resource::ResourceLifetime).
//! - [`barrier`] — [`Barrier`](barrier::Barrier) types inferred
//!   from pass read/write declarations.
//! - [`allocator`] — [`TransientAllocator`](allocator::TransientAllocator).
//! - [`compile`] — [`CompiledGraph`](compile::CompiledGraph) and
//!   [`CompiledPass`](compile::CompiledPass).
//!
//! See `docs/architecture.md` §6.3 for the full design and
//! `docs/roadmap.toml` (R-020, R-021, R-022) for the implementation
//! backlog. This module covers R-020 (skeleton); R-021 fleshes out
//! barrier inference; R-022 fleshes out the transient allocator.

#![warn(missing_docs)]

pub mod allocator;
pub mod barrier;
pub mod compile;
pub mod frame;
pub mod graph;
pub mod pass;
pub mod prelude;
pub mod resource;
