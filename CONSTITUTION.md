# Metheus — 项目宪法

> 弥 · 复杂任务编译系统 — 用精准上下文注入和滚动宪法，把模糊想法编译成可执行、可检查、可回退的代码变更。

---

## 1. 产品定位

### 核心能力

- **复杂任务编译**：将用户模糊的产品想法，经过结构化的拆解、检查和批准流程，编译为精确的代码变更序列。
- **精准上下文注入**：只在需要时注入必要的项目上下文，避免模型被不相关信息干扰。
- **滚动项目宪法**：随着项目进展持续维护一份宪法文档（`CONSTITUTION.md`），第一部分记录用户批准的长期规则，第二部分记录已有项目基线和滚动事实。
- **阶段检查**：在每个关键决策点设置显式检查，确保目标完整、与现实一致、任务可执行。
- **稳定回退**：每个执行单元完成后生成 Git 标签，支持精确回退到任意稳定点。

### 不解决什么问题

- 不是在线托管服务
- 不是 CI/CD 替代品
- 不做多人协作/云同步
- 不是自动编程工具（所有关键推进必须由用户确认）

### 目标用户

独立开发者、产品经理、技术爱好者，有自己的产品想法但缺乏将想法结构化落地为代码的管道。

---

## 2. 技术选型与理由

| 选型 | 理由 |
|------|------|
| **Tauri 2.x** | 桌面壳，Rust 后端 + Web 前端，包体小，跨平台 |
| **Rust (Edition 2021)** | 所有业务逻辑、文件 I/O、进程管理、AI API 调用均在 Rust 侧 |
| **React 19 + TypeScript** | 前端 UI，无路由库（单页应用，统一工作流状态切换视图） |
| **Vite 7** | 构建工具 |
| **DeepSeek API (`deepseek-v4-flash`)** | 所有 AI 角色对话、任务拆解、质检的统一 LLM 后端；当前不做模型分化 |
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
- **允许使用纯前端交互组件库**（如 Radix UI），但禁止引入后端依赖和复杂状态管理库
- **允许使用图标组件库**（如 lucide-react）
- **当前安装的前端组件**：@radix-ui/react-dialog, @radix-ui/react-tabs, @radix-ui/react-tooltip, lucide-react。已有对应能力的交互禁止手写替代品。
- **不使用复杂状态管理库**（Redux、Zustand 等），只用 React 自带的 `useState` / `useEffect`
- **不使用路由库**，视图由统一工作流状态控制
- **不在 MVP 阶段引入 WebSocket**，前端通过 Tauri IPC `invoke()` 调用后端
- **`project.rs` 只定义数据结构**，业务逻辑分散在各功能模块中
- **Rust 端 `project.rs` 与前端 `types.ts` 的数据结构必须保持一一对应**
- **统一工作流状态是业务页面和按钮权限的唯一判断依据**。旧的 Project.status、viewMode 和 isExecuting 只能作为兼容或纯视觉状态。
- **所有业务事实必须由后端确认并持久化**。前端不得通过临时对象或完整项目覆盖完成关键业务变更。
- **禁止前端通过 persist_project 任意提交完整项目对象完成关键业务状态变更**。每个审批、检查、生成、执行和回退动作必须调用对应的后端业务接口。
- **所有关键推进必须由用户明确点击**。当前阶段不做自动推进（禁止自动拆大阶段、自动进入 Console、自动开始执行、自动回退后重生成、自动进入下一大阶段）。
- **自动驾驶（autopilot）语义（2026-07-15 固化）**：autopilot 只在大阶段边界（`MilestoneReview`）停下由人做 A/B/C；大阶段内部的中阶段生成/检查/批准、执行计划生成/检查/批准、执行、确认全部自动代点；只保留暂停键；执行中暂停等同 In Stop 回退到最近已完成小阶段；autopilot 自动选择下一个未完成大阶段，用户不手选；autopilot 永不自动做 A/B/C 决策。
- **稳定性原则（2026-07-15 固化）**：不再保留任何"执行前重新生成提示词 / 固定管线自动重拆"的路径；执行端只执行用户或 autopilot 已确认的既定计划（`execution_prompt`），杜绝 AI 歧义。
- **本轮真实可体验闭环目标**：No Project 和 Half Project 都能走到正式执行；三项检查无法绕过；In Stop 和 ED Stop 都能真实体验；回退有影响预览；大阶段 A、B、C 都能完整走通；任意关键状态刷新后可以恢复。
- **V1 人工治理模式**：批准计划不等于自动执行计划。每个小阶段必须经历"用户点击执行 → 自动验证 → 用户确认结果"后才允许写 Git 稳定标签。`PlanApproval` 是方案审批页面，不代表方案已经批准。Console 链式步骤必须由用户逐级点击推进，禁止自动连续执行下一个小阶段。
- **DeepSeek v4 Flash 任务边界**：当前所有 DeepSeek 对话、检查、方案、大阶段、中阶段和执行计划编译统一使用 `deepseek-v4-flash`，暂不做模型分化。所有模型任务必须有明确边界——单一目标、允许文件范围、上下文证据、验收标准、禁止扩展范围、信息不足时停止规则。返回的阶段、计划、检查结果必须经过结构化解析和本地字段校验；缺字段、范围越界、任务空白、检查失败均不得进入下一步。
- **Console 后端最终事实规则**：关键业务命令必须先持久化，再返回从磁盘重新读取的完整 `Project`；后端持久化后的 `Project` 是唯一业务事实。
- **Console 前端同步规则**：前端必须通过统一入口校验并应用后端返回的完整 `Project`，不得让较旧修订覆盖较新修订，也不得使用临时候选列表或 `persist_project` 拼装业务事实。
- **Console 超时协调规则**：AI 命令的前端等待时间必须长于后端 HTTP 超时并预留解析、持久化时间。前端等待超时不等于业务失败，不得自动重发生成命令；必须有限次读取磁盘项目协调最终状态，并提供只调用 `get_project` 的手动同步入口。

---

## 3. V1 用户路径

### 顶层阶段

Metheus V1 定义四个顶层阶段，所有用户操作都在其中一个阶段中进行：

```
Before → First Discussion → Console → Completed
```

### 3.1 Before（项目入口）

Before 是应用的起始页，用户在此选择项目来源：

**No Project（从零开始）**
- 用户从空白项目开始
- 选择空目录或尚不存在的目录
- 填写项目名称和目标路径
- 路径不存在时将创建目录确认 → 用户确认后创建
- 初始化后进入 First Discussion

**Half Project（改造已有项目）**
- 用户已有现成代码项目
- 目录必须已存在且包含可读取文件
- 自动扫描项目结构和技术栈
- 生成 Already 基线（已有能力、待完成能力、风险等）
- 用户批准基线后进入 First Discussion

### 3.2 First Discussion（首次讨论与方案批准）

用户在 AI 辅助下讨论项目目标，经过以下步骤后生成批准的项目方案：

1. **自由讨论**：用户与策略产品经理角色对话，逐步明确目标
2. **三项显式检查**（必须由用户逐项点击，**必须提供"返回继续讨论"入口**）：
   - **目标完整性检查**：目标、用户、范围、约束和成功标准是否明确
   - **现实一致性检查**：项目路径、Already 基线、技术栈与用户目标是否一致
   - **任务可执行性检查**：目标能否拆成可验证的小任务，列出阻塞问题
3. **生成项目方案草稿**（不写入目标项目 CONSTITUTION.md）
   - **`PlanApproval` 是审批页面，绝不等于"项目方案已批准"**
   - 草稿待审批 ≠ 草稿已批准
   - 用户批准成功之前，禁止显示"进入控制台"
4. **用户批准方案** → 写入 CONSTITUTION.md 第一部分（及第二部分的 Already 基线）
   - 批准后才显示"项目方案已批准"和"进入控制台"
5. **显示"进入控制台"按钮**（不得自动进入）

任何检查失败或方案驳回时，返回讨论状态，用户补充信息后重新检查。
用户发送新需求后，旧检查和旧草稿必须失效。

### 3.3 Console（控制台规划与执行）

Console 是 Metheus 的核心控制界面，用户在此手动推进以下链式步骤：

1. **生成大阶段草稿** → **检查** → **用户批准** → **手动选择一个大阶段**（不得自动选中第一个）
2. **选择一个大阶段**，再点击生成中阶段草稿 → **检查** → **用户批准** → **手动选择一个中阶段**
3. **选择一个中阶段**，再点击生成执行计划（动态任务数量，禁止固定 3 个）
4. **执行计划检查** → **用户批准**执行计划
5. **开始执行**（仅批准后可见）
6. **每次点击只执行一个已批准小阶段**（不得自动连续执行下一个小阶段）
7. **执行结果待人工确认**（用户确认通过后才写 Git 稳定标签）
8. **确认通过后显示下一个待执行小阶段**，用户必须再次点击才能执行

每一步用户明确点击后才能进行下一步。Quick 模式从正常界面隐藏。

**⚠️ 当前 Console 已有新工作流面板，但旧 `ExecutionTree`、旧 `TaskConsole`、前端计划 Map 和旧命令入口仍共存；本轮六阶段施工将按闸门逐步停止这些路径参与业务裁决。**

### 3.4 Completed（项目完成）

最后一个大阶段完成后，项目进入 Completed 状态。

---

## 4. 暂停与回退规则

### 4.1 暂停类型

- **立即暂停（In Stop）**：终止当前子进程，回到上一个稳定检查点。当前未完成的任务不保留部分结果。
- **当前小阶段完成后暂停（ED Stop）**：当前任务通过测试、写入 Git 标签后进入暂停。刚完成的任务得到保留。

暂停后显示三个动作：继续原计划、保留已完成只调整后续、回退到更早稳定点。

### 4.2 暂停约束

- 暂停决策未完成时禁止生成新计划、切换项目或开始另一条流水线
- 暂停讨论记录只收集暂停发生后的消息

### 4.3 回退规则

- 检查点任务本身保留，重生成从检查点之后开始
- 回退确认前必须展示保留范围、作废范围和重生成范围
- 回退完成后不得自动生成计划
- 重生成后保留 Passed 任务的原始标识、结果和标签

### 4.4 大阶段结束 A/B/C 分支

每个大阶段完成后，用户必须选择一条分支：

| 分支 | 含义 | 行为 |
|------|------|------|
| **A：正常继续** | 批准当前大阶段 | 手动选择下一个大阶段（最后一个则进入 Completed） |
| **B：修正过去** | 进入产品经理讨论 | 基于问题和可用检查点生成回退建议 → 确认影响范围 → 执行回退 |
| **C：调整未来** | 保留已完成大阶段 | 只重新生成后续大阶段，新阶段需经质量检查 |

**C 分支实现规则（2026-07-14 固化）：**

1. **分割点元数据**：未来规划草稿（`MilestoneDraft`）必须记录 `draft_kind: "FutureOnly"`、`split_after_milestone_id`（分割点）、`retained_milestone_ids`（保留阶段 ID 列表）。
2. **版本归一化**：AI 只负责未来阶段内容，版本号由后端 `normalize_future_versions()` 基于最后一个保留阶段的版本重新计算。AI 原始版本存入 `original_ai_versions` 仅作参考。
3. **批准前校验**：`approve_future_milestones` 必须验证草稿种类为 FutureOnly、分割点存在、保留列表非空、未来候选非空、版本无重复/无跨层冲突。任一条件不满足则拒绝批准。
4. **前端分割显示**：`FuturePlanApproval` 页面必须明确分成"已保留"和"新规划"两段，中间有分割线和说明文案。保留段只读。

---

## 5. 工作流状态设计

### 5.1 唯一业务状态

统一工作流状态（`WorkflowState`）是前端显示和按钮权限的唯一判断来源，包含：

- **顶层阶段**：Before / FirstDiscussion / Console / Completed
- **当前步骤**：具体步骤标识（如 waiting_entry / path_check / existing_analysis / baseline_approval / discussion / three_checks / plan_approval / console_planning / execution / pause_decision / milestone_review）
- **暂停原因**：无暂停 / InStop / EDStop

### 5.2 状态转换规则

- 所有合法状态转换在 Rust 端集中定义
- 禁止任意组件直接跳到不相邻状态
- 旧数据（无新字段）启动时自动迁移一次

---

## 6. 宪法文档结构

`CONSTITUTION.md` 分为两部分：

### 第一部分：用户批准的长期原则

- 由项目方案批准时写入
- 包含技术选型理由、架构决策记录（ADR）、编码规范
- Half Project 已有的宪法第一部分必须逐字保留
- 仅通过用户批准的项目方案更新

### 第二部分：已有项目基线和滚动项目事实

- Half Project 的 Already 基线初始化时写入
- 在每次小阶段执行完成后由 AI 或机械方式更新
- 包含当前项目结构、关键函数列表、已完成能力、待完成能力
- Token 超阈值时可压缩剪枝

---

## 7. 顶层架构图

```
┌─────────────────────────────────────────────────────────────┐
│                    前端 (React + TypeScript)                  │
│  App.tsx → [ProjectEntry | ChatRoom | ExecutionTree |        │
│             TaskConsole | PreflightPanel | FileTree |         │
│             ExistingBaselinePanel | PauseDecisionPanel |      │
│             MilestoneReviewPanel | RollbackImpactDialog]      │
│  工作流状态驱动视图切换（非路由）                              │
│  所有 AI 调用 → Tauri IPC invoke("command_name", args)        │
│  统一前端组件: Modal / IconButton / Tabs / Tooltip            │
└──────────────────────┬──────────────────────────────────────┘
                       │  IPC (Tauri Bridge)
┌──────────────────────▼──────────────────────────────────────┐
│                 Rust 后端 (lib.rs = 入口)                     │
│                                                              │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────────┐ │
│  │commands/ │ │git_ops   │ │constitution│ │pipeline         │ │
│  │ chat     │ │ 11+ fn   │ │ 7 fn +   │ │ 5 cmd +         │ │
│  │ plan     │ │ (含回退)  │ │ Validation│ │ PipelineState   │ │
│  │ milestone│ │          │ │ Result    │ │ + 2 core fn     │ │
│  │ proj_ops │ │          │ │          │ │ (含暂停决策)     │ │
│  │ checks   │ │          │ │          │ │                  │ │
│  │ analysis │ │          │ │          │ │                  │ │
│  └──────────┘ └──────────┘ └──────────┘ └─────────────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────────┐ │
│  │executor  │ │test_runner│ │diff      │ │  api.rs         │ │
│  │2 fn      │ │ 7 fn     │ │ 2 fn     │ │ 3 fn            │ │
│  └──────────┘ └──────────┘ └──────────┘ └─────────────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌──────────────┐ ┌─────────────┐ │
│  │prompts   │ │constants │ │json_utils    │ │snapshot     │ │
│  │15+ const │ │6 const   │ │ 2 fn         │ │ 2 cmd +     │ │
│  │          │ │          │ │              │ │ 4 辅助 fn   │ │
│  └──────────┘ └──────────┘ └──────────────┘ └─────────────┘ │
│                                                              │
│  project.rs — 所有数据结构 (struct/enum)                       │
│  lib.rs — AppState + run() + generate_handler![50 cmd]     │
└──────────────────────┬──────────────────────────────────────┘
                       │
          ┌────────────┼────────────┐
          ▼            ▼            ▼
    DeepSeek API   Claude Code    Git (本地)
    (HTTP/json)    (子进程 CLI)   (Command 调用)
```

### 数据流方向（单向，不可逆）

```
用户入口 → [No Project | Half Project]
  → First Discussion（讨论 → 三项检查 → 方案起草 → 方案批准）
  → Console（生成大阶段 → 选择 → 生成中阶段 → 检查 → 批准 → 选择 → 生成执行计划 → 检查 → 批准 → 执行）
  → 暂停决策 [In Stop | ED Stop]
  → 分支决策 [A 继续 | B 修正 | C 调整]
  → Completed
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

## 8. 模块清单（当前实现与目标路径）

> **说明**：以下模块清单反映当前代码状态和目标改造方向。带 ⚠️ 标记的模块正在按本蓝图施工中，尚未完全对齐新路径。

### `src-tauri/src/lib.rs` — 应用入口
| 项目 | 内容 |
|------|------|
| **职责** | 模块声明、基础 I/O 函数、AppState 定义、run() 入口、注册全部 Tauri command |
| **依赖** | 所有子模块 |
| **对外接口** | `check_project_path()` (pub(crate)), `save_project()` (pub(crate)), `load_project()` (pub(crate)), `project_data_path()` (pub(crate)), `AppState` (pub), `run()` (pub) |
| **持久化** | `~/.metheus/{project_name}.json` — 单个 JSON 文件存储完整 Project 结构 |

### `src-tauri/src/project.rs` — 数据模型
| 项目 | 内容 |
|------|------|
| **职责** | 所有数据结构定义（enum/struct），零业务逻辑 |
| **关键类型** | `Project`, `ProjectEntryKind`, `WorkflowState`, `Milestone`, `MidStage`(含order字段), `Subtask`, `Message`, `ExecutionResult`, `TestResult`, `DiffSummary`, `QAResult`, `GitTagInfo`, `FileEntry`, `PathValidationResult`, `SubTaskError`, `ConstitutionSummary`, `DiscussionThread`, `ExistingProjectBaseline`, `PreflightCheckResult`, `PlanDraft`, `StagePlanCheckResult`, `PauseContext`, `RollbackImpact`, `BranchDecision` |
| **同步要求** | 每个 struct/enum 必须与 `src/types.ts` 一一对应 |

### `src-tauri/src/prompts.rs` — AI 角色提示词
| 职责 | 15+ 个 `pub(crate) const` system prompt 字符串常量 |

### `src-tauri/src/constants.rs` — 配置常量
| 常量 | `DEEPSEEK_WORKFLOW_MODEL`=`deepseek-v4-flash`, `DEEPSEEK_API_URL`, `SANITIZE_FALLBACK_JSON`="{}", `DEEPSEEK_API_TIMEOUT_SECS`=120, `CLAUDE_CODE_TIMEOUT_SECS`=600, `GIT_INIT_FAILED`, `GIT_AUTO_INIT_COMMIT_MSG`, `COMPACTION_TRIGGER_TOKENS`=3000 |

### `src-tauri/src/api.rs` — DeepSeek API 封装
| 函数 | 说明 |
|------|------|
| `call_deepseek_api` | 纯文本对话（temperature=0.1） |
| `call_deepseek_api_json` | 强制 JSON 输出（temperature=0.5，设置 response_format="json_object"） |
| `call_deepseek_api_inner` | 内部实现 |

### `src-tauri/src/json_utils.rs` — JSON 清洗
| 函数 | `sanitize_json_response`, `parse_json_with_retry` |

### `src-tauri/src/git_ops.rs` — Git 操作
| 函数 | `git_save_node`, `git_save_subtask`, `git_save_subtask_inner`, `git_rollback_to_mid_stage`, `git_rollback_to_subtask`, `rollback_to_subtask_with_reset`(cmd), `get_git_tags_summary`, `get_current_diff`, `compare_version_strings`, `save_tag_to_mid_stage`, `git_stash_and_reset_to_tag` + ⚠️ `create_execution_baseline` |

### `src-tauri/src/constitution.rs` — 宪法管理
| 职责 | 校验 AI 结果、AI 更新第 2 部分、机械兜底、压缩/剪枝、读取宪法、提取摘要 |

### `src-tauri/src/diff.rs` — Diff 解析
| 函数 | `extract_diff_summary`, `extract_function_signature` |

### `src-tauri/src/test_runner.rs` — 测试执行引擎
| 函数 | `check_subtask`, `run_test_command`, `summarize_test_output`, `format_test_result`, `is_test_not_configured`, `get_tracked_files`, `detect_changes` |

### `src-tauri/src/pipeline.rs` — 执行流水线
| 类型/函数 | `PipelineStatus`, `SubtaskStatusItem`, `PipelineState`, `start_execution`, `get_execution_status`, `pause_execution`, `resume_execution`, `stop_execution`, `execute_mid_stage_pipeline`, `execute_quick_pipeline`（保留兼容） + ⚠️ 暂停决策增强 |

### `src-tauri/src/executor.rs` — 子进程执行器
| 函数 | `execute_subtask_inner`, `execute_subtask` |

### `src-tauri/src/snapshot.rs` — 快照与孤儿进程保护
| 函数 | `UISnapshot`(struct), `AppSnapshot`(struct), `save_snapshot`, `load_snapshot`, `update_snapshot_pid`, `is_pid_alive`, `kill_pid`, `cleanup_orphan_processes_at_startup`, `save_snapshot_event`, `restore_snapshot` |

### ⚠️ `src-tauri/src/commands/checks.rs` — 三项检查（新增）
| 函数 | `run_preflight_check` |

### ⚠️ `src-tauri/src/commands/project_analysis.rs` — 项目分析（新增）
| 函数 | `scan_existing_project`, `generate_existing_baseline`, `approve_existing_baseline` |

### `src-tauri/src/commands/` — Tauri 命令模块
| 文件 | 命令 |
|------|------|
| `chat.rs` | `greet`, `send_message`, `chat_with_role` |
| `chat.rs` | `greet`, `send_message`, `chat_with_role`(持久化版本，返回 Project) |
| `plan.rs` | `generate_version_plan`(返回 PlanDraft), `approve_version_plan`(写宪法，不自动拆解), `reject_version_plan`, `enter_console` |
| `milestone.rs` | `generate_milestones`, `regenerate_milestones_with_feedback`, `regenerate_milestones_from_point`, `generate_mid_stages`, `generate_next_prompt`, `regenerate_plan_from_checkpoint`, `summarize_milestone` + ⚠️ `generate_execution_plan`, `check_stage_plan`, `suggest_rollback_checkpoint` |
| `project_ops.rs` | `get_project`(报错而非空项目), `persist_project`, `validate_project_path`, `get_project_files`, `approve_mid_stage`, `reject_mid_stage`, `initialize_project_entry`(安全恢复), ⚠️ `approve_stage_plan`, ⚠️ `approve_milestone_outcome` |
| `checks.rs` | `run_preflight_check` |
| `project_analysis.rs` | `analyze_existing_project`, `scan_existing_project`, `generate_existing_baseline`, `approve_existing_baseline` |
| `workflow.rs` 新增 | `transition_workflow`, `migrate_project_workflow` |

### 前端文件清单
| 文件 | 职责 |
|------|------|
| `src/App.tsx` | **根组件**：所有核心状态、统一工作流状态驱动视图切换、命令回调函数、执行状态轮询、快照持久化 |
| `src/ProjectEntry.tsx` ⚠️ 新增 | Before 入口页面：No Project 和 Half Project 选择、路径校验 |
| `src/ChatRoom.tsx` | 聊天组件：角色对话、版本方案渲染 |
| `src/ExistingBaselinePanel.tsx` ⚠️ 新增 | Half Project：Already 基线展示和批准 |
| `src/PreflightPanel.tsx` ⚠️ 新增 | 三项检查展示和逐项触发 |
| `src/ExecutionTree.tsx` | 执行树：大阶段→中阶段→小阶段三层结构 |
| `src/TaskConsole.tsx` ⚠️ 改造 | 执行控制台：Radix Tabs 替换自制标签页、执行控制、进度显示 |
| `src/PauseDecisionPanel.tsx` ⚠️ 新增 | 暂停决策面板：In Stop / ED Stop 展示 + 继续/调整/回退操作 |
| `src/MilestoneReviewPanel.tsx` ⚠️ 新增 | 大阶段审阅面板：A/B/C 分支选择 |
| `src/RollbackImpactDialog.tsx` ⚠️ 新增 | 回退影响弹窗：保留/作废/重生成范围展示 |
| `src/FileTree.tsx` | 文件树 |
| `src/FloatingChatBalloon.tsx` | 悬浮聊天球 |
| `src/components/Modal.tsx` ⚠️ 改造 | 统一弹窗（Radix Dialog） |
| `src/components/IconButton.tsx` ⚠️ 新增 | 统一图标按钮（Lucide + Tooltip） |
| `src/utils/invokeWithTimeout.ts` | 统一超时包装 |

---

## 9. 数据模型摘要

以下为 `src-tauri/src/project.rs` 定义的核心类型（对应前端 `src/types.ts`）：

| 结构体 | 用途 |
|--------|------|
| `Project` | 根结构 |
| `ProjectEntryKind` ⚠️ 新增 | 项目来源枚举：NoProject / HalfProject |
| `WorkflowState` ⚠️ 新增 | 统一工作流状态 |
| `ExistingProjectBaseline` ⚠️ 新增 | 已有项目基线 |
| `PreflightCheckResult` ⚠️ 新增 | 三项检查结果 |
| `PlanDraft` ⚠️ 新增 | 方案草稿（含宪法第一部分草稿） |
| `StagePlanCheckResult` ⚠️ 新增 | 执行计划检查结果 |
| `PauseContext` ⚠️ 新增 | 暂停上下文 |
| `RollbackImpact` ⚠️ 新增 | 回退影响范围 |
| `BranchDecision` ⚠️ 新增 | 分支决策 |
| `Milestone` | 大阶段 |
| `MidStage` | 中阶段（专业模式） |
| `Subtask` | 最小执行单元 |
| `Message` | 单条聊天 |
| `DiscussionThread` | 讨论线程 |
| `ExecutionResult` | Claude Code 执行输出 |
| `TestResult` | 测试工程师检查结果 |
| `QAResult` / `QADetail` | 需求质检结果 |
| `DiffSummary` | Git diff 解析 |
| `ConstitutionSummary` | 宪法快照 |
| `GitTagInfo` | Git tag 记录 |
| `FileEntry` | 文件树条目 |
| `PathValidationResult` | 路径校验结果 |
| `SubTaskError` | 执行错误类型 |

`src-tauri/src/pipeline.rs` 定义：
| `PipelineStatus` | Idle/Running/Paused/Completed/Failed |
| `PipelineState` | 流水线全状态 |
| `SubtaskStatusItem` | 单个子任务执行状态 |

`src-tauri/src/snapshot.rs` 定义：
| `UISnapshot` | UI状态快照 |
| `AppSnapshot` | 完整快照（含 running_pid） |

---

## 10. 前端组件规则

### 当前安装的依赖（本轮不新增）

| 依赖 | 用途 |
|------|------|
| `lucide-react` | 统一按钮、状态和工具图标。有对应图标时禁止使用表情符号模拟 |
| `@radix-ui/react-dialog` | 所有弹窗（确认、分支、宪法查看、回退预览、危险操作） |
| `@radix-ui/react-tabs` | TaskConsole 标签页交互 |
| `@radix-ui/react-tooltip` | 纯图标按钮的悬浮说明（必须提供） |

已有对应能力的 Radix 交互禁止继续手写替代品（如自制弹窗、自制标签页、无 Tooltip 的纯图标按钮）。

### 项目内部基础组件（减少各业务组件重复代码）

- `ActionButton` — 统一普通、主要、危险和次要操作按钮
- `StatusBadge` — 统一等待、进行中、通过、失败、暂停和过期状态
- `FeedbackBanner` — 统一成功、警告、错误和信息提示（支持重试动作）

### 禁止的前端依赖

- 前端状态管理库（Redux, Zustand, MobX 等）
- 路由库（React Router, TanStack Router 等）
- 网络请求库（axios, SWR, TanStack Query 等）
- 完整主题/UI 框架（Tailwind, Ant Design, Material UI, Chakra UI 等）
- 后端依赖（Node.js 原生模块、数据库驱动等）

---

## 11. 外部依赖（非代码库）

| 依赖 | 用途 |
|------|------|
| **`claude` CLI** | 子进程执行 AI 生成的提示词 |
| **`git`** | 版本控制 |
| **DeepSeek API** | LLM 后端 |
| **Rust 工具链** | 编译后端 |
| **Node.js 20+** | 前端构建 |

---

## 12. 错误处理策略

| 场景 | 处理方式 |
|------|----------|
| **环境变量缺失** | 返回 `Err("API_KEY 环境变量未设置")` |
| **DeepSeek API 超时** | 120s 超时，返回友好错误 |
| **DeepSeek API 返回非 JSON** | `sanitize_json_response` → `parse_json_with_retry` 最多 3 次修正 |
| **3 次 JSON 解析全败** | 返回 `Err` |
| **Claude Code 执行失败** | 写入 `error_log`，最多重试 3 次 |
| **Claude Code 子进程卡死** | 600s 超时强制 kill |
| **用户暂停流水线** | In Stop：立即 kill 子进程；ED Stop：当前任务完成后暂停 |
| **Git 命令失败** | 按场景处理 |
| **宪法更新 AI 连续失败** | 降级为机械更新 |
| **孤儿进程残留** | 启动时 `cleanup_orphan_processes_at_startup()` |
| **测试框架检测** | 自动匹配项目类型 |
| **暂停但稳定标签不存在** | 保持暂停并报告，不直接恢复 |
| **检查接口失败** | 标记为检查失败，不把网络失败当成业务通过 |
| **Console 前端等待超时** | 不标记为生成失败、不自动重发；有限次调用 `get_project` 协调磁盘最终状态，结束后提供手动同步 |
| **Console 保存后回读失败** | 命令整体返回数据一致性错误，不返回未经磁盘确认的内存对象 |

---

## 13. 项目方案批准规则

1. `generate_version_plan` 只返回方案草稿和宪法第一部分草稿，设置草稿状态为"待审批"
2. 生成草稿时禁止写入目标项目 CONSTITUTION.md
3. 草稿保存到 Project.plan_draft，避免刷新丢失
4. 草稿具有生命周期状态：待审批 → 已批准（仅通过 approve 命令）、已驳回（仅通过 reject 命令）、已过期（用户发送新需求）或已被替代（用户主动重新讨论已批准方案，旧方案保留为历史但不可再进入 Console）
5. 用户批准后才写入正式 version_plan 和宪法第一部分
6. 草稿待审批时禁止显示"项目方案已批准"、绿色成功横幅和"进入控制台"
7. 只有草稿状态为"已批准"后，才显示"进入控制台"
8. Half Project 批准方案时必须保留已批准的宪法第二部分基线
9. 批准后显示"进入控制台"按钮，不得自动拆解大阶段
10. 驳回方案后回到讨论，草稿标记为已驳回并移入 draft_history；三项检查标记为过期
11. 用户发送新需求后，旧检查和待审批草稿必须标记为过期
12. 前端不得通过 persist_project 覆盖完成批准、驳回或进入 Console

---

## 14. 执行计划与执行对齐规则

1. `start_execution` 启动前从磁盘读取批准计划
2. 前端传入的任务列表只用于一致性校验
3. 首次执行每个小阶段时使用该 Subtask 已批准的 prompt
4. 测试重试不改变原任务标题、目标和文件边界
5. 每个小阶段通过后立即写回执行结果、状态和 Git 标签
6. 已 Passed 的任务不得再次执行
7. 磁盘计划与前端计划不一致时拒绝启动

---

## 15. 开发环境搭建

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

# 2. 安装前端依赖
cd ~/metheus && npm install

# 3. 开发模式启动
cd ~/metheus && cargo tauri dev
```

### 验证命令

```bash
# 编译检查
cd ~/metheus/src-tauri && cargo check
cd ~/metheus/src-tauri && cargo build

# 前端类型检查与构建
cd ~/metheus && npx tsc --noEmit
cd ~/metheus && npm run build
```

---

## 16. 当前阶段工作

### Phase: 自动驾驶 / 稳定性 / Already 宪法 大型施工（2026-07-15 启动）

**本轮范围**：严格按七阶段顺序执行。前一阶段未通过前端正式构建和后端编译构建时，禁止进入后一阶段。执行工具：DeepCode（deepseek v4 pro）。中途无需用户审批，一次性按序跑完。

| 阶段 | 施工内容 | 状态 |
|------|----------|------|
| 1 | 更新宪法、定义 autopilot 语义与稳定性原则 | ✅ 已完成 |
| 2 | 彻底移除固定管线造成 AI 歧义的旧路径 | ✅ 已完成 |
| 3 | autopilot 后端引擎（自动选阶段、逐步推进、可暂停、不阻断） | ✅ 已完成 |
| 4 | autopilot 前端（立即动作、可见代点、仅暂停键、暂停分级） | ✅ 已完成 |
| 5 | 标签与版本号归一化展示 | ✅ 已完成 |
| 6 | Already 项目宪法（AI 读文件、隔离低权重全局记忆） | ✅ 已完成 |
| 7 | 最终校验与宪法同步 | ✅ 已完成 |

### 执行持久化规则（2026-07-14 固化）

以下规则已通过阶段一施工写入代码：

1. **执行前必须先落盘 Executing**：`execute_current_subtask` 在启动执行器前，必须把当前小阶段状态写入 `SubtaskStatus::Executing`、写入 `Project.execution_session`（状态为 `"executing"`）、保存 Project 成功后，才启动执行器。
2. **执行结束后必须先落盘 AwaitingConfirmation**：执行器返回后，必须把小阶段状态改为 `AwaitingConfirmation`、把执行结果和测试结果写入 Project、把 `execution_session.status` 改为 `"awaiting_confirmation"`、保存 Project 后再返回 `PipelineState`。
3. **启动恢复必须检查三层状态**：App 启动时，若当前步骤为 `Execution`，必须检查 `Project.execution_session`（磁盘）、当前 Project 中小阶段状态（Executing/AwaitingConfirmation）、后端 `get_execution_status` 内存状态。按优先级恢复：内存 Running → 轮询恢复；磁盘 AwaitingConfirmation → 恢复确认界面；磁盘 Executing 但内存丢失 → 显示"执行状态恢复中"。
4. **确认通过后自动推进**：`confirm_subtask_result` 在确认后必须检测中阶段是否全部 Passed，若是则标记 `MidStage.status = Completed`、写入 `completed_at`、推进工作流到 `MidStageSelection` 或 `MilestoneReview`。不再停留在所有小阶段完成但无前进按钮的死胡同。
5. **禁止浏览器 reload 推进**：执行页不得依赖 `window.location.reload` 推进业务。改为重新拉取 Project、应用完整 Project、由 React 状态机直接前进。
6. **执行轮询由多条件共同决定**：轮询开启条件 = 当前步骤为 `Execution` + `execution_session.active` 为真或后端 `PipelineState.status` 为 `Running`。不能只依赖旧的 `isExecuting` 布尔值。
7. **所有改造只在 V1 人工执行链上进行**，不修改旧自动流水线核心逻辑。
8. **本轮不新增任何前后端依赖**。不修改 Cargo.toml、package.json 及锁文件。

---

## 17. 恢复优先级链（2026-07-18 固化）

以下优先级从高到低，启动恢复和运行时状态对账必须严格遵守：

### 17.1 事实源优先级

1. **真实工作目录事实**：项目路径是否存在、是否为目录、.git 是否存在
2. **磁盘 `Project`**（`~/.metheus/{name}.json`）：唯一持久化业务事实
3. **后端内存 `PipelineState`**：执行链实时事实（仅存活于进程生命周期内）
4. **前端临时状态**：纯派生展示态，不得作为恢复判断依据

### 17.2 恢复固定顺序

启动恢复必须按以下顺序执行，前一步未完成时禁止进入后一步：

1. `Project` 加载（`load_project`）
2. `workflow` 迁移（`migrate_project_workflow`）
3. `execution` 对账（`reconcile_execution_state`）
4. `autopilot` sanity（检查 autopilot_state 与当前步骤自洽）
5. `snapshot` 恢复（`restore_snapshot`）
6. 解锁界面（释放 `startupRecoveryDoneRef`）

### 17.3 恢复对账规则

`reconcile_execution_state` 必须区分以下五种情况：

| 情况 | 磁盘 execution_session | 内存 PipelineState | 动作 |
|------|----------------------|-------------------|------|
| 真执行中 | status="executing" | Running | 恢复轮询 |
| 待确认 | status="awaiting_confirmation" | 无或 Idle | 恢复确认界面 |
| 会话失联 | status="executing" | 无（进程已死） | 显示"执行状态恢复中"，不清除磁盘 session |
| 会话无效 | active=false 或字段缺失 | 无关 | 清理 execution_session，回到当前步骤 |
| 数据冲突 | 与当前 milestone/mid_stage 不匹配 | 无关 | cleanup，回 Discussion 或 Before |

### 17.4 禁止事项

- execution 恢复未完成前，禁止启动 autopilot 驱动循环
- 禁止前端自造"恢复执行态"
- 禁止 `handleChatComplete` 中旧修订/旧步骤/旧执行会话覆盖更新状态
- 禁止旧异步结果拉回新状态

### 17.5 命令返回值规则（2026-07-19 固化）

**所有修改 `Project` 的后端命令必须统一使用 `save_and_reload_project` 模式：**

1. 从磁盘 `load_project` 获取当前事实
2. 修改内存中的 `proj` 结构
3. 调用 `save_and_reload_project(&proj)` 保存后重新读取磁盘
4. 返回磁盘最终 `Project`（非内存对象）

**例外必须写清原因和限制用途：**
- `execute_current_subtask`：两阶段保存模式（执行前保存 Executing，执行后保存 AwaitingConfirmation），返回 `PipelineState` 因为前端需要实时执行状态流
- 纯只读命令（`get_project`、`scan_existing_project` 等）不适用此规则

**已确认的不一致点（施工中修复）：**
- `persist_project`：接受前端完整 Project 无验证，应改为验证后返回磁盘事实
- `approve_stage_plan`：幂等路径返回未保存的内存对象，应改为 `save_and_reload_project`
- `write_execution_history`：静默忽略保存失败（`let _ =`），应传播错误
- `approve_existing_baseline`：文件写入无回滚保护，应增加回滚逻辑

### 17.6 前端状态应用规则（2026-07-19 固化）

**`handleChatComplete` 是前端应用后端 Project 的统一入口，必须执行以下校验：**

1. `workflow_state` 合法性：目标 `current_step` 必须在合法 `WORKFLOW_STEPS` 集合中
2. 项目身份匹配：名称和路径必须与当前项目一致
3. 修订单调性：`data_revision` 不得低于当前值（防止旧异步结果覆盖新状态）
4. 子状态过期拒绝：旧 `execution_session`、`managed_flow_state`、`autopilot_state` 在修订更低时不得覆盖新值
5. 通过全部校验后才更新 `projectRef`、`setProject`、`setProjectPath`

**`isExecuting` 应作为派生值而非独立状态：**
- 权威来源：`project.execution_session?.active === true && project.execution_session?.status === 'executing'`
- 前端 `useState<boolean>` 仅作为兼容缓存，不得用于业务裁决
- 执行轮询开启条件 = 当前步骤为 `Execution` + 磁盘 session 活跃或后端内存 Running

---

## 18. 托管层（Managed Flow）定义（2026-07-18 新增）

### 18.1 定位

托管层是一个独立于 autopilot 的轻量状态机，覆盖从 ThreeChecks 通过后到大阶段批准完成的完整链路。它不替代 autopilot，而是填补 autopilot 之前的自动化空白。

### 18.2 作用范围

```
ThreeChecks 通过 → 方案草稿生成 → 方案批准 → 进入 Console → 大阶段生成/检查/批准 → 交接给 autopilot
```

### 18.3 托管层状态字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `managed_active` | bool | 托管是否激活 |
| `managed_state` | string | 当前托管子状态 |
| `managed_target` | string | 托管终点（当前固定为 "MilestoneApproval" 即大阶段批准完成） |
| `managed_last_action` | string | 最近一次托管动作说明 |

### 18.4 托管层命令

| 命令 | 说明 |
|------|------|
| `start_managed_flow` | 从 ThreeChecks 启动托管 |
| `managed_next_step` | 执行下一步托管动作（只读顾问，返回原子命令） |
| `pause_managed_flow` | 暂停托管（仅暂停托管，保留当前步骤） |
| `resume_managed_flow` | 恢复托管 |

### 18.5 托管层与 autopilot 边界

- **托管层**：ThreeChecks 后 → 方案草稿 → 方案批准 → Console → 大阶段生成/检查/批准
- **autopilot**：大阶段批准完成后 → 中阶段生成/检查/批准 → 执行计划生成/检查/批准 → 小阶段执行/确认
- 大阶段批准完成是托管层和 autopilot 的交接点
- 托管层和 autopilot 不得同时激活

---

## 19. 暂停语义分层（2026-07-18 新增）

### 19.1 托管暂停（Managed Pause）

- 仅暂停托管推进
- 保留当前步骤不变
- 不走 InStop / EDStop
- 恢复时调用 `resume_managed_flow`

### 19.2 Autopilot 暂停

- **执行中暂停**：走 InStop 语义，kill 子进程，回退到最近已完成小阶段
- **非执行中暂停**：仅置 autopilot 为 Paused，保留当前步骤
- **完成后暂停**：走 EDStop 语义，当前任务完成后进入 PauseDecision

### 19.3 讨论后恢复区分

| 暂停类型 | 讨论范围 | 恢复命令 |
|----------|---------|---------|
| 托管暂停 | FirstDiscussion | `resume_managed_flow` |
| Autopilot 暂停（非执行中） | PauseAdjustment | `toggle_autopilot(active=true)` |
| Autopilot 暂停（InStop） | PauseAdjustment | `resolve_pause_decision("continue")` |
| EDStop 暂停 | PauseAdjustment | `resolve_pause_decision("continue")` |

---

## 20. Phase: 最终收尾施工（2026-07-19 启动）

**本轮范围**：不扩新功能，只做现有能力的稳定收尾。按 P0 → P1 → P2 顺序执行。每个优先级桶内连续做完，桶结束后执行 `npm run build && cargo build`。前一桶构建通过后自动进入下一桶。阶段之间无需审批。

### 施工规则
1. 第一阶段第一个子任务必须先更新 CONSTITUTION.md（本条）
2. 不改 Cargo.toml、package.json、锁文件
3. 不新增依赖
4. 关键业务事实必须由后端持久化并返回完整 Project
5. 前端不得再用 `persist_project` 拼关键状态
6. 每阶段结束后执行 `npm run build && cargo build`，不通过禁止进入下一阶段

| 优先级 | 施工内容 | 状态 |
|--------|----------|------|
| P0-1 | 统一命令 save-reload 模式（5 个不一致点） | ✅ 已完成 |
| P0-2 | 修旧项目误恢复与假恢复上下文 | ✅ 已完成 |
| P0-3 | 收紧 execution/autopilot/managed 恢复顺序 | ✅ 已完成 |
| P0-4 | 收紧手动模式与规划链按钮语义 | ✅ 已完成 |
| P0-5 | 收紧轻托管层边界与恢复 | ✅ 已完成 |
| P1-1 | Already 宪法做成真正低权重全局记忆链 | ✅ 已完成 |
| P1-2 | 强化 Half Project 全局记忆输入质量 | ✅ 已完成 |
| P1-3 | TaskConsole 与布局承载稳定性收口 | ✅ 已完成 |
| P2-1 | 继续压缩旧状态裁决权 | ✅ 已完成 |
| P2-2 | 文档与当前实现状态同步 | ✅ 已完成 |

### P0 通过标准
- [x] 5 个命令不一致点全部修复 (persist_project, approve_stage_plan, write_execution_history, approve_existing_baseline, execute_current_subtask doc)
- [x] 空目录/无效 JSON 不再误恢复执行链 (reconcile_on_startup + initialize_project_entry 加固)
- [x] execution/autopilot/managed 恢复不再互抢 (startupRecoveryDone guard + 固定恢复顺序)
- [x] 手动模式主动作语义统一 (移除冗余 sync 按钮、统一进入执行语义)
- [x] 托管层可稳定从 ThreeChecks 推到 milestone 批准 (stop_managed_flow 手动过渡)
- [x] `npm run build` 通过
- [x] `cargo build` 通过

### P1 通过标准
- [x] Already 宪法成为真正低权重背景记忆 (read_already_constitution_reference 接入 build_context_injection + checks)
- [x] Half Project 进入讨论时全局记忆就绪 (chat_with_role 注入工作宪法节选)
- [x] TaskConsole 长内容承载稳定
- [x] `npm run build` 通过
- [x] `cargo build` 通过

### P2 通过标准
- [x] 旧状态进一步退出主路径裁决 (_autoAdvance 移除)
- [x] 文档与代码一致
- [x] `npm run build` 通过
- [x] `cargo build` 通过

### 施工完成时间
2026-07-19 — 全部 P0/P1/P2 收尾施工完成。
