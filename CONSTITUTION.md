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
| **tokio** | Rust 异步运行时（Tauri 命令均为 async） |
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
└──────────────────────┬──────────────────────────────────┘
                       │  IPC (Tauri Bridge)
┌──────────────────────▼──────────────────────────────────┐
│                 Rust 后端 (lib.rs = 入口, 146 行)         │
│                                                          │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────┐ │
│  │commands/ │ │git_ops   │ │constitution│ │pipeline     │ │
│  │ chat     │ │ 8 fn     │ │ 7 fn +    │ │ 5 cmd +     │ │
│  │ plan     │ │          │ │ Validation│ │ PipelineState│ │
│  │ milestone│ │          │ │ Result    │ │ + 1 core fn │ │
│  │ proj_ops │ │          │ │          │ │             │ │
│  └──────────┘ └──────────┘ └──────────┘ └─────────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────┐ │
│  │executor  │ │test_runner│ │diff      │ │  api.rs     │ │
│  │2 fn      │ │ 7 fn     │ │ 2 fn     │ │ 3 fn        │ │
│  └──────────┘ └──────────┘ └──────────┘ └─────────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌──────────────┐             │
│  │prompts   │ │constants │ │json_utils    │             │
│  │10 const  │ │6 const   │ │ 2 fn         │             │
│  └──────────┘ └──────────┘ └──────────────┘             │
│                                                          │
│  project.rs — 所有数据结构 (struct/enum, 374 行)          │
│  lib.rs — AppState + run() + generate_handler![32 cmd]  │
└──────────────────────┬──────────────────────────────────┘
                       │
          ┌────────────┼────────────┐
          ▼            ▼            ▼
    DeepSeek API   Claude Code    Git (本地)
    (HTTP/json)    (子进程 CLI)   (Command 调用)
```

### 数据流方向（单向，不可逆）

```
用户输入 → 策略PM(对话) → 版本方案 → 产品经理(拆大阶段) → Milestones
  → 域负责人(拆中阶段) → MidStages → 开发工程师(拆小阶段) → Subtasks
  → Claude Code CLI(执行) → 测试工程师(质检) → 通过/重试
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
```

---

## 4. 关键路径

### 场景 A：用户有一个产品想法 → 生成执行计划
```
用户输入想法 → send_message(DeepSeek API, 策略PM)
  → 多轮对话 → generate_version_plan(DeepSeek API + 自检)
  → approve_version_plan(写入 CONSTITUTION.md 第1部分)
  → generate_milestones(DeepSeek API, 产品经理 + QA质检)
  → generate_mid_stages(DeepSeek API, 域负责人)
  → generate_next_prompt(DeepSeek API, 开发工程师)
→ 得到可执行的 Subtask 列表
```

### 场景 B：用户点击执行 → 流水线自动跑完
```
start_execution(后台 spawn)
  → execute_mid_stage_pipeline(循环每个 subtask)
    → generate_next_prompt(生成提示词)
    → execute_subtask_inner(启动 Claude Code CLI 子进程)
      → 自动应答 trust/yes
      → 轮询等待(检查暂停标志 + 超时)
      → 返回 ExecutionResult(改动文件列表)
    → check_subtask(DeepSeek API, 测试工程师)
    → git_save_subtask(add + commit + tag)
    → extract_diff_summary(解析 git diff)
    → update_constitution(更新 CONSTITUTION.md 第2部分)
    → compact_constitution(如果 token 超阈值)
  → git_save_node(中阶段完成建档)
→ 全部完成 / 用户暂停/停止
```

### 场景 C：用户想回退到之前的版本
```
前端选择 tag → git_rollback_to_mid_stage / git_rollback_to_subtask
  → stash 当前未提交变更
  → git reset --hard <tag>
  → 更新 project.json 状态(RolledBack)
  → 清理后续节点的 git tag
```

---

## 5. 模块清单

### `src-tauri/src/lib.rs`（146 行）— 应用入口
| 项目 | 内容 |
|------|------|
| **职责** | 模块声明、基础 I/O 函数、AppState 定义、run() 入口、注册全部 32 个 Tauri command |
| **依赖** | 所有 12 个子模块 |
| **对外接口** | `check_project_path()` (pub(crate)), `save_project()` (pub(crate)), `load_project()` (pub(crate)), `AppState` (pub), `run()` (pub) |
| **持久化** | `~/.metheus/{project_name}.json` — 单个 JSON 文件存储完整 Project 结构 |

### `src-tauri/src/project.rs`（374 行）— 数据模型
| 项目 | 内容 |
|------|------|
| **职责** | 所有数据结构定义（enum/struct），零业务逻辑 |
| **关键类型** | `Project`, `Milestone`, `MidStage`, `Subtask`, `Message`, `ExecutionResult`, `TestResult`, `DiffSummary`, `QAResult`, `GitTagInfo`, `FileEntry`, `PathValidationResult` |
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
| `call_deepseek_api_json` | 强制 JSON 输出（temperature=0.5） |
| `call_deepseek_api_inner` | 内部实现：构造 HTTP 请求 → 发送 → 解析 → 返回 content 字段 |

### `src-tauri/src/json_utils.rs`（123 行）— JSON 清洗
| 函数 | 说明 |
|------|------|
| `sanitize_json_response` | 从 AI 返回中提取纯净 JSON（处理 ```json 包裹、礼貌前缀、多余文字） |
| `parse_json_with_retry` | 带 3 次重试的 JSON 解析（含 AI 自助修正），3 次全败返回 `T::default()` |

### `src-tauri/src/git_ops.rs`（568 行）— Git 操作
| 函数 | 类型 | 说明 |
|------|------|------|
| `git_save_node` | cmd | 中阶段 Git 存档（add → commit → tag metheus/vX.Y.Z） |
| `git_save_subtask` | cmd | 小阶段 Git 存档（tag metheus/auto/vX.Y.Z/task-N） |
| `git_rollback_to_mid_stage` | cmd | 回退到中阶段 tag（stash → reset --hard → 更新 project.json） |
| `git_rollback_to_subtask` | cmd | 回退到小阶段 auto_tag |
| `get_git_tags_summary` | cmd | 列出所有 metheus/* tag |
| `get_current_diff` | cmd | 获取当前 git diff（非 git 仓库/无提交时返回空） |
| `compare_version_strings` | pub(crate) | 比较 "v0.1.1" vs "v0.1.3" 返回 -1/0/1 |
| `save_tag_to_mid_stage` | pub(crate) | 将 tag 名写入 project.json 的 mid_stage.git_tag |

### `src-tauri/src/constitution.rs`（712 行）— 宪法管理
| 项目 | 类型 | 说明 |
|------|------|------|
| `ValidationResult` | pub enum | Passed / Part1Modified / StructureDamaged / Empty |
| `validate_constitution_update` | pub(crate) | 校验 AI 结果：检查第 1 部分是否被修改 |
| `mechanical_update_constitution` | pub(crate) | 机械兜底更新（不调 AI，追加 [机械更新] 标记） |
| `update_constitution` | cmd | AI 更新第 2 部分（流程：检查→AI→校验→重试→兜底） |
| `estimate_tokens` | pub(crate) | 文本 token 估算（中文=1.0，ASCII=0.25） |
| `compact_constitution` | cmd | 第 2 部分压缩/剪枝（阈值触发，失败保留膨胀版本） |
| `read_constitution` | cmd | 读取项目目录下 CONSTITUTION.md |
| `get_constitution_summary` | cmd | 提取第 2 部分快照（结构描述+函数数+最近变更+token估算） |

### `src-tauri/src/diff.rs`（384 行）— Diff 解析
| 函数 | 说明 |
|------|------|
| `extract_diff_summary` | 解析 `git diff` stdout → `DiffSummary`（新/改/删文件 + 函数 + 依赖） |
| `extract_function_signature` | 从一行代码提取函数签名 |

### `src-tauri/src/test_runner.rs`（543 行）— 测试执行引擎
| 函数 | 类型 | 说明 |
|------|------|------|
| `check_subtask` | cmd | 测试主入口：读取被修改文件 → DeepSeek API（测试工程师）→ 返回 TestResult |
| `run_test_command` | pub(crate) | 执行测试命令 + 超时控制 |
| `summarize_test_output` | pub(crate) | 从测试输出提取关键摘要 |
| `format_test_result` | pub(crate) | 格式化测试结果为字符串 |
| `is_test_not_configured` | pub(crate) | 判断测试是否未配置 |
| `get_tracked_files` | pub(crate) | walkdir 扫描项目目录获取文件列表 |
| `detect_changes` | pub(crate) | 对比执行前后文件列表，返回新增文件 |

### `src-tauri/src/pipeline.rs`（639 行）— 执行流水线
| 项目 | 类型 | 说明 |
|------|------|------|
| `PipelineStatus` | pub enum | Idle/Running/Paused/Completed/Failed |
| `SubtaskStatusItem` | pub struct | 单个子任务的执行状态记录 |
| `PipelineState` | pub struct | 完整流水线状态（进度/日志/错误） |
| `start_execution` | cmd | 启动后台流水线，spawn tokio task |
| `get_execution_status` | cmd | 查询 PipelineState |
| `pause_execution` | cmd | 设置 Paused 标志（execute_subtask_inner 轮询检测） |
| `resume_execution` | cmd | 恢复 Running |
| `stop_execution` | cmd | 标记 Failed，写入错误信息 |
| `execute_mid_stage_pipeline` | pub(crate) | **核心编排函数**：循环 subtask → 生成提示词 → 执行 → 测试 → Git存档 → 宪法更新 |

### `src-tauri/src/executor.rs`（170 行）— 子进程执行器
| 函数 | 类型 | 说明 |
|------|------|------|
| `execute_subtask_inner` | pub(crate) | 启动 Claude Code CLI 子进程：自动应答 → 轮询(暂停检查+超时) → 返回 ExecutionResult |
| `execute_subtask` | cmd | Tauri 壳包装（前端直接调时传临时空 state） |

### `src-tauri/src/commands/` — Tauri 命令模块
| 文件 | 命令 | 说明 |
|------|------|------|
| `chat.rs` | `greet`, `send_message`, `chat_with_role` | 对话：测试连接/自由对话/指定角色对话 |
| `plan.rs` | `generate_version_plan`, `approve_version_plan` | 方案：生成版本方案+宪法第1部分/批准方案 |
| `milestone.rs` | `generate_milestones`, `regenerate_milestones_with_feedback`, `generate_mid_stages`, `generate_next_prompt` | 拆解：大阶段/中阶段/小阶段提示词（均含质检） |
| `project_ops.rs` | `get_project`, `persist_project`, `validate_project_path`, `get_project_files`, `approve_mid_stage`, `reject_mid_stage` | 项目操作：加载/保存/校验/文件树/审批 |

---

## 6. 数据模型摘要

以下为 `src-tauri/src/project.rs` 定义的核心类型（对应前端 `src/types.ts`）：

| 结构体 | 用途 |
|--------|------|
| `Project` | 根结构：name/status/mode/version_plan/milestones/discussion_threads/project_path |
| `Milestone` | 大阶段：version/title/description/tech_stack/status/mode/mid_stages/subtasks/qa_result |
| `MidStage` | 中阶段（专业模式）：version/title/description/tech_focus/status/subtasks/git_tag |
| `Subtask` | 最小执行单元：title/prompt/status/execution_result/test_result/retry_count/auto_tag |
| `Message` | 单条聊天：id/role/content/timestamp |
| `DiscussionThread` | 讨论线程：title/node_id/messages |
| `ExecutionResult` | Claude Code 执行输出：success/output/error_log/file_changes |
| `TestResult` | 测试工程师检查结果：passed/issues/suggestion/warnings |
| `GeneratedSubtask` | 开发工程师动态生成的下一步：title/prompt |
| `QAResult` / `QADetail` | 需求质检结果：passed/reason/details/attention_points/warnings |
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
| `PipelineState` | 流水线全状态：mid_stage_id/status/current_subtask_index/total_subtasks/subtask_statuses/current_log/last_error |
| `SubtaskStatusItem` | 单个子任务：subtask_id/title/status/test_result/retry_count |

---

## 7. 外部依赖（非代码库）

| 依赖 | 用途 | 版本/要求 |
|------|------|----------|
| **`claude` CLI** | 在项目目录下以子进程方式执行 AI 生成的提示词，修改项目文件 | 需在 PATH 中，支持 `--dangerously-skip-permissions --model -p` 参数 |
| **`git`** | 版本控制：存档、回退、diff、tag 管理。项目目录必须是 Git 仓库。 | 需在 PATH 中，兼容 worktree |
| **DeepSeek API** | 所有 AI 角色对话的后端 LLM。通过 `https://api.deepseek.com/v1/chat/completions` 调用。 | API_KEY 在 `.env` 中。模型白名单：`deepseek-v4-pro` / `deepseek-v4-flash`（可配置 `METHEUS_MODEL` 环境变量覆盖） |
| **Rust 工具链** | 编译后端 | `cargo` + `rustc` (Edition 2021) |
| **Node.js** | 前端构建 | `npm` / `pnpm` |

---

## 8. 错误处理策略

| 场景 | 处理方式 |
|------|----------|
| **环境变量缺失**（API_KEY 等） | 返回 `Err("API_KEY 环境变量未设置")` 给前端 |
| **DeepSeek API 超时** | `DEEPSEEK_API_TIMEOUT_SECS=120s`，超时返回友好错误消息 |
| **DeepSeek API 返回非 JSON** | `sanitize_json_response()` 清洗 → `parse_json_with_retry()` 最多 3 次 AI 自助修正 |
| **3 次 JSON 解析全败** | 返回 `T::default()`（兜底），同时 eprintln! 到后端日志 |
| **Claude Code 执行失败** | 写入 `error_log`，最多重试 3 次 |
| **Claude Code 子进程卡死** | `CLAUDE_CODE_TIMEOUT_SECS=600s`，超时强制 kill |
| **用户暂停流水线** | `PipelineStatus::Paused`，`execute_subtask_inner` 轮询检测到后 kill 子进程并返回 `SubTaskError::UserPaused` |
| **Git 命令失败** | 按场景处理：非 git 仓库返回空（幂等）、初始化失败返回明确错误、stash/reset 失败返回 stderr |
| **宪法更新 AI 连续失败** | 降级为 `mechanical_update_constitution`（机械追加 [机械更新] 标记） |
| **宪法压缩失败** | 保留膨胀版本（不降级为机械更新） |
| **前端调用时项目文件不存在** | `get_project` 返回默认空 Project（不报错） |
| **load_env 失败** | `dotenvy::dotenv().ok()` 静默忽略 |

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
| **改 Git tag 命名规则** | 编辑 `src-tauri/src/git_ops.rs`，修改 `git_save_node`/`git_save_subtask` 中的 `format!("metheus/...")` |
| **改前端 IPC 调用** | 编辑 `src/App.tsx` 中的 `invoke("command_name", { args })` 调用 |
| **添加新的 AI 角色** | 1. `prompts.rs` 中加新 prompt 常量 2. `commands/chat.rs` 的 `chat_with_role` 的 match 分支加新角色 3. 前端 `ChatRoom.tsx` 加新按钮 |
| **修改执行流水线逻辑** | 编辑 `src-tauri/src/pipeline.rs` 的 `execute_mid_stage_pipeline` |
| **修改子进程执行逻辑** | 编辑 `src-tauri/src/executor.rs` 的 `execute_subtask_inner` |
| **修改 JSON 清洗逻辑** | 编辑 `src-tauri/src/json_utils.rs` 的 `sanitize_json_response` |
