# Metheus — 项目宪法

> 弥 · 复杂任务编译系统 — 用精准上下文注入和滚动宪法，把模糊想法编译成可执行、可检查、可回退的代码变更。

> 最后同步：2026-08-06。本文描述 Metheus 仓库当前实现；`src-tauri/CONSTITUTION.md` 是测试/目标项目生成出的项目宪法，不是本仓库的开发约束来源。

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
| **OpenAI Compatible API** | 所有 AI 角色对话、任务拆解和质检的可配置决策模型接口；默认配置保持 DeepSeek 行为 |
| **执行引擎抽象层 (`engine/`)** | 统一执行契约；插件适配 Claude Code / Codex / Kimi / Grok Build，并接入 Grok Build Rust 进程内运行时 |
| **Claude Code CLI (`claude`)** | 默认插件执行引擎，本地子进程执行已批准 `execution_prompt` |
| **Codex CLI (`codex`)** | 可选插件执行引擎，与 Claude Code 隔离适配，互不耦合参数 |
| **Kimi CLI (`kimi`)** | 可选插件执行引擎，使用非交互 `stream-json` 输出 |
| **Grok Build CLI (`grok`)** | 可选插件执行引擎，使用无人值守 `streaming-json` 输出；不读取预装 Grok Build API Key |
| **Grok Build 内置运行时** | `builtin-grok` / `full-product` 特性显式启用的受控 Fork 进程内链路；日常默认构建使用不引用 Grok 类型的轻量占位边界 |
| **`metheus-grok-engine`** | 可选的 Metheus 防腐适配层；调用受控 Fork facade，桥接设置、凭据、事件、恢复快照与类型化错误；只在内置特性启用时进入依赖图 |
| **Grok Build 双源码树** | `third_party/grok-build` 是固定修订的原样审计基线；`third_party/grok-build-fork` 是逐文件登记理由的最小受控 Fork |
| **Git（程序化操作）** | 版本控制：只提交任务授权路径，并为完成的小阶段/中阶段创建不可覆盖标签 |
| **reqwest** | Rust HTTP 客户端，调用 OpenAI Compatible 决策模型接口 |
| **keyring** | 跨平台系统凭据库；持久化决策模型和预装 Grok Build API Key，设置文件只保存非敏感配置 |
| **serde/serde_json** | 所有数据结构的序列化/反序列化 |
| **tokio** | Rust 异步运行时（Tauri 命令均为 async，子进程使用 `tokio::process::Command`） |
| **uuid** | 所有实体（Milestone/MidStage/Subtask/Message）的 ID 生成 |
| **chrono** | 时间戳生成 |
| **walkdir** | 项目文件树遍历 |
| **dirs** | 获取系统 home 目录 |
| **dotenvy** | 从 `.env` 加载 API_KEY 等环境变量 |

### 关键架构原则

- **前端不直接调用任何 AI API**，所有 AI 调用必须经过 Rust 后端
- **依赖引入原则**：新增 Cargo 或前端依赖必须在 PR 说明中注明用途、许可证（须与 MIT/Apache/AGPL 兼容）、是否引入远程通信能力、是否增加超过 10MB 的二进制体积。禁止引入商业闭源依赖和不兼容 AGPL 的许可证。
- **允许使用纯前端交互组件库**（如 Radix UI）；桌面前端不得直接接入 Node.js 原生模块或数据库驱动，复杂状态能力优先沿用现有后端事实源与 React 状态模型
- **允许使用图标组件库**（如 lucide-react）
- **当前安装的前端组件**：@radix-ui/react-dialog, @radix-ui/react-tabs, @radix-ui/react-tooltip, lucide-react。已有对应能力的交互禁止手写替代品。
- **不使用复杂状态管理库**（Redux、Zustand 等），只用 React 自带的 `useState` / `useEffect`
- **不使用路由库**，视图由统一工作流状态控制
- **桌面进程通信原则**：Metheus 是桌面应用，进程内通信使用 Tauri IPC/Channel，不引入面向远程网络的通信协议（如 WebSocket）
- **应用级引擎健康同步原则（2026-08-06 固化）**：执行引擎健康、运行时自检结果和进程内自检缓存属于应用级事实，不得写入 `Project`、伪装成项目业务事件或借用 Project State Channel 传播。健康事实变化后，必须通过应用级 IPC/进程内事件或等价失效通知，让所有已挂载且目标匹配的健康消费者无需 Reload 即可重新检查；并发检查继续使用请求序号保护，旧响应不得覆盖新响应。
- **`project.rs` 只定义数据结构**，业务逻辑分散在各功能模块中
- **Rust 端 `project.rs` 与前端 `types.ts` 的数据结构必须保持一一对应**
- **统一工作流状态是业务页面和按钮权限的唯一判断依据**。旧的 Project.status、viewMode 和 isExecuting 只能作为兼容或纯视觉状态。
- **所有业务事实必须由后端确认并持久化**。前端不得通过临时对象或完整项目覆盖完成关键业务变更。
- **禁止前端通过 persist_project 任意提交完整项目对象完成关键业务状态变更**。每个审批、检查、生成、执行和回退动作必须调用对应的后端业务接口。
- **治理模式必须显式区分**：手动模式逐步点击；Managed Flow 只覆盖 ThreeChecks 后到大阶段批准；autopilot 只覆盖已批准大阶段内部流程。任何模式都不得绕过检查、批准、文件范围和 Git 安全边界。
- **任务控制模式事实（2026-08-03 固化）**：新项目默认使用 `SerialTakeover` 模式，新控制器真实接管任务执行阶段。`Shadow` 为对照审计/回退模式，`Legacy` 为历史兼容模式。
- **自动驾驶（autopilot）语义（2026-07-15 固化）**：autopilot 只在大阶段边界（`MilestoneReview`）停下由人做 A/B/C；大阶段内部的中阶段生成/检查/批准、执行计划生成/检查/批准、执行、确认全部自动代点；只保留暂停键；执行中暂停等同 In Stop 回退到最近已完成小阶段；autopilot 自动选择下一个未完成大阶段，用户不手选；autopilot 永不自动做 A/B/C 决策。
- **执行引擎隔离原则（2026-07-25 固化）**：执行层通过 `engine/` 抽象，禁止业务代码直接拼装具体 CLI 参数或引用 `metheus_grok_engine`。`ExecutionProfile` 描述 runtime/provider/permission；执行开始后冻结 profile、应用设置修订、模型、API 后端、接口指纹、内置源码修订或插件可执行路径。合法组合：`Plugin + ClaudeCode`、`Plugin + Codex`、`Plugin + KimiCli`、`Plugin + GrokBuild`、`BuiltIn + GrokBuild`。默认特性为空；只有 `builtin-grok` 或包含它的 `full-product` 才编译内置运行时。轻量模式保留旧项目选择但以 `Disabled` 明确阻断，插件路由不受影响。恢复时任一快照事实不一致必须进入 `WaitingEngine`，经用户明确确认后才能使用新配置。
- **执行器参数边界（2026-08-03 固化）**：执行引擎的具体参数由各适配器负责，宪法只约束安全边界：禁止执行授权范围外的文件修改，禁止越权操作。
- **应用设置与密钥原则（2026-07-23 固化）**：非敏感设置持久化到 `~/.metheus/config/app-settings.json`；决策模型和预装 Grok Build API Key 可安全保存到系统凭据库，或由用户明确选择仅本次会话使用；环境变量仅作兼容回退。密钥不得进入项目、设置文件、执行会话、日志、错误文本或前端持久状态。
- **应用设置活动租约原则（2026-08-06 固化）**：决策请求和执行操作的活动租约必须覆盖后端真实请求生命周期，并在成功、失败或其他终态离开作用域时释放；任一活动租约存续期间继续阻断设置修改。前端等待超时只结束本次前端等待，不取消后端任务、不中止或清零活动租约；禁止通过计时器、强制解锁入口或猜测逻辑修改活动计数。
- **稳定性原则（2026-07-15 固化）**：不再保留任何"执行前重新生成提示词 / 固定管线自动重拆"的路径；执行端只执行用户或 autopilot 已确认的既定计划（`execution_prompt`），杜绝 AI 歧义。
- **确定性验证优先原则（2026-08-02 固化）**：路径、字段、依赖、环、顺序、重复和其他可由本地规则判定的事实，必须先由确定性检查裁决，禁止交给 AI 反复挑刺。AI 只补充本地规则不能可靠判断的语义、设计和体验结论；验证深度按变更风险、证据缺口和影响范围伸缩，不使用固定长度流水线代替风险判断。
- **自适应执行原则（2026-08-03 固化）**：任务的检查深度、验证方式和执行步骤按任务复杂度和风险自适应决定。简单任务可一步执行并通过本地确定性验证；高风险任务才需要多轮计划检查和 AI 语义审查。
- **工作负载画像原则（2026-08-06 固化）**：项目在首项检查中形成后端确定性计算的 `Micro / Small / Standard / System` 工作负载画像；画像是执行拓扑、各层数量上限、检查深度和执行预算的唯一事实源。项目体量、单任务复杂度与风险分别计算；小范围高风险工作保持浅层，只提高验证强度。画像缺失或讨论修订不匹配时必须明确阻断，禁止默认解释为任一规模。
- **Grok Build 受控借用边界（2026-08-06 固化）**：业务模块不得直接引用 `metheus_grok_engine`；唯一进程内链路为 `engine/builtin.rs -> metheus-grok-engine -> metheus_embedded -> SessionActor/sampler`。只复用受控 facade 暴露的执行、重试、Doom Loop、事件与类型化错误能力，不得借此开放 Shell、terminal fallback、网页工具、MCP、插件、技能、记忆、Hook 或子代理。
- **检查结论分级原则（2026-08-02 固化）**：检查结果必须区分硬阻断与建议项。缺失必需产物、越权、契约不满足、不可执行和依赖错误属于硬阻断；优化方向、“可考虑”、可选增强和非必需 criteria 只能属于建议。只有硬阻断可以使检查不通过、触发重生成或进入人工停止，建议不得改变 `passed`。
- **Git 确认事务原则（2026-07-25 固化）**：小阶段与中阶段 V2 标签由大阶段、中阶段、小阶段实体 ID 和稳定确认事务 ID 组成；版本、标题和序号只用于展示。提交、标签和项目收口分阶段持久化，重试复用同一事务和提交。Git 确认受阻必须保留代码与质量结果，不得依据工作区脏状态误分类为执行失败或恢复基线。
- **不可变标签审计原则（2026-07-25 固化）**：V1/V2 标签均不得删除、覆盖或移动；标签树和回滚使用项目保存的实际标签，回退只调整工作树与项目引用。
- **本轮真实可体验闭环目标**：No Project 和 Half Project 都能走到正式执行；三项检查无法绕过；In Stop 和 ED Stop 都能真实体验；回退有影响预览；大阶段 A、B、C 都能完整走通；任意关键状态刷新后可以恢复。
- **V1 手动治理模式**：批准计划不等于在手动模式下自动执行。每个小阶段必须经历"用户点击执行 → 自动验证 → 用户确认结果"后才允许写 Git 稳定标签。`PlanApproval` 是方案审批页面，不代表方案已经批准。autopilot 激活后可代点大阶段内部的合法步骤，但仍不得越过大阶段 A/B/C 人工决策。
- **决策模型任务边界**：所有对话、检查、方案、大阶段、中阶段和执行计划编译使用应用设置中的同一 OpenAI Compatible 决策模型快照。所有模型任务必须有明确边界——单一目标、允许文件范围、上下文证据、验收标准、禁止扩展范围、信息不足时停止规则。返回结果必须经过结构化解析和本地字段校验；缺字段、范围越界、任务空白、检查失败均不得进入下一步。
- **Console 后端最终事实规则**：关键业务命令必须先持久化，再返回从磁盘重新读取的完整 `Project`；后端持久化后的 `Project` 是唯一业务事实。
- **Console 前端同步规则**：前端必须通过统一入口校验并应用后端返回的完整 `Project`，不得让较旧修订覆盖较新修订，也不得使用临时候选列表或 `persist_project` 拼装业务事实。
- **Console 超时协调规则**：AI 命令的前端等待时间必须长于后端 HTTP 超时并预留解析、持久化时间。前端等待超时不等于业务失败或后端取消，不得自动重发生成命令；必须有限次读取磁盘项目协调最终状态，并提供只调用 `get_project` 的手动同步入口。
- **Tauri 命令超时策略规则（2026-08-06 固化）**：普通有界调用的完整命令名、单层 `_runtime` 别名与基础命令必须共享同一套可审计策略，保留各基础命令按风险配置的现有预算。解析顺序固定为调用点显式超时、完整命令精确策略、仅剥离一个末尾 `_runtime` 后的基础策略、最终默认值；新增或漏配的有界运行时命令必须由确定性静态门禁暴露，流式 Channel 和经审计的显式调用点例外必须登记，不得静默依赖默认回退。

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
│             ExecutionEngineSettings | ApplicationSettings]    │
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
 OpenAI Compatible API   执行引擎插件/内置                 Git (本地)
 (HTTP/json)             Claude/Codex/Kimi/Grok CLI       (Command 调用)
                         (受控 Fork SessionActor 进程内运行)
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
engine/ ──→ settings, pipeline(类型), test_runner, constants, project(ExecutionProfile)
git_ops ──→ project
constitution ──→ api, prompts, constants, project
test_runner ──→ api, prompts, json_utils, project
snapshot ──→ project, AppState
```

---

## 8. 模块清单（当前实现与目标路径）

> **说明**：以下模块清单反映 2026-07-23 的当前代码状态；历史兼容入口仍存在，但不得参与新恢复链路的业务裁决。`executor.rs` 已删除，执行能力统一收口到 `engine/`。

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
| 常量 | 决策模型默认地址/模型/超时、应用设置路径、系统凭据服务与账户名、决策模型与预装 Grok Build 环境变量名、预装 Grok Build 默认地址/模型/轮数、`EXECUTION_ENGINE_TIMEOUT_SECS`=600、Git 与宪法常量 |

### `src-tauri/src/settings.rs` — 应用设置与安全密钥
| 职责 | 非敏感设置校验和原子持久化、乐观修订锁、决策/执行活动租约、系统凭据库、会话密钥和环境变量回退、接口指纹 |
| 持久化 | `~/.metheus/config/app-settings.json`，不得包含真实 API Key |

### `src-tauri/src/api.rs` — OpenAI Compatible API 封装
| 函数 | 说明 |
|------|------|
| `call_deepseek_api*` | 保留既有业务入口，内部读取一致的设置和密钥快照 |
| `send_openai_compatible` | 按配置发送普通文本或结构化 OpenAI Chat Completions 请求；正文先按独立超时读取原始字节，再区分网络截断、服务非 JSON 和协议错误并输出脱敏诊断前缀 |
| `test_model_connection` | 返回模型、延迟和脱敏错误类别，不返回密钥 |

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
| 职责 | 识别并执行真实测试命令，压缩输出，采集 tracked/untracked 变更证据，独立保存自动化测试事实与标准/定向/扩展定向 AI 审查结论 |
| 关键函数 | `check_subtask`, `check_subtask_with_context`, `run_test_command`, `summarize_test_output`, `format_test_result`, `is_test_not_configured`, `get_file_snapshot`, `detect_changes` |

### `src-tauri/src/acceptance.rs` — 逐验收项账本
| 职责 | 校验证据引用并优先使用 `criterion_reviews` 裁决满足、不满足、未知与契约冲突；兼容没有逐项结果的旧记录 |

### `src-tauri/src/quality_gate.rs` — 统一质量门禁
| 职责 | 为正常执行、确认和恢复复测统一输出通过、代码不满足、证据不足、契约冲突、审查震荡与测试不可用 |

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
| `mod.rs` | 模块出口：`prepare_engine`、`execute`、`validate_profile`、`check_engine_health` 和公共契约 |
| `contract.rs` | 统一契约：健康状态、配置错误、程序来源、原始文本/JSONL 输出协议和进程规格 |
| `service.rs` | 以一次设置租约冻结健康检查到进程完成；按 runtime/provider 双维路由；注入文件范围约束 |
| `process_runner.rs` | 通用子进程运行器：环境覆盖、JSONL 映射、流式输出、暂停取消、规格超时、PID 清理 |
| `claude_code.rs` | Claude Code 适配器：具体 CLI 参数由适配器维护，并受统一授权路径与越权阻断边界约束 |
| `codex.rs` | Codex 适配器：`codex exec … --sandbox danger-full-access -`（prompt 走 stdin） |
| `kimi_cli.rs` | Kimi CLI 适配器：无人值守 prompt、`stream-json` 与必需能力探测 |
| `grok_cli.rs` | Grok Build CLI 适配器：无人值守、禁用 memory/subagent/web search、`streaming-json` |
| `builtin.rs` | 仅在 `builtin-grok` 启用时接入 `metheus-grok-engine`；健康检查、源码修订、自检缓存、取消、事件桥和类型化错误映射 |
| `builtin_disabled.rs` | 默认轻量构建的同接口占位边界；不引用 Grok 类型、不读取密钥、不发送网络请求 |
| `health.rs` | 使用设置路径覆盖或 PATH/PATHEXT 探测可执行文件、版本、认证和无人值守能力 |

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
| `ExecutionProvider` | GrokBuild / ClaudeCode / Codex / KimiCli |
| `PermissionProfile` | Interactive / Unattended（后台流水线仅 Unattended） |
| `ExecutionProfile` | 项目级执行配置：runtime + provider + permission_profile + profile_revision |
| `EngineHealth` / `EngineHealthStatus` / `EngineAuthState` | 引擎健康探测结果，含认证、本地能力、内置源码修订和运行时自检状态 |
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
| `ExecutionResult` | 执行引擎统一输出（含 provider、runtime、settings/source revision、API backend、exit_code、file_changes） |
| `ExecutionSession` | 活跃执行会话；冻结引擎 profile、设置修订、模型、接口指纹和可执行路径 |
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

### 当前安装的前端交互依赖

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

### 前端架构边界

- 前端状态管理库（Redux, Zustand, MobX 等）
- 路由库（React Router, TanStack Router 等）
- 网络请求库（axios, SWR, TanStack Query 等）
- 完整主题/UI 框架（Tailwind, Ant Design, Material UI, Chakra UI 等）
- Node.js 原生模块、数据库驱动等不得直接进入桌面前端运行时；确需新增的后端能力必须经 Rust 适配层和上述依赖引入审查

---

## 11. 外部依赖（非代码库）

| 依赖 | 用途 |
|------|------|
| **`claude` CLI** | 默认插件引擎（`ExecutionProvider::ClaudeCode`）；需在 PATH 中且已登录 |
| **`codex` CLI** | 可选插件引擎（`ExecutionProvider::Codex`）；需在 PATH 中且已登录 |
| **`kimi` CLI** | 可选插件引擎（`ExecutionProvider::KimiCli`）；需支持 yolo、prompt 和 stream-json |
| **`grok` CLI** | 可选插件引擎（Plugin + GrokBuild）；需支持无人值守和 streaming-json |
| **`git`** | 版本控制 |
| **OpenAI Compatible API** | 可配置决策模型后端（对话、检查、规划、审查） |
| **Rust 工具链** | 编译后端 |
| **Node.js 20+** | 前端构建 |

> 执行引擎按项目 `ExecutionProfile` 选用，不是同时强制依赖全部 CLI。Grok Build CLI 与 Grok Build 内置模式是两条隔离路由，认证、配置和执行方式互不复用。

---

## 12. 错误处理策略

| 场景 | 处理方式 |
|------|----------|
| **决策模型密钥缺失** | 提示在应用设置填写，或使用 `METHEUS_DECISION_API_KEY` / 兼容 `API_KEY` 环境变量 |
| **OpenAI Compatible API 超时** | 使用应用设置超时并返回分类错误 |
| **决策模型返回非 JSON** | 宽松规划路径继续先清洗再修复；强类型路径必须携带目标字段、类型、枚举和必填契约 |
| **强类型 JSON 字段类型不匹配** | 提取真实字段路径、期望类型和实际类型，只允许一次契约修复；无进展时确定性归一化或返回结构化协议失败，不得重复三次同一请求 |
| **审查协议失败** | 归类为协议异常，不得归类为代码缺陷，也不得消耗代码修复次数 |
| **应用设置并发修改** | 以 revision 乐观锁拒绝旧写；决策请求或执行操作租约存续期间拒绝修改。租约覆盖后端真实请求并在终态按所有权释放，不得因前端等待超时而取消、清零或强制解锁 |
| **恢复设置快照漂移** | 修复前核对设置修订、模型、地址指纹和程序路径；不一致进入 `WaitingEngine` 且不消耗修复次数 |
| **控制动作锁残留** | 使用进程启动标识、心跳、开始时间和最长时长判定租约；启动、项目加载和手动同步由后端对账清理陈旧锁并写入审计 |
| **另一进程持有新鲜控制锁** | 保持有效互斥并提示冲突，不得抢锁；心跳过期或超过最长时长后才进入陈旧锁对账 |
| **执行引擎不可用** | 健康检查阻断 NotInstalled / Unauthenticated / UnsupportedVersion / VerificationRequired / VerificationFailed / Disabled；轻量构建选择内置 Grok 时返回稳定的 `Disabled`，禁止启动执行或静默切换引擎 |
| **应用级引擎健康变化** | 自检或健康缓存变化后发送目标明确的应用级失效通知，由所有已挂载且匹配的消费者重新检查；不得写入 Project 或伪造项目事件，旧健康响应不得覆盖新请求结果 |
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
| **Console 前端等待超时** | 不标记为生成失败、不自动重发、也不视为后端取消或活动租约释放；有限次调用 `get_project` 协调磁盘最终状态，结束后提供手动同步 |
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

### 12.3 大阶段内无人值守自动驾驶

1. 自动驾驶推进权属于 Rust 后端作业运行器。React 只能启动、暂停、恢复、关闭、同步和展示状态，不得选择或执行下一原子动作。
2. 项目磁盘状态是唯一事实源。运行器每轮动作前必须重新加载项目，并以 `job_id`、`job_generation` 和动作标识拒绝旧作业回写；同一项目同一代次最多一个运行器。
3. 中阶段草稿与执行计划检查失败后必须进入有限重生成。两次仍失败、失败指纹重复、问题没有减少或出现契约矛盾时，停止并等待人工决策。
4. 网络、限流、服务暂时不可用、超时、进程崩溃和修订冲突最多自动重试三次，使用递增退避；认证、额度、外部工作区修改、范围违规、契约矛盾和 Git V2 完整性冲突不得自动重试。
5. `CommitFailed`、`TagFailed`、`ProjectFinalizationFailed` 和临时 Git 元数据错误只允许对同一幂等确认事务自动续跑两次；代码和质量结果必须保留，不得创建重复提交。
6. 前端刷新或长命令等待不得终止后端作业。控制命令保持短超时，长规划、执行与恢复只通过持久化动作、心跳、重试截止时间和轻量状态同步展示。
7. 自动驾驶只运行到当前大阶段审阅边界。额度不足、认证失效、外部工作区修改、契约矛盾、重试耗尽和大阶段 A/B/C 审阅必须停止派发新动作。
8. 控制动作租约的有效性与清理由 Rust 后端裁决。长时间执行、审查、修复和 Git 确认必须持续刷新心跳；心跳写入失败只记录脱敏诊断，不得中断正在收口的动作。
9. 锁占用展示独立于 `RecoveryState`：有效占用只能等待，陈旧占用只能执行后端清理；没有 `RecoveryState` 时禁止展示人工恢复决策入口。

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
6. 自动化测试事实与 AI 代码审查分别保存；正常执行、人工确认和恢复复测必须调用同一个 `quality_gate`，统一裁决通过、代码不满足、证据不足、契约冲突、审查震荡和测试不可用。
7. 验收账本优先使用逐验收项审查结果：有效证据支持的满足项不受全局文件 `Partial` 影响；只有有效阻断证据可判为不满足，重复编号、无效引用或证据不足均为未知。
8. 未知验收项最多依次执行一次 `Targeted` 和一次 `ExpandedTargeted` 补证；补证不得调用编码引擎、创建代码检查点、消耗修复次数或触发重规划。两次仍不足则等待人工。
9. 每个小阶段确认通过后才写回 Passed 状态和 Git 稳定标签；autopilot 可代点确认，手动模式仍由用户确认。
10. 已 Passed 的任务不得再次执行；重新规划不得静默删除当前中阶段已有的 Passed 任务。
11. 所有后台执行与恢复写回必须携带并核对 `execution_id`，旧任务不得覆盖新会话。
12. 执行启动时把项目 `execution_profile` 复制到 `execution_session.engine_snapshot`；同一次执行与恢复链路必须使用该快照，不得读取可能已被用户改写的项目 profile。
13. 启动执行前必须 `prepare_engine`，在同一设置租约内完成 profile 校验、健康检查、会话快照和实际执行；健康状态阻断时不得启动子进程。
14. 自动修复前必须核对会话设置快照；设置或可执行路径漂移时进入 `WaitingEngine`，不得静默使用新配置。

### 14.1 运行期执行路径强制接入点

1. **受管改动语义**：工作区改动是否受管，判据是“改动文件是否在当前叶子的授权路径内”，与 Git HEAD 是否等于执行会话基线 commit 无关。HEAD 前进（叶子已 commit）不影响后续叶子的受管路径识别；`base_commit` 仍用于恢复和确认事务。
2. **split 按独立产物拆分**：split 的合法依据只能是独立产物、独立验收范围或明确依赖顺序。禁止按验收项数量或 `required_identifiers` 机械拆分；单文件单功能无法证明独立边界时直接执行，单次 split 最多生成 4 个叶子。
3. **新增执行路径收口**：新增执行路径（包括新引擎适配、split 叶子和任务树新路径）必须同时接入受管改动识别、成本账本写记录、事件聚合后推送、JSON 响应容错读取。这四项是架构接入的强制检查点。
4. **Grok Build 内置 token 可见**：Grok Build 内置引擎执行后必须向成本账本写调用记录。供应方不返回 usage 时，记录调用次数和耗时，token 显示“未知”。
5. **流式执行事件聚合**：流式执行引擎（含 Grok Build 内置）的文本 token 必须在适配层聚合后推送，禁止 token 级别事件上报到日志面板；工具调用、完成和错误等结构化事件仍独立推送。

---

## 15. 开发环境搭建

### 前置条件
- Rust 工具链（`cargo` + `rustc`）
- Node.js 20+（`npm` 或 `pnpm`）
- `git`（在 PATH 中）
- 至少一个可用执行引擎：
  - 默认：`claude` CLI（Claude Code，需已登录）
  - 可选：`codex` CLI（Codex，需已登录）
  - 可选：`kimi` CLI（Kimi CLI）
  - 可选：`grok` CLI（Grok Build CLI）
- OpenAI Compatible 决策模型 API Key

### 启动命令

```bash
# 1. 安装前端依赖
cd ~/metheus && npm install

# 2. 开发模式启动
cd ~/metheus && cargo tauri dev
```

启动后通过右上角“应用设置”填写决策模型接口和会话 API Key。密钥只在进程内保存；不得写入项目文件或 `app-settings.json`。也可在启动进程环境中提供 `METHEUS_DECISION_API_KEY`。

### 验证命令

```bash
# 日常 Core：格式和主 Rust 库类型检查，不包含内置 Grok
cd ~/metheus && npm run verify:core-light

# 修改验收/恢复逻辑后：定向 Rust、TypeScript 和前端策略测试
cd ~/metheus && npm run verify:quality

# 修改控制动作锁、恢复展示或强类型 JSON 协议后：运行期故障注入门禁
cd ~/metheus && ./scripts/verify-runtime-fault-recovery.sh

# Grok 专项：单任务库类型检查，不执行最终链接
cd ~/metheus && npm run verify:grok-check
```

聊天交互专项使用独立的 Core 轻量轨道：

```bash
cd ~/metheus && npm run verify:chat-ux
```

该脚本只运行聊天 API/运行状态/持久化 Rust 定向测试、前端策略与生命周期测试、TypeScript 检查和 Vite 生产构建；固定使用 `.build/core`、最多两个 Cargo 任务和 `--no-default-features`，不编译 Grok Build、不启动 Tauri、不发送真实模型请求。

默认开发构建不包含内置 Grok。发布级验证必须在高资源环境显式启用 `full-product`，再执行完整 Rust、前端和 Tauri 打包门禁；发布命令不得混入上述日常轨道，也不得以 Grok 专项 `cargo check` 代替最终产品验收。

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
3. 控制动作租约对账（仅后端判断有效、陈旧和 Git 幂等续跑事实）
4. `execution` 对账（`reconcile_execution_state`）
5. `autopilot` sanity（检查 autopilot_state 与当前步骤自洽）
6. `snapshot` 恢复（`restore_snapshot`）
7. 解锁界面（释放 `startupRecoveryDoneRef`）

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

控制动作租约对账必须额外满足：新鲜本进程租约保持占用；新鲜异进程租约保持互斥并提示冲突；心跳超时、超过最长执行时长或旧字符串锁释放后写入结构化清理事件。Git 确认锁只负责解除阻断，提交是否已完成必须交给既有幂等确认事务判断，禁止根据锁状态创建新提交。

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

## 20. Phase: 最终收尾施工（2026-07-19 启动，历史记录）

**历史范围**：该阶段当时不扩新功能，只做既有能力的稳定收尾，并按 P0 → P1 → P2 顺序推进。下列施工方式和构建结果仅记录 2026-07-19 阶段背景与已完成事实，不再约束后续施工；当前依赖治理遵循第 2 节“依赖引入原则”，当前验证按第 26 节 Core/Grok 分轨及各专项轻量门禁执行，不以完整 Rust/Tauri 构建作为每个阶段的统一硬门槛。

### 当时施工规则（仅历史，不作为当前硬约束）
1. 该阶段第一个子任务先更新了 `CONSTITUTION.md`。
2. 该阶段没有改动 `Cargo.toml`、`package.json` 或锁文件，也没有新增依赖；后续依赖变更改由第 2 节现行原则审查。
3. 关键业务事实由后端持久化并返回完整 `Project`，该架构原则仍由第 2 节现行条款约束。
4. 前端不使用 `persist_project` 拼装关键状态，该架构原则仍由第 2 节现行条款约束。
5. P0、P1、P2 当时分别完成了前端与 Rust 构建检查；下方勾选项保留实际历史结果，但不构成后续工单必须运行完整构建或自动进入下一阶段的规则。

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
| 3. 插件隔离适配 | Claude Code / Codex / Kimi / Grok Build 各自独立 `process_spec`；公共流式运行器 `process_runner` | 已完成 |
| 4. Grok Build 预装边界 | `BuiltIn + GrokBuild` 作为合法组合预留，后由 Phase 25 完成真实嵌入 | 已完成并被后续实现替代 |
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
| `Plugin` | `KimiCli` | 可选可用；依赖支持无人值守 stream-json 的 `kimi` CLI |
| `Plugin` | `GrokBuild` | 可选可用；依赖支持无人值守 streaming-json 的 `grok` CLI |
| `BuiltIn` | `GrokBuild` | 可用；依赖 Metheus 设置中的模型接口、API Key 和已通过的运行时自检 |
| 其他组合 | — | `validate_profile` 拒绝 |

- 后台流水线只接受 `PermissionProfile::Unattended`。
- 入口页可选择初始 profile；Console 内通过设置弹窗更新，必须带 `expected_data_revision` 乐观锁。
- 业务模块禁止直接拼装任何供应方 CLI 参数；新增引擎只允许在 `engine/` 内增加适配器。

### 22.2 与错误恢复的交界

- 自动修复和重规划执行必须使用会话 `engine_snapshot`，防止用户中途切换引擎污染恢复。
- 应用设置修订、模型、地址指纹或可执行路径不一致时进入 `WaitingEngine`；用户确认前不得自动采用新设置。
- 恢复阶段为 Diagnosing / Repairing / Retesting / Replanning 时禁止切换引擎。
- `Replanning` 只覆盖当前失败小阶段，不得重写整个中阶段或已 Passed 任务。

### 22.3 验证说明

2026-07-22 代码面：

- Rust 侧测试清单约 101 项（含 engine health/profile、process_runner、recovery replan 等）
- 前端策略测试覆盖 autopilot / engine / log / managedFlow / workspace
- 真实 Claude/Codex/Kimi/Grok 外部调用仍依赖本机认证与额度，不得把未跑通的外部 smoke 写成已验证

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
- `WaitingEngine` 允许在健康检查通过后切换项目 profile 或确认新的应用设置；审计记录只包含旧/新 runtime、provider 和设置修订，不包含密钥。恢复确认清除旧会话，下一次执行从当前 profile 创建完整新快照。

### 23.3 当前验证基线

2026-07-23 本阶段收口时要求：

- `cargo fmt --all -- --check` 通过
- `cargo test` 全部通过
- `cargo clippy --all-targets --all-features` 通过（历史非阻断告警除外）
- `npm test -- --run` 全部通过
- `npm run build` 通过
- `git diff --check` 通过

真实 Claude/Codex/Kimi/Grok 外部调用仍依赖本机认证、网络与额度，不得将本地状态机测试等同于外部 smoke 验证。

---

## 24. Phase: 执行层 2.0 与应用设置（2026-07-23）

**范围**：引入应用级非敏感设置、密钥租约、OpenAI Compatible 决策接口、四种插件执行器和完整执行设置快照，并为后续 Grok Build 源码嵌入建立边界。

| 任务 | 当前实现 | 状态 |
|------|----------|------|
| 1. 数据契约 | `KimiCli`、runtime/settings revision、模型/地址/程序路径快照与旧数据默认值 | 已完成 |
| 2. 应用设置 | 非敏感设置原子落盘、revision 乐观锁、密钥租约与环境变量回退 | 已完成 |
| 3. 决策接口 | 既有 `call_deepseek_api*` 入口改用可配置 OpenAI Compatible 请求 | 已完成 |
| 4. 公共执行契约 | 程序来源、环境、输出协议、规格超时和一次设置租约 | 已完成 |
| 5. 插件适配 | Claude Code、Codex、Kimi CLI、Grok Build CLI 独立参数与能力探测 | 已完成 |
| 6. 预装 Grok Build 边界 | 审核上游 `7cfcb20d`（Apache-2.0）并明确禁止用 CLI 冒充内置运行时 | 已完成，Phase 25 已完成嵌入 |
| 7. 设置与引擎 UI | 起始页和项目工具栏设置入口；三类设置页签；五组合展示 | 已完成 |
| 8. 恢复一致性 | 修复前核对完整设置快照；`WaitingEngine` 明确确认与脱敏审计 | 已完成 |

### 24.1 密钥与持久化边界

- `app-settings.json` 只保存接口、模型、超时、结构化输出策略、最大轮数和 CLI 路径覆盖。
- 决策模型密钥读取顺序为会话更新 → 系统凭据库 → `METHEUS_DECISION_API_KEY` → `API_KEY`；预装 Grok Build 为会话更新 → 系统凭据库 → `METHEUS_BUILTIN_GROK_BUILD_API_KEY` → `METHEUS_BUILTIN_GROK_API_KEY` → `XAI_API_KEY`。
- 密钥输入在设置弹窗关闭时清空，不进入 localStorage、Project、ExecutionSession、执行历史或日志。
- 安全保存使用系统凭据库，服务标识为 `com.bruce.metheus`；凭据库不可用时只能由用户明确选择会话模式，禁止明文降级落盘。

### 24.2 预装运行时边界

- `/home/bruce/grok-build` 的审计修订为 `7cfcb20d`，许可为 Apache-2.0；原样源码快照和完整来源记录位于 `third_party/grok-build`。
- `third_party/grok-build-fork` 是该快照的受控 Fork；所有允许差异必须登记在 `PATCHSET.md`，不得修改原样基线来隐藏 Metheus 补丁。
- `BuiltIn + GrokBuild` 通过 `metheus-grok-engine` 调用受控 Fork 的 `MvpAgent → AgentBuilder → SessionActor`。只注册 `read_file`、`search_replace`、`list_dir` 和 `grep`，禁止 Shell、网页、MCP、插件、技能、记忆、Hook、子代理和范围外写入。
- `Plugin + GrokBuild` 是独立合法组合，继承本机 Grok CLI 配置和认证，不读取设置页的预装 Grok Build API Key；内置模式也不读取 Grok CLI 配置。

### 24.3 验证门禁

- Rust：格式、完整测试、Clippy 和构建必须通过。
- 前端：策略测试与生产构建必须通过；起始页和项目工具栏均可打开应用设置。
- 安全：差异与全仓搜索不得出现真实 API Key；项目、设置、会话和日志结构不得包含 secret 值。
- 外部 smoke：未经用户明确授权，不发送真实付费模型请求；本地假 CLI 只验证参数、输出、失败、超时和取消协议。

2026-07-23 本阶段结果：设置、多插件和恢复基础已完成；Grok Build 内置运行时的最终验证基线记录在 Phase 25。

---

## 25. Phase: Grok Build 进程内预装与认证收口（2026-07-23 至 2026-07-24）

| 任务 | 当前实现 | 状态 |
|------|----------|------|
| 上游源码 | 固定 `7cfcb20d2b50b0d18801a6c0af2e401c0e060894`，原样导入 `third_party/grok-build`，记录 tree/archive 哈希和 Apache-2.0 许可 | 已完成 |
| 受控 Fork | `third_party/grok-build-fork` 公开 feature-gated facade，执行真实 `MvpAgent → AgentBuilder → SessionActor`；差异逐项登记，原样基线保持不变 | 已完成 |
| 内置适配包 | `metheus-grok-engine` 执行时调用受控 Fork facade；设置、凭据、事件、取消和错误分类跨边界桥接；连接测试单独使用 sampler | 已完成 |
| EngineService | 内置健康检查不查 PATH，执行不启动 `grok`、`rg` 或任意外部子进程；模型连接和完整 SessionActor 运行时自检分离 | 已完成 |
| API Key | 系统凭据库安全保存或用户明确选择会话模式；设置、项目、会话、日志和错误均不持久化密钥 | 已完成 |
| CLI 认证 | Kimi 与 Grok Build CLI 的本地配置探测和用户主动在线验证分离；Grok CLI 验证移除全部内置密钥环境变量 | 已完成 |
| 会话快照 | 冻结设置修订、模型、API 后端、端点指纹、内置源码修订或插件可执行路径；漂移进入 `WaitingEngine` | 已完成 |
| 前端 | 固定显示“Grok Build（内置）”与“Grok Build CLI（本机）”；内置模式可选；四插件状态和主动认证入口已接线 | 已完成 |

### 25.1 内置工具边界

- 读取、目录和文字搜索只能访问规范化后的项目根内路径，且不跟随符号链接越界。
- 写入只接受执行计划冻结的精确文件清单；现有符号链接目标和未授权路径立即拒绝。
- 搜索由受控 Fork 中的进程内 Rust 策略实现，不调用 `rg`；运行时只向模型公布四项工具。
- embedded session 不加载 Grok CLI 模型目录、凭据、URL 派生认证头、部署身份、bearer 刷新器、Shell、网页、MCP、插件、技能、记忆、Hook 或子代理。
- 取消通过上游 `SessionCommand::Cancel` 进入 SessionActor；超时会发送取消并执行最多两秒的有界关闭；上游最大轮数结果映射为独立 `MaxTurns` 错误。
- embedded session 使用 no-op persistence 和 event tracker，不创建普通 Grok 对话、提示历史或 `~/.grok/sessions` 事件文件。

### 25.2 可用性声明

`BuiltIn + GrokBuild` 是真实可用的执行组合，不是占位状态。用户必须先配置 API Key，并针对当前接口后端、地址、模型和编译源码修订通过运行时自检。模型、接口、设置修订、API 后端或源码修订变化后，旧自检和旧失败会话不能静默复用。

自动化测试使用本地假 SSE 模型服务完成两轮采样和四项工具调用，不产生付费请求。测试同时断言实际请求只使用冻结的 endpoint/model/API key/认证方案，不带 Grok CLI 派生认证头。真实模型 smoke 仍需用户自己的 API Key、网络与额度，未执行时必须明确标注，不能用本地假服务结果代替。

### 25.3 验证基线

- 受控 Fork 单元/集成边界：真实 SessionActor 两轮四工具链路、主动取消、超时与有界关闭、最大轮数、路径/符号链接/授权拒绝、认证头纯净性和 grep 无进程 API 均有自动化覆盖。
- 上游完整性：原样基线修订、Git tree 和 archive SHA-256 继续以 `third_party/grok-build/UPSTREAM_SOURCE.md` 为准；Fork 差异以 `PATCHSET.md` 为唯一允许清单。
- 最终 Rust、前端和 Tauri 构建结果必须在本阶段收尾后按实际命令更新，不得沿用切换到 SessionActor 之前的旧测试计数。
- 真实付费模型 smoke 未自动执行；发布验收需要使用用户自己的密钥、网络和额度人工完成，且不得输出密钥。

---

## 26. Phase: 轻量开发隔离与验收自纠错收口（2026-07-25）

| 任务 | 当前实现 | 状态 |
|------|----------|------|
| 构建隔离 | `metheus-grok-engine` 为可选依赖；默认特性为空；`builtin-grok` 启用内置运行时，`full-product` 表示完整产品能力 | 已完成 |
| 模块边界 | 条件编译集中在 `engine/`；轻量占位模块提供相同入口且不引用 Grok 类型；插件执行器始终可用 | 已完成 |
| 独立验证轨道 | Core 使用 `.build/core`、最多双任务；Grok 使用 `.build/grok-full`、单任务且只检查库目标 | 已完成 |
| 验收账本 | `criterion_reviews` 优先驱动逐项裁决；有效逐项证据不再被全局 `Partial` 一票否决 | 已完成 |
| 统一质量门禁 | 正常执行、确认和恢复统一输出六类质量结果，自动化测试明确失败始终阻断 | 已完成 |
| 两级补证 | 生产恢复链路使用 `Targeted → ExpandedTargeted`；与代码修复次数、检查点和重规划完全分离 | 已完成 |
| 前端语义 | 自动化测试、代码审查和验收证据分别展示；证据不足不再显示为自动修复耗尽 | 已完成 |

### 26.1 三种验证等级

| 等级 | 命令与边界 | 用途 |
|------|------------|------|
| 日常 Core | `npm run verify:core-light`；修改验收/恢复逻辑时追加 `npm run verify:quality`；全部使用 `--no-default-features`、`--package metheus --lib`，最多双任务 | 主 Rust 库轻量检查和定向质量回归，不解析 Grok Build |
| Grok 专项 | `npm run verify:grok-check`；`builtin-grok`、单任务、`cargo check --lib` | 验证完整内置类型边界，不执行最终链接 |
| 正式发布 | 高资源环境显式启用 `full-product`，执行完整 Rust/前端/Tauri 打包与人工发布检查 | 唯一可用于发布声明的等级；不得由前两项替代 |

资源预检在 Cargo 启动前检查磁盘和可用内存；Grok 轨道检测到其他 Cargo 进程时拒绝启动。两条轨道不得混用或同时清理缓存，禁止用无包名、无目标限制的 Cargo 命令代替脚本。

### 26.2 2026-07-25 实测记录

| 轨道 | 结果 | 墙钟时间 | 峰值 RSS | 缓存目录大小 |
|------|------|----------|----------|--------------|
| Core | 格式与 `metheus` 库类型检查通过 | 5.49 秒 | 822,748 KiB | 2.0 GiB |
| 轻量质量 | Rust 账本 7 项、门禁 4 项、补证 11 项、恢复 17 项；TypeScript 与前端策略 7 项通过 | 9.70 秒 | 1,044,948 KiB | 与 Core 共用 2.0 GiB 轨道 |
| Grok 专项 | `builtin-grok` 单任务库类型检查通过；无最终链接 | 1 分 22.16 秒 | 3,610,228 KiB | 1.9 GiB |

以上耗时来自已有独立缓存的本机实测，仅用于资源审计，不是性能承诺。检查期间未运行 Tauri dev/build、全工作区构建、真实模型请求或付费 CLI。正式发布级 Tauri 打包未执行，因此本阶段完成只代表开发隔离、质量逻辑和专项类型检查通过，不代表产品发布验收。

---

## 27. Phase: 流式聊天生命周期与交互收口（2026-07-25）

| 任务 | 当前实现 | 状态 |
|------|----------|------|
| 流式超时归属 | 前端流式 invoke 不设固定超时；生命周期由决策模型配置超时和显式停止命令负责 | 已完成 |
| 项目一致性 | 命令异常后回拉磁盘 Project；控制器销毁后只停止视图更新，最终持久化结果仍可交给 App 的项目/修订检查 | 已完成 |
| 范围隔离 | ChatRoom 按项目名和线程 ID 重挂载，切换时取消旧请求并清空流状态、滚动、未读和输入草稿 | 已完成 |
| Channel 与终态 | 用户消息落盘后的 Channel 失败保存为中断回复；取消保留部分内容；终态事件失败不覆盖已成功的磁盘结果 | 已完成 |
| 推理模型兼容 | `reasoning_content`、心跳、角色和用量事件不作为正文或空事件；等待最终 `content` 后再展示和落盘 | 已完成 |
| 滚动与输入 | 粘底时跟随，离底后只显示一个布尔未读提示；多行输入、Enter/Shift+Enter 和输入法组合保护保持有效 | 已完成 |
| 重试 | 重新生成引用既有用户消息，不复制用户消息；失败和取消回复保留 `reply_to_message_id` | 已完成 |

### 27.1 生命周期规则

- 流式聊天禁止重新加入独立的前端固定超时。后端请求超时结束、用户停止或模型终态完成后，原流式命令必须负责返回最终 Project。
- 推理模型的 `reasoning_content` 不向聊天界面展示或持久化，也不得因数量较多而触发协议失败；请求仍受后端超时、单事件大小和最终正文总长度限制。
- `cancel_chat_stream` 调用失败时输入仍保持锁定，避免原请求仍运行时启动同线程第二个请求；原流式命令真正结束后才能解锁。
- 用户消息成功保存后，后续 Channel 断开必须尝试保存中断终态；若终态存储本身失败则返回错误并由前端回拉项目。已收到的最后一个模型分片必须先加入部分回复，再尝试发送前端事件。
- 磁盘持久化是聊天终态的唯一事实。最终事件发送失败不能推翻已经保存的回复；最终保存失败时前端必须回拉项目并清除未落盘的乐观消息。
- 项目或线程变化时丢弃输入草稿和全部视图跟随状态。旧控制器事件不得进入新范围，但旧请求最终返回的项目仍由 App 的项目名、路径和 revision 防护裁决。

### 27.2 2026-07-25 实测记录

- `npm run verify:core-light` 通过：Rust 格式检查和 `metheus` Core 库检查完成，未启用默认特性。
- `npm run verify:chat-ux` 通过：聊天 API 16 项、聊天运行时/持久化 10 项、前端策略与生命周期 18 项全部通过。
- TypeScript 无输出类型检查和 Vite 生产构建通过；`git diff --check` 通过。
- 验证未启用 `builtin-grok`，未编译 Grok Build，未运行 Tauri dev/build、全工作区构建或真实模型请求。

本阶段的自动化结果只证明 Core 轻量轨道中的协议、状态、持久化和前端生命周期。真实模型长回复、桌面窗口切换和发布包仍需在对应的人工或发布级验收中验证。

---

## 28. Phase: Git 标签身份与确认事务收口（2026-07-25）

- 小阶段 V2 标签包含大阶段、中阶段、小阶段与确认事务身份；中阶段节点 V2 标签包含大阶段、中阶段与完成事务身份。同版本、同序号不再构成标签身份。
- 确认事务按准备、提交完成、标签完成和项目收口持久化；提交或标签完成后的重试只续跑缺失阶段，不创建额外空提交。
- `ConfirmationBlocked` 与 `RetryGitConfirmation` 将 Git 元数据失败从执行失败中分离；代码、质量结果、修复次数和重规划次数保持不变。
- 旧 V1 身份碰撞允许生成新确认事务并重试；V2 不可变标签或事务提交完整性冲突只允许人工核对，不提供重新确认或基线恢复入口。
- 启动迁移通过待确认状态、质量结果、授权路径和旧 V1 标签目标交叉识别旧碰撞，不解析历史错误字符串；事实不足时保留人工处理。
- 回滚继续使用任务保存的实际 V1/V2 标签，且不再删除、覆盖或移动历史标签。
- `verify:git-confirmation` 实测通过：Core Rust 定向测试 14 项、前端策略与组件测试 11 项（2 个测试文件）、TypeScript 无输出检查和差异空白检查全部通过；默认特性关闭，Cargo 最多双任务，未运行 Tauri、Grok 或全工作区验证。

---

## 29. Phase: 自动驾驶验证韧性封板（2026-07-26）

| 任务 | 当前实现 | 状态 |
|------|----------|------|
| 审查协议归一化 | 先解析通用 JSON；字符串/数字/布尔/simple object 按字段契约确定性归一化；枚举、编号、置信度和证据引用仍执行严格边界校验 | 已完成 |
| 审查协议恢复 | 协议修复最多一次；修复失败只重新请求审查，不调用代码执行引擎，不恢复 Git 基线，不消耗代码修复次数 | 已完成 |
| 验证重试状态机 | 网络、超时、限流和服务暂时不可用使用独立验证计数与退避；认证/额度阻断和协议耗尽进入人工边界 | 已完成 |
| 验证可见性 | 自动化测试、代码审查、审查协议和验收证据独立展示；验证阶段、重试进度、下一次重试时间和后台心跳可见 | 已完成 |
| 操作来源审计 | 用户、自动驾驶、恢复器和系统历史分开记录；自动执行/确认与验证恢复事件不再伪装为用户操作 | 已完成 |

### 29.1 轻量封板门禁

`npm run verify:validation-resilience` 只使用 `.build/core`、关闭默认 Cargo 特性并限制为双任务，按顺序运行 `review_protocol`、`quality_gate`、`recovery` 和 `autopilot_runtime` 定向 Rust 测试，随后运行 TypeScript 无输出检查、自动驾驶策略/控制条/来源测试和 `git diff --check`。该门禁不启动 Tauri、不编译 Grok Build、不运行全工作区测试，也不发送真实模型请求。

自动化场景覆盖对象字段漂移、不可归一化字段进入协议恢复、带路径和预期类型的 Schema 修复、瞬时审查失败后的有限重试、认证/额度立即阻断、协议重试不改变工作区和代码修复计数、两级定向补证、验证心跳和四类操作来源。真实模型、网络和桌面发布级验收仍需在用户授权的环境中单独完成。

---

## 30. Phase: v0.0.4 第一阶段任务控制面基础（2026-07-28）

本阶段在旧三层任务和串行流水线之上增加兼容控制面，旧字段仍是事实源，旧流程仍保留为回退路径。

| 能力 | 当前实现 | 状态 |
|------|----------|------|
| 控制模式与版本 | `TaskControlMode`（Legacy/Shadow/SerialTakeover）、算法/快照版本、旧项目加载补齐 | 已完成 |
| 统一任务合同 | `TaskContract` 从 `Subtask` 编译，包含范围、依赖、验收、验证方式、风险、复杂度、预算和稳定指纹 | 已完成 |
| 复杂度与原子性 | 确定性文件/验收/依赖权重，支持直接执行、继续拆分和人工边界 | 已完成 |
| 任意深度任务 | `Subtask.child_tasks` 递归字段；按独立产物、符号范围和可独立验收性分组，没有真实依赖时不强制串联 | 已完成 |
| 验证器注册表 | 本地事实、自动化测试、语义审查和人工审查的成本/风险/证明范围描述 | 已完成 |
| 本地验证短路径 | 文件、路径、DOM、CSS、函数、符号和存储键等有精确证据时不调用模型；无法确定时回退 AI 审查 | 已完成 |
| 验收账本增量更新 | 证据补充复测保留旧的已满足/接受偏差项，明确新结论优先 | 已完成 |
| 自适应调度 | 控制动作、决策原因、事实/合同指纹、缓存命中和无进展阈值 | 已完成 |
| 成本账本 | 关键计划、校准、审查、补证、修复、重规划、宪法和执行调用按任务/阶段/项目记录；Token 未返回时显示未知，不保存 Prompt/响应/密钥 | 已完成 |
| 后端控制快照 | 任意深度任务树、合同、验收、决策、心跳、事件和成本明细只读快照 | 已完成 |
| 影子模式 | 新控制器只计算并记录差异，不派发命令、不增加模型调用 | 已完成 |
| 串行接管门禁 | 任务执行阶段由新控制器直接选择叶子和动作；宏观阶段保留旧策略，错误和人工边界不得静默回退旧任务策略 | 已完成 |
| 前端控制中心 | 独立任务控制标签页包含动态树、合同、验收、决策、成本和关联事件；按钮只消费后端能力集合 | 已完成 |

### 30.1 本阶段边界

- 未修改 Grok Build、执行引擎适配器、插件执行器或第二阶段知识库。
- 未实现真正并行执行、补丁合并、跨项目经验学习或机器学习模型。
- 旧项目缺少新字段时默认 `Legacy`；前端不能直接写入合同、验收账本或项目文件。
- 成本账本当前记录调用用途、模型和耗时；OpenAI 兼容供应方未返回 usage 时不推测 Token 数值。
- `SerialTakeover` 已接管任务执行阶段的叶子选择、验证、恢复和确认动作；Legacy 仍供旧项目与显式人工回退使用，宏观阶段策略保持不变。

### 30.2 轻量验证记录

- `./scripts/verify-core-light.sh` 通过：`cargo fmt` 与 `cargo check --locked --package metheus --lib --no-default-features --profile core-dev`，未启用 Grok、未运行全量 Rust/Tauri 构建。
- Rust 定向测试通过：控制状态、任务合同、复杂度、编译器、验证器、调度器、控制快照、成本账本和验收账本测试。
- `npm run typecheck` 通过；`src/taskControlPolicy.test.ts`、自动驾驶策略和控制条定向测试共 22 项通过。
- `git diff --check` 通过；本阶段未执行 `git commit`、`git push`、Tauri dev 或真实模型请求。

---

## 31. Phase: v0.0.4 第一阶段任务控制系统收口（2026-07-28）

本阶段将 Phase 30 的可观测控制骨架收口为真实串行任务控制系统。接管边界仍限定在任务执行阶段；大阶段、中阶段、计划生成与审批继续沿用既有宏观工作流。

| 能力 | 当前实现 | 状态 |
|------|----------|------|
| 递归任务访问 | 后端统一定位任意深度节点、祖先路径和首个依赖满足叶子；重复 ID、循环和深度超限明确失败 | 已完成 |
| 动态拆分 | 按产物、文件、符号和共同实现依赖分组；拆分没有降低复杂度或无法独立验收时保留原子任务或进入人工边界 | 已完成 |
| 叶子执行 | 执行会话冻结叶子 ID、祖先路径、树修订和合同指纹；执行、验证、恢复、确认与结果写回均作用于同一叶子 | 已完成 |
| 父节点聚合 | 子任务终态后逐层映射证据；父任务只有在全部必需子节点和父验收证据完成后才进入终态，不创建父节点空提交或标签 | 已完成 |
| 控制动作执行 | 拆分、执行、本地/定向验证、修复、重编译、接受偏差、Git 确认、等待和人工边界共用后端幂等执行入口 | 已完成 |
| 串行接管 | `SerialTakeover` 的任务级决策与派发不再继续调用旧任务策略；影子模式只记录决策且没有执行副作用 | 已完成 |
| 保守本地验证 | 本地验证只返回确定满足、确定反例或无法证明；关键词命中不能证明复杂行为，全局 CSS 规则要求完整目标扫描 | 已完成 |
| 成本审计 | OpenAI Compatible usage 解析真实输入/输出/总 Token；关键模型调用关联项目、任务、决策和动作，缺失 usage 明确为未知 | 已完成 |
| 人工控制 | 暂停、恢复、停止、重新验证、继续拆分、重编译和接受偏差均由后端校验修订、能力、作用域和幂等身份 | 已完成 |
| 快照与事件 | 快照只展示持久化决策，包含任务树修订、当前/最近动作和后端能力集合；事件可关联验收、决策、动作、验证器与模型调用 | 已完成 |
| 前端独立页面 | 任务控制与执行日志、代码差异、宪法和 Git 标签同级；日志与任务双向定位，历史节点只读，窄屏布局不横向溢出 | 已完成 |
| 旧项目迁移 | 加载时只在内存补合同和树修订；未开始父会话迁移到叶子，执行中父会话进入人工边界，已完成父任务不重开 | 已完成 |

### 31.1 状态与安全规则

- 父节点存在未完成子节点时不得执行；同一任务、动作标识或运行器作业身份不得并发重复执行。
- 控制动作开始前核对项目修订、任务树修订和合同指纹；旧结果的路径或树修订不一致时拒绝写回。
- 父验收项只能由映射到该验收项的子任务证据聚合。缺证据保持 `Unknown`，冲突进入人工边界，接受偏差保留原因和作用范围。
- 本地验证器无法完整证明时回退定向语义审查；验证器异常和证据不足不得改写成代码错误，也不得直接调用编码引擎。
- 成本账本采用调用 ID 幂等合并和最佳努力保存。账本失败不阻断业务，且不得记录完整 Prompt、响应正文、项目敏感内容或 API Key。
- 旧项目迁移失败时加载停止且原项目文件不落盘；活动执行中的父会话不会自动猜测叶子或覆盖已有代码结果。
- 前端不生成控制决策，不直接修改任务树或验收账本；所有按钮能力和动作结果来自后端快照与命令。

### 31.2 轻量封板记录

2026-07-28 实际运行 `./scripts/verify-task-control-closeout.sh`：

- 资源预检通过；Core 使用 `.build/core`、`CARGO_BUILD_JOBS=2`、`--no-default-features`。
- Rust 格式检查与 `cargo check --locked --package metheus --lib --no-default-features --profile core-dev` 通过，仅存在既有未使用代码告警。
- 任务树、父聚合、编译器、迁移、控制动作、调度器、快照、验证器、成本、API usage 及 4 个精确工作流/运行器/嵌套叶子回归共 59 项 Rust 定向测试通过。
- `npm run typecheck` 通过；`taskControlPolicy.test.ts` 与 `logPolicy.test.ts` 共 10 项前端测试通过。
- `git diff --check` 通过；控制中心不再直接嵌入 `App.tsx` 的文本门禁通过。

本次未运行无过滤 Rust 全量测试、全量 Rust 构建、Clippy、Tauri dev/build、Grok Build 编译、真实付费模型请求或发布级桌面验收；未执行 `git commit`、`git push` 或任何 Git 标签写操作。因此本记录证明 v0.0.4 第一阶段的 Core 轻量收口，不替代正式发布验证。

### 31.3 人工复验与项目成本补充收口

- 人工“重新验证”统一使用后端验收选择器：显式索引严格校验并去重；默认覆盖缺失账本项、`Unknown` 和 `Unsatisfied`，不重复检查 `Satisfied`、`AcceptedDeviation` 或 `Contradictory`。
- 成本账本补齐三项检查、版本方案、已有项目分析和项目角色讨论；普通与流式 OpenAI Compatible 回复均记录脱敏元数据，供应方未返回 usage 时继续显示未知。无项目身份的兼容 `send_message` 明确保留为不入项目账本的例外。
- 流式解析支持末尾 usage-only 事件和普通 JSON 回退响应；成功、失败与取消均按调用 ID 幂等记录，只有实际持久化的有效回复才标记为项目变化。
- 新建 No Project 与 Half Project 正式默认进入 `SerialTakeover`；缺少控制字段的旧项目仍按 `Legacy` 加载，磁盘已有模式保持不变。`Shadow` 只返回旧策略命令并持久化对照结果，不派发新控制动作；控制器异常不得自动永久降级。
- 项目成本同时保留旧的全量可选总计和新的已知 Token 合计、usage 已知/未知调用数；控制快照按供应方和调用用途展示生命周期汇总。近期保留 500 条完整明细，更早调用按 `call_id` 转入可幂等合并和更新结果的紧凑归档，任务、阶段、项目及分组累计不再退化为最近 500 次窗口。

2026-07-28 再次实际运行 `./scripts/verify-task-control-closeout.sh`：资源预检、Rust 格式检查、Core `cargo check`、81 项 Rust 定向测试、`npm run typecheck`、12 项指定前端测试、独立控制页文本门禁和 `git diff --check` 全部通过。该结果包含复验选择器、SSE usage-only、普通 JSON usage 回退、失败/幂等成本调用、Shadow 无执行副作用、混合 Token 覆盖、501 条归档、归档合并/结果更新和旧成本 JSON 重建。

至此 v0.0.4 第一阶段完成 Core 轻量封板。仍未运行无过滤 Rust 全量测试、全量构建、Clippy、Tauri dev/build、Grok Build、真实付费模型或大型项目长时间串行接管，因此本结论不等同于发布级产品验收。

---

## 32. Phase: 第一阶段稳定化与任务检查器升级（2026-07-29）

Phase 30、Phase 31 记录的独立任务控制标签页是当时已经完成并通过轻量门禁的设计。本阶段不回退其后端任务控制协议，而是将展示层收敛为单一左侧递归任务树、中间执行与日志区域、右侧任务检查器；日志、左树和检查器共享同一个后端快照选择 ID。

| 能力 | 当前实现 | 状态 |
|------|----------|------|
| 前端递归语义 | 大阶段直属任务、中阶段任务和任意深度 `child_tasks` 共用递归查询；父任务不可执行，当前与选中路径分别标识 | 已完成 |
| 证据引擎 | 按验收项组合任务合同锚点、符号定义/引用、生命周期上下文和 Git hunk，保留授权文件与 30,000 字符总预算 | 已完成 |
| 恢复补证 | `Targeted`、`ExpandedTargeted` 两级补证保持有限；缺证据不进入代码修复，已有有效满足证据不被新 `Unknown` 擦除 | 已完成 |
| 验证通道 | `LocalValidate`、`AutomatedValidate`、`TargetedValidate` 分别代表本地确定性证明、真实测试和 AI 语义审查 | 已完成 |
| 任务检查器 | 概览与合同、验收与证据、决策与恢复、成本与事件四页；历史节点只读，任务级动作仅作用于当前任务 | 已完成 |
| 布局与桌面壳 | 左中右独立滚动，检查器可调整宽度；中小窗口使用遮罩抽屉/全屏详情；窗口最小尺寸与 CSP 已收紧 | 已完成 |

### 32.1 验收与兼容边界

- 自动化测试未配置或测试环境不可用保持 `Unknown`，不等同于代码失败；只有真实自动化测试失败才进入明确未满足链。
- `SerialTakeover` 是新项目的正式默认模式；`Shadow` 继续作为对照审计路径，`Legacy` 继续作为兼容回退路径。旧项目缺少控制字段时仍按 `Legacy` 读取，已有模式不自动迁移。
- 前端只消费后端项目与任务控制快照，不直接写任务合同、验收账本或项目 JSON。面板宽度属于本地 UI 设置。
- 本阶段未修改 `third_party/`、Grok Build、插件执行器或执行引擎适配层，未降低验收置信度、质量门禁或证据预算约束。
- 本轮未执行 `git commit`、`git push`、`git tag`，也未重置、清理、暂存或隐藏用户工作区。

---

## 33. Phase: 工作流三条闭环专项修复（2026-07-30）

本阶段收口未来调整、托管方案生成和中阶段事实路由三条工作流。项目方案生成与审批使用独立步骤；未来调整使用大阶段审阅周期专属线程，草稿绑定线程/项目修订并在继续讨论后过期；托管由 Rust 单项目后台作业派发；中阶段继续操作统一根据正式列表、执行事实和计划事实解析，不再无条件生成整表。

### 33.1 状态与迁移规则

- `PlanApproval` 只表示已有可审批草稿；旧的空审批状态迁移到 `ProjectPlanGeneration`。
- `AdjustFuture` 不回退到初始讨论线程。旧未来草稿只有在唯一有效专属线程可确认时才补齐来源，否则保留展示但标记过期。
- 首次完整中阶段草稿只允许空正式列表。已有 Pending/Ready 时选择既有项，InProgress 或执行事实恢复执行，全部 Completed 进入大阶段审阅；多个活动项保持人工对账。
- 遗留 `MidStageApproval + 已有正式中阶段` 草稿禁用整表替换并保留审计信息，正式中阶段和完成事实不被覆盖。
- 托管大阶段检查失败后使用检查反馈有限重生成并重新检查；连续相同问题、反馈缺失或两次重生成耗尽进入 `WaitingHuman`，不得重复检查未变化草稿。
- 未来规划上下文以当前仓库扫描和已保留大阶段事实为准，并递归收集动态任务树中的接受偏差、拒绝事实和未满足条件；当前代码事实无法读取时停止生成，不回退到过期基线。
- 托管模型动作超时跟随当前模型配置和单动作最大请求次数，并保留收口缓冲；非模型动作继续使用固定上限，避免长模型请求被提前终止和重复派发。
- React 只同步项目状态和发起显式控制命令；托管动作选择、心跳、重试和单飞作业由 Rust 后端持久化管理。

### 33.2 轻量验证记录

2026-07-30 实际运行 `./scripts/verify-workflow-closure.sh`：

- 资源预检通过；Core 使用 `.build/core`、`CARGO_BUILD_JOBS=2` 和 `--no-default-features`。
- Rust 格式检查与 Core 库检查通过；工作流解析、旧项目迁移、首次中阶段契约、后端托管运行器、当前代码事实与递归动态任务约束共 30 项定向 Rust 测试通过。
- `managedFlowPolicy.test.ts` 与 `FuturePlanningWorkspace.test.tsx` 共 6 项前端测试通过；TypeScript 无输出检查通过。
- 前端托管循环、硬编码中阶段跳转、未来工作区路由和整表替换保护的文本门禁通过；`git diff --check` 通过。

本次未运行无过滤 Rust 全量测试、Clippy、Tauri dev/build、完整桌面构建、Grok Build、真实模型请求或发布级验收；未执行 `git commit`、`git push` 或 Git 标签写操作。

---

## 34. Phase: 运行状态同步与恢复体验收口（2026-07-31）

- 正式 Project 原子保存成功后发布按项目隔离的轻量状态事件；事件只承担失效通知，不携带完整 Project、日志、代码、Prompt 或密钥。
- 前端以统一运行时快照对账 Project、PipelineState 和后端恢复展示；Channel 是主同步路径，低频轮询只作断线兜底，并拒绝乱序响应和旧项目响应。
- 恢复主动作只由后端 `RecoveryPresentation` 与能力集合决定；顶部控制条是唯一全局恢复入口，执行面板只展示任务、质量与阻断事实。
- 人工恢复选择使用后端决策选项，并按选项要求收集原因和验收项；Git 确认、验证重试、引擎阻断、证据不足和基线恢复不再由前端组合字段猜测。
- 基线恢复、`restore_and_retry` 和 `skip_task` 在 Git 变更前核对最新影响指纹。指纹覆盖动作、基线、HEAD、已跟踪差异、未跟踪文件内容和规范化工作区状态；陈旧预览必须拒绝。
- 恢复事务在 Pipeline 锁内清理旧 PipelineState 后保存事实；并发重复恢复保持幂等。同步健康、通知通道重连、一次性阻断提醒和辅助技术实时播报均可见。

2026-07-31 实际运行 `./scripts/verify-runtime-sync-ux.sh`：资源预检和 Rust 格式检查通过；Core 使用 `.build/core`、`CARGO_BUILD_JOBS=2`、`--no-default-features`，状态通知、运行时快照、恢复展示、恢复预览、防陈旧执行、恢复幂等与 Pipeline 清理共 16 项定向 Rust 测试通过；TypeScript 无输出检查、10 个指定前端测试文件共 44 项测试及 `git diff --check` 通过。Rust 仅报告 13 条既有未使用代码告警。

本次未运行无过滤 Rust 全量测试、全量构建、Clippy、Tauri dev/build、Grok Build、真实模型请求或发布级桌面验收；未新增依赖、未修改锁文件，未执行 `git commit`、`git push` 或 Git 标签写操作。该结果只证明本专项的 Core 轻量门禁，不替代正式发布验证。

### 34.1 第一阶段状态闭环最终收尾

- 状态变更主路径统一返回 `RuntimeMutationResult`：项目事实、Pipeline、后端恢复展示、任务控制摘要和动作结果在同一封包中返回。执行启动、工作区准备/刷新、已有基线分析/批准、执行引擎设置、聊天和任务控制均不再依赖“先接收单一对象、再补拉项目”的主流程。
- 项目事件携带任务树修订、控制动作、控制模式和任务控制失效标记；任务控制快照与运行时事件序号关联。Project-only 事件不再推进任务控制失效序号；任务检查器关闭后停止详情请求，打开且原子详情健康时也不创建独立轮询，只有 Channel、原子详情时效或连续同步失败时才启用 30 秒异常兜底。
- 执行终态具有“正在读取最终状态 / 最终状态延迟”两级可见对账阶段；项目切换和成功取得终态会清理临时阶段。恢复条与任务检查器共同消费后端恢复展示模型。
- 基线恢复、Git 重新确认、人工恢复和自动恢复返回结果摘要，明确基线、丢弃文件、后台作业是否启动及下一步；窗口失焦时按阻断指纹提供一次性标题和页面提醒，不抢占输入焦点。
- 新项目正式默认使用 `SerialTakeover`；界面显示后端快照中的实际模式。`Shadow` 只做对照审计，`Legacy` 只作兼容回退；旧项目模式不被自动迁移，活动执行或恢复期间禁止切换模式。

2026-07-31 本轮实际验证：Core 使用 `.build/core`、`CARGO_BUILD_JOBS=2` 和 `--no-default-features`，`runtime_snapshot::tests` 2 项通过；最终 Rust 变更文件的独立 `rustfmt --check` 通过。TypeScript 无输出检查通过，12 个指定前端测试文件共 63 项测试通过；验证脚本语法、统一返回调用文本门禁和 `git diff --check` 通过。最后一轮薄命令包装与结果摘要调整后，经用户明确授权对最终工作树追加一次相同 Core 轨道的定向 Cargo 验证：Rust 库重新编译成功，`runtime_snapshot::tests` 仍为 2 项通过、364 项过滤，仅报告 13 条既有未使用代码告警。该结果不替代后续发布级编译。

本轮未运行无过滤 Rust 全量构建、全量测试、Clippy、Tauri dev/build、Grok Build 或真实模型请求；未新增依赖、未修改锁文件，未执行 `git commit`、`git push`、暂存、标签或清理工作区操作。

### 34.2 第一阶段正式运行时验收协议

- `scripts/verify-phase1-runtime-contract.sh` 是第一阶段正式默认启用的专项门禁；固定使用 `.build/core`、关闭默认特性、最多两个 Cargo 任务，并只运行名称包含 `phase1_runtime_contract` 的 Rust 测试和指定前端测试。
- 协议覆盖新项目默认接管、Shadow 无副作用、生产动作派发、无需 Reload 的阻断同步、Channel 兜底、终态延迟、详细快照乱序保护、恢复状态清理、叶子聚合/Git 确认续跑和显式回退审计。
- 真实桌面烟雾测试必须先通过资源预检并复用现有二进制；没有可复用二进制时不得触发构建，必须在 `docs/phase1-runtime-acceptance.md` 准确记录未执行风险。
- 桌面二进制只有同时位于 Core 轨道、带有关闭默认特性的构建元数据、源码指纹与当前树一致且时间戳不早于相关源码时才可启动验收；`src-tauri/target` 候选不得冒充 Core 验收证据。
- 本协议禁止 Tauri dev/build、Grok Build、无过滤 Rust 测试、真实模型请求、依赖或锁文件变更，以及 `git commit`、`git push`、标签等 Git 写操作。

### 34.3 第一阶段正式封板记录

- 2026-07-31 实际运行第一阶段运行时契约轨道：资源预检、Rust 格式、TypeScript、静态命令覆盖和 `git diff --check` 通过；Core 定向 Rust 测试 6 项通过、360 项过滤，11 个指定前端测试文件共 60 项通过。Rust 仅报告 13 条既有未使用代码告警。
- 新项目当前正式默认 `SerialTakeover`；`Shadow` 为对照审计，`Legacy` 为兼容回退。旧项目保留已有模式，缺失控制字段时按 `Legacy` 读取，均不自动迁移。
- 真实桌面烟雾未执行：资源预检通过，但 `.build/core` 没有可复用桌面程序；唯一候选 `src-tauri/target/debug/metheus` 生成于本轮源码变更之前且不属于 Core 轨道，不能作为当前实现的验收证据。无需 Reload 的恢复按钮、后台提醒、三处状态一致性和真实资源峰值仍属于发布前人工风险。
- 本轮未运行全量构建、无过滤 Rust 全量测试、Clippy、Tauri dev/build 或打包、Grok Build、真实模型和付费 CLI；未新增依赖、未修改锁文件，未执行 `git commit`、`git push`、暂存、标签或其他 Git 写操作。因此可以宣布 Core 代码封板，但不宣称真实桌面稳定性百分之百完成。

### 34.4 第一阶段最终安全封板（2026-08-01）

- 人工通过、接受偏差和跳过任务由后端统一策略裁决。人工通过与偏差必须绑定当前叶子或人工恢复会话、成功执行结果和有效验收边界；跳过必须通过显式依赖检查与影响预览。
- 人工验证记录持久化动作来源、执行结果指纹、项目修订和任务树修订；质量确认与父节点聚合再次核对审计，旧记录或伪造记录不能成为终态旁路。
- 每个任务节点由后端下发能力、禁用原因和可操作验收项；非当前、父级、已完成与过期快照保持只读，前端不再自行放宽。
- 详情独立轮询只在通知通道异常、原子详情不可用/超时或连续同步失败时启用，健康恢复后立即停止。
- 2026-08-01 实际运行最终安全封板门禁：资源预检、Rust 格式、TypeScript 和静态旁路门禁通过；Core 在 `.build/core`、`CARGO_BUILD_JOBS=2`、`--no-default-features` 下 10 项人工终态安全测试通过、365 项过滤；5 个指定前端测试文件共 46 项通过，`git diff --check` 通过。
- 2026-08-01 条件式桌面检查：资源条件通过，但 `.build/core` 没有桌面程序；仓库中唯一的 `src-tauri/target/debug/metheus` 不属于 Core 轨道，且缺少匹配当前源码指纹和关闭默认特性的构建元数据，结果为 `DESKTOP_SMOKE_ELIGIBLE=no`。未启动桌面程序，也未触发重建。
- 本轮未运行无过滤 Rust 全量测试或构建、Clippy、Tauri dev/build 或打包、Grok Build、真实模型和付费 CLI；未新增依赖、未修改锁文件，未执行任何 Git 写操作。该结果允许宣布第一阶段 Core 代码封板；真实桌面烟雾仍未执行，不能宣称桌面稳定性已完全验收。

---

## 35. Phase: 检查收敛、同步策略与治理原则对齐（2026-08-02）

### 35.1 治理基线

- 计划和阶段检查遵守“确定性验证优先”与“检查结论分级”：本地可判定的结构事实先检查，AI 仅补语义判断；建议项不得造成失败、重生成或人工停止。
- 检查协议必须使用带目标契约的结构化解析。协议或字段类型修复失败属于协议失败，与计划质量失败、代码修复次数和质量重生成次数分离。
- 新项目默认 `SerialTakeover`；`Shadow` 只用于对照审计和显式回退，`Legacy` 只用于兼容。旧项目已有模式和缺失字段的迁移规则保持不变。
- 验证流程按风险、影响范围和证据缺口伸缩；轻量 Core 门禁继续使用 `.build/core`、关闭默认特性并限制最多两个 Cargo 任务，不以固定长度流水线冒充风险判断。
- 既有动作租约、陈旧锁自愈、恢复展示分类、成本账本和一次 Schema 协议修复继续作为运行时基础设施，不因本阶段调整而绕过或降级。
- 项目状态同步以 Tauri Channel 事件为主，事件修订未前进时不得全量拉取；健康通道只保留低频兜底，断开或重连时才提高兜底频率。Metheus 的进程内同步只使用 Tauri IPC/Channel，不引入面向远程网络的通信协议。

本阶段的实现与轻量验证结果只在对应代码和定向门禁真实通过后补录，不预写未来日期或虚假通过结论。

### 35.2 2026-08-02 轻量验证记录

- 检查结果由后端按硬阻断字段重算，建议关键词误入硬字段时回收到 `suggestions`；失败指纹、重生成反馈和无进展计数不再包含建议。计划只有建议时直接进入批准，真硬阻断仍保持失败并在人工停止时列出具体内容。
- 大阶段、中阶段和执行计划检查均使用独立 `JsonTargetContract`，协议修复限一次；协议失败在写入质量结果前返回。执行计划 AI 检查前新增目标、精确路径、验收、停止规则、依赖顺序/环和明显重复的确定性预检。
- 项目事件携带 `data_revision`、`event_sequence`、`task_control_tree_revision` 和任务控制快照版本。前端只在项目或任务控制修订前进时合并拉取运行时快照；健康 Channel 使用 60 秒兜底，重连或断开使用 15 秒兜底。
- Core 定向 `check_convergence_` 11 项 Rust 测试通过、399 项过滤；TypeScript 无输出检查通过，两个同步测试文件共 17 项通过；Rust 仅有 14 条既有未使用代码告警。新增门禁为 `scripts/verify-check-convergence-sync.sh`。
- 未运行 Tauri dev/build、Grok Build、Clippy、无过滤 Rust 测试、真实模型或发布构建，未新增依赖或修改锁文件，未执行 Git 写操作；真实桌面同步顿挫仍需使用当前源码对应的桌面二进制人工复验。
- 验证过程中首次定向 Cargo 命令漏设 `CARGO_TARGET_DIR`，约 1 秒后中断并未作为证据；该中断触及既有 `src-tauri/target` 缓存。最终通过结果来自显式 `.build/core`、`CARGO_BUILD_JOBS=2` 和 `--no-default-features` 的定向任务。

---

## 36. Phase: 验收标准可证明性原则（2026-08-03）

### 36.1 根本规则

- 每条小阶段验收标准必须具有可证明性标签；原始 `acceptance_criteria` 文本仍是兼容权威源，与其按索引对齐的元数据不得改写验收文本或回退既有验收结论。
- 可证明性分为五档：`Deterministic` 用完整本地扫描证明结构、语法、符号或存在性；`AutomatedTest` 用真实测试命令与退出事实证明；`SemanticReview` 用受限代码证据进行 AI 语义判断；`HumanReview` 用人工或真实运行时观察确认视觉、体验和主观一致性；`Unprovable` 表示当前契约无法证明，规划必须改写或保守降级到人工边界。
- 标签优先于关键词分拣。旧项目缺标签时只能使用纯本地保守推断补齐；不确定项不得冒充 `Deterministic`，已 `Satisfied` 或 `AcceptedDeviation` 的账本状态不得因迁移回退。
- 视觉、体验、美观、手感以及“与……保持一致”等命题必须直接进入人工确认，禁止交给 AI 反复补证。人工确认须由后端策略裁决、绑定具体验收项与依据，并明确区别于接受偏差和自动验证通过。
- `Unprovable` 不得进入普通 AI 补证循环。运行时将其保守路由到人工边界，但规划与审计必须保留它源自不可自动证明这一事实。

### 36.2 补证与动作租约

- 补证升级必须比较证据来源指纹。指纹覆盖验证器类型、证据种类和覆盖文件；只增加同一文件中的更多同类代码片段不构成新证据来源，相邻两轮指纹相同立即熔断并转人工，且不得消耗代码修复次数。
- 熔断诊断必须写明“现有证据来源无法证明”，不得伪装成代码质量失败；来源确实从本地扫描、测试输出、代码证据或人工/运行时事实之间变化时，才允许继续有限补证。
- 控制动作按类型设置最长窗口，验证短于执行；正常 30–105 秒动作保留安全余量。心跳持续新鲜时视为有效占用，即使超过预期时长也不得强制清理；心跳停止后在收紧窗口内进入陈旧锁对账。

本阶段完成状态只以 `scripts/verify-provability-closeout.sh` 的真实定向结果和运行时验收记录为准。真实桌面视觉确认、强退与重启仍必须使用与当前源码匹配的 Core 二进制执行，不得由静态测试代替。

### 36.3 2026-08-03 轻量验证记录

- 实际运行 `scripts/verify-provability-closeout.sh`：资源预检、Rust 格式、TypeScript 无输出检查、静态链路门禁和 `git diff --check` 通过。
- `.build/core`、`CARGO_BUILD_JOBS=2`、`--no-default-features` 下，规划标签校准、标签路由、旧 JSON 兼容迁移、逐项人工边界、同源补证熔断和动作租约窗口共 14 项 Rust 定向测试通过，409 项过滤；仅报告 14 条既有未使用代码告警。
- `TaskInspector.test.tsx` 8 项通过，覆盖人工证明方式徽标与独立确认入口。未运行无过滤 Rust 全量测试/构建、Clippy、Tauri dev/build、Grok Build、真实模型或发布构建；未新增依赖、未修改锁文件、未执行任何 Git 写操作。
- 真实桌面烟雾仍未执行；视觉/体验人工确认、长动作超过窗口后的正常收口、强退重启和陈旧锁释放仍须使用与当前源码指纹匹配的 Core 桌面二进制复验。

---

## 37. Phase: 运行期专项修复与执行路径收口（2026-08-03）

### 37.1 实现记录

- split 后续叶子的受管改动只按当前叶子授权路径识别，不再要求 Git HEAD 等于会话基线；授权范围外改动仍按外部改动阻断。
- OpenAI Compatible 普通响应改为带独立超时的原始字节读取，并区分超时、网络截断、空正文、服务端非 JSON 和完整但形态错误的 JSON；诊断只保留脱敏后的 500 字节前缀。
- Grok Build 内置执行结果携带可选 token usage，并按任务、模型和执行耗时写入成本账本；供应方缺少 usage 时保留调用与耗时、token 记为未知，账本写入失败不阻断执行。
- split 只按精确授权文件形成可独立验收的产物组；`required_identifiers` 和反引号标识符不再成为拆分维度，单文件任务保持直接执行，单次拆分上限为 4 个叶子。
- Grok Build 文本流在适配层按 150ms、换行或 4KB 边界聚合；结构化事件先刷新文本再独立推送。聚合日志通过项目状态 Channel 的 `runtime_dirty` 事件触发前端实时快照同步。
- 依赖、执行步骤、默认控制模式、桌面进程通信和执行器参数等治理原则已按当前架构修订；运行期四项接入检查点已写入第 14.1 节。

### 37.2 轻量验证记录与边界

- `.build/core`、关闭默认特性且限制两个 Cargo 任务：受管/外部/混合工作区与 split 前序提交回归共 9 项通过；OpenAI Compatible 响应分类与既有 API 测试共 25 项通过。
- 在两个 Cargo 任务额度用尽后，事件桥使用直接 `rustc --test` 验证 4 项通过；TypeScript 无输出检查通过，项目同步策略与 Hook 共 17 项前端测试通过。`scripts/verify-runtime-fixes.sh --static-only` 的 Rust 格式、宪法文本、依赖锁文件和 `git diff --check` 门禁通过。
- 成本账本、split 和事件同步的后续 Rust 收口改动只完成了格式、静态链路与直接事件测试验证，未在本轮追加第三个 Cargo 任务，因此本记录不宣称这些后续改动已完成 Cargo 编译封板。新增脚本的完整模式固定为两个定向 Cargo 任务，供下一次独立验证窗口复验。
- 本轮未运行无过滤 Rust 构建/测试、Clippy、Tauri dev/build 或打包、Grok Build 全量编译、真实模型请求；未新增依赖、未修改锁文件，未执行任何 Git 写操作。

---

## 38. Phase: 第一阶段运行期三项收口（2026-08-06）

### 38.1 实现事实

- 普通有界 Tauri 调用统一按“调用点显式超时 → 完整命令精确策略 → 单层 `_runtime` 基础策略 → 默认值”解析；现有基础预算保持不变，五个缺失基础策略已补齐。任务控制的 900 秒/15 秒显式覆盖和两个聊天 Channel 流式命令继续作为静态可审计例外。
- 决策请求与执行操作仍在真实后端生命周期内持有设置活动租约；两类计数独立、以 `saturating_sub` 释放，任一活动计数大于零继续阻断设置更新，全部释放且 revision 正确后恢复允许。没有新增计时过期、强制解锁或前端绕过。
- Grok Build 内置运行时自检取得成功或失败结果后，发送只携带 `BuiltIn + GrokBuild` 目标的应用级进程内健康失效通知；所有已挂载的匹配 `ExecutionEngineSelector` 重新检查健康，插件 profile 不响应，卸载会清理监听，旧健康响应继续由请求序号丢弃。该事实不写入 `Project`，也不借用 Project State Channel。

### 38.2 2026-08-06 轻量验证记录

- 实际运行 `npm run verify:phase1-runtime-contract`：命令退出状态为 0、总耗时约 8.7 秒；资源预检、Rust 格式、TypeScript 无输出检查、运行时命令注册/旁路/超时覆盖静态门禁和 `git diff --check` 通过。Core 使用 `.build/core`、`CARGO_BUILD_JOBS=2`、`--no-default-features`；15 项 Rust 定向测试通过、422 项过滤，仅报告 15 条既有未使用代码告警；19 个指定前端测试文件共 113 项通过。超时门禁动态审计 61 个命令（56 个基础策略、1 个精确策略、2 个显式超时例外、2 个 Channel 例外），`test_grok_build_runtime` 由精确策略覆盖。设置活动租约证据由阻断/释放算法 Rust 单测与 `ActivityGuard::drop` 静态接线契约组成，不宣称已执行真实网络请求成功、失败或取消的生命周期测试；SerialTakeover 测试同时证明夹具被推断为 `SemanticReview` 且派发 `TargetedValidate`。
- 实际运行 `npm run verify:grok-check`：资源预检通过；Grok 使用 `.build/grok-full`、单任务、`builtin-grok` 的库目标 `cargo check`，Cargo 报告耗时 1 分 34 秒并通过，仅报告 21 条未使用代码告警。未执行最终链接、Tauri 构建或真实模型请求。
- 实际运行桌面资源资格预检，结果为 `DESKTOP_SMOKE_ELIGIBLE=no`：仅有的 `src-tauri/target/debug/metheus` 是非 Core 候选，缺少同时匹配当前源码指纹、关闭默认特性和时间戳的构建证据。本轮未启动桌面程序或触发重建；因此不宣称真实桌面无需 Reload 体验、强退恢复或发布级稳定性已完成验收。
- 本轮未运行无过滤 Rust 全量测试/构建、Clippy、Tauri dev/build 或打包、Grok 完整构建、真实模型与付费 CLI；未新增依赖、未修改清单或锁文件。本轮执行未调用 `git commit`、`git push`、`git tag`，也未调用暂存、取消暂存或其他修改 Git index 的命令。

---

## 39. Phase: 自适应执行闭环与 Grok 受控执行单元（2026-08-06）

### 39.1 工作负载、拓扑与检查原则

- 项目体量由首项目标完整性检查同一次模型调用提供的结构化范围事实与 Half Project 确定性基线共同分类；模型不得直接决定规模。画像必须绑定讨论修订与稳定指纹，缺失或过期时明确阻断，不得默认为任一规模。
- `contract_snapshot` 是可重建派生缓存，当前严格版本为 `task-contract-v2`：已知 `task-contract-v1` 只失效缓存，未知版本和损坏 v2 继续拒绝；缺失或过期画像的历史项目保持任务、验收和执行事实可加载，但合同编译、SerialTakeover 与 autopilot 明确阻断，直到重新完成目标完整性检查。
- 四档固定矩阵为：`Micro` 使用 Milestone 1 / MidStage 0 / Subtask 1 / split 0 / `Lean` / executor turns 4；`Small` 为 1 / 0 / 3 / 0 / `Lean` / 8；`Standard` 为 3 / 3 / 6 / 1 / `Standard` / 16；`System` 为 5 / 5 / 8 / 1 / `Strict` / 32。transport retry 预算依次为 0 / 1 / 2 / 3，Doom Loop retry 预算依次为 0 / 0 / 1 / 2。
- 正式执行拓扑只有 `Milestone -> Subtask` 与 `Milestone -> MidStage -> Subtask` 两种；`Standard/System` 只允许在非原子 Subtask 下增加一层 ChildTask，32 层仅作为损坏防护。高风险只把检查深度提高为 `Strict`，不得据此增加阶段或 split 深度。
- 三项检查的目标完整性、现实一致性和任务可执行性维度及顺序全部保留；`Lean/Standard/Strict` 只改变证据深度与阻断门槛。建议项不得使检查失败，System 的跨端、数据库、权限、外部集成与依赖顺序必须给出严格证据。
- 阶段与任务数量同时受 Prompt 和本地确定性上限约束；每层允许只生成一个实体，零个或超过画像上限均在 AI 语义检查前失败。
- 首次进入或从 Discussion 重返 `MilestoneReview` 的所有生产路径统一调用同步、无 I/O 且不自行修改 revision 的 `apply_milestone_review_boundary`：该函数校验有效画像、Quick/Professional 拓扑和全部任务终态，并一次建立 Milestone `Completed`、`review_status=pending_review`、`review_conclusion=None`、正确 `review_node_id`、`current_step=MilestoneReview` 与 active autopilot 的 `WaitingMilestoneReview`。正式命令、select/continue、pipeline 终态收敛、AcceptDeviation、Skip、workflow migration 和启动对账均复用该边界；Discussion B/C 重返保留讨论线程与历史事件。持久化调用者只增加一次 `data_revision` 并只写一次 `last_transition_at`，AcceptDeviation 继续保留 CAS 与动作租约检查。

### 39.2 执行预算、Grok 边界与错误恢复

- 任务合同冻结 scale、split 深度和 `max_executor_turns`、`max_transport_retries`、`max_doom_loop_retries`；Pipeline 将同一预算复制到执行请求，内置执行器轮数取任务预算与用户设置上限的较小值，插件不得拼装未支持的参数。
- 模型连接 adapter 由单一 `retry_policy(config)` 驱动，`SamplerConfig.max_retries` 与 `RetryPolicy.max_retries` 都严格读取 `max_transport_retries`；零预算不重试，非零预算不被内部默认值改写。
- Grok 只能经 `engine/builtin.rs -> metheus-grok-engine -> metheus_embedded -> SessionActor/sampler` 使用。受控 fork 修订为 `metheus.4`，复用上游 sampler 重试、错误分类、退避与 `DoomLoopRecoveryPolicy`，不扩大 Shell、terminal、网页、MCP、plugin、skill、memory、hook 或 subagent 工具面。
- Embedded 会话继承 facade 已冻结的 Doom policy，重采样时只丢弃当前未接受尝试的聚合输出，不复制上游检测或恢复算法；Responses fake-SSE 证明 transport retry 为 0 时，Doom 预算 1 恰好发出两次请求并只返回 clean response，预算 0 恰好发出一次请求并保留原响应。
- `ToolCompleted` 与 `ToolFailed` 分流；只转发 `x.ai/session_notification` 的结构化 retry 状态。结构化事件到达前刷新文本缓存，事件桥不阻塞 SessionActor，诊断经过脱敏和限长。
- `ToolRejected`、`ProtocolError`、`MaxTurnsExceeded` 与 `RuntimeError` 进入人工边界且禁止自动代码修复；网络、服务不可用、限流与超时继续沿用有限恢复。Core 决策 API 保持独立，不由 Grok sampler 替换。

### 39.3 2026-08-06 历史分轨验收事实（已由 39.4 取代）

- 本次最终封板以 `/usr/bin/time -p` 记录墙钟，按顺序实际运行 `npm run verify:adaptive-execution-closeout` 并退出 0：静态边界、pristine 基线、PATCHSET、`metheus.3` revision、资源预检与 `git diff --check` 通过；`.build/core`、两任务、`--no-default-features` 下 `adaptive_execution_contract` 40 项通过、457 项过滤（Cargo 0.70 秒、测试 0.27 秒），6 个前端文件 22 项通过（Vitest 2.57 秒），`.build/grok-full`、单任务下 `adaptive_grok_contract` 7 项通过（BuiltIn 3、engine 4，Cargo 3 分 46 秒）；整条命令墙钟 238.16 秒。
- 随后实际运行 `npm run verify:phase1-runtime-contract` 并退出 0：Rust 15 项通过、482 项过滤（Cargo 1.46 秒、测试 0.05 秒），19 个前端文件 115 项通过（Vitest 3.60 秒），超时策略与设置活动租约接线审计通过；整条命令墙钟 11.53 秒。
- 实际运行 `npm run verify:grok-check` 并退出 0；`.build/grok-full`、`CARGO_BUILD_JOBS=1` 的单任务 `cargo check` 报告 1 分 31 秒，仅有 25 条未使用代码告警，整条命令墙钟 91.79 秒。随后以同一 target/jobs 运行 fork `metheus_embedded_runtime`，11 项本地 fake-SSE 测试全部通过（Cargo 2 分 26 秒、测试 2.54 秒、墙钟 149.35 秒）；最后 `git diff --check` 与 `git diff --cached --check` 均退出 0。
- 本次未运行桌面 smoke、Tauri dev/build、发布构建、无过滤 Rust 全量测试、Clippy、真实模型或付费请求；未启用 MCP、subagent、plugin、skills、terminal 或其他禁用工具面，也未执行 Git 提交、暂存、标签、清理或推送。因此本记录证明自适应执行的 Core、前端契约与 Grok 受控边界，不宣称真实桌面或真实模型长链路已经验收。

### 39.4 2026-08-06 Review 旁路修复后复验

- 39.3 的封板发生在全部 `MilestoneReview` 旁路清除之前，现由本节结果取代。当前 Review 事实只由 `apply_milestone_review_boundary` 写入；正式进入命令、select/continue 路由、Passed/AcceptedDeviation/Skipped 的 pipeline 收敛、workflow closure migration、启动对账及 Discussion B/C 重返均可追溯到该共享边界。迁移与启动对账通过 `Result` 传播缺失或过期画像错误，不保存 `MilestoneReview` 与未完成状态并存的半状态；每次持久化转换仍只增加一次 revision。
- 使用 `/usr/bin/time -p` 实际运行 `npm run verify:adaptive-execution-closeout` 并退出 0：Core `adaptive_execution_contract` 60 项通过、455 项过滤（Cargo 0.35 秒、测试 0.20 秒），6 个前端文件 22 项通过（Vitest 1.38 秒），Grok `adaptive_grok_contract` 7 项通过（BuiltIn 3、engine 4，Cargo 3 分 45 秒）；整条命令墙钟 232.46 秒，静态边界、pristine 基线、PATCHSET、`metheus.3` revision、资源预检和 diff 门禁均通过，仅报告 25 条未使用代码告警。
- 随后实际运行 `npm run verify:phase1-runtime-contract` 并退出 0：Rust 15 项通过、500 项过滤（Cargo 1.62 秒、测试 0.12 秒），19 个前端文件 115 项通过（Vitest 2.75 秒），整条命令墙钟 10.81 秒。`npm run verify:grok-check` 也退出 0（Cargo 1 分 29 秒、墙钟 89.51 秒，仅有 25 条未使用代码告警）；同一 `.build/grok-full`、`CARGO_BUILD_JOBS=1` 轨道的 fork `metheus_embedded_runtime` 11 项全部通过（Cargo 2 分 05 秒、测试 2.39 秒、墙钟 128.11 秒）。最后 `git diff --check` 与 `git diff --cached --check` 均退出 0，Git index 为空。
- 本次未运行桌面 smoke、桌面资源资格复核、Tauri dev/build、发布构建、无过滤 Rust 全量测试、Clippy、真实模型或付费请求，也未执行提交、暂存、标签、清理或推送。Grok 受控工具集合仍严格为 `read_file/search_replace/list_dir/grep`；本记录不宣称真实桌面、真实模型长链路或发布级资源稳定性已经验收。
- 2026-08-07 增加 Review 唯一写入静态门禁：专项脚本遍历 `src-tauri/src/**/*.rs`，只扫描每个文件首个 `#[cfg(test)]` 之前的生产部分，并以跨行空白归一化匹配要求 `current_step = project::WorkflowStep::MilestoneReview` 恰好出现一次、位于 `workflow_resolution.rs`，同时确认 `apply_milestone_review_boundary` 存在；违规时列出文件和行号并退出 1。Shell 语法检查、生产调用链只读审计和实际专项执行均通过，未发现 select、continue、通用 transition、migration、startup 或 pipeline 新旁路。
- 2026-08-07 最终复验前以 `git status --short`、`git diff --cached --name-status` 和 `git diff --name-only` 交叉核实：Git index 为空，不存在 staged `ONSTITUTION.md` 或其他 staged 路径；`CONSTITUTION.md` 等现有交付文件均为未暂存修改，WO-015 修改的 `scripts/verify-adaptive-execution-closeout.sh` 是开工前即存在且继续保持未跟踪的文件，没有执行暂存、取消暂存、恢复或清理。
- 2026-08-07 按顺序实际运行最终四轨并全部退出 0。`npm run verify:adaptive-execution-closeout` 的 Review 唯一写入门禁、静态边界、资源预检与 diff 门禁通过；Core 60 项通过、455 项过滤（Cargo 0.37 秒、测试 0.27 秒、18 条 unused-code 告警），6 个前端文件 22 项通过（Vitest 1.41 秒），Grok 7 项通过（BuiltIn 3、engine 4，Cargo 3 分 39 秒；lib 报告 25 条告警，test 目标报告 14 条且其中 11 条重复），整条命令墙钟 229.23 秒。`npm run verify:phase1-runtime-contract` 为 Rust 15 项通过、500 项过滤（Cargo 1.25 秒、测试 0.14 秒、18 条告警），19 个前端文件 115 项通过（Vitest 3.40 秒），墙钟 11.05 秒；`npm run verify:grok-check` 为 Cargo 1 分 28 秒、墙钟 88.58 秒、25 条告警；fork `metheus_embedded_runtime` 为 11 项通过（Cargo 2 分 09 秒、测试 2.24 秒、墙钟 131.52 秒）。随后 `git diff --check`（0.03 秒）与 `git diff --cached --check`（0.00 秒）均通过。
- 2026-08-07 本轮仍未运行桌面 smoke、Tauri dev/build、发布构建、无过滤 Rust 全量测试、Clippy、真实模型或付费请求，也未执行 Git 提交、暂存、标签、清理或推送；上述自动化结果不替代真实桌面与真实模型长链路验收。
