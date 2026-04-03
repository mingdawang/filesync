# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
cargo build              # debug build
cargo build --release    # optimized release (~5.8 MB, stripped)
cargo run                # debug run
cargo check              # fast syntax/type check (no codegen)
cargo clippy             # lint
cargo fmt                # format
```

No test suite exists yet. There are no pre-build steps beyond resource embedding handled automatically by `build.rs` (embeds `res/app.rc` for Windows icon/manifest).

## Architecture Overview

Single-binary Windows desktop app: **egui UI → app.rs orchestrator → tokio engine → Win32 FS layer**.

### Layer responsibilities

| Layer | Key files | What it does |
|---|---|---|
| **UI** | `ui/job_list.rs`, `ui/job_editor.rs`, `ui/progress.rs`, `ui/preview.rs` | Render egui widgets; read/mutate `FileSyncApp` state |
| **App** | `app.rs` | Central state machine; owns config, session, channels; bridges UI ↔ engine |
| **Engine** | `engine/executor.rs` + siblings | Async orchestration; scanning, diffing, hashing, copying |
| **FS** | `fs/volume.rs`, `fs/usn_journal.rs`, `fs/long_path.rs` | Win32 API wrappers |
| **Model** | `model/` | Data structs; `AppConfig`/`SyncJob` serialize to JSON; `SyncSession` is runtime-only |
| **Config** | `config/storage.rs` | Atomic JSON persistence to `%APPDATA%\FileSync\config.json` |

### Data flow for a sync

1. **UI thread**: user triggers `app.start_sync()` → clones `SyncJob` from config, spawns OS thread with tokio runtime
2. **Engine thread**: `executor::run_sync()` runs all phases, sends `SyncEvent` via flume channel
3. **UI thread**: each egui frame calls `app.drain_events()` → `handle_event()` updates `SyncSession`, triggers repaint

```
SyncJob (config) ──clone──► executor::run_sync()
                                 │
                    ┌────────────┼──────────────────────┐
                    ▼            ▼                       ▼
                 scanner      diff.rs              copier / delta
                 (walkdir)    (metadata)           (CopyFileEx / rsync)
                    │                                    │
                    └────── SyncEvent via flume ─────────┘
                                 │
                           app.handle_event()
                           (updates SyncSession)
```

### Concurrency model

- egui runs on the main thread
- Each sync job spawns **one OS thread** with its own `tokio::runtime::Runtime`
- Within that runtime, file copies run as concurrent tokio tasks gated by a `Semaphore(concurrency)`
- All engine→UI communication goes through a `flume::bounded(4096)` channel; UI drains it non-blocking every frame

### Engine phases (`executor::run_sync`)

1. **Pre-scan**: query USN Journal (`fs/usn_journal.rs`) to get changed FRNs since last sync
2. **Scan**: `scanner::scan_directory()` on both source and destination (WalkDir + globset)
3. **Diff**: `diff::compute_diff()` → `DiffEntry { action: Create|Update|Skip|Orphan }`
4. **Hash refinement** (if `CompareMethod::Hash`): BLAKE3 hash `Update` entries to downgrade to `Skip`; USN optimization skips hash for files where both src and dst FRNs are absent from the changed set
5. **Copy**: parallel tasks using `copier::copy_file_with_caps()` or `delta::delta_sync()`
6. **Mirror cleanup**: delete orphan files if `SyncMode::Mirror`
7. **Complete**: send `SyncEvent::Completed { stats, usn_checkpoints }`

### USN Journal optimization (important design constraint)

Checkpoints are **in-memory only** (`#[serde(skip)]` on `SyncJob::last_sync_checkpoints`). They are never written to disk. On app restart the field starts empty → full scan always happens on first sync after startup. This handles the case where destination files are modified while the app is closed.

Within a session: after a clean (zero-error) sync, `app.handle_event` populates `self.config.jobs[idx].last_sync_checkpoints`. The next sync clones that job and the checkpoints flow into `executor::run_sync` automatically.

### Copy strategy selection (`copier.rs`)

```
file size ≥ delta_threshold_mb AND dst exists → delta::delta_sync() (rsync algorithm)
  └─ on delta failure → copier::copy_file_with_caps()
       ├─ unbuffered I/O  (size ≥ unbuffered_threshold_mb, local drive)
       └─ CopyFileEx      (Windows native, default)
            └─ buffered fallback (256 KB chunks, 3 retries)
```

Post-copy BLAKE3 verification is opt-in (`engine_options.verify_after_copy`).

### Adding a new engine feature

Most changes touch: `model/job.rs` (add config field with `#[serde(default)]`), `ui/job_editor.rs` (expose in UI), `engine/executor.rs` (read the field, apply behavior). Use `#[serde(default)]` for any new `SyncJob` field to keep backward compatibility with existing `config.json` files.

### Windows-specific APIs in use

- `CopyFileEx` (progress callback for file copy)
- `DeviceIoControl` with `FSCTL_QUERY_USN_JOURNAL` / `FSCTL_READ_USN_JOURNAL`
- `GetVolumeInformationW` (fs type detection)
- `GetVolumePathNameW` (volume root from path)
- `GetFileInformationByHandle` (FRN / file index)
- `GetDriveTypeW` (remote drive detection)
- `MessageBeep` (completion sound)
- Long path prefix `\\?\` via `fs/long_path.rs`
