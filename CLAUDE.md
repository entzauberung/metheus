# CLAUDE.md — 「弥」项目开发宪法

## 一、项目核心定位

**「弥」（Metheus）是一个专为独立开发者设计的本地桌面应用，核心价值是将「决策层」与「执行层」分离，让用户成为 AI 专家团队的指挥官，而不是手动搬运工。**

核心理念：
- **审批流是灵魂**：所有关键产出（方案、版本、代码）必须经过用户审批才能进入下一环节
- **分层决策**：战略讨论 → 产品拆分 → 技术落地 → 原子执行，每一层解决不同粒度的问题
- **降本增效**：通过原子化任务拆分，让便宜的弱模型达到贵模型的执行质量
- **两种模式**：快速模式（两级：大阶段→小阶段）适用于小项目；专业模式（三级：大阶段→中阶段→小阶段）适用于大型项目

---

## 二、技术栈与开发环境

| 层 | 技术 | 说明 |
|------|------|------|
| 前端 UI | React 19 + TypeScript 5.8 | 函数组件 + Hooks，不使用 class 组件 |
| 桌面容器 | Tauri 2 | 本地桌面应用，不是 Web 应用 |
| 后端服务 | Rust（Tauri 内置）+ reqwest 0.12 | 所有 API 调用在 Rust 后端进行，前端通过 invoke 通信 |
| AI 模型 | DeepSeek API（`deepseek-v4-flash`） | 通过 Rust reqwest 调用 `/v1/chat/completions` 接口 |
| 包管理 | npm（前端）/ Cargo（Rust 后端） | 不要混用包管理器 |
| 运行环境 | Ubuntu 24.04 | 项目路径 `/home/bruce/metheus` |
| 编辑器 | VS Code | 使用 VS Code 内置的 AI 助手辅助编码 |

**关键约束：**
- 前端不直接调任何 AI API。所有 AI 调用必须通过 Rust 后端。
- 不使用任何前端 UI 组件库（Tailwind、Ant Design 等）。所有样式手写 CSS。
- 不使用复杂状态管理库（Redux、Zustand 等）。只用 React 自带的 `useState` / `useEffect`。
- 不在 MVP 阶段引入 WebSocket。

---

## 三、代码规范与命名约定

### 3.1 命名规范

| 类别 | 规范 | 示例 |
|------|------|------|
| Rust 文件名 | `snake_case` | `project.rs`, `lib.rs` |
| Rust 结构体 | `PascalCase` | `Milestone`, `MidStage`, `DiscussionThread` |
| Rust 枚举 | `PascalCase` | `ProjectStatus`, `StageMode` |
| Rust 函数 | `snake_case` | `generate_milestones`, `load_project` |
| Rust 常量 | `SCREAMING_SNAKE_CASE` | `STRATEGY_PROMPT`, `PM_PROMPT` |
| TypeScript 文件名 | `PascalCase`（组件）或 `camelCase`（工具） | `App.tsx`, `ExecutionTree.tsx`, `types.ts` |
| TypeScript 接口/类型 | `PascalCase` | `Milestone`, `MidStage`, `ChatMessage` |
| React 组件函数 | `PascalCase` | `ExecutionTree`, `ChatRoom` |
| 事件处理函数 | `handle` + `PascalCase` | `handleGeneratePlan`, `handleApprove` |
| 普通变量/函数 | `camelCase` | `inputValue`, `getStatusIcon` |
| CSS 类名 | `kebab-case` | `tree-item`, `version-edit-input`, `mid-stage-list` |

### 3.2 TypeScript 使用原则

- 所有组件文件使用 `.tsx` 扩展名
- 所有非组件文件使用 `.ts` 扩展名
- 禁止使用 `any` 类型。必须使用明确的接口定义（类型在 `types.ts` 中集中管理）
- 前后端通信的数据结构必须与 `types.ts` 和 `project.rs` 保持一致
- 使用 `interface` 定义对象类型，使用 `type` 定义联合类型/枚举

### 3.3 代码格式化

- 使用 TypeScript 内置的类型检查（`tsc`）
- Rust 代码使用 `cargo fmt` 格式化（但不强制，MVP 阶段以功能优先）
- 注释用中文，清晰说明"这段代码在做什么"和"为什么这么做"
- 复杂的条件逻辑必须写注释解释判断意图

---

## 四、项目架构与文件结构

### 4.1 目录结构

```
/home/bruce/metheus/
├── src/                          # 前端代码（React + TypeScript）
│   ├── App.tsx                   # 主组件：状态管理 + 版本方案 + 拆解控制
│   ├── App.css                   # 所有样式（单一文件，不拆分）
│   ├── ChatRoom.tsx              # 聊天室组件：多角色对话 + @召唤 + 模式选择
│   ├── ExecutionTree.tsx         # 执行树组件：三级树状渲染 + 版本号编辑
│   └── types.ts                  # 类型定义（与 Rust project.rs 一一对应）
│
├── src-tauri/                    # 后端代码（Rust）
│   └── src/
│       ├── lib.rs                # 核心逻辑：9 个 Tauri 命令 + 5 个角色 prompt
│       └── project.rs            # 数据模型：6 个枚举 + 6 个结构体
│
├── CLAUDE.md                     # 本文件 — AI 协作的最高准则
├── package.json                  # 前端依赖
└── .gitignore                    # Git 忽略配置
```

### 4.2 模块职责边界

**前端（React）：**
- `App.tsx`：项目的状态管理中枢。维护 `Project` 状态，定义所有事件处理函数，决定条件渲染哪些 UI 区域。
- `ChatRoom.tsx`：聊天室的渲染和交互。发送消息、@ 召唤解析、模式选择器。**不直接修改项目状态**，通过回调函数将事件向上传递。
- `ExecutionTree.tsx`：执行树的渲染和交互。三级树状结构、版本号编辑、状态图标。**不直接修改项目状态**，通过回调函数将事件向上传递。
- `types.ts`：纯类型定义文件。**不允许包含任何业务逻辑**。

**后端（Rust）：**
- `lib.rs`：所有 Tauri 命令的定义。每个命令是独立的异步函数。包含 5 个角色的 system prompt 常量定义。
- `project.rs`：纯数据模型定义。包含 `Project`、`Milestone`、`MidStage`、`Subtask`、`DiscussionThread`、`Message` 的结构体和枚举定义。**不允许包含任何业务逻辑**（业务逻辑在 `lib.rs` 的命令函数中实现）。

### 4.3 数据流

```
用户操作 → React 组件（状态驱动）
    │
    ├── invoke("chat_with_role")  ──→  Rust → DeepSeek API → 返回消息
    ├── invoke("generate_*")       ──→  Rust → DeepSeek API → 返回结构化数据
    └── invoke("persist_project")  ──→  Rust → 写入 ~/.metheus/*.json
```

**严禁打破以下规则：**
- ❌ 前端直接调任何 API（包括 DeepSeek、Claude 等）
- ❌ 前端直接读写本地文件（必须经过 Rust 后端）
- ❌ 后端逻辑写入 `project.rs`（必须写在 `lib.rs` 的命令函数里）

---

## 五、核心开发流程与协作准则

### 5.1 五层角色架构

| 角色 | 职责 | 出现时机 | 输出 |
|------|------|---------|------|
| **策略产品经理** | 和用户讨论想法，明确方向 | 项目启动 / 版本复盘时 | `version_plan`（Markdown 格式版本方案） |
| **产品经理** | 将版本方案拆成可交付的小版本 | 方案批准后 | 大阶段列表（v0.1, v0.2...） |
| **域负责人** | 将大阶段拆成技术实现模块 | 大阶段确认后（专业模式） | 中阶段列表（v0.1.1, v0.1.2...） |
| **开发工程师** | 将中阶段拆成 Claude Code 原子任务 | 中阶段确认后 | 小阶段列表 + 提示词 |
| **测试工程师** | 检查代码质量，输出测试报告 | 每个小阶段执行后 | 测试报告 |

**关键规则：**
- **决策层（策略产品经理、产品经理）的 AI 输出必须是文本格式（Markdown 或 JSON），不得在决策层的 prompt 中直接生成代码。**
- **执行层（全栈技术顾问/开发工程师）才负责生成 Claude Code 可执行的提示词。**
- **分层对话**：不是一问四答。当前进展到哪一步，只有对应角色回复。用户通过 `@角色名` 召唤其他角色参与。

### 5.2 状态机流转

```
Idle（空闲）
  → 用户开始讨论 → Discussing（讨论中）
    → 生成版本方案 → 用户批准 → Planning（规划中）
      → 拆解大阶段完成 → MilestoneReady（大阶段就绪）
        → 用户开始执行 → Executing（执行中）
          → 遇到问题 → Paused（暂停）
            → 讨论恢复 → 回到 Executing 或 Discussing
```

**状态机的每个状态决定：**
- 聊天室的默认回复角色（`getDefaultRole` 函数）
- 哪些按钮可见（生成方案、拆解大阶段、执行、审批）

### 5.3 版本号规范

| 层级 | 版本号格式 | 由谁命名 | 示例 |
|------|-----------|---------|------|
| 大阶段 | `v0.x` | 产品经理自动生成，用户可改 | `v0.1`, `v0.2`, `v0.3` |
| 中阶段 | `v0.x.y` | 域负责人自动生成，用户可改 | `v0.1.1`, `v0.1.2`, `v0.2.1` |
| 小阶段 | 无独立版本号 | 不命名 | — |

### 5.4 两种模式

| 模式 | 层级数 | 适用场景 | 说明 |
|------|--------|---------|------|
| **快速模式（Quick）** | 2 级 | 小想法、原型验证 | 大阶段 → 小阶段，跳过中阶段 |
| **专业模式（Professional）** | 3 级 | 大型项目、企业级应用 | 大阶段 → 中阶段 → 小阶段 |

**模式切换规则：**
- 项目启动时在聊天室选择（`modeLocked` 为 false 时可选）
- 一旦生成版本方案，模式锁定（`modeLocked` 为 true）
- 每个大阶段可独立选择模式（`Milestone.mode`），不强制跟随项目模式

---

## 六、资产保护清单

以下文件或代码块在 AI 生成代码时**必须保持原有逻辑不变**，不得删除、重写或改变其核心设计。

### 🔴 受保护的核心文件

| 文件 | 保护级别 | 保护原因 |
|------|---------|---------|
| `src/types.ts` | 🔴 禁止修改结构 | 与 Rust `project.rs` 一一对应，修改此处必须同步修改 Rust 端 |
| `src-tauri/src/project.rs` | 🔴 禁止修改结构 | 项目的核心数据模型，所有功能依赖此数据结构 |
| `src/App.tsx` 中的状态机 `getDefaultRole` | 🟡 仅允许新增状态分支 | 状态机流转是产品的核心交互逻辑 |
| `src/ChatRoom.tsx` 中的 `mentionRegex` 和 `roleMap` | 🟡 仅允许新增角色映射 | @ 召唤机制是聊天室的核心功能 |
| `src/App.css` | 🟡 不允许删除已有的 CSS 类 | 样式是手动写的，没有 UI 库，删除某类可能导致界面崩溃 |
| `src-tauri/src/lib.rs` 中的 5 个 `*_PROMPT` 常量 | 🟡 仅允许优化 prompt 内容 | 角色定义是产品灵魂，不能删除或替换角色 |

### 🟢 可以自由修改的部分

- 组件内部的 UI 细节（按钮文字、颜色、布局微调）
- 错误处理的提示文字
- 注释的内容
- `handleVersionEdit`、`handleModeChange` 等事件处理函数的具体实现（只要不改变外部接口）

---

## 七、协作规则（给所有 AI 助手）

1. **你在生成代码前，必须先读取 CLAUDE.md。**
2. **不得在决策层生成可执行代码。** 策略产品经理、产品经理、域负责人的输出只能是文本/JSON。只有开发工程师和测试工程师可以涉及代码。
3. **不得修改数据结构而不通知前端修改对应的类型定义。** Rust 端的 `project.rs` 和前端 `types.ts` 必须保持一致。
4. **所有 AI 调用的 API Key 必须从 `.env` 文件读取，不能硬编码。**
5. **新功能的实现需遵循"先决策层讨论，再执行层实现"的原则。** 不要绕过用户审批直接生成代码。
6. **遇到不确定的架构决策，不要自行决定。** 记录为"待确认"问题，交给用户决策。