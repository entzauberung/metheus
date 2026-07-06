# Metheus — 项目宪法

> 弥 · 工作流指挥中心 — 用 AI 多人角色协作把产品想法自动拆成代码、执行、质检的桌面应用。

---

## 1. 项目定位

- **解决什么问题**：非技术用户有产品想法但不知道如何落地为代码。Metheus 用 5 个 AI 角色（策略 PM → 产品经理 → 域负责人 → 开发工程师 → 测试工程师）分层对话，自动生成版本方案、拆解任务、通过 Claude Code CLI 在本地 Git 仓库中执行和质检。
- **不解决什么问题**：不是在线托管服务、不是 CI/CD 替代品、不做多人协作/云同步。
- **目标用户**：独立开发者、产品经理、技术爱好者，有自己的产品想法但缺乏将想法结构化落地为代码的管道。

---

## 2. 技术选型与理由

| 选型 | 理由 |
|------|------|
| **Tauri 2.x** | 桌面壳，Rust 后端 + Web 前端，包体小，跨平台 |
| **Rust (Edition 2021)** | 所有业务逻辑、文件 I/O、进程管理、AI API 调用均在 Rust 侧 |
| **React 19 + TypeScript** | 前端 UI，无路由库（单页应用，`ViewMode` 状态切换视图） |
| **Vite 7** | 构建工具 |
| **DeepSeek API (v4-flash / v4-pro)** | 所有 AI 角色对话、任务拆解、质检的 LLM 后端 |
| **Claude Code CLI (`claude`)** | 本地子进程，执行 Rust 后端生成的精确提示词 |
| **Git（程序化操作）** | 版本控制：每次中阶段/小阶段完成后 `git add` + `commit` + `tag -f` |
| **reqwest** | Rust HTTP 客户端，调用 DeepSeek API |
| **serde/serde_json** | 所有数据结构的序列化/反序列化 |
| **tokio** | Rust 异步运行时（Tauri 命令均为 async，子进程使用 `tokio::process::Command`） |
| **uuid** | 所有实体（Milestone/MidStage/Subtask/Message）的 ID 生成 |
| **chrono** | 时间戳生成 |
| **walkdir** | 项目文件树遍历 |
| **dirs** | 获取系统 home 目录 |
| **dotenvy** | 从 `.env` 加载 API_KEY 等环境变量 |

### 关键架构原则

- **前端不直接调用任何 AI API**，所有 AI 调用必须经过 Rust 后端
- **不使用前端 UI 组件库**（Tailwind、Ant Design 等），所有样式手写 CSS
- **不使用复杂状态管理库**（Redux、Zustand 等），只用 React 自带的 `useState` / `useEffect`
- **不在 MVP 阶段引入 WebSocket**，前端通过 Tauri IPC `invoke()` 调用后端
- **`project.rs` 只定义数据结构**，业务逻辑分散在各功能模块中
- **Rust 端 `project.rs` 与前端 `types.ts` 的数据结构必须保持一一对应**

---

## 3. 顶层架构图

```
┌─────────────────────────────────────────────────────────┐
│                    前端 (React + TypeScript)              │
│  App.tsx → [ChatRoom | ExecutionTree | TaskConsole |      │
│             FileTree | FloatingChatBalloon]               │
│  所有 AI 调用 → Tauri IPC invoke("command_name", args)    │
│  工具函数: utils/invokeWithTimeout.ts (统一超时包装)       │
└──────────────────────┬──────────────────────────────────┘
                       │  IPC (Tauri Bridge)
┌──────────────────────▼──────────────────────────────────┐
│                 Rust 后端 (lib.rs = 入口, 152 行)         │
│                                                          │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────┐ │
│  │commands/ │ │git_ops   │ │constitution│ │pipeline     │ │
│  │ chat     │ │ 9 fn     │ │ 7 fn +    │ │ 5 cmd +     │ │
│  │ plan     │ │          │ │ Validation│ │ PipelineState│ │
│  │ milestone│ │          │ │ Result    │ │ + 2 core fn │ │
│  │ proj_ops │ │          │ │          │ │ (含快速模式)  │ │
│  └──────────┘ └──────────┘ └──────────┘ └─────────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────┐ │
│  │executor  │ │test_runner│ │diff      │ │  api.rs     │ │
│  │2 fn      │ │ 7 fn     │ │ 2 fn     │ │ 3 fn        │ │
│  └──────────┘ └──────────┘ └──────────┘ └─────────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌──────────────┐ ┌─────────┐ │
│  │prompts   │ │constants │ │json_utils    │ │snapshot │ │
│  │10 const  │ │6 const   │ │ 2 fn         │ │ 2 cmd + │ │
│  │          │ │          │ │              │ │4辅助fn  │ │
│  └──────────┘ └──────────┘ └──────────────┘ └─────────┘ │
│                                                          │
│  project.rs — 所有数据结构 (struct/enum, 374 行)          │
│  lib.rs — AppState + run() + generate_handler![34 cmd]  │
└──────────────────────┬──────────────────────────────────┘
                       │
          ┌────────────┼────────────┐
          ▼            ▼            ▼
    DeepSeek API   Claude Code    Git (本地)
    (HTTP/json)    (子进程 CLI)   (Command 调用)
```

### 数据流方向（单向，不可逆）

专业模式：
```
用户输入 → 策略PM(对话) → 版本方案 → 产品经理(拆大阶段) → Milestones
  → 域负责人(拆中阶段) → MidStages → 开发工程师(拆小阶段) → Subtasks
  → Claude Code CLI(执行) → 测试工程师(质检) → 通过/重试(最多3次)
  → Git存档 → 宪法更新 → 下一子任务 → 下一中阶段
```

快速模式：
```
用户输入 → 策略PM(对话) → 版本方案 → 产品经理(拆大阶段,Quick)
  → 开发工程师(直接拆小阶段,跳过中阶段) → Subtasks
  → Claude Code CLI(执行) → 测试工程师(质检) → 通过/重试
  → Git存档(metheus/q/前缀) → 下一子任务
```

### 模块依赖方向（箭头 = 被调用）

```
commands/ ──→ api, prompts, json_utils, constants, lib(基础fn)
pipeline ──→ executor, commands/milestone, test_runner, git_ops,
              constitution, diff, constants, lib(基础fn)
executor ──→ pipeline(类型), test_runner, constants
git_ops ──→ project
constitution ──→ api, prompts, constants, project
test_runner ──→ api, prompts, json_utils, project
snapshot ──→ project, AppState
```

---

## 4. 关键路径

### 场景 A：用户有一个产品想法 → 生成执行计划
```
用户输入想法 → send_message(DeepSeek API, 策略PM)
  → 多轮对话 → generate_version_plan(DeepSeek API + 自检 + 宪法第1部分写入)
  → approve_version_plan(写入 CONSTITUTION.md 第1部分)
  → generate_milestones(DeepSeek API, 产品经理 + QA质检)
  → (专业模式) generate_mid_stages(DeepSeek API, 域负责人 + attention_points)
  → (快速模式) generate_next_prompt 循环(DeepSeek API, 开发工程师, 3轮)
→ 得到可执行的 Subtask 列表
```

### 场景 B：用户点击执行 → 流水线自动跑完
```
start_execution(后台 spawn)
  → 校验项目路径有效性，非git仓库则自动 git init
  → 区分专业/快速模式
    
  专业模式 execute_mid_stage_pipeline(循环每个 subtask):
    → generate_next_prompt(生成提示词,含技术栈约束)
    → execute_subtask_inner(启动 Claude Code CLI 子进程)
      → tokio::process::Command 自动应答 yes(20次上限)
      → 子进程模型白名单校验(METHEUS_MODEL, deepseek-v4-*)
      → 轮询等待(检查暂停标志 + CLAUDE_CODE_TIMEOUT_SECS 超时)
      → 返回 ExecutionResult(改动文件列表)
    → check_subtask(DeepSeek API, 测试工程师 + 自动真测试)
      → JS/TS: npm/pnpm/yarn test → Rust: cargo test → 自定义:.metheus-test
    → 通过 → git_save_subtask(add + commit + tag metheus/auto/vX.Y.Z/task-N)
    → 宪法更新链:
        git diff HEAD → extract_diff_summary → update_constitution(AI)
        → 校验(validate_constitution_update) → 重试 → 机械更新兜底
        → token估算超阈值 → compact_constitution(剪枝)
    → 未通过 → retry_count < 3 → 重试 → 超过3次 → 报错终止
  → 全部完成 → 写回 project.json → git_save_node(中阶段存档)
  
  快速模式 execute_quick_pipeline(流程同上):
    → git tag 使用 metheus/q/ 前缀，回写 milestone.subtasks
```

### 场景 C：用户想回退到之前的版本
```
前端选择 tag → git_rollback_to_mid_stage / git_rollback_to_subtask
  → stash 当前未提交变更
  → git reset --hard <tag>
  → 清理被跳过节点的 Git tag(版本号大于目标的tag被删除)
  → 更新 project.json 状态(RolledBack)
  → 清理后续节点的 git tag
```

---

## 5. 模块清单

### `src-tauri/src/lib.rs`（152 行）— 应用入口
| 项目 | 内容 |
|------|------|
| **职责** | 模块声明、基础 I/O 函数、AppState 定义、run() 入口、注册全部 34 个 Tauri command |
| **依赖** | 所有 13 个子模块（含 snapshot） |
| **对外接口** | `check_project_path()` (pub(crate)), `save_project()` (pub(crate)), `load_project()` (pub(crate)), `project_data_path()` (pub(crate)), `AppState` (pub), `run()` (pub) |
| **持久化** | `~/.metheus/{project_name}.json` — 单个 JSON 文件存储完整 Project 结构 |
| **新增** | `project_data_path()` — 统一路径生成函数(2026-07-05添加) |

### `src-tauri/src/project.rs`（374 行）— 数据模型
| 项目 | 内容 |
|------|------|
| **职责** | 所有数据结构定义（enum/struct），零业务逻辑 |
| **关键类型** | `Project`, `Milestone`, `MidStage`(含order字段), `Subtask`, `Message`, `ExecutionResult`, `TestResult`, `DiffSummary`, `QAResult`, `GitTagInfo`, `FileEntry`, `PathValidationResult`, `SubTaskError`, `ConstitutionSummary`, `DiscussionThread` |
| **同步要求** | 每个 struct/enum 必须与 `src/types.ts` 一一对应 |

### `src-tauri/src/prompts.rs`（130 行）— AI 角色提示词
| 项目 | 内容 |
|------|------|
| **职责** | 10 个 `pub(crate) const` system prompt 字符串常量 |
| **常量** | `STRATEGY_PROMPT`, `PM_PROMPT`, `DOMAIN_LEAD_PROMPT`, `TECH_PROMPT`, `TEST_PROMPT`, `SELF_CHECK_PROMPT`, `QA_CHECK_PROMPT`, `CONSTITUTION_PART1_PROMPT`, `CONSTITUTION_UPDATE_PROMPT`, `COMPACT_CONSTITUTION_PROMPT` |

### `src-tauri/src/constants.rs`（14 行）— 配置常量
| 项目 | 内容 |
|------|------|
| **常量** | `SANITIZE_FALLBACK_JSON`="{}", `DEEPSEEK_API_TIMEOUT_SECS`=120, `CLAUDE_CODE_TIMEOUT_SECS`=600, `GIT_INIT_FAILED`, `GIT_AUTO_INIT_COMMIT_MSG`, `COMPACTION_TRIGGER_TOKENS`=3000 |

### `src-tauri/src/api.rs`（83 行）— DeepSeek API 封装
| 函数 | 说明 |
|------|------|
| `call_deepseek_api` | 纯文本对话（temperature=0.1） |
| `call_deepseek_api_json` | 强制 JSON 输出（temperature=0.5，设置 response_format="json_object"） |
| `call_deepseek_api_inner` | 内部实现：构造 HTTP 请求 → 发送 → 解析 → 返回 content 字段，支持自定义 temperature |

### `src-tauri/src/json_utils.rs`（136 行）— JSON 清洗
| 函数 | 说明 |
|------|------|
| `sanitize_json_response` | 从 AI 返回中提取纯净 JSON（处理 ```json 包裹、礼貌前缀、多余文字、括号计数器精准截断） |
| `parse_json_with_retry` | 带 3 次重试的 JSON 解析（第1次直接解析，第2/3次 AI 自助修正），3 次全败返回 Err(不再返回 T::default()) |

### `src-tauri/src/git_ops.rs`（568 行）— Git 操作
| 函数 | 类型 | 说明 |
|------|------|------|
| `git_save_node` | cmd | 中阶段 Git 存档（add → commit → tag metheus/vX.Y.Z） |
| `git_save_subtask` | cmd | 小阶段 Git 存档（tag metheus/auto/vX.Y.Z/task-N） |
| `git_save_subtask_inner` | pub(crate) | 小阶段存档内部实现，支持快速/专业模式两种 tag 前缀 |
| `git_rollback_to_mid_stage` | cmd | 回退到中阶段 tag（stash → reset --hard → 清理后续tag → 更新 project.json） |
| `git_rollback_to_subtask` | cmd | 回退到小阶段 auto_tag（粒度更细，只回退到特定小阶段） |
| `get_git_tags_summary` | cmd | 列出所有 metheus/* tag（按创建日期倒序） |
| `get_current_diff` | cmd | 获取当前 git diff（非 git 仓库/无提交时返回空，兼容 worktree） |
| `compare_version_strings` | pub(crate) | 比较 "v0.1.1" vs "v0.1.3" 返回 -1/0/1 |
| `save_tag_to_mid_stage` | pub(crate) | 将 tag 名写入 project.json 的 mid_stage.git_tag |

### `src-tauri/src/constitution.rs`（713 行）— 宪法管理
| 项目 | 类型 | 说明 |
|------|------|------|
| `ValidationResult` | pub enum | Passed / Part1Modified / StructureDamaged / Empty |
| `validate_constitution_update` | pub(crate) | 校验 AI 结果：检查第 1 部分是否被修改（长度比对+字符级对比） |
| `mechanical_update_constitution` | pub(crate) | 机械兜底更新（不调 AI，追加 [机械更新] 标记，处理增/删/改文件） |
| `update_constitution` | cmd | AI 更新第 2 部分（流程：检查→AI→校验→重试→兜底，空变更跳过） |
| `estimate_tokens` | pub(crate) | 文本 token 估算（中文=1.0，ASCII=0.25，CJK符号=1.0） |
| `compact_constitution` | cmd | 第 2 部分压缩/剪枝（阈值触发，失败保留膨胀版本，重试1次） |
| `read_constitution` | cmd | 读取项目目录下 CONSTITUTION.md（不存在返回友好提示） |
| `get_constitution_summary` | cmd | 提取第 2 部分快照（项目结构描述+函数数+最近5条变更+token估算） |

### `src-tauri/src/diff.rs`（384 行）— Diff 解析
| 函数 | 说明 |
|------|------|
| `extract_diff_summary` | 解析 `git diff` stdout → `DiffSummary`（新/改/删文件 + 函数 + 依赖），跳过 node_modules/target/__pycache__/.git/.lock |
| `extract_function_signature` | 从一行代码提取函数签名（支持 Rust/TS/JS/Python/Go/C++/Java） |

### `src-tauri/src/test_runner.rs`（528 行）— 测试执行引擎
| 函数 | 类型 | 说明 |
|------|------|------|
| `check_subtask` | cmd | 测试主入口：git diff 获取改动文件 → 降级文件系统扫描 → 读取内容 → 真测试 → DeepSeek API（测试工程师），返回 TestResult |
| `run_test_command` | pub(crate) | 执行测试命令 + 超时控制（spawn + try_wait 轮询 500ms） |
| `summarize_test_output` | pub(crate) | 从测试输出提取关键摘要（通过取末500字符，失败搜索关键词附近±500字符） |
| `format_test_result` | pub(crate) | 格式化测试结果为字符串 |
| `is_test_not_configured` | pub(crate) | 判断测试是否未配置（missing script/no tests found） |
| `get_tracked_files` | pub(crate) | walkdir 扫描项目目录获取文件列表（跳过 .git/node_modules/target） |
| `detect_changes` | pub(crate) | 对比执行前后文件列表，返回新增文件（相对路径） |
| **真测试系统** | — | 自动检测项目类型：.metheus-test 自定义 > npm/pnpm/yarn > cargo test > go test > pytest/unittest > ctest > mvn test > gradle test |

### `src-tauri/src/pipeline.rs`（1080 行）— 执行流水线
| 项目 | 类型 | 说明 |
|------|------|------|
| `PipelineStatus` | pub enum | Idle/Running/Paused/Completed/Failed |
| `SubtaskStatusItem` | pub struct | 单个子任务的执行状态记录 |
| `PipelineState` | pub struct | 完整流水线状态（含 child_pid: Option<u32> 用于快速终止） |
| `start_execution` | cmd | 启动后台流水线：路径校验→自动git init→解析subtasks→区分专业/快速模式→spawn tokio task |
| `get_execution_status` | cmd | 查询 PipelineState |
| `pause_execution` | cmd | 设置 Paused 标志 |
| `resume_execution` | cmd | 恢复 Running |
| `stop_execution` | cmd | 标记 Failed + 通过 PID 立即 SIGKILL 子进程 |
| `execute_mid_stage_pipeline` | pub(crate) | **专业模式核心编排函数**：循环 subtask → 生成提示词 → 执行 → 测试 → 宪法更新链 → Git存档 → 全部完成→写回project.json→git_save_node |
| `execute_quick_pipeline` | pub(crate) | **快速模式核心编排函数**：跳过中阶段，直接操作 milestone.subtasks，tag使用 metheus/q/ 前缀 |

### `src-tauri/src/executor.rs`（195 行）— 子进程执行器
| 函数 | 类型 | 说明 |
|------|------|------|
| `execute_subtask_inner` | pub(crate) | **关键函数**：tokio::process::Command 启动 claude CLI → 自动应答 yes（20次上限）→ 模型名白名单校验 → 轮询(暂停检查+超时600s) → 文件变更检测 → 返回 ExecutionResult |
| `execute_subtask` | cmd | Tauri 壳包装（前端直接调时传临时空 state） |
| **自动应答机制** | — | 启动后先写 1\n（信任确认），300ms 后写 20 个 yes\n（文件写入确认） |

### `src-tauri/src/snapshot.rs`（345 行）— 快照与孤儿进程保护（2026-07-05 新增）
| 函数 | 类型 | 说明 |
|------|------|------|
| `UISnapshot` | pub struct | 前端 UI 状态快照：view_phase/selected_milestone_id/selected_mid_stage_id/generated_plan_keys/quick_generated_plan_keys/saved_at |
| `AppSnapshot` | pub struct | 完整快照：ui/project_id/snapshot_version/running_pid/saved_at |
| `save_snapshot` | pub(crate) | 将 UI 状态 + running_pid 持久化到 ~/.metheus/{project_id}_snapshot.json |
| `load_snapshot` | pub(crate) | 从磁盘读取快照（文件损坏/版本不兼容时静默删除并返回 None） |
| `update_snapshot_pid` | pub(crate) | 仅更新快照中的 running_pid，保留 UI 部分不变 |
| `is_pid_alive` | pub(crate) | 检查指定 PID 是否存活（Unix: kill -0; Windows: tasklist） |
| `kill_pid` | 辅助 | 终止指定 PID 的进程（Unix: kill -9; Windows: taskkill /F） |
| `cleanup_orphan_processes_at_startup` | pub(crate) | **应用启动时调用**：扫描所有 `_snapshot.json` → 检测存活孤儿进程 → kill -9 → 清除快照 PID |
| `save_snapshot_event` | cmd | Tauri 命令：前端保存 UI 状态快照（fire-and-forget） |
| `restore_snapshot` | cmd | Tauri 命令：前端加载快照（首次启动/刷新时调用） |

### `src-tauri/src/commands/` — Tauri 命令模块
| 文件 | 命令 | 说明 |
|------|------|------|
| `chat.rs` | `greet`, `send_message`, `chat_with_role` | 对话：测试连接/自由对话/指定角色对话（返回结构化 Message） |
| `plan.rs` | `generate_version_plan`, `approve_version_plan` | 方案：生成版本方案+宪法第1部分(含自检SELF_CHECK)/批准方案 |
| `milestone.rs` | `generate_milestones`, `regenerate_milestones_with_feedback`, `generate_mid_stages`, `generate_next_prompt` | 拆解：大阶段(含QA质检)/带反馈重拆/中阶段(含attention_points)/小阶段提示词(含技术栈约束) |
| `project_ops.rs` | `get_project`, `persist_project`, `validate_project_path`, `get_project_files`, `approve_mid_stage`, `reject_mid_stage` | 项目操作：加载/保存/校验/文件树/审批中阶段/驳回中阶段(含自动推进下一阶段) |

### 前端文件清单
| 文件 | 职责 |
|------|------|
| `src/App.tsx`（1020 行） | **根组件**：所有核心状态、视图模式切换(discussion↔execution带动画)、侧边栏拖拽缩放、执行状态轮询、快照持久化、所有命令回调函数 |
| `src/ChatRoom.tsx`（159 行） | 聊天组件：发送消息、@角色切换（@策略/@产品/@技术/@测试/@域）、模式选择器 |
| `src/ExecutionTree.tsx`（740 行） | 执行树：三层嵌套（大阶段→中阶段→小阶段）、版本号编辑、QA质检标记、回退确认弹窗、宪法查看弹窗、快速模式操作区 |
| `src/TaskConsole.tsx`（688 行） | 执行控制台：执行控制按钮、进度条、4标签页（代码变更/执行日志/宪法更新/Git标签）、测试日志列表、QA驳回弹窗 |
| `src/FileTree.tsx`（278 行） | 文件树：平铺列表转树状结构、hover/pin 展开、文件类型图标 |
| `src/FloatingChatBalloon.tsx`（74 行） | 悬浮聊天球：执行模式下快捷查看讨论记录的只读浮窗 |
| `src/utils/invokeWithTimeout.ts`（88 行） | **统一超时包装**：所有 Tauri invoke 的 Promise.race 超时保护，命令→超时秒数映射表 |
| `src/components/Modal.tsx`（51 行） | 通用弹窗组件 |
| `src/main.tsx`（9 行） | React DOM 挂载入口 |

---

## 6. 数据模型摘要

以下为 `src-tauri/src/project.rs` 定义的核心类型（对应前端 `src/types.ts`）：

| 结构体 | 用途 |
|--------|------|
| `Project` | 根结构：name/status/mode/version_plan/milestones/discussion_threads/project_path |
| `Milestone` | 大阶段：version/title/description/tech_stack/status/mode/mid_stages/subtasks/qa_result |
| `MidStage` | 中阶段（专业模式）：version/title/description/tech_focus/status/subtasks/git_tag/order(排序字段!)/domain/test_log/created_at/completed_at/approved_at |
| `Subtask` | 最小执行单元：title/prompt/status/execution_result/test_result/retry_count/auto_tag/test_report |
| `Message` | 单条聊天：id/role/content/timestamp |
| `DiscussionThread` | 讨论线程：title/node_id/messages |
| `ExecutionResult` | Claude Code 执行输出：success/output/error_log/file_changes |
| `TestResult` | 测试工程师检查结果：passed/issues/suggestion/warnings(诊断信息) |
| `GeneratedSubtask` | 开发工程师动态生成的下一步：title/prompt |
| `QAResult` / `QADetail` | 需求质检结果：passed/reason/details/attention_points/warnings/checked_at |
| `DiffSummary` | Git diff 解析：new_files/modified_files/deleted_files/new_functions/... |
| `ConstitutionSummary` | 宪法快照：structure_description/function_count/recent_changes/total_tokens |
| `GitTagInfo` | Git tag 记录：name/date/subject |
| `FileEntry` | 文件树条目：path/is_dir/file_type |
| `PathValidationResult` | 路径校验：is_valid/exists/is_directory/is_git_repo/error_message |
| `SubTaskError` | 执行错误：UserPaused / ExecutionFailed{message} / Timeout |

以下为 `src-tauri/src/pipeline.rs` 定义：

| 结构体 | 用途 |
|--------|------|
| `PipelineStatus` | Idle/Running/Paused/Completed/Failed |
| `PipelineState` | 流水线全状态：mid_stage_id/status/current_subtask_index/total_subtasks/subtask_statuses/current_log/last_error/child_pid(新增!快速终止用) |
| `SubtaskStatusItem` | 单个子任务：subtask_id/title/status/test_result/retry_count |

以下为 `src-tauri/src/snapshot.rs` 定义：

| 结构体 | 用途 |
|--------|------|
| `UISnapshot` | UI状态快照：view_phase/selected_milestone_id/selected_mid_stage_id/generated_plan_keys/quick_generated_plan_keys |
| `AppSnapshot` | 完整快照：ui/project_id/snapshot_version/running_pid/saved_at |

以下为前端 `src/types.ts` 独有类型（纯前端内部）：

| 类型 | 用途 |
|------|------|
| `TestLog` | 测试日志条目：subtask_title/status('passed'|'rejected'|'retried')/reason/files/full_report |
| `ViewPhase` / `DiscussionReason` / `ViewMode` | 视图模式控制：phase('discussion'|'execution') + reason |
| `RollbackToSubtaskPayload` | 小阶段回退参数聚合 |

---

## 7. 外部依赖（非代码库）

| 依赖 | 用途 | 版本/要求 |
|------|------|----------|
| **`claude` CLI** | 在项目目录下以子进程方式执行 AI 生成的提示词，修改项目文件 | 需在 PATH 中，支持 `--dangerously-skip-permissions --model -p` 参数 |
| **`git`** | 版本控制：存档、回退、diff、tag 管理。项目目录必须是 Git 仓库。 | 需在 PATH 中，兼容 worktree |
| **DeepSeek API** | 所有 AI 角色对话的后端 LLM。通过 `https://api.deepseek.com/v1/chat/completions` 调用。 | API_KEY 在 `.env` 中。模型白名单：`deepseek-v4-pro` / `deepseek-v4-flash`（可配置 `METHEUS_MODEL` 环境变量覆盖，不在白名单则降级为默认值） |
| **Rust 工具链** | 编译后端 | `cargo` + `rustc` (Edition 2021) |
| **Node.js 20+** | 前端构建 | `npm` / `pnpm` / `yarn` |

---

## 8. 错误处理策略

| 场景 | 处理方式 |
|------|----------|
| **环境变量缺失**（API_KEY 等） | 返回 `Err("API_KEY 环境变量未设置")` 给前端 |
| **DeepSeek API 超时** | `DEEPSEEK_API_TIMEOUT_SECS=120s`，超时返回友好错误消息 |
| **DeepSeek API 返回非 JSON** | `sanitize_json_response()` 清洗 → `parse_json_with_retry()` 最多 3 次 AI 自助修正 |
| **3 次 JSON 解析全败** | 返回 `Err`（不再兜底 T::default()），同时 eprintln! 到后端日志 |
| **Claude Code 执行失败** | 写入 `error_log`，最多重试 3 次 |
| **Claude Code 子进程卡死** | `CLAUDE_CODE_TIMEOUT_SECS=600s`，超时强制 kill |
| **用户暂停流水线** | `PipelineStatus::Paused`，`execute_subtask_inner` 轮询检测到后 kill 子进程并返回 `SubTaskError::UserPaused`；恢复后从当前子任务继续（不增加 retry_count） |
| **用户停止流水线** | `stop_execution` 通过 `child_pid` 立即 SIGKILL 子进程 |
| **Git 命令失败** | 按场景处理：非 git 仓库返回空（幂等）、初始化失败返回明确错误、stash/reset 失败返回 stderr(reset 失败尝试恢复 stash) |
| **宪法更新 AI 连续失败** | 降级为 `mechanical_update_constitution`（机械追加 [机械更新] 标记） |
| **宪法压缩失败** | 保留膨胀版本（不降级为机械更新） |
| **前端调用时项目文件不存在** | `get_project` 返回默认空 Project（不报错） |
| **load_env 失败** | `dotenvy::dotenv().ok()` 静默忽略 |
| **孤儿进程残留** | 应用启动时 `cleanup_orphan_processes_at_startup()` 扫描快照文件，kill 存活孤儿进程 |
| **子进程模型配置错误** | 模型名不在白名单 `["deepseek-v4-pro", "deepseek-v4-flash"]` 时降级为 `deepseek-v4-flash` |
| **git_save_subtask 失败后的宪法回退** | 如果宪法已更新但 Git 存档失败，自动将宪法回退到更新前的内容 |
| **测试框架检测** | 自动匹配项目类型：自定义 .metheus-test > npm/pnpm/yarn > cargo > go > pytest > ctest > maven > gradle |
| **测试未配置** | JS/TS 项目 `test` script 缺失时不作为失败处理，仅基于代码审查判定 |

---

## 9. 开发环境搭建

### 前置条件
- Rust 工具链（`cargo` + `rustc`）
- Node.js 20+（`npm` 或 `pnpm`）
- `git`（在 PATH 中）
- `claude` CLI（在 PATH 中，需已登录）
- DeepSeek API Key

### 启动命令

```bash
# 1. 配置环境变量
echo 'API_KEY="sk-your-deepseek-key"' > ~/metheus/.env
# 可选：指定模型
echo 'METHEUS_MODEL="deepseek-v4-flash"' >> ~/metheus/.env

# 2. 安装前端依赖
cd ~/metheus && npm install

# 3. 开发模式启动（热重载）
cd ~/metheus && cargo tauri dev
```

### 测试命令

```bash
# 编译检查
cd ~/metheus/src-tauri && cargo check

# 完整构建
cd ~/metheus/src-tauri && cargo build

# 前端类型检查
cd ~/metheus && npx tsc --noEmit
```

### 运行要求
- 项目目录必须是 **Git 仓库**（或程序会在 `start_execution` 时自动 `git init`）
- 桌面环境（Tauri 需要窗口系统）

---

## 10. 常见修改场景索引

| 你想做的事 | 操作 |
|-----------|------|
| **改 AI 角色提示词** | 编辑 `src-tauri/src/prompts.rs`，找到对应 `const *_PROMPT` 常量 |
| **改 DeepSeek API 超时** | 编辑 `src-tauri/src/constants.rs`，修改 `DEEPSEEK_API_TIMEOUT_SECS` |
| **改 Claude Code 执行超时** | 编辑 `src-tauri/src/constants.rs`，修改 `CLAUDE_CODE_TIMEOUT_SECS` |
| **改宪法压缩阈值** | 编辑 `src-tauri/src/constants.rs`，修改 `COMPACTION_TRIGGER_TOKENS` |
| **加一个新 Tauri 命令** | 1. 在对应模块文件中写 `#[tauri::command] pub(crate) async fn xxx()` 2. 在 `lib.rs` 的 `generate_handler![]` 中注册 `crate::模块::xxx` |
| **加一个新数据结构** | 1. 在 `src-tauri/src/project.rs` 中定义 struct/enum 2. 在 `src/types.ts` 中添加对应 TypeScript 类型 |
| **改前端视图** | 编辑 `src/App.tsx`（根组件+状态管理）或 `src/ChatRoom.tsx`（聊天）/ `src/TaskConsole.tsx`（执行控制台） |
| **改持久化格式** | `~/.metheus/{project_name}.json` 文件由 `save_project()`/`load_project()` 读写，数据结构在 `project.rs` 的 `Project` struct |
| **改 Git tag 命名规则** | 编辑 `src-tauri/src/git_ops.rs`，修改 `git_save_node`/`git_save_subtask`/`git_save_subtask_inner` 中的 `format!("metheus/...")` |
| **改前端 IPC 调用** | 编辑 `src/App.tsx` 中的 `invoke("command_name", { args })` 调用 |
| **添加新的 AI 角色** | 1. `prompts.rs` 中加新 prompt 常量 2. `commands/chat.rs` 的 `chat_with_role` 的 match 分支加新角色 3. 前端 `ChatRoom.tsx` 加新按钮 |
| **修改执行流水线逻辑** | 编辑 `src-tauri/src/pipeline.rs` 的 `execute_mid_stage_pipeline`（专业模式）或 `execute_quick_pipeline`（快速模式） |
| **修改子进程执行逻辑** | 编辑 `src-tauri/src/executor.rs` 的 `execute_subtask_inner` |
| **修改 JSON 清洗逻辑** | 编辑 `src-tauri/src/json_utils.rs` 的 `sanitize_json_response` |
| **修改快照/孤儿进程保护** | 编辑 `src-tauri/src/snapshot.rs` |
| **添加/修改项目类型自动检测** | 编辑 `src-tauri/src/test_runner.rs` 的 `check_subtask`（项目类型测试框架匹配） |
| **修改中阶段排序逻辑** | 编辑 `src-tauri/src/commands/milestone.rs` 的 `generate_mid_stages` 中的 `order` 赋值；编辑 `src/App.tsx` 的 `handleNextMidStage` 排序 |
| **修改 invoke 超时配置** | 编辑 `src/utils/invokeWithTimeout.ts` 的 `INVOKE_TIMEOUT_MAP` |
| **添加/修改 Tauri 前端 invoke 调用超时** | 编辑 `src/utils/invokeWithTimeout.ts`，在 `INVOKE_TIMEOUT_MAP` 添加/修改命令超时秒数 |
