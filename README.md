# FileSync

**轻量高性能的 Windows 文件同步工具** | **Lightweight high-performance file sync tool for Windows**

[English](#english) | [中文](#中文)

---

## 中文

### 简介

FileSync 是一款使用 Rust 构建的 Windows 文件夹单向同步工具。它以极致性能和简洁体验为目标，利用 NTFS USN Journal 实现毫秒级增量变更检测，结合并发复制和 rsync 差量传输算法，大幅提升同步效率。

### 功能特性

- **单文件运行** — 无需安装，无运行时依赖，开箱即用
- **安全模式兼容** — 自动回退到 Mesa 软件渲染，支持在 Windows 安全模式下运行
- **NTFS USN Journal 加速** — 利用文件系统变更日志跳过未修改文件的扫描和哈希，极大缩短增量同步耗时
- **并发文件复制** — 可配置并发数，充分利用磁盘带宽
- **rsync 差量传输** — 大文件仅传输变更部分，节省时间和带宽
- **BLAKE3 校验** — 可选的复制后校验，确保数据完整性
- **智能复制策略** — 根据文件大小和卷类型自动选择最优复制方式（无缓冲 I/O / CopyFileEx / 差量传输）
- **双同步模式** — 保留模式（保留目标端多余文件）和镜像模式（删除目标端孤立文件/目录，通过回收站）
- **灵活过滤** — 支持 glob 模式的包含/排除规则（默认排除 `$RECYCLE.BIN`、`Thumbs.db` 等）
- **每任务文件比较** — 每个任务可独立配置元数据比较或 BLAKE3 内容哈希
- **定时同步** — 内置调度器，无需外部任务计划程序
- **系统托盘** — 最小化到系统托盘，后台静默运行
- **中英双语 UI** — 自动跟随系统语言（简体中文 / English）
- **应用日志** — 自动记录错误信息，便于问题排查（dev 模式记录完整日志，release 模式仅记录错误）
- **同步日志** — 每次同步生成独立日志文件，记录每个文件的操作结果，便于事后追溯
- **输入校验** — 保存任务、启动同步、触发预览前校验配置完整性，防止配置残缺导致错误
- **崩溃报告** — 程序崩溃时自动生成 minidump 文件，保留现场信息便于排查

### 系统要求

- Windows 10 或更高版本
- USN Journal 加速功能需要管理员权限（非必需，无权限时自动回退到常规扫描）
- 安全模式需要将 Mesa3D 的 `opengl32.dll`（以及 `libgallium_wgl.dll`）放在 exe 所在目录（非必需，正常模式不需要）

### 构建

需要 [Rust 工具链](https://rustup.rs/)（1.75+）。

```bash
# 调试构建
cargo build

# 发布构建（优化，约 5.8 MB）
cargo build --release
```

生成的可执行文件位于 `target/release/filesync.exe`。

### 使用

1. 运行 `filesync.exe`
2. 点击添加按钮创建同步任务
3. 设置源文件夹和目标文件夹
4. 根据需要配置同步模式、过滤规则和引擎选项
5. 点击开始同步

配置文件保存在 `%LOCALAPPDATA%\FileSync\config.json`。
应用日志保存在 `%LOCALAPPDATA%\FileSync\app_YYYY-MM-DD.log`。
同步日志保存在 `%LOCALAPPDATA%\FileSync\logs\{任务名}_{日期时间}.log`。

### 架构

```
egui UI → app.rs 状态机 → tokio 异步引擎 → Win32 文件系统层
```

| 层级 | 职责 |
|------|------|
| **UI** | egui 界面渲染，任务列表、编辑器、进度、预览 |
| **App** | 中心状态机，管理配置、会话和通道 |
| **Engine** | 异步执行：扫描、差异比较、哈希、复制 |
| **FS** | Win32 API 封装：卷信息、USN Journal、长路径 |

详细架构文档见 [CLAUDE.md](CLAUDE.md)。

---

## English

### About

FileSync is a one-way folder synchronization tool for Windows, built with Rust. It leverages NTFS USN Journal for millisecond-level incremental change detection, combined with concurrent copying and rsync delta transfer algorithm for maximum sync performance.

### Features

- **Single binary** — No installation required, no runtime dependencies
- **Safe Mode compatible** — Automatically falls back to Mesa software rendering for Windows Safe Mode
- **NTFS USN Journal acceleration** — Skips scanning and hashing for unchanged files using filesystem change journal
- **Concurrent file copying** — Configurable concurrency to fully utilize disk bandwidth
- **rsync delta transfer** — Only transfers changed portions of large files
- **BLAKE3 verification** — Optional post-copy checksum for data integrity
- **Smart copy strategy** — Automatically selects optimal copy method based on file size and drive type (unbuffered I/O / CopyFileEx / delta sync)
- **Two sync modes** — Preserve mode (keep extra files at destination) and Mirror mode (delete orphan files/directories via recycle bin at destination)
- **Flexible filtering** — Glob-based include/exclude rules (default excludes `$RECYCLE.BIN`, `Thumbs.db`, etc.)
- **Per-job file comparison** — Each job can independently use metadata comparison or BLAKE3 content hashing
- **Scheduled sync** — Built-in scheduler, no external task scheduler needed
- **System tray** — Minimize to system tray for background operation
- **Bilingual UI** — Automatically follows system language (Simplified Chinese / English)
- **Application logging** — Automatic error logging for troubleshooting (full logs in dev, errors only in release)
- **Sync logging** — Per-sync log file recording each file operation for post-mortem analysis
- **Input validation** — Validates configuration completeness before save, sync start, or preview to prevent misconfigured jobs from running
- **Crash reporting** — Automatically generates a minidump on unhandled exceptions for debugging

### Requirements

- Windows 10 or later
- Administrator privileges for USN Journal acceleration (optional — falls back to regular scanning without it)
- For Safe Mode: place Mesa3D `opengl32.dll` and `libgallium_wgl.dll` in the same directory as the exe (optional — not needed for normal operation)

### Build

Requires [Rust toolchain](https://rustup.rs/) (1.75+).

```bash
# Debug build
cargo build

# Release build (optimized, ~5.8 MB)
cargo build --release
```

The executable is at `target/release/filesync.exe`.

### Usage

1. Run `filesync.exe`
2. Click the add button to create a sync job
3. Set source and destination folders
4. Configure sync mode, filter rules, and engine options as needed
5. Click start to begin syncing

Configuration is saved to `%LOCALAPPDATA%\FileSync\config.json`.
Application logs are saved to `%LOCALAPPDATA%\FileSync\app_YYYY-MM-DD.log`.
Sync logs are saved to `%LOCALAPPDATA%\FileSync\logs\{job}_{datetime}.log`.

### Architecture

```
egui UI → app.rs orchestrator → tokio async engine → Win32 FS layer
```

| Layer | Responsibility |
|-------|---------------|
| **UI** | egui widgets: job list, editor, progress, preview |
| **App** | Central state machine; manages config, sessions, channels |
| **Engine** | Async orchestration: scanning, diffing, hashing, copying |
| **FS** | Win32 API wrappers: volume info, USN Journal, long paths |

See [CLAUDE.md](CLAUDE.md) for detailed architecture documentation.

---

## License

[MIT](LICENSE)
