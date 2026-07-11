# Goal: R-088 Viewport Frame Transport and Input Bridge

Use Krypton Execution to complete `docs/goals/r-088-viewport-frame-transport/PLAN.md`.

Keep the renderer and ECS engine-owned, send pixels only through the Windows
shared-memory ring, and prove transport/reconnect/shutdown behavior with real
producer-consumer evidence.
