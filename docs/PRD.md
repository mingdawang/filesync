# FileSync 产品需求文档（PRD）

**版本：** v1.5
**日期：** 2026-04-04
**状态：** 已完成（核心功能）

---

## 目录

1. [产品概述](#1-产品概述)
2. [目标用户](#2-目标用户)
3. [功能需求](#3-功能需求)
4. [非功能需求](#4-非功能需求)
5. [界面设计规范](#5-界面设计规范)
6. [数据模型](#6-数据模型)
7. [同步引擎规范](#7-同步引擎规范)
8. [文件系统加速特性](#8-文件系统加速特性)
9. [差量传输（Delta Sync）](#9-差量传输delta-sync)
10. [配置与持久化](#10-配置与持久化)
11. [错误处理规范](#11-错误处理规范)
12. [安全模式兼容](#12-安全模式兼容)
13. [应用日志](#13-应用日志)
14. [技术栈](#14-技术栈)
15. [项目结构](#15-项目结构)
16. [开发阶段规划](#16-开发阶段规划)
17. [验收标准](#17-验收标准)

---

## 1. 产品概述

### 1.1 产品定位

FileSync 是一款面向 Windows 平台的文件夹同步工具，使用 Rust 构建，提供图形用户界面（GUI）。它以高性能文件同步引擎为核心，结合 NTFS 文件系统特性实现增量同步加速，目标是成为比 FreeFileSync 更轻量、更快速的单向同步工具。

### 1.2 核心价值

- **轻量**：单一可执行文件，无需安装，无运行时依赖
- **高性能**：利用 NTFS USN Journal 实现毫秒级增量扫描，并发复制文件
- **易用**：GUI 操作，无需命令行知识；UI 语言自动跟随系统语言（简体中文 / 英文）
- **可靠**：错误不中断整体任务，完整日志可追溯
- **安全模式兼容**：支持在 Windows 安全模式下运行（通过 Mesa 软件渲染回退）
- **自动化**：内置定时同步调度，无需外部任务计划程序

### 1.3 范围限制

- 仅支持 Windows 10 及以上版本
- 仅支持单向同步（源 → 目标）
- 支持镜像模式（Mirror，删除目标端孤立文件）和更新模式（Update，保留目标端多余文件）两种策略
- 不支持网络同步协议（FTP、SFTP、WebDAV 等）
- 不支持云存储直接同步

---

## 2. 目标用户

### 2.1 主要用户

| 用户类型 | 描述 | 核心诉求 |
|----------|------|----------|
| 个人用户 | 需要定期备份文件到外置硬盘或 NAS | 操作简单，备份可靠 |
| 开发者 | 同步代码、构建产物到多个位置 | 速度快，支持排除 `.git` 等目录 |
| 内容创作者 | 大量图片/视频文件的备份 | 大文件传输快，稀疏文件支持 |
| IT 运维 | 服务器文件同步、批量备份任务 | 并发控制，错误日志，自动调度 |

### 2.2 使用场景

- **场景 A**：用户每周将工作文档从 `C:\Users\Documents` 同步到外置硬盘 `E:\Backup`
- **场景 B**：开发者将多个项目目录同步到 NAS，排除 `node_modules`、`.git`、`target` 等目录
- **场景 C**：摄影师将相机素材（含大量 RAW 文件）同步到备份盘，大文件需要快速传输
- **场景 D**：运维人员配置多个文件夹对的批量同步任务，设置定时自动执行，查看详细错误日志

---

## 3. 功能需求

### 3.1 任务管理

#### FR-01：创建同步任务

- 用户可以创建一个命名的同步任务（Job）
- 任务名称：必填，不可重复
- 每个任务包含一个或多个**文件夹对**（Folder Pair）
- 任务创建后自动保存到配置文件

#### FR-02：编辑同步任务

- 用户可以修改任务名称
- 用户可以添加、删除、启用/禁用、排序文件夹对
- 用户可以添加、删除排除规则
- 修改后需手动点击"保存"（`Ctrl+S`）才生效；未保存时顶栏显示黄色提示

#### FR-03：删除同步任务

- 右键菜单触发删除，弹出确认对话框，需二次确认
- 删除后从配置文件移除，不可撤销

#### FR-04：任务列表

左侧面板展示所有任务，每个任务条目显示：

- 任务名称（选中时加粗）
- 运行状态徽标：运行中（绿色 ●）/ 完成（✓）/ 带错误完成（⚠）/ 失败/停止（✗）
- 上次同步时间（`MM-DD HH:MM` 格式）
- 定时同步下次执行时间（⏰，若已启用定时）
- 上次运行统计摘要（复制 N / 跳过 N / 错误 N / 删除 N）

任务右键菜单提供：上移、下移、复制任务、删除任务。

多任务时显示"▶▶ 运行全部"按钮，按顺序依次执行所有任务。

---

### 3.2 文件夹对配置

#### FR-05：添加文件夹对

- 每个任务支持多个文件夹对
- 每个文件夹对包含：
  - 源目录路径（必填）
  - 目标目录路径（必填）
  - 启用/禁用开关

#### FR-06：路径选择

- 提供"浏览"按钮，调用 Windows 原生文件夹选择对话框
- 也支持直接在输入框手动输入路径
- 保存时校验路径格式合法性（不要求路径必须存在，允许可移除设备离线）

#### FR-06a：配置完整性校验

保存任务（`Ctrl+S`）、启动同步（`F5`）、触发预览之前，执行两级校验：

- **保存校验**：若任意已启用的文件夹对只设置了源路径或只设置了目标路径（部分配置），则阻止保存并提示用户补全
- **启动/预览校验**：除上述部分配置检查外，还要求至少存在一个源和目标路径均已设置的已启用文件夹对；否则阻止启动并提示用户

> 允许保存"完全空白"的任务（新建任务后尚未配置路径），但不允许保存"半填"状态（只填了源或只填了目标）。

#### FR-07：文件夹对排序

- 支持通过上移/下移按钮调整文件夹对的执行顺序

---

### 3.3 排除规则

#### FR-08：任务级排除规则

- 每个任务可配置若干排除规则，对该任务所有文件夹对生效
- 规则格式：Glob 模式（如 `*.tmp`、`.git/**`、`Thumbs.db`）
- 规则说明：

| 示例 | 含义 |
|------|------|
| `*.tmp` | 排除所有 `.tmp` 扩展名文件 |
| `.git/**` | 排除 `.git` 目录及其所有内容 |
| `Thumbs.db` | 排除名为 `Thumbs.db` 的文件 |
| `temp_*` | 排除所有以 `temp_` 开头的文件/目录 |
| `**/node_modules/**` | 排除任意层级的 `node_modules` 目录 |

#### FR-09：内置默认排除规则（可关闭）

以下规则默认启用，用户可手动移除：

- `$RECYCLE.BIN/**`（回收站）
- `Thumbs.db`
- `desktop.ini`
- `*.tmp`
- `~$*`（Office 临时文件）

#### FR-10：规则验证

- 输入排除规则时实时验证 Glob 格式是否合法
- 非法格式显示红色边框和错误提示

---

### 3.4 同步执行

#### FR-11：预览同步

- 触发前执行配置完整性校验（见 FR-06a），不通过则显示错误提示，不启动扫描
- 用户点击"预览同步"后，后台执行扫描和差异计算，不执行实际复制
- 显示加载中动画，可取消
- 预览结果窗口展示：
  - 待复制文件数量和总大小
  - 跳过文件数量
  - 孤立文件数量（目标端多余，当前同步不删除；Mirror 模式下将删除）
  - 文件列表（最多显示 1000 条，含图标、相对路径、大小）
- 用户可在预览窗口直接点击"立即同步"执行

#### FR-12：开始同步

- 触发前执行配置完整性校验（见 FR-06a），不通过则显示错误提示，不启动同步
- 点击"▶ 立即同步"直接执行（跳过预览）
- 同步期间禁止修改当前任务配置

#### FR-13：停止同步

- 点击"■ 停止"发送停止信号
- 当前正在复制的文件完成后停止，不强制中断（避免残留不完整文件）
- 同时清空任务执行队列（正在顺序执行多任务时，停止后续任务）

#### FR-14：并发控制

- 每个任务可配置并发线程数，范围 1–16，默认 4
- 并发针对文件级别：同时复制 N 个文件
- 不同文件夹对之间也可并发执行

#### FR-15：同步模式

- **镜像模式（Mirror）**：目标端严格以源为准，删除目标端中源没有的孤立文件
- **更新模式（Update，默认）**：只将源端文件同步到目标端，不删除目标端多余文件（孤立文件）
- 每个任务独立配置同步模式

---

### 3.5 进度与日志

#### FR-16：实时进度显示

进度面板（窗口底部，可调节高度）显示以下信息：

- 同步状态标签（运行中 / 已暂停 / 已完成 / 失败 / 已停止）
- 文件数进度条（已处理 / 总文件数 + 百分比）
- 数据量进度条（已复制字节数 / 总字节数，仅在有数据时显示）
- 统计行：复制数 / 跳过数 / 错误数 / 总量 / 差量节省 / 已删除 / 传输速度 / ETA
- 活跃 Worker 列表（每个线程正在复制的文件名、大小和进度）
- 完成摘要（完成后显示：复制数、跳过数、错误数、耗时）

#### FR-17：错误日志

- 错误不中断整体同步任务
- 每条错误记录：时间戳 + 文件路径 + 错误原因
- 错误日志在进度面板底部实时显示（最多显示最新 100 条）
- 支持"复制日志到剪贴板"
- 支持"导出日志到文件"（`.txt` 格式）

#### FR-18：应用内完成通知

- 同步完成后，在窗口右下角显示浮动通知 overlay
- 通知内容：任务名称 + 复制数 / 跳过数 / 错误数
- 3 秒后自动消失，也可手动关闭（× 按钮）
- 有错误时显示警告样式（黄色），无错误时显示成功样式（绿色）
- 播放系统提示音（`MessageBeep`）

---

### 3.6 定时同步

#### FR-19：定时自动同步

- 每个任务可独立配置定时同步（启用/禁用 + 间隔分钟数）
- 间隔时间：5 分钟 ~ 1440 分钟（24 小时），以分钟为单位输入
- 程序运行期间每 30 秒检查一次调度，无需外部任务计划程序
- 任务列表中显示下次执行时间（⏰ HH:MM）
- 从未执行过的任务：启动后立即执行一次
- 上次同步时间持久化到配置文件，程序重启后继续遵守调度间隔

---

### 3.7 设置与配置

#### FR-20：全局设置

通过顶栏"⚙ 设置"窗口配置：

- **文件比较方式**：元数据（大小 + 修改时间，默认）/ 内容哈希（BLAKE3，精确但较慢）— **per-job 配置**
- **界面主题**：跟随系统 / 浅色 / 深色
- **新建任务默认并发数**：1–16（默认 4）
- **关闭按钮行为**：询问 / 最小化到托盘 / 退出（仅在系统托盘可用时显示）

#### FR-21：配置导入/导出

- 导出：将当前所有任务和设置序列化为 JSON 文件（用户选择保存位置）
- 导入：从 JSON 文件加载配置，**覆盖当前所有任务和设置**，导入前给出警告提示

#### FR-22：界面语言跟随系统

- 程序启动时检测 Windows 系统语言（`GetUserDefaultUILanguage`）
- 系统语言为中文（zh-CN / zh-TW 等所有变体）→ 显示中文界面
- 其他语言 → 显示英文界面
- 检测结果在进程生命周期内缓存，不支持运行时切换

#### FR-23：系统托盘

- 程序支持最小化到系统托盘后台运行
- 托盘图标为蓝色同步图标（32×32 RGBA，程序化生成）
- 左键单击：显示主窗口
- 右键菜单：显示主窗口 / 退出
- 关闭按钮行为可在设置中配置（询问 / 最小化到托盘 / 退出）

#### FR-24：单实例保护

- 使用 Windows 全局命名互斥体（Named Mutex）防止多实例运行
- 若已有实例运行，弹出提示对话框后退出

---

## 4. 非功能需求

### 4.1 性能需求

| 指标 | 要求 |
|------|------|
| 增量扫描时间（USN Journal） | 10 万文件中 100 个变更 < 500ms |
| 全量扫描速度 | > 50,000 文件/秒 |
| 小文件并发复制（4 线程）| > 500 文件/秒（SSD to SSD） |
| 大文件复制速度 | 不低于同卷 Windows Explorer 复制速度的 90% |
| 内存占用 | 正常工作 < 100MB |
| 启动时间 | 冷启动 < 2 秒 |

### 4.2 可靠性需求

- 复制过程中程序崩溃，目标端不产生损坏的不完整文件（使用临时文件 + rename 原子替换）
- 配置文件写入使用原子操作（先写临时文件再替换），防止配置损坏
- 源路径不存在时，跳过该文件夹对并记录警告，不影响其他文件夹对

### 4.3 兼容性需求

- 支持 Windows 10（1809+）及 Windows 11
- 支持 x86_64 架构
- 文件路径支持最长 32767 字符（`\\?\` 长路径前缀）
- 文件名支持 Unicode（含中文、日文、表情符号等）

### 4.4 安全性需求

- 不收集任何用户数据，不联网
- 配置文件存储明文路径，不存储密码或敏感信息
- 不请求管理员权限（普通用户权限运行）

---

## 5. 界面设计规范

### 5.1 窗口布局

```
┌─────────────────────────────────────────────────────────────────┐
│  FileSync   ● 有未保存的修改        Ctrl+S 保存  F5 同步  ⚙ 设置  ℹ 关于 │
├──────────────┬──────────────────────────────────────────────────┤
│  任务列表     │                  主内容区（任务编辑器）              │
│  (210px)     │                                                   │
│  [▶▶ 运行全部]│                                                   │
│  [＋ 新建任务]│                                                   │
│              │                                                   │
├──────────────┴──────────────────────────────────────────────────┤
│  进度 / 日志面板（可拖拽调整高度，默认 180px）                        │
└─────────────────────────────────────────────────────────────────┘
```

- 最小窗口尺寸：800 × 500 px，默认启动尺寸：1000 × 650 px
- 支持窗口缩放；进度面板可拖拽上边框调整高度

### 5.2 颜色规范

| 用途 | 颜色 |
|------|------|
| 成功/完成 | #52C41A（绿色） |
| 警告 | #FAAD14（橙色） |
| 错误 | #FF4D4F（红色） |
| 运行中 | 绿色 |
| 孤立文件删除 | RGB(255, 140, 60)（橙色） |
| 差量节省 | RGB(100, 220, 100)（亮绿） |
| 定时信息 | RGB(100, 180, 255)（蓝色） |
| 背景 | 跟随系统（深色/浅色/系统默认，可在设置中切换） |

### 5.3 交互规范

- 所有破坏性操作（删除任务）需二次确认对话框
- 快捷键：`Ctrl+S` 保存任务，`F5` 开始同步
- 配置有修改未保存时，顶栏黄色 `● 有未保存的修改` 提示

---

## 6. 数据模型

### 6.1 核心结构（持久化到 config.json）

```
AppConfig
├── version: u32                         // 配置文件版本号，用于迁移
├── settings: AppSettings
│       ├── default_concurrency: usize   // 新建任务默认并发数（4）
│       ├── theme: Theme                 // System | Light | Dark
│       └── close_action: CloseAction    // Ask | MinimizeToTray | Quit
└── jobs: Vec<SyncJob>

SyncJob
├── id: Uuid                             // 唯一标识，不随重命名变化
├── name: String                         // 用户可见名称
├── concurrency: usize                   // 并发线程数（1-16）
├── sync_mode: SyncMode                  // Mirror（删孤立）| Update（保留孤立，默认）
├── compare_method: CompareMethod        // Metadata（默认）| Hash，per-job 配置
├── folder_pairs: Vec<FolderPair>
├── exclusions: Vec<ExclusionRule>
├── engine_options: EngineOptions
├── schedule: ScheduleConfig
│       ├── enabled: bool
│       └── interval_minutes: u32        // 0 = 不启用
├── last_sync_time: Option<DateTime<Utc>>     // 上次同步完成时间
└── last_run_summary: Option<RunSummary>      // 上次运行统计
        ├── copied: u64
        ├── skipped: u64
        ├── errors: u64
        ├── deleted: u64
        ├── bytes: u64                   // 实际写入字节数
        └── elapsed_secs: u64

// 注：last_sync_checkpoints 为纯内存字段（#[serde(skip)]），不写入 config.json
// 见 6.2 运行时结构

FolderPair
├── id: Uuid
├── source: PathBuf
├── destination: PathBuf
└── enabled: bool

ExclusionRule
├── pattern: String                      // Glob 表达式
└── enabled: bool

EngineOptions
├── verify_after_copy: bool              // 复制后 BLAKE3 校验（默认 false）
├── unbuffered_threshold_mb: u64         // 无缓冲 IO 阈值（默认 128）
└── delta_threshold_mb: u64              // 差量传输阈值（默认 4），0 = 禁用
```

### 6.2 运行时结构（仅内存，不持久化）

```
SyncSession（单次同步会话状态）
├── job_id: Uuid
├── status: SessionStatus                // Running | Paused | Completed | Failed | Stopped
├── started_at: DateTime<Utc>
├── stats: SyncStats
│       ├── total_files: u64
│       ├── processed_files: u64
│       ├── copied_files: u64
│       ├── delta_files: u64             // 使用差量传输的文件数
│       ├── skipped_files: u64
│       ├── deleted_files: u64
│       ├── orphan_files: u64            // 孤立项目总数（含目录，Mirror/Update 均统计）
│       ├── error_count: u64
│       ├── total_bytes: u64
│       ├── copied_bytes: u64
│       ├── saved_bytes: u64             // 差量传输节省的字节数
│       └── speed_bps: u64              // 当前传输速度
├── active_workers: Vec<WorkerState>     // 每个并发 worker 的当前状态
├── deleted_paths: Vec<PathBuf>          // Mirror 模式已删除路径
├── copied_log: Vec<CopiedFileEntry>     // 已复制文件记录（路径、大小、是否 delta）
├── orphan_log: Vec<PathBuf>             // 孤立文件路径（Update 模式下保留，用于日志）
└── errors: Vec<SyncError>

SyncError
├── timestamp: DateTime<Utc>
├── path: PathBuf
├── kind: ErrorKind                      // IoError | AccessDenied | ...
└── message: String

// USN Journal 增量扫描检查点（纯内存，#[serde(skip)]）
// 保存在 SyncJob.last_sync_checkpoints: HashMap<String, UsnCheckpoint>
// 其中 Key = 卷根路径（如 "C:\\"），Value = UsnCheckpoint
UsnCheckpoint
├── journal_id: u64                      // USN Journal ID，用于检测 Journal 重建
└── next_usn: i64                        // 下次扫描的起始 USN

DiffEntry（差异计算结果）
├── source: PathBuf
├── destination: PathBuf
├── relative_path: PathBuf
├── size: u64
└── action: DiffAction                   // Create | Update | Skip | Orphan

CopiedFileEntry（已复制文件记录，用于同步日志）
├── path: PathBuf
├── size: u64
└── delta: bool                          // 是否使用差量传输
```

---

## 7. 同步引擎规范

### 7.1 同步模式

**更新模式（Update，默认）**

> 以源目录为准，将源端文件同步到目标端。  
> 目标中有而源中没有的文件（孤立文件）→ **保留，不删除**。  
> 预览界面显示孤立文件数量（仅提示，不操作）。

**镜像模式（Mirror）**

> 以源目录为准，目标目录与源目录完全一致。  
> 目标中有而源中没有的文件（孤立文件/目录）→ **删除**（优先移入回收站，Shell 不可用时直接删除）。  
> 每个任务独立设置，默认不启用。

### 7.2 文件变更判断策略

文件比较方式为 **per-job 配置**（`SyncJob::compare_method`），每个任务可独立选择元数据比对或内容哈希比对。

#### 策略一：元数据比对（默认，快速）

满足以下任一条件认为文件已变更：
1. 目标文件不存在
2. 文件大小不同
3. 源文件修改时间 > 目标文件修改时间（允许 1 秒误差，兼容 FAT32 时间戳精度）

#### 策略二：内容哈希比对（可选，精确）

使用 BLAKE3 计算源和目标文件哈希值，不同则视为变更。  
适用场景：修改时间不可信（如从网络路径复制来的文件）。

**USN 哈希跳过优化**（哈希模式下）：在同一 session 内，若源和目标两端的 FRN 均不在 USN Journal 变更集中，则跳过哈希计算直接判定为 Skip，大幅减少哈希 I/O 开销。详见 [8.1](#81-usn-journal-增量扫描)。

### 7.3 文件复制决策

```
文件需要复制（Create 或 Update）？
    │
    ├── 文件大小 < delta_threshold（默认 4MB），或目标文件不存在（Create）
    │       ├── 大文件（>= unbuffered_threshold，默认 128MB，且 caps.supports_unbuffered_io() 为真）
    │       │       └── → 无缓冲 IO（FILE_FLAG_NO_BUFFERING）
    │       └── 其他
    │               └── → CopyFileEx（Windows 原生，含进度回调）
    │                       └── 失败时 → 缓冲分块复制（256KB，重试 3 次）
    │
    └── 文件大小 >= delta_threshold 且目标文件存在（Update）
            └── → 差量传输（Delta Sync，rsync 算法，固定 2KB 块）
                    └── 失败或节省量 < 文件大小 5% → 降级为 CopyFileEx
```

`copier::copy_file_with_caps()` 接受 `VolumeCapabilities` 参数（由 `detect_volume()` 探测），根据卷类型判断是否支持无缓冲 I/O（远程驱动器自动禁用）。

所有策略均使用：**临时文件 + 原子 rename**（`MoveFileEx`）保障写入安全。  
可选：复制完成后 BLAKE3 校验（`engine_options.verify_after_copy`）。

### 7.4 并发执行规范

- 使用 `tokio` 异步运行时管理并发任务
- 使用 `Semaphore` 控制最大并发数（`concurrency` 配置项）
- 不同文件夹对的文件可交叉并发
- 同一文件只会被一个 worker 处理

### 7.5 同步执行流程

```
Step 0: USN Journal 预扫描（NTFS/ReFS 本地卷，session 内有检查点时）
  ├── 对每个启用的文件夹对的源和目标路径，获取卷根（GetVolumePathNameW）
  ├── 查询 USN Journal 信息（FSCTL_QUERY_USN_JOURNAL）
  ├── 若 journal_id 匹配且 next_usn 有推进：
  │       读取变更 FRN 集合（FSCTL_READ_USN_JOURNAL）
  └── 保存新的 (journal_id, next_usn) 用于本次同步后更新检查点

Step 1: 扫描（Scan）
  ├── 对源和目标目录执行全量 walkdir 扫描（ScanResult: HashMap<相对路径, 元数据>）
  └── 应用排除规则（globset）过滤文件列表

Step 2: 差异计算（Diff）
  ├── 对比源和目标，生成 DiffEntry 列表（action: Create | Update | Skip | Orphan）
  └── Hash 模式（per-job compare_method == Hash）：
      ├── 对 Update 条目**并发**计算 BLAKE3 哈希（tokio JoinSet，全部任务同时提交）
      ├── 若 USN 跳过优化可用：src_frn 和 dst_frn 均不在变更集 → 直接降级为 Skip（跳过哈希）
      └── 哈希相同 → 降级为 Skip

Step 3: 预览模式（可选）
  └── 向 UI 返回 DiffEntry 列表，等待用户确认

Step 3b: 卷能力探测（detect_volume）
  └── 检测目标卷类型（本地/远程、文件系统类型、无缓冲 IO 支持），传递给 copier

Step 4: 执行（Execute）
  ├── 按 7.3 决策树并发执行复制（tokio + Semaphore）
  ├── 实时发送 SyncEvent（Started / FileStarted / FileProgress / FileCompleted / FileSkipped / FileError / SpeedUpdate）
  ├── 孤立文件处理：
  │     Mirror 模式 → 移入回收站删除（FileDeleted 事件），失败时直接删除
  │     Update 模式 → 不删除，发送 FileOrphan 事件（仅记录）
  └── Mirror 模式清理孤立目录（由深到浅排序，移入回收站或直接删除）

Step 5: 收尾（Complete）
  ├── 发送 SyncEvent::Completed { stats, usn_checkpoints }
  │       usn_checkpoints 仅在 error_count == 0 且未被停止时携带
  ├── app.rs 接收后：更新 last_sync_time、last_run_summary、last_sync_checkpoints（内存）
  ├── 生成同步日志文件（%LOCALAPPDATA%\FileSync\logs\{任务名}_{日期时间}.log）
  └── 保存配置（持久化 last_sync_time / last_run_summary）
```

---

## 8. 文件系统加速特性

### 8.1 USN Journal 增量扫描

**适用：** NTFS / ReFS 本地卷  
**目的：** 在哈希比对模式下，跳过自上次同步以来未发生变化的文件的哈希计算，减少 I/O 开销。

**设计约束：检查点仅保存在内存中（`#[serde(skip)]`），不写入磁盘。**

原因：若程序关闭后目标端文件被手动修改，磁盘上的检查点无法反映这一变化，会导致漏同步。内存检查点只在本次运行期间有效，程序启动后首次同步总是执行完整扫描，确保正确性。

**工作流程：**

```
app 启动
  └── last_sync_checkpoints = {} （空，无检查点）

首次同步
  ├── 无检查点 → 全量元数据/哈希扫描
  └── 完成（error_count == 0）→ 查询当前 USN 状态，保存检查点到内存
         { "C:\\" → UsnCheckpoint { journal_id, next_usn } }

同一 session 内第二次同步
  ├── 有检查点，且 journal_id 匹配
  ├── 读取 FSCTL_READ_USN_JOURNAL（从 checkpoint.next_usn 到当前 next_usn）
  │       → 获取变更文件的 FRN 集合
  └── 哈希 Update 文件时：
        若 src_frn ∉ 变更集 AND dst_frn ∉ 变更集 → 跳过哈希，直接 Skip
        否则 → 正常哈希比对

程序重启
  └── last_sync_checkpoints = {} （清空）→ 下次同步为全量扫描
```

**API 使用：**
- `GetVolumePathNameW` → 获取卷根路径（如 `C:\`）作为 HashMap 键
- `FSCTL_QUERY_USN_JOURNAL` → 获取 `UsnJournalID` 和 `NextUsn`
- `FSCTL_READ_USN_JOURNAL` → 读取变更记录，提取 `FileReferenceNumber`
- `GetFileInformationByHandle` → 获取文件 FRN（仅对 Update 文件按需调用）

**降级策略：**
- Journal 不存在 / 被禁用 → 跳过优化，正常哈希比对
- Journal ID 变化（Journal 被重建）→ 跳过优化，正常哈希比对
- 检查点为空（app 刚启动 / 上次同步有错误）→ 跳过优化，正常哈希比对

### 8.2 大文件无缓冲 IO

**适用：** 所有本地卷  
**目的：** 超大文件复制时绕过 Windows 页面缓存，避免污染内存。

**触发条件：** 文件大小 ≥ `unbuffered_threshold_mb`（默认 128MB），且目标为本地驱动器（非网络路径）

**实现：**
- 打开源文件：`FILE_FLAG_NO_BUFFERING | FILE_FLAG_SEQUENTIAL_SCAN`
- 打开目标文件：`FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH`
- 使用 sector 对齐的缓冲区（通常 4096 字节）

### 8.3 CopyFileEx

**适用：** 所有路径（含网络路径）  
**目的：** 利用 Windows 内核优化的文件复制路径，适用于小文件和中等文件。

**实现：**
- 传入进度回调函数获取字节级进度，通过 `SyncEvent::FileProgress` 上报

### 8.4 稀疏文件感知复制（未实现）

**适用：** NTFS / ReFS 本地卷（目标卷）  
**目的：** 对稀疏文件（虚拟机镜像、数据库文件等）只传输实际分配的数据区域。  
**状态：** 卷能力检测已实现（`volume.rs` 可识别稀疏文件支持），但复制逻辑尚未利用此信息。

### 8.5 ReFS Block Clone（未实现）

**适用：** ReFS 本地卷，且源和目标在同一卷  
**目的：** 同卷内复制，由文件系统在元数据层完成，不移动实际数据。  
**状态：** 卷能力检测已实现（`volume.rs` 可识别 ReFS Block Clone 支持），但复制逻辑尚未利用此信息。

---

## 9. 差量传输（Delta Sync）

差量传输是纯软件算法，**与文件系统类型无关**，适用于所有卷（包括网络路径）。

### 9.1 目的

对已存在于目标端的文件，不整体重传，只传输发生变化的数据块，大幅减少 IO 量。

**最大收益场景：**

| 文件类型 | 文件大小 | 变化量 | 整体复制 | 差量传输 |
|----------|----------|--------|----------|----------|
| 数据库 .db | 2 GB | 追加 10 MB | 2 GB | ~10 MB |
| Word 文档 | 50 MB | 修改 2 段文字 | 50 MB | ~200 KB |
| VM 镜像 .vhdx | 100 GB | 写了 500 MB | 100 GB | ~500 MB |
| 日志文件 | 500 MB | 追加 20 MB | 500 MB | ~20 MB |

### 9.2 算法：滚动校验 + 强哈希（rsync 算法）

```
Step 1: 目标端分块
  将目标文件切成固定大小的块（block_size 自适应，见 9.3）
  每块计算：Adler-32 弱校验（快速，过滤）+ BLAKE3 强哈希（精确确认）
  构建哈希索引表（本地直接读取，无需网络传输）

Step 2: 源端滚动扫描
  在源文件上滚动 block_size 大小的窗口
  使用 O(1) 滚动公式维护 Adler-32（每字节仅需加减运算，无需重算整块）
  每个位置计算 Adler-32 → 查哈希表 → 命中则校验 BLAKE3 确认
    命中（match）  → 记录"引用目标块 #N"（无需传输），窗口跳过整块重新播种
    未命中（literal）→ 记录"原始字节数据"，滑动 1 字节

Step 3: 重建目标文件
  按指令序列写入临时文件：引用已有块 or 写入新数据
  完成后原子 rename 替换目标文件
```

### 9.3 块大小

当前实现使用固定块大小 **2 KB**，以最大化块命中率。

### 9.4 触发条件

```
启用差量传输，当且仅当：
  1. 文件大小 >= delta_threshold_mb（默认 4MB）
  2. 目标文件已存在
  3. delta_threshold_mb != 0（用户未禁用）

执行过程中动态降级：
  若差量节省量 < 文件大小 × 5%（收益过低）
  → 放弃差量，直接 CopyFileEx 整体复制
```

### 9.5 进度显示

统计行中显示差量节省量（亮绿色）：

```
Delta saved: 1.8 GB
```

同步完成摘要中显示节省量：

```
同步完成：复制 1,204 个，跳过 890 个，耗时 3m42s
```

---

## 10. 配置与持久化

### 10.1 配置文件位置

```
%LOCALAPPDATA%\FileSync\config.json
```

示例：`C:\Users\<用户名>\AppData\Local\FileSync\config.json`

### 10.2 配置文件格式

```json
{
  "version": 1,
  "settings": {
    "default_concurrency": 4,
    "theme": "System",
    "close_action": "Ask"
  },
  "jobs": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "工作备份",
      "concurrency": 4,
      "sync_mode": "Update",
      "compare_method": "Metadata",
      "folder_pairs": [
        {
          "id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
          "source": "C:\\Users\\me\\Documents",
          "destination": "D:\\Backup\\Documents",
          "enabled": true
        }
      ],
      "exclusions": [
        { "pattern": "*.tmp", "enabled": true },
        { "pattern": ".git/**", "enabled": true }
      ],
      "engine_options": {
        "verify_after_copy": false,
        "unbuffered_threshold_mb": 128,
        "delta_threshold_mb": 4
      },
      "schedule": {
        "enabled": false,
        "interval_minutes": 60
      },
      "last_sync_time": "2026-04-03T10:00:00Z",
      "last_run_summary": {
        "copied": 12,
        "skipped": 890,
        "errors": 0,
        "deleted": 0,
        "bytes": 52428800,
        "elapsed_secs": 8
      }
    }
  ]
}
```

**注意：** `last_sync_checkpoints`（USN 检查点）不出现在 JSON 中（`#[serde(skip)]`），程序重启后始终从空检查点开始。

### 10.3 配置写入策略

1. 序列化新配置内容到内存
2. 写入临时文件：`config.json.tmp`
3. 原子替换：`MoveFileEx(tmp, config.json, MOVEFILE_REPLACE_EXISTING)`
4. 若写入失败，保留原配置文件不变

### 10.4 配置版本迁移

- 配置文件中 `version` 字段标识格式版本
- 程序启动时检测版本号，低版本自动迁移

---

## 11. 错误处理规范

### 11.1 错误分类与处理策略

| 错误类型 | 示例 | 处理策略 |
|----------|------|----------|
| 源路径不存在 | 移动硬盘未插入 | 跳过该文件夹对，记录警告 |
| 目标路径不存在 | 目标目录未创建 | 自动创建目录，继续执行 |
| 文件访问被拒绝 | 系统文件、权限不足 | 跳过该文件，记录错误 |
| 文件被锁定 | 程序正在使用的文件 | 跳过该文件，记录错误 |
| 磁盘空间不足 | 目标盘满 | 立即停止同步，显示全局错误弹窗 |
| 路径过长 | 超过 MAX_PATH | 长路径前缀（`\\?\`）自动处理 |
| IO 错误 | 硬盘坏道 | 重试 3 次，仍失败则跳过并记录错误 |
| 配置文件损坏 | JSON 解析失败 | 加载失败时使用默认配置，打印错误到 stderr |

### 11.2 错误重试策略

- 可重试错误（IO 错误、临时锁定）：间隔 500ms 重试，最多 3 次
- 不可重试错误（访问被拒绝）：直接跳过，记录错误

### 11.3 崩溃处理

程序通过以下机制尽量保留崩溃现场：

- **SEH 异常处理**（`SetUnhandledExceptionFilter`）：捕获未处理的 Windows 结构化异常（如访问违规、栈溢出），自动生成 minidump 文件（`dbghelp.dll` 运行时动态加载，无需编译时依赖）
- **Panic 钩子**：自定义 Rust panic hook，将 panic 信息写入应用日志后再继续默认处理流程
- **Minidump 位置**：exe 所在目录，文件名格式 `filesync_YYYY-MM-DD_HH-MM-SS.dmp`
- **异常对话框**：SEH 捕获后弹出 MessageBox 显示异常代码和地址（支持中英文）

---

## 12. 安全模式兼容

### 12.1 问题

Windows 安全模式下 GPU 驱动未加载，导致图形渲染初始化失败：
- **wgpu** 需要 DX12/Vulkan GPU 驱动 → 安全模式下不可用
- **glow** 需要 OpenGL 2.0+ → 安全模式仅提供 OpenGL 1.1

### 12.2 解决方案

采用三级回退策略，依次尝试渲染后端：

```
wgpu（GPU 硬件加速）→ glow（原生 OpenGL）→ glow + Mesa llvmpipe（软件渲染）
```

| 回退级别 | 渲染后端 | 适用场景 |
|----------|----------|----------|
| 1 | wgpu | 正常模式，GPU 驱动可用 |
| 2 | glow | GPU 驱动不可用但 OpenGL 2.0+ 可用 |
| 3 | glow + Mesa | 安全模式或其他 OpenGL 不可用的环境 |

### 12.3 Mesa 软件渲染

- 将 Mesa3D 的 `opengl32.dll`（llvmpipe 构建）和 `libgallium_wgl.dll` 放在 exe 所在目录
- 仅在前两级渲染失败后自动激活，不影响正常模式下的渲染性能
- 激活时设置环境变量：
  - `GALLIUM_DRIVER=llvmpipe`
  - `GLUTIN_WGL_OPENGL_DLL=./opengl32.dll`
- Mesa 来源：[pal1000/mesa-dist-win](https://github.com/pal1000/mesa-dist-win)

### 12.4 错误提示

所有渲染后端均失败时，弹出 MessageBox 显示错误详情（支持中英文），内容包含各级别的错误信息。应用日志同时记录完整的回退过程。

### 12.5 系统托盘与关闭按钮

安全模式下系统托盘服务不可用，`AppTray::new()` 返回 `None`，应用继续运行但无托盘功能。退出时需通过窗口关闭按钮。

关闭按钮行为通过 `wndproc` 钩子拦截 `WM_SYSCOMMAND(SC_CLOSE)` 实现，独立于托盘可用性：
- `CloseAction::Ask`：弹出确认对话框（含"记住选择"复选框）
- `CloseAction::MinimizeToTray`：隐藏到托盘（托盘不可用时忽略）
- `CloseAction::Quit`：直接退出程序

---

## 13. 应用日志

### 13.1 设计目标

记录应用运行过程中的关键事件和错误信息，便于问题排查。

### 13.2 日志级别

| 级别 | dev 构建 | release 构建 |
|------|----------|-------------|
| INFO | 记录 | 跳过 |
| ERROR | 记录 | 记录 |

### 13.3 日志文件

**应用日志**

- **路径**：`%LOCALAPPDATA%\FileSync\app_YYYY-MM-DD.log`
- **格式**：每天一个文件，追加写入
- **编码**：UTF-8
- **内容格式**：`[时间戳] [级别] 消息`

示例：
```
[2026-04-04 10:00:00.123 UTC] [INFO ] ===== FileSync starting =====
[2026-04-04 10:00:00.456 UTC] [INFO ] loading config from: C:\Users\...\AppData\Roaming\FileSync\config.json
[2026-04-04 10:00:01.789 UTC] [ERROR] wgpu failed: No suitable adapter found
[2026-04-04 10:00:02.012 UTC] [INFO ] falling back to glow (native OpenGL)...
```

**同步日志**

每次同步任务启动时创建独立日志文件，记录每个文件的详细操作结果，便于事后审计与排查。

- **路径**：`%LOCALAPPDATA%\FileSync\logs\{任务名}_{YYYY-MM-DD_HH-MM-SS}.log`
- **格式**：每次同步一个文件，追加写入
- **编码**：UTF-8

示例：
```
[2026-04-04 10:05:01.234 UTC] [COPY ] docs\report.pdf  (52428800 bytes)
[2026-04-04 10:05:02.456 UTC] [SKIP ] images\logo.png
[2026-04-04 10:05:02.789 UTC] [DEL  ] old\archive.zip  (orphan, mirror mode)
[2026-04-04 10:05:03.012 UTC] [ERR  ] locked\file.db — 拒绝访问
```

### 13.4 覆盖范围

以下模块的关键错误和事件记录到日志：

| 模块 | 记录内容 |
|------|----------|
| 启动流程 | 单实例检测、渲染后端尝试与回退、Mesa DLL 检测 |
| 配置加载/保存 | APPDATA 缺失、配置文件读写失败、格式错误 |
| 系统托盘 | 图标创建失败、托盘注册失败 |
| 同步引擎 | tokio runtime 创建失败、扫描失败、复制错误 |
| 文件复制 | CopyFileEx 失败、缓冲复制重试耗尽、临时文件清理失败 |
| 目录扫描 | 权限拒绝、GlobSet 解析失败 |
| USN Journal | 卷句柄打开失败、Journal 查询失败、FRN 读取错误 |
| 卷信息 | 驱动器类型检测失败、卷信息查询失败、扇区大小查询失败 |

### 13.5 实现位置

| 文件 | 说明 |
|------|------|
| `src/log/app_log.rs` | 日志函数 `app_log(msg, level)` 和日志级别定义 |
| `src/log/sync_log.rs` | 同步操作日志（每次同步生成独立日志文件） |
| `src/log/mod.rs` | 模块导出 |

---

## 14. 技术栈

### 14.1 核心依赖

```toml
[dependencies]
# GUI 框架（显式指定 wgpu + glow 渲染后端，禁用其他默认特性）
eframe = { version = "0.29", default-features = false, features = ["default_fonts", "wgpu", "glow"] }
egui = "0.29"

# 文件系统遍历
walkdir = "2"

# 异步运行时
tokio = { version = "1", features = ["full"] }

# 文件哈希（内容比对 + Delta Sync 强哈希）
blake3 = "1"

# Delta Sync 滚动弱校验（自实现 Adler-32，兼备外部 crate）
adler = "1"

# 镜像模式孤立文件删除（移入回收站）
trash = "3"

# Glob 排除规则匹配
globset = "0.4"

# 配置序列化
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# 跨线程进度事件通道
flume = "0.11"

# 错误处理
anyhow = "1"

# UUID 生成
uuid = { version = "1", features = ["v4", "serde"] }

# 原生文件/文件夹选择对话框
rfd = "0.14"

# Windows API（USN Journal / CopyFileEx / Win32）
windows = { version = "0.58", features = [
    "Win32_Storage_FileSystem",
    "Win32_System_IO",
    "Win32_System_Ioctl",
    "Win32_System_Threading",
    "Win32_System_LibraryLoader",
    "Win32_Foundation",
    "Win32_Security",
    "Win32_UI_WindowsAndMessaging",
] }

# 系统托盘
tray-icon = "0.14"

# 时间处理
chrono = { version = "0.4", features = ["serde"] }
```

### 14.2 构建配置

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
strip = true         # 去除调试符号，减小体积（约 5.8 MB）
```

Release 模式隐藏控制台窗口：

```rust
// src/main.rs 顶部
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
```

---

## 15. 项目结构

```
filesync/
├── Cargo.toml
├── Cargo.lock
├── build.rs                         // 嵌入 Windows 图标和清单（res/app.rc）
├── README.md                        // 项目说明（中英双语）
├── LICENSE                          // MIT 许可证
├── CLAUDE.md                        // Claude Code 开发指引
├── docs/
│   └── PRD.md                       // 产品需求文档（本文件）
├── res/
│   ├── app.rc                       // Windows 资源定义
│   └── app.manifest                 // Windows 应用清单
│
│   // 安全模式软件渲染支持（放在 exe 所在目录，非仓库内）：
│   //   opengl32.dll      — Mesa3D llvmpipe OpenGL 实现
│   //   libgallium_wgl.dll — Gallium WGL 后端（Mesa 全功能模式所需）
└── src/
    ├── main.rs                      // 程序入口，渲染回退链，单实例保护
    ├── app.rs                       // 顶层 App 状态，实现 eframe::App trait，调度逻辑
    ├── i18n.rs                      // 国际化：系统语言检测，t()/is_zh() 工具函数
    ├── tray.rs                      // 系统托盘支持（图标、菜单、事件中继线程）
    │
    ├── ui/                          // 界面层
    │   ├── mod.rs
    │   ├── job_list.rs              // 左侧任务列表面板
    │   ├── job_editor.rs            // 任务编辑器（文件夹对 + 排除规则 + 设置）
    │   ├── progress.rs              // 进度和日志面板
    │   └── preview.rs               // 差异预览浮动窗口
    │
    ├── engine/                      // 同步引擎层
    │   ├── mod.rs
    │   ├── scanner.rs               // 目录扫描（walkdir + globset 过滤）
    │   ├── diff.rs                  // 差异计算（元数据 / 哈希比对）
    │   ├── executor.rs              // 并发执行（tokio + Semaphore）+ USN 预扫描 + 卷能力探测
    │   ├── copier.rs                // 文件复制策略（CopyFileEx / 无缓冲 IO / 缓冲回退）
    │   ├── delta.rs                 // 差量传输（rsync 算法：Adler-32 + BLAKE3）
    │   ├── hash.rs                  // 文件哈希工具（BLAKE3）
    │   └── events.rs                // 进度事件定义（SyncEvent enum，含 FileOrphan 等）
    │
    ├── fs/                          // 文件系统特性封装
    │   ├── mod.rs
    │   ├── volume.rs                // 卷类型探测（VolumeCapabilities）
    │   ├── usn_journal.rs           // USN Journal 读取（NTFS/ReFS）
    │   └── long_path.rs             // 长路径前缀（\\?\）处理
    │
    ├── log/                         // 应用日志
    │   ├── mod.rs                   // 模块导出
    │   ├── app_log.rs               // 应用日志（app_log，写入 app_YYYY-MM-DD.log）
    │   └── sync_log.rs              // 同步日志（每次同步独立日志文件）
    │
    ├── model/                       // 数据模型
    │   ├── mod.rs
    │   ├── job.rs                   // SyncJob, FolderPair, ExclusionRule, UsnCheckpoint
    │   ├── config.rs                // AppConfig, AppSettings, EngineOptions
    │   ├── session.rs               // SyncSession, SyncStats, SyncError
    │   └── preview.rs               // PreviewEntry, PreviewState
    │
    └── config/
        ├── mod.rs
        └── storage.rs               // 配置读写（AppData 路径，原子写入）
```

---

## 16. 开发阶段规划

### Phase 1：基础骨架 ✅ 已完成

- [x] 项目初始化，`Cargo.toml` 依赖配置
- [x] 数据模型定义（`model/`）
- [x] 配置读写（`config/storage.rs`，原子写入）
- [x] 基础 UI 骨架：任务列表 + 任务编辑器
- [x] 原生文件夹选择对话框（`rfd`）
- [x] 排除规则编辑组件（含 Glob 实时验证）

### Phase 2：同步引擎 ✅ 已完成

- [x] 全量目录扫描（`walkdir`）
- [x] 元数据差异计算
- [x] 临时文件 + 原子 rename 复制策略
- [x] 进度事件定义和 `flume` channel 通信
- [x] 基础进度 UI 显示

### Phase 3：并发与加速特性 ✅ 已完成

- [x] `tokio` 并发 worker pool（Semaphore 控制）
- [x] `CopyFileEx` 替换手动 IO
- [x] USN Journal 增量扫描（in-memory 检查点，`FSCTL_READ_USN_JOURNAL`）
- [x] 差量传输（Delta Sync，`engine/delta.rs`，rsync 算法）
- [x] 大文件无缓冲 IO
- [x] 卷类型探测（`fs/volume.rs`，支持 NTFS/ReFS/ExFAT/FAT32 识别）

**未实现（可作为后续优化方向）：**
- [ ] 稀疏文件感知复制（已检测能力，未利用）
- [ ] ReFS Block Clone（已检测能力，未利用）
- [ ] Move/Rename 检测

### Phase 4：完善与打磨 ✅ 已完成

- [x] 预览同步功能（后台扫描，结果窗口）
- [x] 停止控制（发送 AtomicBool 信号，优雅停止）
- [x] 错误日志显示、复制、导出
- [x] 应用内浮动通知（右下角 overlay，3 秒自动消失）
- [x] 系统提示音（`MessageBeep`）
- [x] 应用图标（程序化生成 32×32 同步图标）
- [x] 长路径支持（`\\?\` 前缀，`res/app.rc` Manifest）
- [x] 深色/浅色/系统主题切换
- [x] 系统托盘支持（最小化到托盘，右键菜单）
- [x] 单实例保护（全局命名互斥体）
- [x] 关闭行为可配置（询问 / 最小化到托盘 / 退出）

### Phase 5：扩展功能 ✅ 已完成

- [x] 定时自动同步（interval_minutes，程序内调度）
- [x] 顺序执行多任务队列（"运行全部"）
- [x] 镜像模式（删除孤立文件，SyncMode::Mirror）
- [x] 配置导入/导出（JSON，rfd 文件对话框）
- [x] 任务复制（含重置调度状态）
- [x] 任务上下移动排序
- [x] 上次运行统计摘要（任务列表展示）
- [x] 界面语言跟随系统（`GetUserDefaultUILanguage`，zh/en）
- [x] 全局设置窗口（比较方式 / 主题 / 默认并发数）
- [x] 关于窗口（版本、配置路径链接）

### Phase 6：运维与兼容性 ✅ 已完成

- [x] 安全模式兼容（wgpu → glow → Mesa llvmpipe 三级回退）
- [x] 应用日志系统（dev 全量日志，release 仅错误日志，按日期分文件）
- [x] 引擎与文件系统层错误日志覆盖（copier/scanner/executor/usn_journal/volume）
- [x] 同步日志（每次同步独立日志文件，记录每个文件操作结果）
- [x] 孤立文件删除使用系统回收站（`trash` crate）

### Phase 7：性能优化与健壮性 ✅ 已完成

- [x] Delta Sync 滚动 Adler-32：O(N) 全量重算 → O(1) 滚动公式（每次 miss 仅需加减运算）
- [x] Hash 模式并发化：串行 `await` → tokio JoinSet 全并发提交，充分利用多核 I/O
- [x] 原子操作精简：按访问线程正确选用 Relaxed / Release / Acquire，消除多余 SeqCst
- [x] 输入校验（两级）：保存前检测部分配置；启动/预览前检测部分配置 + 空任务
- [x] 崩溃保护：SEH 异常过滤器 + minidump 生成（`dbghelp.dll` 动态加载）+ panic 钩子

---

## 17. 验收标准

### 15.1 功能验收

| 编号 | 验收项 | 通过条件 |
|------|--------|----------|
| AC-01 | 创建任务 | 可创建包含多个文件夹对的任务并持久化 |
| AC-02 | 排除规则 | Glob 规则正确过滤文件，`*.tmp` 不被复制 |
| AC-03 | 全量同步 | 首次同步后目标目录内容与源完全一致 |
| AC-04 | 增量同步 | 二次同步时只复制变更文件，跳过未变更文件 |
| AC-05 | USN Journal | 同一 session 内第二次同步，哈希模式下未变更文件跳过哈希计算 |
| AC-06 | USN 重启清零 | 程序重启后首次同步执行完整扫描，不使用旧检查点 |
| AC-07 | Delta Sync | 2GB 数据库文件追加 10MB 后，实际传输量 < 15MB |
| AC-08 | Delta Sync 降级 | 文件差异 > 80% 时自动降级为整体复制 |
| AC-09 | 并发 | 4 线程并发时速度明显高于单线程（SSD to SSD 场景） |
| AC-10 | 镜像模式 | Mirror 模式同步后目标端不存在源端已删除的文件 |
| AC-11 | 单向模式 | OneWay 模式同步后目标端保留源端没有的文件 |
| AC-12 | 定时同步 | 设置 1 分钟间隔后，程序内 1 分钟后自动触发同步 |
| AC-13 | 运行全部 | "运行全部"按顺序依次执行所有任务 |
| AC-14 | 错误处理 | 单文件权限错误不中断整体同步，记录到日志 |
| AC-15 | 磁盘满 | 目标盘满时停止同步并显示错误弹窗 |
| AC-16 | 原子复制 | 复制中断后目标端无残留的不完整文件 |
| AC-17 | 配置安全 | 配置文件写入异常后原文件不损坏 |
| AC-18 | 配置导入导出 | 导出后重新导入，任务配置完整还原 |
| AC-19 | i18n | 中文系统显示中文界面；英文系统显示英文界面 |
| AC-20 | 预览 | 预览结果与实际同步操作的文件列表一致 |
| AC-21 | 安全模式 | 在安全模式下应用可正常启动并使用（需 Mesa DLL） |
| AC-22 | 渲染回退 | GPU 驱动不可用时自动回退到可用渲染后端 |
| AC-23 | 应用日志 | 错误信息记录到 `%LOCALAPPDATA%\FileSync\app_YYYY-MM-DD.log` |

### 15.2 性能基准测试环境

- CPU：Intel i7 或同等 AMD 处理器
- 存储：NVMe SSD（源和目标同卷或不同卷各一份测试）
- 测试数据集：10 万个文件，总大小约 50GB，文件大小分布模拟真实工作目录

---

*文档结束*
