# ADR-0010: kira + Spatial 3D + HRTF (Optional)

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

Audio in a 3D engine must:

- Support a structured mixer (multiple buses with volume control: master / SFX / voice / UI / ambient / music).
- Render spatial 3D audio (positional emitters with attenuation and rolloff).
- Stream long-form audio (music) without loading the entire file into memory.
- Optionally support HRTF for headphone-grade positional audio.

## Decision

Adopt **`kira` 0.9+** as the audio backend, with **`kira-spatial-audio`** for spatial rendering. HRTF is gated behind the **`audio-hrtf` feature flag** and defaults to off (since the dataset licensing must be verified before shipping). When HRTF is enabled, we use a KEMAR-derived dataset (public domain) via `oddio` or a future `kira-hrtf` crate.

## Consequences

### Positive

- **Pure Rust:** no C deps; consistent with the rest of the engine.
- **Modern mixer API:** `kira::track::TrackHandle` per bus, volume changes are sample-accurate.
- **Spatial 3D out of the box** via `kira-spatial-audio`.
- **Streaming** for music via `kira::sound::streaming`; long tracks do not bloat memory.
- **Optional HRTF** lets us ship without a problematic license and add it later without changing the public API.

### Negative

- HRTF is feature-gated and not the default; users wanting binaural audio must opt in and supply (or accept the bundled) dataset.
- `kira`'s feature set is broad; we pin a specific version and use the subset we need.
- Audio tests are mock-only (no real audio device in CI); we test the bus graph and the spatial math, not actual playback.

## Alternatives Considered

### `rodio`

- **Pros:** Simple; widely used in Rust games.
- **Cons:** No spatial 3D; no HRTF; weaker mixer story.
- **Rejected because:** the spatial audio and HRTF stories are part of the v0.1 feature list.

### `oddio`

- **Pros:** Low-level; HRTF-friendly.
- **Cons:** Lower-level than we want; no built-in mixer.
- **Considered as a building block** for HRTF; not the main engine.

### FMOD / Wwise (commercial)

- **Pros:** Industry standard; full feature set.
- **Cons:** Closed source; license fees; C bindings; not pure Rust.
- **Rejected for v0.1** because the licensing and C dependency contradict the engine's "open and pure Rust" stance. Re-evaluate for v0.3+ if a commercial game ships on Hyge.

## References

- `docs/architecture.md` §6.8 (hyge-audio)
- `kira` documentation: <https://docs.rs/kira>
- KEMAR HRTF dataset: public domain (Ohio State University)
