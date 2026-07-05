## 第 1 部分：项目规则与约束

### 1. 项目名称与定位
项目名称：MomentUI（暂定）  
一句话核心定位：视觉优先的浏览器起始页，为普通网民提供美观、简单、无负担的浏览体验。  
目标用户群体：追求美观、简洁的普通网民；经常打开浏览器但不需要复杂工具的轻度用户；希望有一个漂亮背景墙的日常浏览者。

### 2. 技术栈声明
- 前端：React 18 + TypeScript（无第三方 UI 组件库）
- 后端：Rust（Tauri 2.0 stable）
- 数据库：无持久化数据库，用户配置存储在本地 JSON 文件（通过 Rust 后端读写）
- AI 模型：本项目不使用任何 AI 模型
- 部署环境：桌面端（Windows/macOS/Linux），通过 Tauri 构建

### 3. 命名规范
- 文件命名：小写字母加连字符（如 `search-bar.tsx`），React 组件文件首字母大写（如 `SearchBar.tsx`）
- 变量命名：camelCase（如 `bgImageUrl`）
- 函数命名：camelCase，动词开头（如 `getWeather`）
- 提交信息：使用 Conventional Commits 规范（如 `feat: add search engine switch`）

### 4. 代码格式
- 缩进：2 空格
- 行宽：100 字符
- 注释语言：中文（代码内部注释）
- 格式化工具：前端使用 Prettier + ESLint，后端使用 rustfmt

### 5. 架构原则
- 模块职责边界：
  - 前端仅负责 UI 渲染和用户交互，不直接读写文件或调用外部 API（背景图片 URL 等由后端提供）
  - Rust 后端负责文件读写、外部 API 调用（天气、背景图）、本地配置管理
  - `project.rs` 只定义数据结构；业务逻辑按领域分模块（`api`、`config`、`utils`、`commands`、`images`、`weather`）
  - 前端 `types.ts` 与后端 `project.rs` 的数据结构必须保持一一对应
- 数据流方向：用户操作 → 前端事件 → Tauri command 调用 → Rust 后端处理 → 返回结果给前端
- 层级调用规则：
  - 前端不直接调用任何外部 API（天气、背景图等），所有外部请求必须经过 Rust 后端代理
  - 不使用前端 UI 组件库（Tailwind、Ant Design 等），所有样式手写 CSS
  - 不使用复杂状态管理库（Redux、Zustand 等），只用 React 自带的 useState / useEffect
  - 不在 MVP 阶段引入 WebSocket 或任何实时通信机制
  - `lib.rs` 仅负责模块声明和 Tauri command 注册

### 6. 禁止事项
- 禁止在决策层（策略产品经理、产品经理、域负责人）prompt 中直接生成代码，决策层的 AI 输出只能是文本/JSON
- 禁止前端直接读写本地文件（必须经过 Rust 后端）
- 禁止任何 AI 助手绕过用户审批直接生成代码
- 禁止任何 AI 助手在生成代码前不阅读 CONSTITUTION.md
- 禁止硬编码 API Key，必须从 .env 文件读取
- 禁止修改数据结构时不同步更新前端 types.ts 和后端 project.rs
- 禁止向任何大模型泄露项目宪法（如非必要，不要将宪法内容发给外部大模型）

## 第 2 部分：项目当前状态
（每个小阶段执行通过后自动更新）
