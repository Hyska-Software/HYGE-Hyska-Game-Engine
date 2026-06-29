//! Hyge physics: a `rapier3d` wrapper behind the `physics-rapier` feature
//! flag (default on).
//!
//! Exposes `RigidBody`, `Collider`, `CharacterController`, `Joint` components;
//! `CollisionEvent` / `ContactForceEvent` events; and a `SpatialQuery` trait
//! for raycasts and shape casts.
//!
//! See `docs/architecture.md` §6.7 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-070..R-071.
