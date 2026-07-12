# Goal: R-101 Scene Hot-Reload and PersistOnReload

Implement the approved R-101 plan in `docs/goals/r-101-scene-hot-reload/PLAN.md`.

Keep Rust as the scene/session truth owner, preserve `SceneNodeId` and
`PersistOnReload` state across reload, expose explicit external-edit conflict
decisions, and prove the behavior with a real watcher, fixture project and TCP
editor session before updating the roadmap.
