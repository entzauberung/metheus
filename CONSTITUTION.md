# Metheus — 项目宪法

> 弥 · 复杂任务编译系统 — 用精准上下文注入和滚动宪法，把模糊想法编译成可执行、可检查、可回退的代码变更。

> 最后同步：2026-07-23。本文描述 Metheus 仓库当前实现；`src-tauri/CONSTITUTION.md` 是测试/目标项目生成出的项目宪法，不是本仓库的开发约束来源。

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
- 不是无边界的一键自动编程工具；自动化只能在用户已批准的目标、计划和文件范围内运行

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
| **执行引擎抽象层 (`engine/`)** | 统一执行契约；插件适配 Claude Code / Codex，预留 Grok Build 内置引擎 |
| **Claude Code CLI (`claude`)** | 默认插件执行引擎，本地子进程执行已批准 `execution_prompt` |
| **Codex CLI (`codex`)** | 可选插件执行引擎，与 Claude Code 隔离适配，互不耦合参数 |
| **Git（程序化操作）** | 版本控制：只提交任务授权路径，并为完成的小阶段/中阶段创建不可覆盖标签 |
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
- **治理模式必须显式区分**：手动模式逐步点击；Managed Flow 只覆盖 ThreeChecks 后到大阶段批准；autopilot 只覆盖已批准大阶段内部流程。任何模式都不得绕过检查、批准、文件范围和 Git 安全边界。
- **自动驾驶（autopilot）语义（2026-07-15 固化）**：autopilot 只在大阶段边界（`MilestoneReview`）停下由人做 A/B/C；大阶段内部的中阶段生成/检查/批准、执行计划生成/检查/批准、执行、确认全部自动代点；只保留暂停键；执行中暂停等同 In Stop 回退到最近已完成小阶段；autopilot 自动选择下一个未完成大阶段，用户不手选；autopilot 永不自动做 A/B/C 决策。
- **执行引擎隔离原则（2026-07-22 固化）**：执行层通过 `engine/` 抽象，禁止业务代码直接拼装具体 CLI 参数。`ExecutionProfile` 描述 runtime/provider/permission；执行开始后完整复制到 `ExecutionSession.engine_snapshot`，恢复/修复必须沿用快照，不得中途换引擎。当前后台流水线只允许 `PermissionProfile::Unattended`。合法组合：`Plugin + ClaudeCode`、`Plugin + Codex`、`BuiltIn + GrokBuild`（Grok Build 尚未启用，健康检查返回 Disabled）。切换引擎必须走 `update_execution_profile`，并在执行中、恢复中、autopilot/托管 Running 时阻断。
- **稳定性原则（2026-07-15 固化）**：不再保留任何"执行前重新生成提示词 / 固定管线自动重拆"的路径；执行端只执行用户或 autopilot 已确认的既定计划（`execution_prompt`），杜绝 AI 歧义。
- **本轮真实可体验闭环目标**：No Project 和 Half Project 都能走到正式执行；三项检查无法绕过；In Stop 和 ED Stop 都能真实体验；回退有影响预览；大阶段 A、B、C 都能完整走通；任意关键状态刷新后可以恢复。
- **V1 手动治理模式**：批准计划不等于在手动模式下自动执行。每个小阶段必须经历"用户点击执行 → 自动验证 → 用户确认结果"后才允许写 Git 稳定标签。`PlanApproval` 是方案审批页面，不代表方案已经批准。autopilot 激活后可代点大阶段内部的合法步骤，但仍不得越过大阶段 A/B/C 人工决策。
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

Console 是 Metheus 的核心控制界面。手动模式按以下链式步骤推进；用户激活 autopilot 后，由后端按同一状态机代点大阶段内部步骤：

1. **生成大阶段草稿** → **检查** → **用户批准** → **手动选择一个大阶段**（不得自动选中第一个）
2. **选择一个大阶段**，再点击生成中阶段草稿 → **检查** → **用户批准** → **手动选择一个中阶段**
3. **选择一个中阶段**，再点击生成执行计划（动态任务数量，禁止固定 3 个）
4. **执行计划检查** → **用户批准**执行计划
5. **开始执行**（仅批准后可见）
6. **手动模式每次点击只执行一个已批准小阶段**
7. **手动模式的执行结果待人工确认**（用户确认通过后才写 Git 稳定标签）
8. **autopilot 模式可在质量门禁通过后自动确认并继续下一个小阶段**

手动模式每一步必须由用户明确点击；autopilot 只代点已经存在的合法命令，不得自造状态转换。Quick 模式从正常界面隐藏。

错误发生后，正常推进立即让位于独立恢复分支：分类 → 诊断 → 有限修复 → 复测 → 成功后回到原流程；达到停止条件后必须进入人工处理。

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
- **当前步骤**：`WorkflowStep` 枚举（如 `WaitingEntry`, `ExistingAnalysis`, `Discussion`, `ThreeChecks`, `PlanApproval`, `MilestoneGeneration`, `PlanGeneration`, `Execution`, `PauseDecision`, `MilestoneReview`, `Completed`）
- **暂停原因**：无暂停 / InStop / EDStop
- **自动驾驶状态**：`AutopilotState`，包含运行状态和后端给出的单一 `recovery_action`
- **托管状态**：`ManagedFlowState`，只负责 autopilot 之前的有限链路
- **错误恢复状态**：`RecoveryState`，记录错误类型、恢复阶段（含 `Replanning`）、尝试次数、错误签名、结构化问题、重规划标志和诊断证据
- **执行配置**：`ExecutionProfile`（项目级）与 `ExecutionSession.engine_snapshot`（会话级快照）

### 5.2 状态转换规则

- 所有合法状态转换在 Rust 端集中定义
- 禁止任意组件直接跳到不相邻状态
- 前端恢复按钮只能由后端 `recovery_action` 决定，不得解析错误文本猜测动作
- `WaitHumanDecision` 不允许通用恢复，必须通过明确的人工恢复命令退出
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
│             MilestoneReviewPanel | RollbackImpactDialog |     │
│             ExecutionEngineSettings]                          │
│  工作流状态驱动视图切换（非路由）                              │
│  所有 AI 调用 → Tauri IPC invoke("command_name", args)        │
│  统一前端组件: Modal / IconButton / Tabs / Tooltip            │
│  策略模块: autopilot/engine/log/managedFlow/workspacePolicy  │
└──────────────────────┬──────────────────────────────────────┘
                       │  IPC (Tauri Bridge)
┌──────────────────────▼──────────────────────────────────────┐
│                 Rust 后端 (lib.rs = 入口)                     │
│                                                              │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌───────────┐ │
│  │ commands/  │ │ pipeline   │ │ recovery   │ │ git_ops   │ │
│  │ 业务命令    │ │ 正常执行/暂停│ │ 错误恢复编排 │ │ 基线/标签  │ │
│  └────────────┘ └────────────┘ └────────────┘ └───────────┘ │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌───────────┐ │
│  │ engine/    │ │ test_runner│ │plan_contract│ │constitution│ │
│  │ 多引擎适配  │ │ 测试与审查  │ │ 精确范围契约 │ │ 滚动宪法   │ │
│  └────────────┘ └────────────┘ └────────────┘ └───────────┘ │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌───────────┐ │
│  │ api/prompts│ │ json_utils │ │ snapshot   │ │ diff      │ │
│  │ 决策模型    │ │ 结构化解析  │ │ 启动快照     │ │ 变更摘要   │ │
│  └────────────┘ └────────────┘ └────────────┘ └───────────┘ │
│                                                              │
│  project.rs — 所有数据结构 (struct/enum)                       │
│  lib.rs — AppState + run() + Tauri command 注册               │
└──────────────────────┬──────────────────────────────────────┘
                       │
     ┌─────────────────┼─────────────────┐
     ▼                 ▼                 ▼
 DeepSeek API    执行引擎插件/内置      Git (本地)
 (HTTP/json)     Claude Code / Codex   (Command 调用)
                 (Grok Build 预留)
```

### 数据流方向（单向，不可逆）

```
用户入口 → [No Project | Half Project]
  → First Discussion（讨论 → 三项检查 → 方案起草 → 方案批准）
  → Console（生成大阶段 → 选择 → 生成中阶段 → 检查 → 批准 → 选择 → 生成执行计划 → 检查 → 批准 → 执行）
      ↳ 错误分支（分类 → 诊断 → 有限修复 → 复测 → 继续或等待人工）
  → 暂停决策 [In Stop | ED Stop]
  → 分支决策 [A 继续 | B 修正 | C 调整]
  → Completed
```

### 模块依赖方向（箭头 = 被调用）

```
commands/ ──→ api, prompts, json_utils, constants, engine, lib(基础fn)
pipeline ──→ engine, commands/milestone, test_runner, git_ops,
              constitution, recovery, plan_contract, diff, constants, lib(基础fn)
recovery ──→ pipeline, engine, test_runner, plan_contract, project, lib(基础fn)
engine/ ──→ pipeline(类型), test_runner, constants, project(ExecutionProfile)
git_ops ──→ project
constitution ──→ api, prompts, constants, project
test_runner ──→ api, prompts, json_utils, project
snapshot ──→ project, AppState
```

---

## 8. 模块清单（当前实现与目标路径）

> **说明**：以下模块清单反映 2026-07-22 的当前代码状态；历史兼容入口仍存在，但不得参与新恢复链路的业务裁决。`executor.rs` 已删除，执行能力统一收口到 `engine/`。

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
| **关键类型** | `Project`, `WorkflowState`, `AutopilotState`, `RecoveryState`, `RecoveryErrorKind`, `RecoveryPhase`, `ExecutionProfile`, `ExecutionProvider`, `ExecutionRuntime`, `PermissionProfile`, `ExecutionSession`, `Milestone`, `MidStage`, `Subtask`, `ExecutionResult`, `TestResult`, `AutomatedTestStatus`, `VerificationKind`, `HumanVerification`, `PauseContext`, `ManagedFlowState` |
| **同步要求** | 每个 struct/enum 必须与 `src/types.ts` 一一对应 |

### `src-tauri/src/prompts.rs` — AI 角色提示词
| 职责 | 15+ 个 `pub(crate) const` system prompt 字符串常量 |

### `src-tauri/src/constants.rs` — 配置常量
| 常量 | `DEEPSEEK_WORKFLOW_MODEL`=`deepseek-v4-flash`, `DEEPSEEK_API_URL`, `SANITIZE_FALLBACK_JSON`="{}", `DEEPSEEK_API_TIMEOUT_SECS`=120, `EXECUTION_ENGINE_TIMEOUT_SECS`=600, `GIT_INIT_FAILED`, `GIT_AUTO_INIT_COMMIT_MSG`, `COMPACTION_TRIGGER_TOKENS`=3000 |

### `src-tauri/src/api.rs` — DeepSeek API 封装
| 函数 | 说明 |
|------|------|
| `call_deepseek_api` | 纯文本对话（temperature=0.1） |
| `call_deepseek_api_json` | 强制 JSON 输出（temperature=0.5，设置 response_format="json_object"） |
| `call_deepseek_api_inner` | 内部实现 |

### `src-tauri/src/json_utils.rs` — JSON 清洗
| 函数 | `sanitize_json_response`, `parse_json_with_retry` |

### `src-tauri/src/git_ops.rs` — Git 操作
| 职责 | 只提交授权路径、创建不可覆盖的小阶段/中阶段标签、读取 diff/历史、执行受控标签回退 |
| 关键函数 | `capture_authorized_diff`, `git_save_node`, `git_save_subtask`, `git_reset_to_tag_clean`, `delete_tags`, `get_git_tags_summary`, `get_current_diff`, `get_change_history` |

### `src-tauri/src/constitution.rs` — 宪法管理
| 职责 | 校验 AI 结果、AI 更新第 2 部分、机械兜底、压缩/剪枝、读取宪法、提取摘要 |

### `src-tauri/src/constitution_context.rs` — 宪法上下文注入
| 职责 | 读取 Already 宪法低权重背景，按当前任务构建受限上下文注入 |

### `src-tauri/src/diff.rs` — Diff 解析
| 函数 | `extract_diff_summary`, `extract_function_signature` |

### `src-tauri/src/test_runner.rs` — 测试执行引擎
| 职责 | 识别并执行真实测试命令，压缩输出，采集 tracked/untracked 变更证据，独立保存自动化测试事实与 AI 代码审查结论 |
| 关键函数 | `check_subtask`, `run_test_command`, `summarize_test_output`, `format_test_result`, `is_test_not_configured`, `get_file_snapshot`, `detect_changes` |

### `src-tauri/src/pipeline.rs` — 执行流水线
| 职责 | 正常小阶段执行、质量门禁、工作区准备/刷新、执行基线恢复、暂停/回退、执行状态对账和持久化执行历史 |
| 关键类型/函数 | `PipelineState`, `execute_current_subtask`, `confirm_subtask_result`, `reject_subtask_result`, `get_execution_workspace_status`, `prepare_execution_workspace`, `refresh_execution_workspace`, `reconcile_on_startup`, `acknowledge_execution_recovery` |

### `src-tauri/src/recovery.rs` — 错误恢复编排器
| 职责 | 分类执行/测试/审查错误，构建压缩诊断，执行受限修复和复测，应用次数与签名停止条件，提供人工恢复出口 |
| 关键函数 | `begin_execution_recovery`, `ensure_quality_recovery`, `run_error_recovery`, `finish_retest`, `resolve_human_recovery` |

### `src-tauri/src/plan_contract.rs` — 执行范围契约
| 职责 | 校验 `allowed_file_paths` / `new_file_paths`，检测正常执行和恢复修复产生的范围外变更 |

### `src-tauri/src/engine/` — 多引擎执行抽象层（取代已删除的 `executor.rs`）
| 文件 | 职责 |
|------|------|
| `mod.rs` | 模块出口：`execute`、`validate_profile`、`check_engine_health`、`ExecutionRequest`、`EngineError`、`EngineHealth` |
| `contract.rs` | 统一契约：`ExecutionRequest`、`EngineError`、`EngineHealth`/`EngineHealthStatus`/`EngineAuthState`、`ProcessSpec`/`ProcessOutput` |
| `service.rs` | 校验 profile 组合与 Unattended 权限；按 provider 分发执行；注入 V1 文件范围约束 |
| `process_runner.rs` | 通用子进程运行器：流式 stdout/stderr、暂停取消、超时、`execution_id` 隔离与 PID 清理 |
| `claude_code.rs` | Claude Code 适配器：`claude --dangerously-skip-permissions --model … -p <prompt>` |
| `codex.rs` | Codex 适配器：`codex exec … --sandbox danger-full-access -`（prompt 走 stdin） |
| `builtin.rs` | Grok Build 内置引擎占位；健康检查返回 Disabled，执行返回 Unavailable |
| `health.rs` | 探测 PATH 可执行文件、版本、认证状态；阻断 NotInstalled/Unauthenticated/UnsupportedVersion/Disabled |

### `src-tauri/src/snapshot.rs` — 快照与孤儿进程保护
| 函数 | `UISnapshot`(struct), `AppSnapshot`(struct), `save_snapshot`, `load_snapshot`, `update_snapshot_pid`, `is_pid_alive`, `kill_pid`, `cleanup_orphan_processes_at_startup`, `save_snapshot_event`, `restore_snapshot` |

### `src-tauri/src/commands/checks.rs` — 三项检查
| 函数 | `run_preflight_check` |

### `src-tauri/src/commands/project_analysis.rs` — 项目分析
| 函数 | `scan_existing_project`, `generate_existing_baseline`, `approve_existing_baseline` |

### `src-tauri/src/commands/` — Tauri 命令模块
| 文件 | 命令 |
|------|------|
| `chat.rs` | `greet`, `send_message`, `chat_with_role`(持久化版本，返回 Project) |
| `plan.rs` | `generate_version_plan`(返回 PlanDraft), `approve_version_plan`(写宪法，不自动拆解), `reject_version_plan`, `enter_console` |
| `milestone.rs` | 大/中阶段草稿生成、检查、批准和选择；执行计划生成、检查、批准；大阶段审阅 A/B/C；回退建议和未来阶段重规划 |
| `project_ops.rs` | `get_project`, `check_engine_health`, `update_execution_profile`, `initialize_project_entry`, `validate_project_path`, `get_project_files`, `persist_project`(兼容入口), `approve_mid_stage`, `reject_mid_stage` |
| `checks.rs` | `run_preflight_check` |
| `project_analysis.rs` | `analyze_existing_project`, `scan_existing_project`, `generate_existing_baseline`, `approve_existing_baseline` |
| `workflow.rs` | 工作流迁移与转换、ThreeChecks 入口、讨论恢复、Managed Flow（含 `wait_managed_flow_for_human` / `stop_managed_flow` / `reconcile_managed_milestone_state`）、autopilot 启停/暂停/恢复/下一步路由 |
| `pipeline.rs` | 当前小阶段执行/确认/驳回、Git 工作区准备与只读刷新、In Stop / ED Stop、回退、启动对账 |
| `recovery.rs` | `run_error_recovery`, `resolve_human_recovery` |

### 前端文件清单
| 文件 | 职责 |
|------|------|
| `src/App.tsx` | **根组件**：所有核心状态、统一工作流状态驱动视图切换、命令回调函数、执行状态轮询、快照持久化 |
| `src/ProjectEntry.tsx` | Before 入口页面：No Project 和 Half Project 选择、路径校验、入口引擎选择 |
| `src/ChatRoom.tsx` | 聊天组件：角色对话、版本方案渲染 |
| `src/ExistingBaselinePanel.tsx` | Half Project：Already 基线展示和批准 |
| `src/PreflightPanel.tsx` | 三项检查展示和逐项触发 |
| `src/PlanApprovalPanel.tsx` | 项目方案草稿审批 |
| `src/ConsoleWorkflowPanel.tsx` | Console 工作流视图总入口 |
| `src/console/*.tsx` | 大阶段、中阶段和执行计划的分步规划视图 |
| `src/ExecutionTree.tsx` | 执行树：大阶段→中阶段→小阶段三层结构 |
| `src/TaskConsole.tsx` | 执行控制台：Radix Tabs、执行控制、进度显示 |
| `src/PauseDecisionPanel.tsx` | 暂停决策面板：In Stop / ED Stop 展示 + 继续/调整/回退操作 |
| `src/MilestoneReviewPanel.tsx` | 大阶段审阅面板：A/B/C 分支选择 |
| `src/RollbackImpactDialog.tsx` | 回退影响弹窗：保留/作废/重生成范围展示 |
| `src/FileTree.tsx` | 文件树 |
| `src/FloatingChatBalloon.tsx` | 悬浮聊天球 |
| `src/components/AutopilotControlBar.tsx` | 自动驾驶状态、单一恢复动作和人工恢复出口 |
| `src/components/ExecutionEngineSelector.tsx` | 执行引擎选择与健康检查展示 |
| `src/components/ExecutionEngineSettings.tsx` | Console 内切换引擎设置弹窗（调用 `update_execution_profile`） |
| `src/components/Modal.tsx` | 统一弹窗（Radix Dialog） |
| `src/components/IconButton.tsx` | 统一图标按钮（Lucide + Tooltip） |
| `src/components/ActionButton.tsx` / `StatusBadge.tsx` / `FeedbackBanner.tsx` | 统一按钮、状态徽标和反馈条 |
| `src/components/ConsoleStepShell.tsx` / `DecisionStepHeader.tsx` / `StageCandidateCard.tsx` / `WorkflowActionBar.tsx` / `EmptyState.tsx` | Console 规划步骤壳层与决策头 |
| `src/autopilotPolicy.ts` | 将后端 `recovery_action` 映射为允许显示的前端动作 |
| `src/enginePolicy.ts` | 引擎健康阻断判定、切换引擎阻塞原因 |
| `src/workspacePolicy.ts` | Git 工作区动作映射（prepare / resolve / refresh 等） |
| `src/managedFlowPolicy.ts` | 托管层展示文案与大阶段批准按钮策略 |
| `src/logPolicy.ts` | 合并历史与运行时执行日志，去重排序 |
| `src/utils/invokeWithTimeout.ts` | 统一超时包装 |
| `src/*Policy.test.ts` | 前端策略单测（autopilot / engine / log / managedFlow / workspace） |

---

## 9. 数据模型摘要

以下为 `src-tauri/src/project.rs` 定义的核心类型（对应前端 `src/types.ts`）：

| 结构体 | 用途 |
|--------|------|
| `Project` | 根结构 |
| `ProjectEntryKind` | 项目来源枚举：NoProject / HalfProject |
| `WorkflowState` | 统一工作流状态，持有 autopilot、managed flow 和 recovery 子状态 |
| `AutopilotState` / `AutopilotRecoveryAction` | 自动驾驶运行事实和后端指定的单一恢复动作 |
| `ExecutionRuntime` | BuiltIn / Plugin |
| `ExecutionProvider` | GrokBuild / ClaudeCode / Codex |
| `PermissionProfile` | Interactive / Unattended（后台流水线仅 Unattended） |
| `ExecutionProfile` | 项目级执行配置：runtime + provider + permission_profile + profile_revision |
| `EngineHealth` / `EngineHealthStatus` / `EngineAuthState` | 引擎健康探测结果（Available / NotInstalled / Unauthenticated / UnsupportedVersion / Disabled / Unknown） |
| `RecoveryState` | 当前小阶段的持久化错误恢复状态（含结构化问题、重规划标志和 attempt_history） |
| `RecoveryErrorKind` | Workspace/Transient/Execution/Scope/Test/Review/Unavailable/Conflict/Human 分类 |
| `RecoveryPhase` | Diagnosing / Repairing / Retesting / Replanning / Recovered / WaitingHuman |
| `RecoveryIssue` / `RecoveryAttemptRecord` | 结构化验收问题与每轮修复前后问题集合变化 |
| `ExistingProjectBaseline` | 已有项目基线 |
| `PreflightCheckResult` | 三项检查结果 |
| `PlanDraft` | 方案草稿（含宪法第一部分草稿） |
| `StagePlanCheckResult` | 执行计划检查结果 |
| `PauseContext` | 暂停上下文 |
| `RollbackImpact` | 回退影响范围 |
| `BranchDecision` | 分支决策 |
| `Milestone` | 大阶段 |
| `MidStage` | 中阶段（专业模式） |
| `Subtask` | 最小执行单元 |
| `Message` | 单条聊天 |
| `DiscussionThread` | 讨论线程 |
| `ExecutionResult` | 执行引擎统一输出（含 `engine_provider`、exit_code、file_changes） |
| `ExecutionSession` | 活跃执行会话；含 `engine_snapshot` 以锁定本次执行引擎 |
| `TestResult` | 自动化测试事实、压缩输出和 AI 代码审查结论 |
| `AutomatedTestStatus` | Unknown / Passed / Failed / NotConfigured / Unavailable |
| `VerificationKind` | Legacy / AutomatedTestAndReview / CodeReviewOnly / HumanOverride |
| `HumanVerification` | 人工核验原因、时间和原始测试失败，不篡改真实测试结果 |
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
| `SubtaskStatusItem` | 单个子任务执行状态，恢复中可使用 `repairing` |

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
| **`claude` CLI** | 默认插件引擎（`ExecutionProvider::ClaudeCode`）；需在 PATH 中且已登录 |
| **`codex` CLI** | 可选插件引擎（`ExecutionProvider::Codex`）；需在 PATH 中且已登录 |
| **`git`** | 版本控制 |
| **DeepSeek API** | LLM 后端（对话、检查、规划、审查） |
| **Rust 工具链** | 编译后端 |
| **Node.js 20+** | 前端构建 |

> 执行引擎按项目 `ExecutionProfile` 选用，不是同时强制依赖全部 CLI。Grok Build 内置引擎仍为预留位，当前不可用。

---

## 12. 错误处理策略

| 场景 | 处理方式 |
|------|----------|
| **环境变量缺失** | 返回 `Err("API_KEY 环境变量未设置")` |
| **DeepSeek API 超时** | 120s 超时，返回友好错误 |
| **DeepSeek API 返回非 JSON** | `sanitize_json_response` → `parse_json_with_retry` 最多 3 次修正 |
| **3 次 JSON 解析全败** | 返回 `Err` |
| **执行引擎不可用** | 健康检查阻断 NotInstalled / Unauthenticated / UnsupportedVersion / Disabled；禁止启动执行或切换到不可用引擎 |
| **执行引擎执行失败** | 保存执行证据（含 `engine_provider`）；手动模式提供基线恢复，autopilot 进入 `ExecutionError` 恢复分支并先恢复执行基线 |
| **执行引擎子进程卡死** | `EXECUTION_ENGINE_TIMEOUT_SECS`=600 强制 kill，按执行错误收尾，不在未知工作区上继续 |
| **活跃执行/恢复中切换引擎** | `update_execution_profile` 拒绝；前端 `enginePolicy` 同步阻断 |
| **用户暂停流水线** | In Stop：立即 kill 子进程；ED Stop：当前任务完成后暂停 |
| **Git 工作区未准备** | 后端返回 `PrepareExecutionWorkspace` 或 `ResolveWorkspaceChanges`；刷新工作区只读，不得隐式初始化或提交 |
| **修复越出文件范围** | 立即恢复执行基线并进入 `WaitingHuman`，禁止在越界结果上继续修复 |
| **宪法更新 AI 连续失败** | 降级为机械更新 |
| **孤儿进程残留** | 启动时 `cleanup_orphan_processes_at_startup()` |
| **测试框架检测** | 自动匹配项目类型并记录真实命令、退出码和压缩输出 |
| **自动化测试失败** | `AutomatedTestStatus::Failed` 为硬失败，AI 审查不得覆盖为通过 |
| **测试未配置** | 标记 `NotConfigured`，允许进入代码审查通道，但不得伪装成自动化测试通过 |
| **测试/审查服务不可用** | 标记 `Unavailable` 并进入人工阻断，不得当成业务通过 |
| **自动修复连续失败** | 默认最多 2 次；相同错误签名连续出现 2 次立即停止并等待人工处理 |
| **暂停但稳定标签不存在** | 保持暂停并报告，不直接恢复 |
| **检查接口失败** | 标记为检查失败，不把网络失败当成业务通过 |
| **Console 前端等待超时** | 不标记为生成失败、不自动重发；有限次调用 `get_project` 协调磁盘最终状态，结束后提供手动同步 |
| **Console 保存后回读失败** | 命令整体返回数据一致性错误，不返回未经磁盘确认的内存对象 |
| **旧异步恢复结果返回** | 复测和修复落盘前必须再次核对 `execution_id`，不匹配时忽略旧结果 |

### 12.1 错误恢复固定链路

```text
正常自动驾驶
  → 捕获并分类错误
  → 持久化 RecoveryState 与失败证据
  → 构建当前小阶段的压缩诊断
  → 在原计划精确文件范围内有限修复（最多 max_attempts）
  → 检查范围外修改
  → 重新测试与代码审查
  → 常规修复耗尽后进入 Replanning（仅当前小阶段一次受限重规划）
  → 重规划后最多再执行一次
  → 通过后清除恢复状态并返回正常流程
  → 达到停止条件后进入 WaitingHuman
```

恢复诊断只允许包含当前目标、验收标准、`allowed_file_paths`、`new_file_paths`、受限 diff、测试命令/退出码/压缩输出、审查问题、上次修复摘要、结构化 `active_issues` 和 Git 基线。禁止重新规划整个项目或注入全量历史。重规划只改写当前小阶段执行提示/验收对齐，不得扩展文件范围或替换已 Passed 任务。恢复执行必须沿用 `ExecutionSession.engine_snapshot`，不得改用项目当前 profile。

### 12.2 人工恢复出口

`WaitHumanDecision` 不是可直接恢复状态。只能通过 `resolve_human_recovery` 的明确动作退出：

- `retest`：用户手动修复后重新测试，仍需执行范围检查
- `restore_and_retry`：恢复执行基线并重新执行当前小阶段
- `regenerate_plan`：恢复基线后重新生成当前执行计划；若当前中阶段已有 Passed 小阶段则拒绝替换，必须走稳定点回退
- `human_override`：记录 `HumanVerification` 后继续；必须填写原因，且不得修改原 `TestResult.passed`

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

1. `execute_current_subtask` 启动前必须从磁盘读取已批准计划，并在持有流水线锁时完成校验和 Running 预留。
2. 前端不得提交新的任务内容参与执行裁决；执行器使用磁盘 Subtask 的 `execution_prompt`。
3. 首次执行和恢复修复都必须复用经 `plan_contract` 校验的精确 `allowed_file_paths` / `new_file_paths`。
4. 执行前记录 `base_commit`；执行失败、进程失联或范围越界时按错误策略恢复该基线。
5. 测试重试和自动修复不得改变原任务标题、目标、验收标准和文件边界。
6. 自动化测试事实与 AI 代码审查分别保存；质量门禁要求所选核验通道真实通过。
7. 每个小阶段确认通过后才写回 Passed 状态和 Git 稳定标签；autopilot 可代点确认，手动模式仍由用户确认。
8. 已 Passed 的任务不得再次执行；重新规划不得静默删除当前中阶段已有的 Passed 任务。
9. 所有后台执行与恢复写回必须携带并核对 `execution_id`，旧任务不得覆盖新会话。
10. 执行启动时把项目 `execution_profile` 复制到 `execution_session.engine_snapshot`；同一次执行与恢复链路必须使用该快照，不得读取可能已被用户改写的项目 profile。
11. 启动执行前必须 `validate_profile` 并通过 `check_engine_health`；健康状态阻断时不得启动子进程。

---

## 15. 开发环境搭建

### 前置条件
- Rust 工具链（`cargo` + `rustc`）
- Node.js 20+（`npm` 或 `pnpm`）
- `git`（在 PATH 中）
- 至少一个可用执行引擎：
  - 默认：`claude` CLI（Claude Code，需已登录）
  - 可选：`codex` CLI（Codex，需已登录）
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
cd ~/metheus/src-tauri && cargo fmt --all -- --check
cd ~/metheus/src-tauri && cargo test
cd ~/metheus/src-tauri && cargo clippy --all-targets --all-features

# 前端测试、类型检查与构建
cd ~/metheus && npm test -- --run
cd ~/metheus && npx tsc --noEmit
cd ~/metheus && npm run build
```

---

## 16. 已完成阶段记录

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

`reconcile_execution_state` 必须区分以下情况：

| 情况 | 磁盘 execution_session | 内存 PipelineState | 动作 |
|------|----------------------|-------------------|------|
| 真执行/恢复中 | status="executing" 或 `"recovering"` | 同 `execution_id` 的 Running | 恢复轮询 |
| 待确认 | status="awaiting_confirmation" | 无或 Idle | 恢复确认界面 |
| 人工阻断 | status="quality_blocked" | 无或终态 | 保留 session 与 `RecoveryState::WaitingHuman`，显示明确人工动作 |
| 普通会话失联 | status="executing" | 无（进程已死） | 保留失败证据并要求恢复执行基线 |
| 恢复会话失联 | status="recovering" | 无（进程已死） | 将恢复阶段转回 Diagnosing，下次从基线安全重试 |
| 会话无效 | active=false 或字段缺失 | 无关 | 清理 execution_session，回到当前步骤 |
| 数据冲突 | 与当前 milestone/mid_stage 不匹配 | 无关 | cleanup，回 Discussion 或 Before |

### 17.4 禁止事项

- execution 恢复未完成前，禁止启动 autopilot 驱动循环
- 禁止前端自造"恢复执行态"
- 禁止刷新时删除 `recovering` 或 `quality_blocked` 会话及其失败证据
- 禁止 `handleChatComplete` 中旧修订/旧步骤/旧执行会话覆盖更新状态
- 禁止旧异步结果拉回新状态
- 禁止在 `RecoveryState::WaitingHuman` 下通过通用 autopilot resume 跳过人工处理

### 17.5 命令返回值规则（2026-07-19 固化）

**所有修改 `Project` 的后端命令必须统一使用 `save_and_reload_project` 模式：**

1. 从磁盘 `load_project` 获取当前事实
2. 修改内存中的 `proj` 结构
3. 调用 `save_and_reload_project(&proj)` 保存后重新读取磁盘
4. 返回磁盘最终 `Project`（非内存对象）

**例外必须写清原因和限制用途：**
- `execute_current_subtask`：两阶段保存模式（执行前保存 Executing，执行后保存 AwaitingConfirmation），返回 `PipelineState` 因为前端需要实时执行状态流
- `run_error_recovery`：长时恢复命令，按 Diagnosing/Repairing/Retesting 分阶段保存；最终返回磁盘重新加载的 `Project`
- 纯只读命令（`get_project`、`scan_existing_project` 等）不适用此规则

**2026-07-19 已修复的不一致点（历史记录）：**
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
- 权威来源：后端 `PipelineState.status === 'Running'`；磁盘恢复时同时识别活跃的 `execution_session.status === 'executing' | 'recovering'`
- 前端 `useState<boolean>` 仅作为兼容缓存，不得用于业务裁决
- 执行轮询开启条件 = 当前步骤为 `Execution` + 磁盘 session 活跃或后端内存 Running

### 17.7 恢复动作优先级（2026-07-21 固化）

前端只能按后端给出的单一 `AutopilotRecoveryAction` 展示主恢复动作：

1. `PrepareExecutionWorkspace`：显式初始化/准备 Git
2. `ResolveWorkspaceChanges`：用户在应用外处理完成后只读刷新
3. `RestoreExecutionBaseline`：恢复失败执行的 Git 基线
4. `RegenerateExecutionPlan`：重新生成不满足契约且尚未产生稳定执行事实的计划
5. `RetryAutopilotAdvance`：仅用于瞬时的非执行推进错误
6. `RunAutomaticRecovery`：进入持久化诊断、修复、复测循环
7. `WaitHumanDecision`：禁止通用恢复，只显示人工恢复出口或边界提示

准备 Git、只读刷新、恢复基线、人工恢复完成后，前端必须重新读取 Project、Git 工作区和 PipelineState；不得依赖旧前端缓存推断已恢复。

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
| `active` | bool | 托管是否激活 |
| `run_status` | ManagedRunStatus | Running / Paused / WaitingHuman / ErrorStopped |
| `managed_state` | string | 当前托管子状态（对应 WorkflowStep） |
| `managed_target` | string | 托管终点（当前固定为 `"MilestoneSelection"`，表示大阶段已批准） |
| `last_action` / `last_action_at` | string | 最近一次托管动作说明与时间 |
| `error_message` | string | 出错信息 |

### 18.4 托管层命令

| 命令 | 说明 |
|------|------|
| `start_managed_flow` | 从 ThreeChecks 启动托管 |
| `managed_next_step` | 执行下一步托管动作（只读顾问，返回原子命令） |
| `pause_managed_flow` | 暂停托管（仅暂停托管，保留当前步骤） |
| `wait_managed_flow_for_human` | 托管进入 WaitingHuman，等待人工处理 |
| `resume_managed_flow` | 恢复托管 |
| `stop_managed_flow` | 停止托管并交还手动控制 |
| `reconcile_managed_milestone_state` | 对账托管与大阶段草稿状态，修复不一致 |

### 18.5 托管层与 autopilot 边界

- **托管层**：ThreeChecks 后 → 方案草稿 → 方案批准 → Console → 大阶段生成/检查/批准（终点 `MilestoneSelection`）
- **autopilot**：大阶段批准完成后 → 中阶段生成/检查/批准 → 执行计划生成/检查/批准 → 小阶段执行/确认
- 大阶段批准完成（进入 `MilestoneSelection`）是托管层和 autopilot 的交接点
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

---

## 21. Phase: 错误自动纠正链路（2026-07-21 完成）

**范围边界**：只升级错误发生后的恢复分支，不重新设计已经稳定的正常规划与推进流程，不模仿无边界的通用 Auto 模式。

| 任务 | 当前实现 | 状态 |
|------|----------|------|
| 1. 修复恢复硬阻断 | 工作区刷新改为只读；Git 准备和基线恢复后可继续；相关动作统一重读 Project/Workspace/Pipeline | 已完成 |
| 2. 错误分类与状态模型 | `RecoveryErrorKind`、`RecoveryPhase`、持久化 `RecoveryState`、`RunAutomaticRecovery` | 已完成 |
| 3. 压缩诊断上下文 | 只注入当前目标、验收标准、精确文件范围、受限 diff、测试证据、执行错误和上次修复摘要 | 已完成 |
| 4. 受限自动纠错循环 | 诊断 → 最多 2 次修复 → 范围检查 → 复测 → 成功后回到 autopilot；重复签名提前停止 | 已完成 |
| 5. 人工处理出口 | 手动修复后复测、恢复基线并重试、重新生成当前计划、带原因的人工核验 | 已完成 |
| 6. UI、日志与回归测试 | 恢复阶段展示、恢复事件历史、前端动作策略测试、Rust 状态/对账/恢复测试 | 已完成 |

### 21.1 安全边界

- 测试/审查失败在当前代码改动上修复；执行器崩溃或进程失联先恢复执行基线。
- 范围外修改必须恢复基线并人工阻断；正常执行启动前的外部脏工作区必须阻断，测试/审查失败恢复只保留当前会话且位于授权范围内的改动。
- 测试未配置走代码审查通道；测试或审查服务不可用进入人工阻断。
- `allowed_file_paths`、`new_file_paths`、`base_commit` 和 `execution_id` 必须贯穿每次修复。
- 自动修复默认最多 2 次；相同错误签名连续出现 2 次时立即进入 `WaitingHuman`。
- 人工核验通过必须保存原因、时间和原始失败，不能把 `test_result.passed` 强制改为 `true`。
- `RecoveryStarted`、`ErrorDiagnosed`、`RepairAttemptStarted`、`RepairAttemptCompleted`、`RetestCompleted`、`RecoverySucceeded`、`RecoveryExhausted`、`HumanVerificationAccepted` 必须进入持久化执行历史。

### 21.2 当前验证基线

2026-07-21 本阶段完成时：

- `cargo fmt --all -- --check` 通过
- `cargo test`：76 passed，0 failed，1 ignored
- `cargo clippy --all-targets --all-features` 成功；仓库仍有不阻断构建的历史告警
- `npm test -- --run`：2 个测试文件、9 个测试通过
- `npm run build` 通过
- `git diff --check` 通过

唯一未执行的端到端验证是需要已认证 Claude CLI、网络和模型额度的 `real_claude_cli_smoke_test`；该测试继续显式标记为 ignored，不得据此宣称真实外部恢复调用已经验证。

---

## 22. Phase: 多引擎隔离与 autopilot 完善（2026-07-22）

**范围**：在不改动正常规划主链的前提下，把执行层从单一 `executor.rs` 重构为可插拔 `engine/`，并稳定 autopilot / recovery 与引擎快照的边界。

| 任务 | 当前实现 | 状态 |
|------|----------|------|
| 1. 删除单体 `executor.rs` | 执行能力收口到 `engine/`；pipeline/recovery 只依赖 `engine::execute` | 已完成 |
| 2. 统一执行契约 | `ExecutionProfile` + `ExecutionRequest` + `EngineError` + `EngineHealth` | 已完成 |
| 3. Claude Code / Codex 隔离适配 | 各自独立 `process_spec`；公共流式运行器 `process_runner` | 已完成 |
| 4. Grok Build 预留 | `BuiltIn + GrokBuild` 合法但 Disabled / Unavailable | 已完成 |
| 5. 健康检查与切换阻断 | `check_engine_health` / `update_execution_profile`；执行中、恢复中、autopilot/托管 Running 不可切换 | 已完成 |
| 6. 会话引擎快照 | `ExecutionSession.engine_snapshot`；恢复/修复沿用快照 | 已完成 |
| 7. 前端引擎 UI 与策略 | `ExecutionEngineSelector` / `ExecutionEngineSettings` / `enginePolicy` | 已完成 |
| 8. 恢复重规划 | `RecoveryPhase::Replanning`；常规修复耗尽后仅一次当前小阶段受限重规划 | 已完成 |
| 9. 前端策略模块补齐 | `workspacePolicy` / `managedFlowPolicy` / `logPolicy` 及对应测试 | 已完成 |

### 22.1 引擎组合规则

| runtime | provider | 状态 |
|---------|----------|------|
| `Plugin` | `ClaudeCode` | 默认可用；依赖 `claude` CLI |
| `Plugin` | `Codex` | 可选可用；依赖 `codex` CLI |
| `BuiltIn` | `GrokBuild` | 预留，健康检查 Disabled，执行 Unavailable |
| 其他组合 | — | `validate_profile` 拒绝 |

- 后台流水线只接受 `PermissionProfile::Unattended`。
- 入口页可选择初始 profile；Console 内通过设置弹窗更新，必须带 `expected_data_revision` 乐观锁。
- 业务模块禁止直接拼装 `claude` / `codex` 参数；新增引擎只允许在 `engine/` 内增加适配器。

### 22.2 与错误恢复的交界

- 自动修复和重规划执行必须使用会话 `engine_snapshot`，防止用户中途切换引擎污染恢复。
- 恢复阶段为 Diagnosing / Repairing / Retesting / Replanning 时禁止切换引擎。
- `Replanning` 只覆盖当前失败小阶段，不得重写整个中阶段或已 Passed 任务。

### 22.3 验证说明

2026-07-22 代码面：

- Rust 侧测试清单约 101 项（含 engine health/profile、process_runner、recovery replan 等）
- 前端策略测试覆盖 autopilot / engine / log / managedFlow / workspace
- 真实 Claude/Codex 外部调用仍依赖本机认证与额度，不得把未跑通的外部 smoke 写成已验证

---

## 23. Phase: 自适应纠错证据一致性收口（2026-07-23）

**范围**：让验收账本、回归检查点、滚动校准、项目事实和纠错经验始终服从当前磁盘工作区，避免已经撤销或过期的证据继续驱动代码修改。

| 任务 | 当前实现 | 状态 |
|------|----------|------|
| 1. 回归撤销证据一致性 | 修复结果先暂存；新增回归恢复文件检查点后清除暂存证据，并对恢复后的真实工作区重新测试 | 已完成 |
| 2. 滚动校准 CAS | AI 调用前记录 revision、任务、步骤、autopilot、Git HEAD 和事实指纹；提交时持有 pipeline 锁重读并逐项核对 | 已完成 |
| 3. 失败责任域 | `PlanFailure` 进入一次受限重规划；`ValidationFailure` 只重建一次证据，不直接修改代码 | 已完成 |
| 4. 纠错经验隔离 | 以精确标识符、验收契约指纹或同失败域下的高相似签名匹配，单纯同文件不命中 | 已完成 |
| 5. 定向项目事实 | 全文件哈希检测漂移，并提取任务标识符上下文、DOM、storage、事件和行内脚本符号 | 已完成 |

### 23.1 恢复与证据规则

- 修复引擎的输出在复测确认前只保存在 `RecoveryState.pending_execution_result`；无新增回归后才写入任务 `execution_result`。
- 新增回归必须先恢复本轮文件检查点，再清除被撤销代码对应的测试结果和验收账本，并设置 `rollback_retest_pending`。下一次恢复动作必须先真实复测，禁止直接继续修代码。
- `ValidationFailure` 最多自动重建一次测试/审查证据；仍无法可靠映射验收项时进入 `WaitingHuman`。
- `PlanFailure` 仅在执行成功、审查证据完整但问题无法绑定当前不可变任务契约时产生，并进入当前任务的一次受限重规划。
- 验收账本中存在任一 `Unknown` 都表示证据不足；其他验收项的通过结论不能掩盖未证明项。

### 23.2 校准与学习边界

- `calibrate_next_subtask_command` 的 AI 调用在锁外执行，最终提交必须持有 pipeline 锁并重新加载磁盘事实；revision、任务、步骤、autopilot、Git HEAD 或结构指纹任一变化即丢弃旧补丁。
- 正常执行不会仅因文件路径相同而注入纠错经验。恢复期间的文件兜底还必须同时满足相同失败域和失败签名高相似度。
- `ProjectFactSnapshot.identifier_contexts` 只保存有限的任务相关上下文，完整文件继续只用于哈希和机械事实提取，不向 AI 注入全量大文件。
- `WaitingEngine` 允许在健康检查通过后切换项目 profile；恢复确认清除旧会话，下一次执行从当前 profile 创建新的 `engine_snapshot`。

### 23.3 当前验证基线

2026-07-23 本阶段收口时要求：

- `cargo fmt --all -- --check` 通过
- `cargo test` 全部通过
- `cargo clippy --all-targets --all-features` 通过（历史非阻断告警除外）
- `npm test -- --run` 全部通过
- `npm run build` 通过
- `git diff --check` 通过

真实 Claude/Codex 外部调用仍依赖本机认证、网络与额度，不得将本地状态机测试等同于外部 smoke 验证。
