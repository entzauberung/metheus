# Metheus — 项目宪法

> 弥 · 工作流指挥中心
>
> Metheus 带来你的灵感，输出你的创意！

**一句话描述**：Metheus 是一个基于 AI 多人角色协作的桌面应用，通过策略产品经理、产品经理、域负责人、开发工程师、测试工程师五个 AI 角色分层对话，将用户的产品想法自动拆解为可执行的代码任务，并通过 Claude Code CLI 在本地 Git 仓库中自动执行和质检。

---

## 1. 技术栈

| 层 | 技术 | 版本 |
|---|------|------|
| 桌面壳 | Tauri 2.x | `^2` |
| 后端 | Rust (via Tauri) | edition 2021 |
| 前端 | React + TypeScript | React 19, TS 5.8 |
| 构建 | Vite 7 | `^7` |
| AI 模型 | DeepSeek API (v4-flash / v4-pro) | — |
| CLI 执行 | Claude Code (`claude` CLI) | 环境依赖 |
| 版本控制 | Git (程序化操作) | 环境依赖 |

**关键 Rust 依赖**：`reqwest`（HTTP 客户端）、`serde`/`serde_json`（序列化）、`tokio`（异步运行时）、`uuid`（ID 生成）、`chrono`（时间戳）、`walkdir`（文件遍历）、`dirs`（系统目录）

**关键前端依赖**：`@tauri-apps/api`（IPC 桥接）、React 19、无路由库（单页应用，通过 `ViewMode` 状态切换视图）

---

## 2. 核心目录结构

```
metheus/
├── CONSTITUTION.md          ← 本文件（项目架构宪法）
├── src/                     ← 前端（React + TypeScript）
│   ├── main.tsx             ← 入口，挂载 App
│   ├── App.tsx              ← 根组件：状态管理 + 视图切换 + 所有回调
│   ├── App.css              ← 全局样式
│   ├── types.ts             ← 前后端共享类型定义（与 project.rs 对齐）
│   ├── ChatRoom.tsx         ← 讨论模式：聊天界面 + 模式选择
│   ├── ExecutionTree.tsx    ← 侧边栏：执行树（里程碑 / 中阶段 / 子任务）
│   ├── TaskConsole.tsx      ← 执行模式：流水线控制台 + diff 查看 + QA 弹窗
│   ├── FileTree.tsx         ← 执行模式：项目文件树
│   ├── FloatingChatBalloon.tsx ← 执行模式：悬浮聊天球（快速讨论）
│   ├── components/
│   │   └── Modal.tsx        ← 通用弹窗组件
│   ├── utils/
│   │   └── invokeWithTimeout.ts ← IPC 调用超时包装器
│   └── assets/
├── src-tauri/               ← 后端（Rust + Tauri）
│   ├── Cargo.toml           ← Rust 依赖
│   ├── tauri.conf.json      ← Tauri 窗口/构建配置
│   ├── src/
│   │   ├── main.rs          ← Tauri 入口
│   │   ├── lib.rs           ← 核心逻辑（~4400 行，30+ 命令）
│   │   └── project.rs       ← 数据结构定义（~370 行）
│   └── capabilities/
│       └── default.json     ← Tauri 权限声明
├── dist/                    ← 前端构建产物
├── package.json             ← Node 依赖和脚本
├── tsconfig.json            ← TypeScript 配置
├── vite.config.ts           ← Vite 构建配置
└── PROGRESS.md              ← 月度进度跟踪
```

---

## 3. 架构概述

### 3.1 整体数据流

```
用户输入（ChatRoom / TaskConsole）
    │
    ▼
React 组件调用 invokeWithTimeout("command", args)
    │
    ▼  (Tauri IPC, JSON 序列化)
Rust #[tauri::command] 函数
    │
    ├── 调用 DeepSeek API（HTTP, reqwest）
    ├── 调用 Claude Code CLI（子进程, tokio::process::Command）
    ├── 操作 Git 仓库（子进程, std::process::Command）
    └── 读写 ~/.metheus/<project>.json（文件系统）
    │
    ▼  (Tauri IPC, JSON 序列化)
React 组件更新 state → 重新渲染
```

### 3.2 前端架构

- **单页应用**：无路由库。通过 `ViewMode` 状态 (`{ phase: 'discussion' | 'execution', reason? }`) 切换讨论模式和执行模式，附带 CSS 动画过渡。
- **状态提升**：所有共享状态（`project`、`viewMode`、`isExecuting`、`executionStatus`、`generatedPlan` 等）集中在 `App.tsx`，通过 props 向下传递。
- **IPC 通信**：前端通过 `invokeWithTimeout()` (对 `@tauri-apps/api/core` 的 `invoke` 的包装) 调用后端命令，所有调用均有超时保护。
- **轮询机制**：执行模式下每 2 秒轮询 `get_execution_status`，根据返回的 `PipelineState` 自动更新 UI 和切换阶段。

### 3.3 后端架构

- **单体 lib.rs**：所有业务逻辑集中在一个文件中，按功能区分为：AI 角色通信、方案生成、阶段拆解、执行引擎、测试检查、Git 操作、宪法管理。
- **数据结构分离**：`project.rs` 定义所有序列化结构体（`Project`、`Milestone`、`MidStage`、`Subtask`、`TestResult`、`QAResult` 等），与前端 `types.ts` 一一对应。
- **全局状态**：通过 `Arc<Mutex<Option<PipelineState>>>` 管理执行流水线的运行状态（当前子任务索引、测试结果、日志等）。
- **持久化**：项目数据存储在 `~/.metheus/<项目名>.json`，每次状态变更通过 `persist_project` 命令序列化保存。
- **AI 调用超时**：所有 DeepSeek API 请求统一使用 120 秒 HTTP 超时；Claude Code 子进程统一使用 600 秒超时。

---

## 4. 后端命令清单（30 个 Tauri Command）

### 4.1 讨论与方案生成

| 命令 | 功能 |
|------|------|
| `send_message` | 向 DeepSeek 发送单条消息 |
| `chat_with_role` | 指定 AI 角色（策略/产品/技术/测试）对话 |
| `generate_version_plan` | 根据讨论记录生成版本方案摘要 |
| `approve_version_plan` | 批准版本方案，进入规划阶段 |

### 4.2 阶段拆解

| 命令 | 功能 |
|------|------|
| `generate_milestones` | 产品经理将版本方案拆解为 3-5 个大阶段（含需求质检） |
| `regenerate_milestones_with_feedback` | 根据质检驳回反馈重新拆解大阶段 |
| `generate_mid_stages` | 域负责人将大阶段拆解为技术实现模块（中阶段） |

### 4.3 执行引擎

| 命令 | 功能 |
|------|------|
| `execute_subtask` | 启动 Claude Code CLI 执行单个子任务 |
| `check_subtask` | 测试工程师检查代码质量（含自动化测试运行） |
| `generate_next_prompt` | 开发工程师根据上一步结果生成下一子任务的提示词 |
| `start_execution` | 启动后台流水线，逐个执行子任务 |
| `get_execution_status` | 查询流水线当前状态（轮询用） |
| `pause_execution` | 暂停流水线 |
| `resume_execution` | 恢复流水线 |
| `stop_execution` | 停止流水线 |
| `approve_mid_stage` | 批准中阶段，推进到下一个 |
| `reject_mid_stage` | 驳回中阶段 |

### 4.4 Git 版本控制

| 命令 | 功能 |
|------|------|
| `git_save_node` | 提交当前变更并打 tag（节点粒度） |
| `git_save_subtask` | 提交变更并打 auto tag（子任务粒度） |
| `git_rollback_to_mid_stage` | 回退到指定中阶段的 Git tag |
| `git_rollback_to_subtask` | 回退到指定子任务的 Git tag |
| `get_git_tags_summary` | 获取 Git tag 列表摘要 |
| `get_current_diff` | 获取当前工作区 diff（文件变更列表） |

### 4.5 项目与宪法管理

| 命令 | 功能 |
|------|------|
| `get_project` | 从文件加载项目数据 |
| `persist_project` | 保存项目数据到文件 |
| `validate_project_path` | 校验项目目录有效性 |
| `get_project_files` | 获取项目文件树 |
| `read_constitution` | 读取 CONSTITUTION.md（运行时宪法） |
| `get_constitution_summary` | 获取宪法摘要（结构简述 + 变更历史） |
| `update_constitution` | 更新运行时宪法（执行后自动调用） |
| `compact_constitution` | 压缩宪法内容（移除冗余条目） |

---

## 5. 前端组件树

```
App
├── 侧边栏（aside.sidebar）
│   ├── ExecutionTree          ← 里程碑 / 中阶段 / 子任务树
│   │   ├── Modal（回退确认弹窗）
│   │   └── QA 结果展示
│   └── .resize-handle         ← 侧边栏拖拽缩放
│
├── 主内容区（main.main-content）
│   ├── 项目目录设置栏
│   ├── 测试面板（开发者工具）
│   │
│   ├── [讨论模式 phase='discussion']
│   │   ├── 生成版本方案按钮
│   │   ├── ChatRoom           ← 聊天 + 模式切换
│   │   ├── 版本方案面板（批准/驳回）
│   │   └── 拆解大阶段按钮
│   │
│   ├── [执行模式 phase='execution']
│   │   ├── FileTree           ← 项目文件浏览器
│   │   └── TaskConsole        ← 执行控制台
│   │       ├── 生成计划 / 启动执行 / 暂停 / 恢复 / 停止
│   │       ├── diff 查看
│   │       ├── QA 弹窗
│   │       └── 测试日志
│   │
│   └── FloatingChatBalloon    ← 执行模式下的悬浮聊天入口
```

---

## 6. 项目工作流（五阶段流水线）

```
[讨论] → [版本方案] → [大阶段拆解] → [中阶段拆解] → [小阶段执行]
  Idle       ↓              ↓               ↓               ↓
Discussing  Planning    MilestoneReady   (专业模式)      Executing
                         (快速模式跳过中阶段)
```

### 6.1 快速模式 vs 专业模式

- **快速模式**：大阶段 → 小阶段（两级），适合原型验证
- **专业模式**：大阶段 → 中阶段 → 小阶段（三级），适合正式开发

### 6.2 子任务执行流水线

```
for each subtask:
    1. 生成执行计划 (generate_next_prompt × N)
    2. Claude Code CLI 执行 (execute_subtask)
    3. 测试工程师检查 (check_subtask)
       ├── 通过 → 下一个子任务
       └── 驳回 → 重试 (最多 3 次)
    4. Git 提交 + tag (git_save_subtask)
    5. 更新 CONSTITUTION.md
```

---

## 7. 五角色 System Prompt

| 角色 | 常量名 | 职责 |
|------|--------|------|
| 策略产品经理 | `STRATEGY_PROMPT` | 与用户讨论产品想法，输出版本方案摘要 |
| 产品经理 | `PM_PROMPT` | 将版本方案拆为可执行的大阶段 |
| 域负责人 | `DOMAIN_LEAD_PROMPT` | 将大阶段拆为技术实现模块（中阶段） |
| 开发工程师 | `TECH_PROMPT` | 生成精确的子任务执行提示词 |
| 测试工程师 | `TEST_PROMPT` | 检查代码质量和功能正确性 |

**辅助 Prompt**：
- `SELF_CHECK_PROMPT` — 版本方案自检
- `QA_CHECK_PROMPT` — 大阶段需求质检
- `CONSTITUTION_PART1_PROMPT` — 生成运行时宪法第 1 部分
- `CONSTITUTION_UPDATE_PROMPT` — 滚动更新宪法
- `COMPACT_CONSTITUTION_PROMPT` — 压缩宪法

---

## 8. 共享类型约定

### 8.1 前后端类型对应

| Rust (`project.rs`) | TypeScript (`types.ts`) | 说明 |
|---------------------|------------------------|------|
| `Project` | `Project` | 项目根结构 |
| `Milestone` | `Milestone` | 大阶段 |
| `MidStage` | `MidStage` | 中阶段 |
| `Subtask` | `Subtask` | 小阶段（执行单元） |
| `ExecutionResult` | `ExecutionResult` | Claude Code 执行输出 |
| `TestResult` | `TestResult` | 测试工程师检查结果 |
| `QAResult` | `QAResult` | 需求质检结果 |
| `PipelineState` | `PipelineState` | 流水线状态 |
| `SubTaskError` | `SubTaskError` | 执行错误枚举 |
| `DiffSummary` | `DiffSummary` | Git diff 摘要 |
| `GitTagInfo` | `GitTagInfo` | Git tag 信息 |
| `FileEntry` | `FileEntry` | 文件树条目 |
| `QADetail` | `QADetail` | QA 偏差明细 |
| `GeneratedSubtask` | `GeneratedSubtask` | AI 动态生成的子任务 |
| `PathValidationResult` | `PathValidationResult` | 路径校验结果 |
| `ConstitutionSummary` | `ConstitutionSummary` | 宪法摘要 |

### 8.2 命名规范

- **Tauri 命令**：`snake_case`（后端 Rust 函数名，前端 `invoke` 字符串参数与之对应）
- **TypeScript 类型**：`PascalCase` 接口，`camelCase` 字段（Rust 端 `#[serde(rename_all = "camelCase")]` 自动转换）
- **Rust 结构体字段**：`snake_case`，通过 serde 自动转为前端 `camelCase`
- **组件文件**：`PascalCase.tsx`
- **CSS 类名**：`kebab-case`（BEM 风格）
- **Git tag**：`metheus/<版本号>`（中阶段）、`metheus/auto/<版本号>/task-<序号>`（子任务自动 tag）

### 8.3 数据流约定

- **前后端通信**：仅通过 Tauri IPC（`invoke`/`invokeWithTimeout`），不直接操作文件系统或 HTTP
- **状态管理**：React `useState` + props 传递，无 Redux/Context
- **持久化**：仅在用户显式触发操作时保存（按钮点击、执行完成），不自动保存中间状态
- **错误传递**：Rust `Result<T, String>` → Tauri IPC → 前端 `try/catch`；字符串错误信息直接展示给用户

---

## 9. 关键配置与约束

### 9.1 超时配置

| 项目 | 值 |
|------|----|
| DeepSeek API HTTP 请求 | 120 秒 |
| Claude Code 子进程 | 600 秒 |
| 前端轮询间隔 | 2 秒 |
| 前端 IPC 默认超时 | 30 秒 |
| 聊天 IPC 超时 | 60 秒 |
| 方案生成 IPC 超时 | 180 秒 |

### 9.2 安全白名单

- **AI 模型白名单**（`execute_subtask_inner`）：仅允许 `deepseek-v4-pro` 和 `deepseek-v4-flash`，其余降级为 flash
- **前端 IPC 调用**：全部通过 `invokeWithTimeout`，无裸 `invoke`

### 9.3 执行约束

- 子任务提示词禁止包含完整代码块（仅描述功能目标 + 文件路径 + 函数签名）
- 最大自动应答次数：20 次
- AI 返回空数组时自动判定子任务检查通过（含 warning 诊断）
- QA 解析失败时默认判定为**不通过**（需人工审查）

---

## 10. 当前开发状态

### 已完成
- [x] 五人角色分层对话（策略 / 产品 / 域负责人 / 开发 / 测试）
- [x] 版本方案生成与自检
- [x] 大阶段拆解 + 需求质检
- [x] 中阶段拆解（专业模式）
- [x] Claude Code CLI 子任务执行
- [x] 测试工程师检查 + 自动化测试命令检测
- [x] 执行流水线（启动 / 暂停 / 恢复 / 停止）
- [x] 快速模式 + 专业模式
- [x] Git 存档与回退（节点粒度 + 子任务粒度）
- [x] 运行时宪法滚动更新
- [x] 前端 IPC 超时保护
- [x] 后端 HTTP + 子进程超时保护
- [x] JSON 解析三次重试 + 兜底
- [x] QA 解析失败的 warning 诊断传递

### 进行中 / 待完善
- [ ] 全流程集成测试
- [ ] 错误恢复与断点续传
- [ ] 多项目管理
- [ ] 国际化
- [ ] 打包分发

---

## 11. 维护说明

- 本文件是项目级别的架构参考，**不包含任何具体代码实现**。
- 运行时应用会在用户项目目录生成另一份 `CONSTITUTION.md`（包含第 1 部分常量规则和第 2 部分滚动状态），与本文件不同。
- 每次重要架构变更、新增命令、修改数据模型时，应同步更新本文件的相关章节。
- 最后更新：2026-06-28
