# App Module Boundaries

`src/app.rs` now keeps state ownership and core non-UI helpers. Submodules should stay narrow.

## Module Ownership

- `src/app.rs`
  - Owns `FileSyncApp`, shared runtime state, and small pure helpers.
  - Owns tests that need access to private app state.
- `src/app/flow.rs`
  - Owns sync completion, failure handling, run history writes, and schedule outcome updates.
- `src/app/schedule.rs`
  - Owns due-job collection, queue ordering, queue de-duplication, and scheduled start decisions.
- `src/app/dialogs.rs`
  - Owns modal dialog rendering and keyboard shortcut handling.
- `src/app/shell.rs`
  - Owns `eframe::App` integration and top-level layout/render sequencing.
- `src/app/support.rs`
  - Owns config import/export, preview scan support, notifications, and font setup.
- `src/app/strings.rs`
  - Owns user-facing strings used by the split modules.

## Boundary Rules

- Prefer explicit imports over `super::*`.
- Prefer module functions for cross-cutting behavior instead of growing `FileSyncApp` with UI-only methods.
- Keep queue ordering rules inside `schedule.rs`.
- Keep dialog text and formatting in `strings.rs` when it is user-facing and reused.
- Keep tests near the state owner when they need private-field access.

## Follow-up Guardrails

- New dialog copy should go through `strings.rs`.
- New scheduled-run state transitions should be tested in `src/app.rs` regression tests.
- If a helper is only used inside one submodule, keep it private to that submodule.
