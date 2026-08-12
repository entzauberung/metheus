# 弥 Metheus

<p align="center">
  <img src="https://img.shields.io/badge/status-alpha-orange" alt="status: alpha">
  <img src="https://img.shields.io/badge/license-AGPL%203.0-blue" alt="license: AGPL 3.0">
  <img src="https://img.shields.io/badge/platform-Tauri%20%2B%20React-9cf" alt="platform: Tauri + React">
  <img src="https://img.shields.io/badge/language-Rust%20%7C%20TypeScript-brightgreen" alt="language: Rust | TypeScript">
</p>

<p align="center"><em>运行在你电脑上的本地任务编译系统。</em></p>

<p align="center">Metheus 把复杂目标编译成有范围、依赖、验收和恢复边界的执行单元，再交给合适的模型完成。</p>

<p align="center"><strong>目标是更少 Token、更低成本、更快执行，让更多人能把想法变成软件。</strong> v0.0.4 正在开发中。</p>

---

## ◆ 1. 这是什么？

弥不是一个聊天机器人，也不把“虚拟 AI 软件公司”作为产品定义。它是一个**运行在本地的任务编译系统**：把你的目标转成可检查、可恢复的执行计划，再由执行单元逐项完成。

任务编译闭环包括：

| 阶段 | 作用 |
|------|------|
| 目标编译 | 把自然语言目标整理成明确的版本与验收方向 |
| 任务分解 | 按文件、依赖、风险和复杂度划分有边界的执行单元 |
| 受控执行 | 为每个单元选择合适的执行模型和工具边界 |
| 验证与证据 | 运行可用的本地或自动化验证，记录证据和未知项 |
| 恢复与收口 | 在失败、暂停或人工边界处恢复、确认并保存项目记忆 |

“AI 团队”可以帮助理解这条协作链，但当前重点是任务合同、执行边界和证据闭环，不是已经完成的并行团队。

---

## ◆ 2. 和你用过的东西有什么不同

| 你熟悉的工作方式 | 弥的任务编译方式 |
|------------------|------------------|
| 从行级补全或一次长对话开始 | 先把复杂目标编译成有范围、依赖、验收和恢复边界的任务单元 |
| 把完整项目上下文反复塞给模型 | 用项目宪法和任务合同保存可复用的项目记忆，只注入当前单元所需事实 |
| 让同一个强模型包办所有步骤 | 高层决策使用更强模型；受约束、可验证的执行单元可以交给更小、更便宜的模型 |
| 失败后重新描述目标、手动判断是否继续 | 每个单元保留验证证据、恢复分类和人工边界，便于暂停、重试或回退 |
| 以堆叠更多模型换取更高成本 | 通过任务边界和精确上下文减少无效 Token；更低成本、更快执行是架构目标，不是已测性能结论 |

---

## ◆ 3. 关键特性

### 🧩 3.1 决策-执行分离流水线

策略产品经理通过可配置的 OpenAI Compatible 决策接口完成对话、检查和规划；执行层可选择 Grok Build 内置运行时，或 Claude Code、Codex、Kimi CLI、Grok Build CLI 本机插件。

> 这不是把更多模型叠在一起，而是先把工作变成可验证的单元，再让模型强度匹配任务风险。这样才能朝着更少 Token、更低成本和更快执行推进；实际收益仍需在真实项目中验证。

流水线支持：`暂停` → `恢复` → `自动重试` → `测试检查` → `宪法更新` → `Git 存档`

Grok Build 内置模式是已接入的**受控 Rust 进程内运行时**：它与本机 CLI 插件和认证隔离，路径与写入受执行计划约束，密钥不写入项目或应用设置 JSON。模型连接和运行时自检通过后才可选择；这不保证真实模型调用或桌面环境已经验收。

当前只保留必要的嵌入与审计基础。上游来源、受控 Fork 和最小改动理由见 [UPSTREAM_SOURCE.md](./third_party/grok-build/UPSTREAM_SOURCE.md)、[FORK_SOURCE.md](./third_party/grok-build-fork/FORK_SOURCE.md) 与 [PATCHSET.md](./third_party/grok-build-fork/PATCHSET.md)。v0.0.5 才规划对 Grok Build 的上下文、工具边界、重试和资源控制做全面重构与定制，并把执行层推进为真正并行系统。

### 📜 3.2 项目宪法（滚动知识库）

每个项目自动维护一份 `CONSTITUTION.md`：

| 部分 | 内容 | 维护方式 |
|------|------|---------|
| **第 1 部分** | 用户确认的核心原则 | 不可变 |
| **第 2 部分** | 项目架构 · 文件结构 · 变更记录 | AI 自动维护 |

三层防御机制：

```
🛡️ 校验    → AI 更新后检查合法性
🛡️ 降级    → 校验失败 → 机械兜底更新
🛡️ 剪枝    → 内容过长 → 自动压缩，保留关键信息
```

> **AI 团队不再依赖你的个人记忆力。**

### 🔖 3.3 细粒度 Git 回退

```
metheus/v0.1.1                    ← 中阶段 tag
metheus/auto/v0.1.1/task-0        ← 子任务 tag
```

回退时自动 `stash` 未提交变更。你可以回到**任何一个子任务执行前的状态**。

### 🖥 3.4 任务控制工作区

执行阶段使用三栏工作区：左侧是一棵支持任意深度子任务的递归任务树，中间保留当前任务、执行日志与代码变更，右侧任务检查器展示所选任务的完整控制事实。左、中、右区域分别滚动；右侧宽度可拖动，在中小窗口中切换为抽屉或全屏详情。

任务检查器分为四页：

| 页面 | 内容 |
|------|------|
| **概览与合同** | 目标、文件范围、依赖、复杂度、风险和合同指纹 |
| **验收与证据** | 逐项状态、证据文件与行号、自动测试状态和接受偏差入口 |
| **决策与恢复** | 当前控制决策、Shadow 对照、恢复分类、次数和人工边界 |
| **成本与事件** | 项目/阶段/任务成本、模型调用和关联控制事件 |

日志中的任务链接、左侧选中状态和右侧详情使用同一个任务 ID。父任务可展开和查看，但不能作为执行叶子。

验证结果严格区分三种通道：`LocalValidate` 是本地确定性证明，`AutomatedValidate` 是真实测试命令，`TargetedValidate` 是 AI 语义审查。测试未配置、测试环境不可用和证据不足都保持未知状态，不会被当作通过，也不会被误判为代码失败。

v0.0.4 第一阶段已完成的控制基础中，`SerialTakeover` 是新项目的默认串行模式，负责任务执行阶段的派发与恢复决策；`Shadow` 只做对照审计，`Legacy` 只用于兼容和显式人工回退。旧项目保留磁盘中已有模式，不会自动迁移。这个基础不表示 v0.0.4 整体或真正并行执行已经完成。

### 🔓 3.5 模型边界

决策模型与执行模型各自配置；Grok Build 内置密钥与 Grok Build CLI 的本机认证完全隔离。

> **不被任何 AI 厂商绑定。**

---

## ◆ 4. 快速开始

### 你需要什么

| 依赖 | 说明 |
|------|------|
| Rust (stable) | 后端编译 |
| Node.js 20+ | 前端运行 |
| Tauri CLI | `cargo install tauri-cli` |
| 至少一个执行引擎 | Grok Build 内置模式，或 Claude Code / Codex / Kimi / Grok Build CLI |

### 三步启动

```bash
git clone https://github.com/entzauberung/metheus.git
cd metheus
npm install && npm run tauri dev
```

在“应用设置”中配置决策模型。在显式包含 `full-product` 的产品构建中使用 Grok Build 内置模式时，另行选择接口后端、地址和模型，将 API Key 保存到系统凭据库或仅用于本次会话，然后依次执行“测试模型连接”和“运行时自检”。API Key 不写入项目或应用设置 JSON。

### 开发构建与验证

日常开发默认不编译预装 Grok Build：Cargo 默认特性为空，Claude Code、Codex、Kimi 和 Grok Build CLI 插件仍可使用。轻量构建读取到已选择内置 Grok Build 的旧项目时会保留项目设置，但明确阻止执行，不会静默切换引擎。

```bash
npm run verify:core-light   # Core 格式与库目标类型检查
npm run verify:quality      # 定向 Rust 测试、TypeScript 和前端策略测试
npm run verify:phase1-runtime-contract # 第一阶段正式接管运行时契约，不做产品构建
npm run verify:grok-check   # 单任务检查内置 Grok 特性，不做最终链接
```

`builtin-grok` 只启用内置运行时，`full-product` 是包含该能力的产品特性。正式发布必须在高资源环境中显式传入 `full-product`，例如 `npm run tauri -- build --features full-product`；不能用轻量检查结果代替发布打包验收。Core 与 Grok 专项分别使用 `.build/core` 和 `.build/grok-full`，不得共享缓存。

### 首次使用的路径

```
1. 打开弥，输入项目名称
       ↓
2. 和策略产品经理对话，描述你的想法
       ↓
3. 审批生成的版本方案
       ↓
4. 弥自动拆任务 → 写代码 → 跑测试 → 存档
       ↓
5. 在任务控制台查看进度和变更
```

---

## ◆ 5. 项目结构一瞥

```
metheus/
├── src/                          # React 前端
│   ├── App.tsx                   # 状态机中心枢纽
│   ├── ExecutionTree.tsx         # 唯一递归任务树
│   ├── TaskConsole.tsx           # 日志、Diff、宪法与标签
│   ├── TaskInspector.tsx         # 右侧任务检查器
│   ├── V1ExecutionPanel.tsx      # 当前任务执行与确认面板
│   ├── types.ts                  # 数据结构定义
│   └── components/
│       ├── AutopilotControlBar.tsx
│       ├── ApplicationSettings.tsx
│       └── ExecutionEngineSettings.tsx
├── src-tauri/                    # Rust 后端
│   ├── src/
│   │   ├── lib.rs                # 核心逻辑，30+ Tauri 命令
│   │   ├── project.rs            # 数据结构定义
│   │   └── main.rs               # 入口
│   ├── Cargo.toml
│   └── tauri.conf.json
├── third_party/
│   ├── grok-build/               # 固定修订的原样审计基线
│   └── grok-build-fork/          # 受控 SessionActor 嵌入 Fork
├── package.json
└── vite.config.ts
```

---

## ◆ 6. 哲学

> **降本增效。** 以任务边界和精准上下文减少无效 Token，朝着更低成本、更快执行推进；具体收益仍需真实项目验证。
>
> **模型自由。** 不被任何 AI 厂商绑定，让模型强度服从任务风险。
>
> **AI 领导 AI。** 让决策和受控执行各自承担清晰职责，而不是把产品定义成聊天机器人。
>
> **代码记忆 > 人脑记忆。** 项目的关键事实、约束和证据固化在项目记忆里。
>
> **开源实践。** 坚持马列毛主义，把软件生产能力交给更多普通人；不拿 VC 的钱，也不把上市作为目标。

---

## ◆ 7. 项目状态

<p align="center">
  <strong>Alpha</strong> · 核心工作流已跑通 · 尚未达到日常使用稳定性
</p>

### 一个独立开发者的说明

我来自甘肃农村，高二因双相情感障碍和语言障碍休学，在粮仓里自学编程走到今天。现在把饭钱投入 API，生活困难、资金不足让开发很艰难；我仍在持续开发，把全部精力投入 GitHub 开源事业，也坚持马列毛主义实践。

这不是募资请求或项目承诺。长期愿景是在五年内把软件生产成本压到接近电费、接近一杯奶茶的程度，让更多人能够生产软件。

### 当前主要技术问题

```
⚠️ 真实桌面窗口的多尺寸端到端回归仍需发布环境复验
⚠️ 真实模型与 CLI 烟雾测试依赖用户自己的认证、网络和额度
⚠️ 全量 Rust 库测试仍有少量既有工作流/引擎健康场景待单独收口
⚠️ 前端入口仍承担宏观状态机与多视图协调，后续重构需独立进行
```

> 持久化恢复、暂停/回退、自动驾驶状态和执行输出管道已完成本轮稳定性修复；当前不把历史遗留问题继续当作未完成项。

### 为什么仍然相信

```
✓ 任务编译可以把"想法"变成有边界、可验证的执行单元
✓ 项目记忆让关键事实不依赖某一次对话
✓ 细粒度 Git 回退给了用户真正的安全感
✓ 便宜的决策 + 精准的执行，是独立开发者负担得起的方向
✓ Grok Build 已作为受控的 Rust 进程内执行引擎接入统一流水线
```

每解决一个问题，就离**用一杯奶茶钱改变世界**更近一步。

### 长期愿景

我不会放弃。**二十步天注定，逆流河上任我行。** 即使资金困难，我也会继续推进这条低成本、开源的软件生产道路。

> 如果你对这个项目感兴趣，欢迎提 Issue 或 PR。如果你认同它的愿景，一个 ⭐ Star 也是支持。

---

## ◆ 8. 路线图

| 版本 | 状态 | 已有基础与主要方向 |
|------|------|------------------|
| **v0.0.4** | **开发中** | 任务控制、任务合同、`SerialTakeover`、验证/恢复/证据基础已完成定向验证；第二阶段推进 Hermes 记忆模式、轻度机器学习和分级成本控制；第三阶段由决策层向执行单元分派任务 |
| **v0.0.5** | **规划中** | 对 Grok Build 做全面定制化，并将执行层推进为真正并行执行系统 |
| **v0.0.6** | **规划中** | 规划 Metheus 特化的 [metheus-prp](https://github.com/entzauberung/metheus-prp)，让小模型承担渐进式推理执行单元，继续压低成本、挖掘性能 |

通用 [PRP](https://github.com/entzauberung/prp) 是厂商无关的独立协议仓库；[metheus-prp](https://github.com/entzauberung/metheus-prp) 是面向 Metheus v0.0.6 的特化实现。该规划希望用渐进式推理让小模型承担执行单元，进一步压榨性能、降低 Token 与成本、提高执行速度；当前尚未集成。

> 当前内置引擎仍按小阶段串行执行。v0.0.4 正在推进由决策层向执行单元分派任务；v0.0.5 才规划把执行层做成真正并行系统，二者都不是当前可用性声明。
>
> 将所有精力投入支持那些用「弥」创造社会价值的人。**用一杯奶茶钱改变世界。**

---

## ◆ 9. 许可证

**GNU Affero General Public License v3.0 (AGPL-3.0)**

Copyright (c) 2025 entzauberung

本项目采用 AGPL 3.0 许可证。这意味着：

- 你可以自由使用、修改、分发
- 如果你基于它提供网络服务，你必须公开你的全部修改
- 这是我们有意的选择——我们不希望任何人把「弥」变成闭源的商业服务

详见 [LICENSE](./LICENSE) 文件。

---

<p align="center">
  <br>
  <strong>仍在持续开发。</strong>
  <br><br>
  <em>— entzauberung · Искров</em>
</p>
