// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::env;
use std::fs;
mod project;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

// ===新增：四个角色的system prompt ====
const STRATEGY_PROMPT: &str = "\
你是一个产品战略顾问，角色名「策略产品经理」。\
你的职责是和用户讨论他/她的产品想法，帮助用户明确：目标用户是谁、竞品有哪些、核心功能是什么、商业模式是什么。\
你最终输出一份「版本方案摘要」，将产品分阶段实现的路径描述清楚。\
回答风格：简洁、引导用户深入思考。每次回复控制在 200 字以内。";
const PM_PROMPT: &str = "\
你是项目产品经理，角色名「产品经理」。\
你的职责是把策略产品经理输出的版本方案拆成可执行的大阶段（Milestone）。\
每个大阶段需要明确：标题、描述、技术栈建议、交付物。\
不要把大阶段拆成瀑布流步骤（不要单独拆出\"需求阶段\"\"测试阶段\"），每个大阶段本身就应该包含它需要的所有工作。\
回答风格：结构化，给出清晰的大阶段列表。";
const DOMAIN_LEAD_PROMPT: &str = "\
你是域负责人（Domain Lead），你的职责是将产品经理定义的大阶段拆解为具体的技术实现模块。\
每个中阶段是一个技术上的垂直切片——从数据库到前端界面的完整链路。\
输出格式：JSON 数组，每个元素包含：version（字符串，格式 v0.1.1、v0.1.2…）、\
title（字符串）、description（字符串）、tech_focus（字符串）。\
你只输出 JSON 数组，不要包含 markdown 代码块标记。不要任何解释文字。输出必须以 [ 开头。";
const TECH_PROMPT: &str = "\
你是全栈技术专家，角色名「开发工程师」。\
你的职责是把产品经理定义的大阶段拆成可执行的小阶段（Subtask），每个小阶段生成精确的提示词供 Claude Code 执行。\
每个小阶段控制在 10-30 行代码以内，确保可以被一次性正确执行。\
回答风格：精确、技术向，输出可直接执行的提示词。\
请严格按 JSON 格式输出，不要包含 markdown 代码块标记：\n{\"title\": \"子任务标题\", \"prompt\": \"可执行的 Claude Code 提示词\"}\n\n**重要约束：**\n- 不得在提示词中包含完整的代码块\n- 提示词应描述「做什么」（功能目标），而不是「写什么」（具体代码实现）\n- 必须指定要操作的文件路径（相对于项目根目录）\n- 涉及修改已有函数时，需要提供现有函数签名作为参考";
const TEST_PROMPT: &str = "\
你是测试工程师，角色名「测试工程师」。\
你的职责是检查代码质量和功能正确性。\
你需要读取被修改的文件，验证逻辑是否正确、边界情况是否处理、代码风格是否规范。\
输出格式：通过/不通过 + 问题列表（如果不通过）。\
回答风格：客观、具体，指出具体文件和行号。\
请严格按 JSON 格式输出，不要包含 markdown 代码块标记：\n{\"passed\": true或false, \"issues\": [\"问题1\"], \"suggestion\": \"改进建议\", \"warnings\": []}\n\n若自动化测试结果显示「未配置测试用例」或「测试命令不存在」，不要因此判定代码不通过，应仅基于代码审查本身判断。";
const SELF_CHECK_PROMPT: &str = "\
你是版本方案自检专家。\
请对照【用户与策略产品经理的讨论记录】检查【刚产出的版本方案】，从以下三个维度进行核查：\
1. 遗漏检查：讨论记录中用户明确提出的功能需求和约束条件，是否都在版本方案中有所体现？如有遗漏，请补充。\
2. 多余检查：版本方案中是否存在讨论记录中从未提及的内容？如果存在且不合理（属于幻觉或过度设计），请移除。\
3. 偏好/约束检查：讨论记录中用户表达的偏好（如技术栈偏好、设计风格、目标平台等），版本方案是否遵循？如有偏离，请修正。\
如果发现任何问题，请输出修正后的完整版本方案（Markdown格式，包含所有章节标题和内容）。\
如果方案完全对齐无问题，请直接原样输出版本方案。\
你只输出版本方案的Markdown内容，不要包含任何解释、前言或后缀文字。";
const QA_CHECK_PROMPT: &str = "\
你是需求质检员。\
请对照【原始需求（版本方案）】检查【当前产出（大阶段列表）】，判断两者是否对齐。\
检查要点：\
1. 大阶段列表中的所有内容是否都能在版本方案中找到对应依据。\
2. 版本方案中的所有关键需求是否在大阶段列表中都有对应覆盖。\
3. 大阶段列表中是否存在版本方案中不存在的内容（过度设计）。\
输出格式：JSON 对象，包含以下字段：\
- passed：布尔值，是否通过质检。\
- reason：字符串，未通过时写具体偏差内容，通过时写\"全部对齐\"。\
- details：数组，每个元素包含 issue_type（字符串，如\"遗漏\"、\"多余\"、\"偏离\"）、description（字符串）、related_requirement（字符串）。\
- attention_points：字符串数组，从版本方案中提取的需特别关注的要点。\
- checked_at：字符串，当前时间的 ISO 8601 格式（如 2026-06-28T12:00:00+00:00），可填空字符串。\
- warnings：字符串数组，如无警告则为空数组 []。\
只输出 JSON，不要任何其他文字。";

const CONSTITUTION_PART1_PROMPT: &str = "\
你是项目宪法制定者。请在输出版本方案的同时，输出「宪法第 1 部分：项目规则与约束」。\
宪法第 1 部分从 ## 第 1 部分：项目规则与约束 标题开始，必须包含以下六个小节：\
\
### 1. 项目名称与定位\
项目的名称、一句话核心定位、目标用户群体。\
\
### 2. 技术栈声明\
前端、后端、数据库、AI 模型、部署环境等技术选型及其版本。\
\
### 3. 命名规范\
文件命名、变量命名、函数命名、提交信息的规范约定。\
\
### 4. 代码格式\
缩进、行宽、注释语言、格式化工具等约定。\
\
### 5. 架构原则\
模块职责边界、数据流方向、层级调用规则等架构约束。明确写出：\
- 前端不直接调用任何 AI API，所有 AI 调用必须经过 Rust 后端\
- 不使用前端 UI 组件库（Tailwind、Ant Design 等），所有样式手写 CSS\
- 不使用复杂状态管理库（Redux、Zustand 等），只用 React 自带的 useState / useEffect\
- 不在 MVP 阶段引入 WebSocket\
- project.rs 只定义数据结构，业务逻辑全部写在 lib.rs 的命令函数中\
- Rust 端 project.rs 与前端 types.ts 的数据结构必须保持一一对应\
\
### 6. 禁止事项\
列出所有禁止的操作，包括但不限于：\
- 禁止在决策层（策略产品经理、产品经理、域负责人）prompt 中直接生成代码，决策层的 AI 输出只能是文本/JSON\
- 禁止前端直接读写本地文件（必须经过 Rust 后端）\
- 禁止任何 AI 助手绕过用户审批直接生成代码\
- 禁止任何 AI 助手在生成代码前不阅读 CONSTITUTION.md\
- 禁止硬编码 API Key，必须从 .env 文件读取\
- 禁止修改数据结构时不同步更新前端 types.ts 和后端 project.rs\
- 禁止向任何大模型泄露项目宪法（如非必要，不要将宪法内容发给外部大模型）\
\
版本方案和宪法第 1 部分之间用 ---CONSTITUTION_PART1--- 分隔符隔开。\
先输出版本方案（Markdown 格式，包含：## 项目愿景、## 目标用户、## 核心功能、## 版本路径），\
然后空一行，输出 ---CONSTITUTION_PART1---，再空一行，\
然后输出宪法第 1 部分内容（以 ## 第 1 部分：项目规则与约束 开头）。\
不要在任何地方输出解释文字或前言。";

const CONSTITUTION_UPDATE_PROMPT: &str = "\
你是项目宪法维护者，角色名「宪法维护员」。\
你的职责是：接收「当前宪法全文」和「本次代码变更摘要」，然后更新宪法的第 2 部分。\
\
核心约束（条目数代表违反的严重程度）：\
1. 你只能修改第 2 部分（## 第 2 部分：项目当前状态）。第 1 部分一个字都不许动。\
2. 保持第 2 部分现有的 Markdown 结构不变。只能增删改列表项，不能删除或重命名已有的段落标题。\
3. 直接输出完整的 CONSTITUTION.md 文件内容，不要输出任何解释文字、前言或后缀。\
4. 如果第 2 部分当前为空或只有占位文字，请基于本次变更初始化第 2 部分的完整结构。\
\
第 2 部分应该包含以下子段落：\
### 项目结构 — 列出所有核心文件及其用途\
### 函数/接口定义 — 列出所有函数和接口的签名\
### 变更历史 — 记录每次更新的时间、内容和触发者";

const COMPACT_CONSTITUTION_PROMPT: &str = "\
你是项目宪法维护者，角色名「宪法压缩员」。\
你的职责是：接收「当前宪法全文」，压缩宪法的第 2 部分以控制其膨胀。\
\
核心约束（条目数代表违反的严重程度）：\
1. 你只能修改第 2 部分（## 第 2 部分：项目当前状态）。第 1 部分一个字都不许动。\
2. 保留最新的项目结构（文件树），删除已被后续覆盖的过时条目。\
3. 如果旧函数名已被新函数替代，只保留最新的函数定义。\
4. 变更历史：保留最近 5 条完整记录，更早的合并为一行概述（如「v0.1.1/task-1~5：完成了用户认证模块的初始开发」）。\
5. 保持 Markdown 结构和标题层级不变（### 项目结构、### 函数/接口定义、### 变更历史）。\
6. 压缩后第 2 部分的目标：约 1500 token。\
7. 直接输出完整的 CONSTITUTION.md 文件内容，不要输出任何解释文字、前言或后缀。\
\
压缩技巧：\
- 合并相似的文件条目（如多个测试文件合并为「测试文件：test_*.rs」）。\
- 删除已被后续提交覆盖的条目（如 v0.1.1/task-1 新增的 foo.rs，v0.1.1/task-3 又删除了它——两者都可以从历史中移除）。\
- 函数签名相同的重复条目只保留一个。\
- 变更历史的早期条目用一句话概括每个小阶段的关键变更。";

/// sanitize_json_response 的兜底值：当清洗结果为空时返回最小合法 JSON 对象
const SANITIZE_FALLBACK_JSON: &str = "{}";

/// DeepSeek API HTTP 请求超时秒数，防止网络故障导致永久阻塞
const DEEPSEEK_API_TIMEOUT_SECS: u64 = 120;

/// Claude Code 子进程整体执行超时秒数，防止子进程卡死
const CLAUDE_CODE_TIMEOUT_SECS: u64 = 600;

///获取项目文件的存储路径
fn get_project_path(name: &str) -> String {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{}/.metheus/{}.json", home, name)
}

const GIT_INIT_FAILED: &str = "自动初始化 Git 仓库失败";
const GIT_AUTO_INIT_COMMIT_MSG: &str = "初始提交（由 Metheus 自动创建）";

/// 校验项目路径：存在性、目录类型、git 仓库
fn check_project_path(path: &str) -> project::PathValidationResult {
    let p = std::path::Path::new(path);
    let exists = p.exists();
    let is_directory = exists && p.is_dir();
    // 兼容 worktree：.git 可能是文件而非目录
    let is_git_repo = is_directory && p.join(".git").exists();

    let mut errors: Vec<&str> = Vec::new();
    if !exists {
        errors.push("路径不存在");
    } else if !is_directory {
        errors.push("路径不是目录");
    }

    project::PathValidationResult {
        is_valid: exists && is_directory,
        exists,
        is_directory,
        is_git_repo,
        error_message: if errors.is_empty() {
            String::new()
        } else {
            errors.join("；")
        },
    }
}

///保存项目数据到文件
fn save_project(project: &project::Project) -> Result<(), String> {
    //1. 确保 .metheus项目存在
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = format!("{}/.metheus", home);
    fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败：{}", e))?;
    //2.序列化为JSON
    let json = serde_json::to_string_pretty(project).map_err(|e| format!("序列化失败: {}", e))?;
    //3.写入文件
    let path = get_project_path(&project.name);
    fs::write(&path, json).map_err(|e| format!("写入文件失败: {}", e))?;
    Ok(())
}

/// 根据项目名字，从硬盘文件里加载项目数据
// 比如输入 "my_game"，就去 ~/.metheus/my_game.json 里读取，还原成 Project 对象
fn load_project(name: &str) -> Result<project::Project, String> {
    // 1. 根据名字生成文件路径（例如 "/home/张三/.metheus/my_game.json"）
    let path = get_project_path(name);

    // 2. 读取整个文件内容 → 得到一个 JSON 字符串
    //    如果文件不存在或无法读取，就返回错误
    let data = fs::read_to_string(&path).map_err(|e| format!("读取文件失败：{}", e))?;

    // 3. 把 JSON 字符串解析成 Project 结构体
    //    如果格式不对（比如缺少必要字段），就返回错误
    let project = serde_json::from_str(&data).map_err(|e| format!("解析 JSON 失败：{}", e))?;

    // 4. 成功时，把 Project 对象装进 Ok 信封返回
    Ok(project)
}

fn load_env() {
    dotenvy::dotenv().ok();
}
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
async fn send_message(message: String) -> Result<String, String> {
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!("[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端", e);
            reqwest::Client::new()
        });
    let request_body = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": [
            {"role": "user", "content": message}
        ]
    });
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;
    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;
    let reply = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复异常".to_string())?
        .to_string();
    Ok(reply)
}
// ===新增: 多角色对话命令 ===
#[tauri::command]
async fn chat_with_role(
    message: String,
    role: String,
    _thread_id: String,
) -> Result<project::Message, String> {
    //1. 根据角色选择system prompt
    let system_prompt = match role.as_str() {
        "策略产品经理" => STRATEGY_PROMPT,
        "产品经理" => PM_PROMPT,
        "域负责人" => DOMAIN_LEAD_PROMPT,
        "全栈技术顾问" => TECH_PROMPT,
        "测试工程师" => TEST_PROMPT,
        _ => return Err(format!("未知角色: {}", role)),
    };
    //4.发送请求 -> 调用ai  3.4.1b改动: 换为封装函数
    let reply = call_deepseek_api(&system_prompt, &message).await?;
    //5.返回结构化Message对象（非纯字符串）
    let ai_message = project::Message {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content: reply.clone(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
    };
    Ok(ai_message)
}

#[tauri::command]
async fn generate_version_plan(
    messages: Vec<project::Message>,
    project_path: String,
) -> Result<String, String> {
    //1.读群API密钥
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    //2.构造API消息列表：system_prompt + 对话历史
    let mut api_messages: Vec<serde_json::Value> = vec![serde_json::json!({
        "role": "system",
        "content": format!(
            "{} {}",
            "你是一个产品战略顾问，角色名「策略产品经理」。\
             请根据以下对话历史，输出一份结构化的「版本方案摘要」。\
             使用 Markdown 格式，包含以下章节：\
             ## 项目愿景\n## 目标用户\n## 核心功能\n## 版本路径\n\
             每个版本路径下的版本要清晰列出。\
             回答风格：结构化、清晰、可直接用于执行。",
            CONSTITUTION_PART1_PROMPT
        )
    })];
    //把对话历史换成API格式
    for msg in &messages {
        let api_role = if msg.role == "user" {
            "user"
        } else {
            "assistant"
        };
        api_messages.push(serde_json::json!({
            "role":api_role,
            "content": msg.content
        }));
    }
    let request_body = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": api_messages
    });
    //3.发送请求
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!("[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端", e);
            reqwest::Client::new()
        });
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;
    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败：{}", e))?;
    let mut plan = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复异常".to_string())?
        .to_string();

    // === 宪法第 1 部分拆分与写入 ===
    // 按分隔符将 AI 返回内容拆成两部分：版本方案 + 宪法第 1 部分
    if let Some(constitution_start) = plan.find("---CONSTITUTION_PART1---") {
        let constitution_part1 = plan[constitution_start + "---CONSTITUTION_PART1---".len()..]
            .trim()
            .to_string();
        plan = plan[..constitution_start].trim().to_string();

        // 构造完整宪法内容（第 1 部分 + 第 2 部分占位）
        let constitution_full = if !constitution_part1.is_empty() {
            format!(
                "{}\n\n## 第 2 部分：项目当前状态\n（每个小阶段执行通过后自动更新）\n",
                constitution_part1.trim()
            )
        } else {
            // 第 1 部分为空也写入占位模板
            "## 第 1 部分：项目规则与约束\n（由策略产品经理在版本方案阶段自动生成）\n\n## 第 2 部分：项目当前状态\n（每个小阶段执行通过后自动更新）\n".to_string()
        };

        // 写入项目根目录的 CONSTITUTION.md
        // 写入失败只记录警告，不中断版本方案返回
        let constitution_path = std::path::Path::new(&project_path).join("CONSTITUTION.md");
        if let Err(e) = std::fs::write(&constitution_path, &constitution_full) {
            eprintln!(
                "[generate_version_plan] 警告：写入 CONSTITUTION.md 失败：{}",
                e
            );
        }
    }
    // 如果 AI 返回中不包含分隔符，不写入 CONSTITUTION.md，流程继续

    // === 自检逻辑：对照讨论记录检查版本方案是否遗漏内容 ===
    // 第一步：拼接讨论记录字符串
    let mut discussion = String::new();
    for msg in &messages {
        let display_name = if msg.role == "user" {
            "用户"
        } else {
            &msg.role
        };
        discussion.push_str(&format!("{}：{}\n", display_name, msg.content));
    }
    // 讨论记录超过3000字符则只保留最后3000字符（统一按字符数截断，不依赖字节长度）
    {
        let discussion_chars: Vec<char> = discussion.chars().collect();
        if discussion_chars.len() > 3000 {
            discussion = discussion_chars[discussion_chars.len() - 3000..]
                .iter()
                .collect();
        }
    }

    // 第二步：构造自检请求用的用户消息内容
    let mut self_check_user_message = format!(
        "【用户与策略产品经理的讨论记录】\n{}\n\n【刚产出的版本方案】\n{}",
        discussion, plan
    );

    // 如果自检消息总长超过8000字符，截断plan部分（保留前4000字符）
    if self_check_user_message.chars().count() > 8000 {
        let plan_chars: Vec<char> = plan.chars().collect();
        let truncated_plan: String = if plan_chars.len() > 4000 {
            format!(
                "{}...（以下省略）",
                plan_chars[..4000].iter().collect::<String>()
            )
        } else {
            plan.clone()
        };
        self_check_user_message = format!(
            "【用户与策略产品经理的讨论记录】\n{}\n\n【刚产出的版本方案】\n{}",
            discussion, truncated_plan
        );
    }

    // 第三步：调用自检 API（不强制 JSON 输出的纯文本版本）
    match call_deepseek_api(SELF_CHECK_PROMPT, &self_check_user_message).await {
        Ok(reply) => {
            let trimmed = reply.trim();
            if trimmed.is_empty() {
                // 第四步：自检返回空内容，保留原始方案
                eprintln!("[generate_version_plan] 自检返回空内容，保留原始方案");
            } else {
                plan = trimmed.to_string();
            }
        }
        Err(e) => {
            // 第四步：自检调用失败，保留原始方案
            eprintln!(
                "[generate_version_plan] 自检调用失败：{}，使用原始版本方案",
                e
            );
        }
    }

    Ok(plan)
}

#[tauri::command]
async fn approve_version_plan(
    project_json: String,
    version_plan: String,
) -> Result<String, String> {
    //1. 把JSON字符串转成真正的Project对象
    let mut project: project::Project =
        serde_json::from_str(&project_json).map_err(|e| format!("解析项目失败：{}", e))?;

    //2. 更新版本方案和状态
    project.version_plan = version_plan;
    project.status = project::ProjectStatus::Planning;
    //3. 保存到文件
    save_project(&project)?;
    Ok("批准成功".to_string())
}

///接收前端传来的 version_plan（已批准的方案文本）和 mode（"Quick" / "Professional"）
///用产品经理角色（PM_PROMPT）+ 模式信息构造 system prompt
///调 DeepSeek API，让 AI 根据方案输出大阶段列表（JSON 数组）
///解析 JSON，构造 Vec<Milestone> 返回给前端
#[tauri::command]
async fn generate_milestones(
    version_plan: String,
    mode: String,
) -> Result<Vec<project::Milestone>, String> {
    //1. 读API  读取 API 密钥
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    //拼好提示词
    //2. 构造 system prompt （产品经理角色 + 模式信息）
    let system_prompt = format!(
        "{}\n\n当前项目模式：{}。\
         如果是专业模式，输出的每个大阶段应包含 mid_stages 字段（空列表）；\
         如果是快速模式，输出的每个大阶段应包含 subtasks 字段（空列表）。\
         每个大阶段的 version 字段格式为 v0.1、v0.2 等。\
         你只输出 JSON 数组，不要输出其他文字，不要包含 markdown 代码块标记。\
         每个大阶段包含：version（字符串）, title（字符串）, description（字符串）, tech_stack（字符串）。",
        PM_PROMPT, mode
    );
    //3. 构造 API 消息
    let request_body = serde_json::json!({
        "model": "deepseek-v4-flash",
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": format!("请根据以下版本方案拆解为3-5个大阶段：\n{}", version_plan)}
            ]
    });
    //叫AI干活
    //4. 发送请求
    //创建HTTP客户端
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!("[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端", e);
            reqwest::Client::new()
        });
    //发出请求等待回复
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;
    //从HTPP响应里提取JSON格式的正文
    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;
    //从回复JSON抽出AI回答原文
    let content = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复格式异常".to_string())?
        .to_string();
    //（上面的文字）JSON数组转化为Rust数组,json改动替换
    let raw_milestones: Vec<serde_json::Value> = parse_json_with_retry(&content)
        .await
        .map_err(|e| format!("解析大阶段 JSON 失败：{}", e))?;
    //创建一个空的、可变的、专门用来存放 project::Milestone 结构体的数组，变量名叫 milestones
    let mut milestones: Vec<project::Milestone> = Vec::new();
    //把这个数组里面的每个对象，转化为Rust里Milestone结构体（补上id,状态...）
    for raw in raw_milestones {
        milestones.push(project::Milestone {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_stack: raw["tech_stack"].as_str().unwrap_or("").to_string(),
            status: project::MilestoneStatus::Pending,
            mode: if mode == "Quick" {
                project::StageMode::Quick
            } else {
                project::StageMode::Professional
            },
            //vec!创建空的Vec数组= vec::new()
            mid_stages: vec![],
            subtasks: vec![],
            qa_result: None,
            //创建空的可变的字符串
            git_commit_hash: "".to_string(),
        });
    }

    // === 质检逻辑：对比版本方案检查大阶段列表是否对齐 ===
    // 步骤 1：将 milestones 序列化为 JSON 字符串
    let milestones_json = match serde_json::to_string(&milestones) {
        Ok(json) => json,
        Err(e) => {
            eprintln!(
                "[generate_milestones] 大阶段 JSON 序列化失败：{}，跳过质检",
                e
            );
            return Ok(milestones);
        }
    };

    // 步骤 2：构造质检请求的 user_message
    let qa_user_message = format!(
        "【原始需求（版本方案）】\n{}\n\n【当前产出（大阶段列表）】\n{}",
        version_plan, milestones_json
    );

    // 步骤 3：调用 DeepSeek Flash 执行质检（纯文本模式，低 temperature）
    let qa_response =
        match call_deepseek_api_inner(QA_CHECK_PROMPT, &qa_user_message, false, 0.1).await {
            Ok(reply) => reply,
            Err(e) => {
                eprintln!("[generate_milestones] 质检 API 调用失败：{}，跳过质检", e);
                return Ok(milestones);
            }
        };

    // 步骤 4：清洗并解析 AI 返回的 QAResult JSON
    let qa_result = {
        let cleaned = sanitize_json_response(&qa_response);
        // 兜底：AI 有时返回空数组 [] 而非对象，直接走降级
        if cleaned == "[]" {
            eprintln!("[generate_milestones] 质检 AI 返回空数组 []，使用兜底不通过结果");
            project::QAResult {
                passed: false,
                reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案".to_string(),
                details: vec![],
                attention_points: vec![],
                checked_at: chrono::Utc::now().to_rfc3339(),
                warnings: vec!["AI 返回空数组 []".to_string()],
            }
        } else {
            match serde_json::from_str::<project::QAResult>(&cleaned) {
                Ok(mut result) => {
                    result.checked_at = chrono::Utc::now().to_rfc3339();
                    result
                }
                Err(e) => {
                    eprintln!(
                        "[generate_milestones] 质检 JSON 解析失败：{}，默认判定为不通过",
                        e
                    );
                    project::QAResult {
                        passed: false,
                        reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案".to_string(),
                        details: vec![],
                        attention_points: vec![],
                        checked_at: chrono::Utc::now().to_rfc3339(),
                        warnings: vec![format!("质检 JSON 解析失败：{}", e)],
                    }
                }
            }
        }
    };

    // 步骤 5：将 QAResult 写入每个 Milestone
    for milestone in &mut milestones {
        milestone.qa_result = Some(qa_result.clone());
    }

    Ok(milestones)
}

///根据质检驳回的反馈，重新让产品经理拆解大阶段
///与 generate_milestones 的区别：user_message 中包含驳回原因，引导 AI 修正
#[tauri::command]
async fn regenerate_milestones_with_feedback(
    version_plan: String,
    mode: String,
    feedback: String,
) -> Result<Vec<project::Milestone>, String> {
    // 1. 读取 API 密钥
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    // 2. 构造 system prompt （产品经理角色 + 模式信息）
    let system_prompt = format!(
        "{}\n\n当前项目模式：{}。\
         如果是专业模式，输出的每个大阶段应包含 mid_stages 字段（空列表）；\
         如果是快速模式，输出的每个大阶段应包含 subtasks 字段（空列表）。\
         每个大阶段的 version 字段格式为 v0.1、v0.2 等。\
         你只输出 JSON 数组，不要输出其他文字，不要包含 markdown 代码块标记。\
         每个大阶段包含：version（字符串）, title（字符串）, description（字符串）, tech_stack（字符串）。",
        PM_PROMPT, mode
    );
    // 3. 构造 API 消息（包含驳回反馈）
    let request_body = serde_json::json!({
        "model": "deepseek-v4-flash",
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": format!(
                    "上次拆解被需求质检驳回，原因：\n{}\n\n请根据此反馈，重新根据以下版本方案拆解为3-5个大阶段：\n{}",
                    feedback, version_plan
                )}
            ]
    });
    // 4. 发送请求
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!("[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端", e);
            reqwest::Client::new()
        });
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;
    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;
    let content = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复格式异常".to_string())?
        .to_string();
    // 5. 解析 JSON 数组
    let raw_milestones: Vec<serde_json::Value> = parse_json_with_retry(&content)
        .await
        .map_err(|e| format!("解析大阶段 JSON 失败：{}", e))?;
    // 6. 构造 Milestone 结构体
    let mut milestones: Vec<project::Milestone> = Vec::new();
    for raw in raw_milestones {
        milestones.push(project::Milestone {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_stack: raw["tech_stack"].as_str().unwrap_or("").to_string(),
            status: project::MilestoneStatus::Pending,
            mode: if mode == "Quick" {
                project::StageMode::Quick
            } else {
                project::StageMode::Professional
            },
            mid_stages: vec![],
            subtasks: vec![],
            qa_result: None,
            git_commit_hash: "".to_string(),
        });
    }

    // === 质检逻辑：对比版本方案检查大阶段列表是否对齐 ===
    // 步骤 7.1：将 milestones 序列化为 JSON 字符串
    let milestones_json = match serde_json::to_string(&milestones) {
        Ok(json) => json,
        Err(e) => {
            eprintln!(
                "[regenerate_milestones_with_feedback] 大阶段 JSON 序列化失败：{}，跳过质检",
                e
            );
            return Ok(milestones);
        }
    };

    // 步骤 7.2：构造质检请求的 user_message
    let qa_user_message = format!(
        "【原始需求（版本方案）】\n{}\n\n【当前产出（大阶段列表）】\n{}",
        version_plan, milestones_json
    );

    // 步骤 7.3：调用 DeepSeek Flash 执行质检（纯文本模式，低 temperature）
    let qa_response =
        match call_deepseek_api_inner(QA_CHECK_PROMPT, &qa_user_message, false, 0.1).await {
            Ok(reply) => reply,
            Err(e) => {
                eprintln!(
                    "[regenerate_milestones_with_feedback] 质检 API 调用失败：{}，跳过质检",
                    e
                );
                return Ok(milestones);
            }
        };

    // 步骤 7.4：清洗并解析 AI 返回的 QAResult JSON
    let qa_result = {
        let cleaned = sanitize_json_response(&qa_response);
        // 兜底：AI 有时返回空数组 [] 而非对象，直接走降级
        if cleaned == "[]" {
            eprintln!("[regenerate_milestones_with_feedback] 质检 AI 返回空数组 []，使用兜底不通过结果");
            project::QAResult {
                passed: false,
                reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案".to_string(),
                details: vec![],
                attention_points: vec![],
                checked_at: chrono::Utc::now().to_rfc3339(),
                warnings: vec!["AI 返回空数组 []".to_string()],
            }
        } else {
            match serde_json::from_str::<project::QAResult>(&cleaned) {
                Ok(mut result) => {
                    result.checked_at = chrono::Utc::now().to_rfc3339();
                    result
                }
                Err(e) => {
                    eprintln!("[regenerate_milestones_with_feedback] 质检 JSON 解析失败：{}，默认判定为不通过", e);
                    project::QAResult {
                        passed: false,
                        reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案".to_string(),
                        details: vec![],
                        attention_points: vec![],
                        checked_at: chrono::Utc::now().to_rfc3339(),
                        warnings: vec![format!("质检 JSON 解析失败：{}", e)],
                    }
                }
            }
        }
    };

    // 步骤 7.5：将 QAResult 写入每个 Milestone
    for milestone in &mut milestones {
        milestone.qa_result = Some(qa_result.clone());
    }

    Ok(milestones)
}

///中阶段控制
#[tauri::command]
async fn generate_mid_stages(
    _milestone_id: String,
    milestone_title: String,
    milestone_description: String,
    version_plan: String,
    mode: String,
    attention_points: Vec<String>,
) -> Result<Vec<project::MidStage>, String> {
    // 1. 读取 API 密钥
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    // 2. 构造 system prompt
    let mut system_prompt = format!(
        "{}\n\n当前项目模式：{}。请根据版本方案，将大阶段拆解为 3-6 个中阶段。\
         每个中阶段是一个垂直切片。",
        DOMAIN_LEAD_PROMPT, mode
    );
    // 注入 attention_points（若不为空）
    if !attention_points.is_empty() {
        system_prompt.push_str("\n【需求关注点】\n该大阶段在需求对齐检查中确认了以下要点，请在拆分中阶段时确保覆盖：\n");
        for point in &attention_points {
            system_prompt.push_str(&format!("- {}\n", point));
        }
    }
    // 3. 构造请求体
    let request_body = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": format!(
                "请根据版本方案，为大阶段「{} - {}」拆解中阶段：\n{}",
                milestone_title, milestone_description, version_plan
            )}
        ]
    });
    // 4. 发送请求
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!("[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端", e);
            reqwest::Client::new()
        });
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;
    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;
    let content = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复格式异常".to_string())?
        .to_string();
    // 5. 解析 JSON,json 解析改动
    let raw_mid_stages: Vec<serde_json::Value> = parse_json_with_retry(&content)
        .await
        .map_err(|e| format!("解析中阶段 JSON 失败：{}", e))?;
    // 6. 转换成 MidStage 结构体
    let mut mid_stages: Vec<project::MidStage> = Vec::new();
    for raw in raw_mid_stages {
        mid_stages.push(project::MidStage {
            id: uuid::Uuid::new_v4().to_string(),
            version: raw["version"].as_str().unwrap_or("v0.0.0").to_string(),
            title: raw["title"].as_str().unwrap_or("未命名").to_string(),
            description: raw["description"].as_str().unwrap_or("").to_string(),
            tech_focus: raw["tech_focus"].as_str().unwrap_or("").to_string(),
            status: project::MidStageStatus::Pending,
            subtasks: vec![],
            test_report: "".to_string(),
            domain: None,
            test_log: None,
            created_at: chrono::Utc::now().to_rfc3339(), // 或 String::new()
            completed_at: None,
            approved_at: None,
            git_tag: String::new(),
        });
    }
    Ok(mid_stages)
}

// src-tauri/src/lib.rs
/// 通用 Git 存档命令
///
/// 在项目目录下执行 git add . → git commit --allow-empty → git tag -f
/// 专业模式：从中阶段完成处调用（version = "v0.1.1"）
/// 快速模式：从大阶段完成处调用（version = "v0.1"）
///
/// 返回 tag 名，如 "metheus/v0.1.1"
#[tauri::command]
async fn git_save_node(
    project_path: String,
    version: String,
    title: String,
) -> Result<String, String> {
    // 1. git add . 加暂存
    let add_output = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git add失败: {}", e))?;
    if !add_output.status.success() {
        return Err(format!(
            "git add 执行失败：\n{}",
            String::from_utf8_lossy(&add_output.stderr)
        ));
    }
    // 2. git commit -m 记录文档
    // --allow-empty 确保即使没有文件变更也能提交
    //    （比如一个中阶段只改了文案，没有代码变更）
    let commit_message = format!("【弥】节点 {}: {}", version, title);
    let commit_output = std::process::Command::new("git")
        .args(["commit", "-m", &commit_message, "--allow-empty"])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git commit 执行失败：{}", e))?;
    if !commit_output.status.success() {
        // "nothing to commit"不是错误，只是没有变更
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        if !stderr.contains("nothing to commit") {
            return Err(format!("git commit 执行失败:\n{}", stderr));
        }
    }
    // 3. git tag 打标签
    // tag 格式 metheus/v0.1.1，用 metheus/ 前缀避免和用户自己的 tag 冲突
    // -f 允许覆盖已有 tag（如果同一个节点重做后重新存档）
    let tag_name = format!("metheus/{}", version);
    let tag_output = std::process::Command::new("git")
        .args(["tag", "-f", &tag_name])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git tag 失败: {}", e))?;
    if !tag_output.status.success() {
        return Err(format!(
            "git tag 执行失败: \n{}",
            String::from_utf8_lossy(&tag_output.stderr)
        ));
    }
    // 返回 tag 名， 让调用方决定写回哪个节点
    Ok(tag_name)
}

/// 小阶段 Git 存档命令
///
/// 在项目目录下执行 git add . → git commit --allow-empty → git tag -f
/// tag 格式：metheus/auto/{mid_stage_version}/task-{subtask_index}
/// 返回 tag 名，调用方可写回 Subtask.auto_tag
#[tauri::command]
async fn git_save_subtask(
    project_path: String,
    subtask_index: u32,
    mid_stage_version: String,
    subtask_title: String,
) -> Result<String, String> {
    // 1. git add . 暂存所有变更
    let add_output = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git add 失败: {}", e))?;
    if !add_output.status.success() {
        return Err(format!(
            "git add 执行失败：\n{}",
            String::from_utf8_lossy(&add_output.stderr)
        ));
    }

    // 2. git commit（--allow-empty 确保即使无文件变更也能提交）
    let commit_message = format!(
        "【弥】小阶段 {}/{}：{}",
        subtask_index, mid_stage_version, subtask_title
    );
    let commit_output = std::process::Command::new("git")
        .args(["commit", "-m", &commit_message, "--allow-empty"])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git commit 执行失败：{}", e))?;
    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        if !stderr.contains("nothing to commit") {
            return Err(format!("git commit 执行失败:\n{}", stderr));
        }
    }

    // 3. git tag -f（覆盖已有 tag）
    let tag_name = format!("metheus/auto/{}/task-{}", mid_stage_version, subtask_index);
    let tag_output = std::process::Command::new("git")
        .args(["tag", "-f", &tag_name])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git tag 失败: {}", e))?;
    if !tag_output.status.success() {
        return Err(format!(
            "git tag 执行失败:\n{}",
            String::from_utf8_lossy(&tag_output.stderr)
        ));
    }

    Ok(tag_name)
}

/// Git 回退命令
///
/// 把项目代码和执行树状态一起回退到指定 tag 对应的版本
/// 1. 检查工作区是否有未提交变更 → 有则 stash
/// 2. git reset --hard 到目标 tag → 代码回退
/// 3. 遍历 project.json → 回退点之后的节点标记为 RolledBack
#[tauri::command]
async fn git_rollback_to_mid_stage(
    project_path: String,
    tag_name: String,
    project_id: String,
) -> Result<String, String> {
    // 1. 检查工作区是否有未提交变更
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git status 失败: {}", e))?;
    let status = String::from_utf8_lossy(&status_output.stdout);
    let has_uncommitted = !status.trim().is_empty();
    // 如有未提交变更，先 stash 起来，避免被 reset --hard 永久清除
    if has_uncommitted {
        let stash_output = std::process::Command::new("git")
            .args(["stash", "push", "-m", "metheus_rollback_auto_stash"])
            .current_dir(&project_path)
            .output()
            .map_err(|e| format!("git stash 失败: {}", e))?;
        if !stash_output.status.success() {
            return Err(format!(
                "git stash 执行失败:\n{}",
                String::from_utf8_lossy(&stash_output.stderr)
            ));
        }
    }
    // 2. git reset --hard 到目标 tag
    let reset_output = std::process::Command::new("git")
        .args(["reset", "--hard", &tag_name])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git reset 失败: {}", e))?;
    if !reset_output.status.success() {
        return Err(format!(
            "回退到 {} 失败:\n{}",
            tag_name,
            String::from_utf8_lossy(&reset_output.stderr)
        ));
    }
    // 2.5 清理被跳过节点的 Git tag
    // 遍历所有 mid_stage，删除版本号大于目标版本的节点的 git tag
    {
        let target_version = tag_name
            .strip_prefix("metheus/")
            .unwrap_or(&tag_name)
            .to_string();
        let pp = std::path::Path::new(&project_path);
        let md = pp.join(".metheus");
        let pf = md.join(format!("{}.json", project_id));
        if pf.exists() {
            if let Ok(content) = std::fs::read_to_string(&pf) {
                if let Ok(proj) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(milestones) = proj["milestones"].as_array() {
                        for milestone in milestones {
                            if let Some(mid_stages) = milestone["mid_stages"].as_array() {
                                for mid in mid_stages {
                                    let version = mid["version"].as_str().unwrap_or("");
                                    let git_tag = mid["git_tag"].as_str().unwrap_or("");
                                    if !git_tag.is_empty()
                                        && compare_version_strings(version, &target_version) > 0
                                    {
                                        match std::process::Command::new("git")
                                            .args(["tag", "-d", git_tag])
                                            .current_dir(&project_path)
                                            .output()
                                        {
                                            Ok(output) => {
                                                if !output.status.success() {
                                                    eprintln!(
                                                        "警告: 删除 git tag {} 失败: {}",
                                                        git_tag,
                                                        String::from_utf8_lossy(&output.stderr)
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "警告: 执行 git tag -d {} 失败: {}",
                                                    git_tag, e
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // 3. 更新 project.json 中的执行树状态
    // 从tag_name 中提取版本号，去掉 "metheus/" 前缀
    let target_version = tag_name
        .strip_prefix("metheus/")
        .unwrap_or(&tag_name)
        .to_string();
    // 读取 project.json
    let project_path_obj = std::path::Path::new(&project_path);
    let metheus_dir = project_path_obj.join(".metheus");
    let project_file = metheus_dir.join(format!("{}.json", project_id));
    if !project_file.exists() {
        return Err(format!("项目文件不存在: {}", project_file.display()));
    }
    let content =
        std::fs::read_to_string(&project_file).map_err(|e| format!("读取项目文件失败: {}", e))?;
    let mut project: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析项目文件失败: {}", e))?;
    // 遍历 milestones -> mid_stages, 标记回点之后的节点
    if let Some(milestones) = project["milestones"].as_array_mut() {
        for milestone in milestones.iter_mut() {
            if let Some(mid_stages) = milestone["mid_stages"].as_array_mut() {
                for mid in mid_stages.iter_mut() {
                    let version = mid["version"].as_str().unwrap_or("");
                    // 比较版本号：如果当前节点版本 > 目标版本，标记为 RolledBack
                    if compare_version_strings(version, &target_version) > 0 {
                        mid["status"] = serde_json::Value::String("RolledBack".to_string());
                    }
                }
            }
        }
    }
    // 写回文件
    let json_str =
        serde_json::to_string_pretty(&project).map_err(|e| format!("序列化项目文件失败: {}", e))?;
    std::fs::write(&project_file, &json_str).map_err(|e| format!("写入项目文件失败: {}", e))?;
    // 组装返回消息
    let stash_note = if has_uncommitted {
        "\n（你有未提交的变更已被临时存储，回退完成后可执行 git stash pop 恢复）"
    } else {
        ""
    };
    Ok(format!("已回退到 {}{}", tag_name, stash_note))
}

/// Git 回退到指定小阶段
///
/// 把项目代码回退到指定 subtask auto_tag 对应的版本。
/// 与 git_rollback_to_mid_stage 的区别：回退粒度更细，只回退到某个小阶段（而非中阶段）。
/// 1. 检查工作区是否有未提交变更 → 有则 stash
/// 2. git reset --hard 到目标 tag → 代码回退
/// 3. 遍历 project.json → 回退点之后的 subtasks 和 mid_stages 标记为 RolledBack
#[tauri::command]
async fn git_rollback_to_subtask(
    project_path: String,
    project_id: String,
    tag_name: String,
) -> Result<String, String> {
    // 1. 检查工作区是否有未提交变更
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git status 失败: {}", e))?;
    let status = String::from_utf8_lossy(&status_output.stdout);
    let has_uncommitted = !status.trim().is_empty();
    // 如有未提交变更，先 stash 起来
    if has_uncommitted {
        let stash_output = std::process::Command::new("git")
            .args(["stash", "push", "-m", "metheus_rollback_auto_stash"])
            .current_dir(&project_path)
            .output()
            .map_err(|e| format!("git stash 失败: {}", e))?;
        if !stash_output.status.success() {
            return Err(format!(
                "git stash 执行失败:\n{}",
                String::from_utf8_lossy(&stash_output.stderr)
            ));
        }
    }

    // 2. git reset --hard 到目标 tag
    let reset_output = std::process::Command::new("git")
        .args(["reset", "--hard", &tag_name])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git reset 失败: {}", e))?;
    if !reset_output.status.success() {
        // reset 失败，尝试恢复 stash
        if has_uncommitted {
            let _ = std::process::Command::new("git")
                .args(["stash", "pop"])
                .current_dir(&project_path)
                .output();
        }
        return Err(format!(
            "回退到 {} 失败:\n{}",
            tag_name,
            String::from_utf8_lossy(&reset_output.stderr)
        ));
    }

    // 3. 更新 project.json 中的执行树状态
    // 解析 tag_name 获取目标信息：格式 metheus/auto/{version}/task-{index}
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let project_file = std::path::Path::new(&home)
        .join(".metheus")
        .join(format!("{}.json", project_id));
    if !project_file.exists() {
        return Err(format!("项目文件不存在: {}", project_file.display()));
    }
    let content =
        std::fs::read_to_string(&project_file).map_err(|e| format!("读取项目文件失败: {}", e))?;
    let mut project: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析项目文件失败: {}", e))?;

    // 找到目标 subtask 所属的 mid_stage，标记其后的 subtasks 和 mid_stages 为 RolledBack
    let mut target_found = false;
    let mut target_mid_stage_id = String::new();
    let mut passed_target = false;

    if let Some(milestones) = project["milestones"].as_array_mut() {
        for milestone in milestones.iter_mut() {
            if let Some(mid_stages) = milestone["mid_stages"].as_array_mut() {
                for mid in mid_stages.iter_mut() {
                    // 提前提取 mid_id，避免后续 borrow checker 冲突
                    let mid_id = mid["id"].as_str().unwrap_or("").to_string();

                    if passed_target {
                        // 目标之后的 mid_stage → 标记为 RolledBack
                        mid["status"] = serde_json::Value::String("RolledBack".to_string());
                        continue;
                    }

                    if let Some(subtasks) = mid["subtasks"].as_array_mut() {
                        for subtask in subtasks.iter_mut() {
                            let auto_tag = subtask["auto_tag"].as_str().unwrap_or("");
                            if auto_tag == tag_name {
                                target_found = true;
                                target_mid_stage_id = mid_id.clone();
                                // 当前 subtask 保持原状态
                            } else if target_found && mid_id == target_mid_stage_id {
                                // 同一 mid_stage 内，目标 subtask 之后的 subtask
                                subtask["status"] =
                                    serde_json::Value::String("RolledBack".to_string());
                            }
                        }
                    }

                    if target_found && mid_id == target_mid_stage_id {
                        // 当前 mid_stage 完成后，后续 mid_stages 标记为 RolledBack
                        passed_target = true;
                    }
                }
            }
        }
    }

    if !target_found {
        return Err(format!("未找到 tag {} 对应的小阶段", tag_name));
    }

    // 写回文件
    let json_str =
        serde_json::to_string_pretty(&project).map_err(|e| format!("序列化项目文件失败: {}", e))?;
    std::fs::write(&project_file, &json_str).map_err(|e| format!("写入项目文件失败: {}", e))?;

    // 组装返回消息
    let stash_note = if has_uncommitted {
        "\n（你有未提交的变更已被临时存储，回退完成后可执行 git stash pop 恢复）"
    } else {
        ""
    };
    Ok(format!("已回退到 {}{}", tag_name, stash_note))
}

/// 获取 metheus/ 前缀的 Git tag 摘要列表
///
/// 执行 git tag -l "metheus/*" --sort=-creatordate 获取所有 metheus tag，
/// 解析为 GitTagInfo 列表返回。非 git 仓库或无匹配 tag 时返回空数组。
#[tauri::command]
async fn get_git_tags_summary(project_path: String) -> Result<Vec<project::GitTagInfo>, String> {
    let output = std::process::Command::new("git")
        .args([
            "tag",
            "-l",
            "metheus/*",
            "--sort=-creatordate",
            "--format=%(refname:short)|%(creatordate:short)|%(subject)",
        ])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git tag 命令执行失败: {}", e))?;

    if !output.status.success() {
        // 非 git 仓库或无权限等情况，返回空数组
        return Ok(vec![]);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut tags: Vec<project::GitTagInfo> = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 3 {
            // 脏数据保护：跳过不满足 3 段的行
            continue;
        }
        tags.push(project::GitTagInfo {
            name: parts[0].to_string(),
            date: parts[1].to_string(),
            subject: parts[2].to_string(),
        });
    }

    Ok(tags)
}

/// 获取当前工作区的 git diff
///
/// 执行 git diff 获取未暂存的变更。非 git 仓库或工作区干净时返回空字符串。
#[tauri::command]
async fn get_current_diff(project_path: String) -> Result<String, String> {
    // 1. 检查 .git 是否存在（目录或文件，兼容 worktree）
    let git_path = std::path::Path::new(&project_path).join(".git");
    if !git_path.exists() {
        eprintln!("[get_current_diff] 不是 git 仓库，返回空");
        return Ok(String::new());
    }

    // 2. 执行 git diff
    let output = std::process::Command::new("git")
        .args(["diff"])
        .current_dir(&project_path)
        .output()
        .map_err(|e| {
            eprintln!("[get_current_diff] git 命令不可用: {}", e);
            format!("git 命令不可用: {}", e)
        })?;

    // 3. 退出码为零 → 返回 diff 内容（可能为空字符串 = 无变更）
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    // 4. 退出码非零 → 分析 stderr 区分场景
    let stderr_str = String::from_utf8_lossy(&output.stderr);
    let stderr_lower = stderr_str.to_lowercase();

    if stderr_lower.contains("not a git repository") {
        eprintln!("[get_current_diff] 不是 git 仓库");
        return Ok(String::new());
    }

    if stderr_lower.contains("does not have any commits")
        || stderr_lower.contains("ambiguous argument")
        || stderr_lower.contains("unknown revision")
    {
        eprintln!("[get_current_diff] 仓库尚无提交");
        return Ok(String::new());
    }

    // 5. 未知错误 → 截断日志 + 返回空
    let truncated: String = stderr_str.chars().take(200).collect();
    eprintln!("[get_current_diff] 未知 git 错误: {}", truncated);
    Ok(String::new())
}

/// 校验项目路径的前端可调用命令
#[tauri::command]
async fn validate_project_path(
    project_path: String,
) -> Result<project::PathValidationResult, String> {
    Ok(check_project_path(&project_path))
}

/// 获取项目文件列表
///
/// 使用 walkdir 递归遍历项目目录（最大深度 5），跳过 .git、node_modules、
/// target 等构建产物目录，同时跳过隐藏文件（以 . 开头），但保留 .env.example。
#[tauri::command]
async fn get_project_files(project_path: String) -> Result<Vec<project::FileEntry>, String> {
    let project = std::path::Path::new(&project_path);
    if !project.exists() || !project.is_dir() {
        return Ok(vec![]);
    }

    // 需要跳过的目录名
    const SKIP_DIRS: &[&str] = &[
        ".git",
        "node_modules",
        "target",
        "__pycache__",
        "dist",
        ".next",
        "build",
        "coverage",
    ];

    let mut entries: Vec<project::FileEntry> = Vec::new();

    for entry in walkdir::WalkDir::new(&project_path)
        .max_depth(5)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        // 跳过根目录自身
        if entry.path() == project {
            continue;
        }

        // 计算相对路径
        let rel_path = entry
            .path()
            .strip_prefix(&project_path)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();

        // 检查路径的每一级是否在排除目录中
        let is_skipped = rel_path
            .split('/')
            .any(|component| SKIP_DIRS.contains(&component));
        if is_skipped {
            continue;
        }

        // 跳过隐藏文件/目录（以 . 开头），但保留 .env.example 等 .env* 文件
        if let Some(file_name) = entry.file_name().to_str() {
            if file_name.starts_with('.') && !file_name.starts_with(".env") {
                continue;
            }
        }

        let is_dir = entry.file_type().is_dir();
        let file_type = if is_dir {
            String::new()
        } else {
            entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };

        entries.push(project::FileEntry {
            path: rel_path,
            is_dir,
            file_type,
        });
    }

    Ok(entries)
}

/// 比较两个版本号字符串（eg: "v0.1.1"  "v0.1.3"）
/// 返回 -1：a < b，0：a == b，1：a > b
fn compare_version_strings(a: &str, b: &str) -> i32 {
    let parts_a: Vec<u32> = a
        .strip_prefix('v')
        .unwrap_or(a)
        .split('.')
        .filter_map(|s| s.parse::<u32>().ok())
        .collect();
    let parts_b: Vec<u32> = b
        .strip_prefix('v')
        .unwrap_or(b)
        .split('.')
        .filter_map(|s| s.parse::<u32>().ok())
        .collect();
    for i in 0..parts_a.len().max(parts_b.len()) {
        let num_a = parts_a.get(i).copied().unwrap_or(0);
        let num_b = parts_b.get(i).copied().unwrap_or(0);
        if num_a < num_b {
            return -1;
        }
        if num_a > num_b {
            return 1;
        }
    }
    0
}

/// 解析 git diff 的完整 stdout，提取变更摘要
///
/// 扫描 diff 输出中的文件增删改、函数增删改、依赖变更，
/// 自动跳过 node_modules/、target/、__pycache__/、.git/ 和 .lock 文件。
/// 纯文本解析，不涉及 I/O 或 AI 调用。
fn extract_diff_summary(diff_stdout: &str) -> project::DiffSummary {
    let mut summary = project::DiffSummary {
        new_files: Vec::new(),
        modified_files: Vec::new(),
        deleted_files: Vec::new(),
        new_functions: Vec::new(),
        modified_functions: Vec::new(),
        deleted_functions: Vec::new(),
        changed_dependencies: Vec::new(),
    };

    if diff_stdout.trim().is_empty() {
        return summary;
    }

    // 需要跳过的目录和文件模式
    let skip_patterns = ["node_modules/", "target/", "__pycache__/", ".git/"];
    let is_skipped = |path: &str| -> bool {
        if path.ends_with(".lock") {
            return true;
        }
        for pat in &skip_patterns {
            if path.contains(pat) {
                return true;
            }
        }
        false
    };

    // 收集文件名集合用于去重
    let mut new_files_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut deleted_files_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut modified_files_set: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut new_funcs_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut deleted_funcs_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut modified_funcs_set: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut deps_set: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 依赖文件名集合
    let dep_files: std::collections::HashSet<&str> = [
        "package.json",
        "Cargo.toml",
        "go.mod",
        "requirements.txt",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
    ]
    .iter()
    .cloned()
    .collect();

    let lines: Vec<&str> = diff_stdout.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // 检测新增文件
        if line.starts_with("new file mode") {
            // 下一行可能包含路径
            if i + 1 < lines.len() && lines[i + 1].starts_with("+++ b/") {
                let path = lines[i + 1]
                    .strip_prefix("+++ b/")
                    .unwrap_or("")
                    .replace('\\', "/");
                if !path.is_empty() && !is_skipped(&path) {
                    new_files_set.insert(path);
                }
            }
            i += 1;
            continue;
        }

        // 检测删除文件
        if line.starts_with("deleted file mode") {
            if i + 1 < lines.len() && lines[i + 1].starts_with("--- a/") {
                let path = lines[i + 1]
                    .strip_prefix("--- a/")
                    .unwrap_or("")
                    .replace('\\', "/");
                if !path.is_empty() && !is_skipped(&path) {
                    deleted_files_set.insert(path);
                }
            }
            i += 1;
            continue;
        }

        // 检测 --- /dev/null（新增文件，无 new file mode 前缀时）
        if line.starts_with("--- /dev/null") {
            if i + 1 < lines.len() && lines[i + 1].starts_with("+++ b/") {
                let path = lines[i + 1]
                    .strip_prefix("+++ b/")
                    .unwrap_or("")
                    .replace('\\', "/");
                if !path.is_empty() && !is_skipped(&path) {
                    new_files_set.insert(path);
                }
            }
            i += 1;
            continue;
        }

        // 检测 +++ /dev/null（删除文件）
        if line.starts_with("+++ /dev/null") {
            if i >= 1 && lines[i - 1].starts_with("--- a/") {
                let path = lines[i - 1]
                    .strip_prefix("--- a/")
                    .unwrap_or("")
                    .replace('\\', "/");
                if !path.is_empty() && !is_skipped(&path) {
                    deleted_files_set.insert(path);
                }
            }
            i += 1;
            continue;
        }

        // 检测 --- a/ 和 +++ b/ 同时出现 → 修改文件
        if line.starts_with("--- a/") {
            if i + 1 < lines.len() && lines[i + 1].starts_with("+++ b/") {
                let old_path = line.strip_prefix("--- a/").unwrap_or("").replace('\\', "/");
                let new_path = lines[i + 1]
                    .strip_prefix("+++ b/")
                    .unwrap_or("")
                    .replace('\\', "/");
                let path = if !new_path.is_empty() {
                    new_path
                } else {
                    old_path
                };
                if !path.is_empty() && !is_skipped(&path) {
                    modified_files_set.insert(path.clone());

                    // 检测是否为依赖文件变更
                    if let Some(filename) = std::path::Path::new(&path)
                        .file_name()
                        .and_then(|n| n.to_str())
                    {
                        if dep_files.contains(filename) {
                            // 扫描该 diff 块内的 +/- 行
                            let mut j = i + 2;
                            while j < lines.len() {
                                let l = lines[j];
                                if l.starts_with("diff --git") || l.starts_with("--- a/") {
                                    break;
                                }
                                let content = if l.starts_with('+') && !l.starts_with("+++") {
                                    Some(l[1..].trim())
                                } else if l.starts_with('-') && !l.starts_with("---") {
                                    Some(l[1..].trim())
                                } else {
                                    None
                                };
                                if let Some(c) = content {
                                    if !c.is_empty() && c != "---" && c != "+++" {
                                        deps_set.insert(c.to_string());
                                    }
                                }
                                j += 1;
                            }
                        }
                    }
                }
            }
            i += 1;
            continue;
        }

        // 提取新增函数（以 + 开头的行）
        if line.starts_with('+') && !line.starts_with("+++") {
            let content = &line[1..];
            if let Some(func_sig) = extract_function_signature(content) {
                new_funcs_set.insert(func_sig);
            }
        }

        // 提取删除函数（以 - 开头的行）
        if line.starts_with('-') && !line.starts_with("---") {
            let content = &line[1..];
            if let Some(func_sig) = extract_function_signature(content) {
                deleted_funcs_set.insert(func_sig);
            }
        }

        // 从 @@ 上下文行中提取可能被修改的函数名
        if line.starts_with("@@") {
            // 守卫：只处理包含已知语言函数定义关键字的 @@ 行
            let lang_keywords = ["fn ", "def ", "func ", "function ", "class "];
            let has_lang_keyword = lang_keywords.iter().any(|kw| line.contains(kw));
            if has_lang_keyword {
                if let Some(at_end) = line.rfind("@@") {
                    let ctx = &line[at_end + 2..];
                    let mut start = 0;
                    while start < ctx.len() {
                        if let Some(rest) = ctx.get(start..) {
                            if let Some(paren) = rest.find('(') {
                                let before = &rest[..paren];
                                // 向前找函数名起始（仅允许 ASCII 字母数字下划线）
                                if let Some(func_start) =
                                    before.rfind(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                                {
                                    let fname = before[func_start + 1..].to_string();
                                    // 长度过滤：2-80 字符，且以字母或下划线开头
                                    if fname.len() >= 2
                                        && fname.len() <= 80
                                        && fname
                                            .chars()
                                            .next()
                                            .map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
                                    {
                                        modified_funcs_set.insert(fname);
                                    }
                                } else if !before.is_empty()
                                    && before.len() >= 2
                                    && before.len() <= 80
                                    && before
                                        .chars()
                                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                                {
                                    modified_funcs_set.insert(before.to_string());
                                }
                                start += paren + 1;
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        }

        i += 1;
    }

    // 从 modified_files 中排除已归类为 new/deleted 的文件
    for f in &new_files_set {
        modified_files_set.remove(f);
    }
    for f in &deleted_files_set {
        modified_files_set.remove(f);
    }

    // 填充结果
    summary.new_files = new_files_set.into_iter().collect();
    summary.new_files.sort();
    summary.deleted_files = deleted_files_set.into_iter().collect();
    summary.deleted_files.sort();
    summary.modified_files = modified_files_set.into_iter().collect();
    summary.modified_files.sort();
    summary.new_functions = new_funcs_set.into_iter().collect();
    summary.new_functions.sort();
    summary.deleted_functions = deleted_funcs_set.into_iter().collect();
    summary.deleted_functions.sort();
    summary.modified_functions = modified_funcs_set.into_iter().collect();
    summary.modified_functions.sort();
    summary.changed_dependencies = deps_set.into_iter().collect();
    summary.changed_dependencies.sort();

    summary
}

/// 从一行代码中提取函数/方法签名
/// 支持 Rust / TypeScript / JavaScript / Python / Go / C++ / Java
/// 返回 None 表示该行不包含函数定义
fn extract_function_signature(line: &str) -> Option<String> {
    let trimmed = line.trim();

    // 跳过注释行
    if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("/*") {
        return None;
    }

    // Rust: fn / pub fn / pub async fn / unsafe fn
    if let Some(rest) = trimmed
        .strip_prefix("pub async fn ")
        .or_else(|| trimmed.strip_prefix("pub fn "))
        .or_else(|| trimmed.strip_prefix("async fn "))
        .or_else(|| trimmed.strip_prefix("unsafe fn "))
        .or_else(|| trimmed.strip_prefix("fn "))
    {
        let sig = rest.trim();
        if !sig.is_empty() && sig.contains('(') {
            let end = sig.find('{').unwrap_or(sig.len());
            let end = sig[..end].find(';').unwrap_or(end);
            return Some(format!("fn {}", sig[..end].trim()));
        }
    }

    // TypeScript/JS: function / export function / async function
    if let Some(rest) = trimmed
        .strip_prefix("export async function ")
        .or_else(|| trimmed.strip_prefix("export function "))
        .or_else(|| trimmed.strip_prefix("async function "))
        .or_else(|| trimmed.strip_prefix("function "))
    {
        let sig = rest.trim();
        if !sig.is_empty() && sig.contains('(') {
            let end = sig.find('{').unwrap_or(sig.len());
            return Some(format!("function {}", sig[..end].trim()));
        }
    }

    // TypeScript 箭头函数: const name = (...) =>
    if trimmed.starts_with("const ")
        && trimmed.contains('=')
        && (trimmed.contains("=>") || trimmed.contains(": ("))
    {
        let after_const = &trimmed[6..].trim();
        if let Some(eq) = after_const.find('=') {
            let name = after_const[..eq].trim();
            let name = name.split(':').next().unwrap_or(name).trim();
            if !name.is_empty()
                && name
                    .chars()
                    .next()
                    .map_or(false, |c| c.is_alphabetic() || c == '_')
            {
                return Some(format!("const {} = (...) => {{...}}", name));
            }
        }
    }

    // Python: def / async def
    if let Some(rest) = trimmed
        .strip_prefix("async def ")
        .or_else(|| trimmed.strip_prefix("def "))
    {
        let sig = rest.trim();
        if !sig.is_empty() && sig.contains('(') {
            let end = sig.find(':').unwrap_or(sig.len());
            return Some(format!("def {}", sig[..end].trim()));
        }
    }

    // Go: func / func (
    if let Some(rest) = trimmed.strip_prefix("func ") {
        let sig = rest.trim();
        if !sig.is_empty() && sig.contains('(') {
            let end = sig.find('{').unwrap_or(sig.len());
            return Some(format!("func {}", sig[..end].trim()));
        }
    }

    // Java: public/private/protected/static 后跟 (
    let java_modifiers = ["public ", "private ", "protected "];
    for modifier in &java_modifiers {
        if trimmed.starts_with(modifier) && trimmed.contains('(') {
            let rest = &trimmed[modifier.len()..];
            // 确保不是 class/interface/enum 声明
            if !rest.trim().starts_with("class ")
                && !rest.trim().starts_with("interface ")
                && !rest.trim().starts_with("enum ")
            {
                let end = rest.find('{').unwrap_or(rest.len());
                return Some(format!("{}{}", modifier, rest[..end].trim()));
            }
        }
    }

    // C++: ClassName::methodName(...) 模式
    if trimmed.contains("::") && trimmed.contains('(') && !trimmed.starts_with("//") {
        let end = trimmed.find('{').unwrap_or(trimmed.len());
        let end = trimmed[..end].find(';').unwrap_or(end);
        let candidate = trimmed[..end].trim();
        if candidate.contains('(') && candidate.contains("::") {
            return Some(candidate.to_string());
        }
    }

    None
}

// src-tauri/src/lib.rs
/// 把 Git tag 名写入指定中阶段节点，并持久化到 project.json（辅助函数）
fn save_tag_to_mid_stage(
    project_id: &str,
    mid_stage_id: &str,
    tag_name: &str,
) -> Result<(), String> {
    // 读取 project 文件, ~/.metheus/{project_id}.json
    let app_dir = dirs::home_dir().ok_or("无法获取 home 目录".to_string())?;
    let project_file = app_dir
        .join(".metheus")
        .join(format!("{}.json", project_id));

    let content = std::fs::read_to_string(&project_file)
        .map_err(|e| format!("读取 project 文件失败: {}", e))?;

    let mut project: project::Project =
        serde_json::from_str(&content).map_err(|e| format!("解析 project 文件失败: {}", e))?;

    // 遍历找到对应 mid_stage
    let mut found = false;
    for milestone in &mut project.milestones {
        for mid in &mut milestone.mid_stages {
            if mid.id == mid_stage_id {
                mid.git_tag = tag_name.to_string();
                found = true;
                break;
            }
        }
        if found {
            break;
        }
    }

    if !found {
        return Err(format!("未找到中阶段: {}", mid_stage_id));
    }

    // 写回文件
    let json = serde_json::to_string_pretty(&project)
        .map_err(|e| format!("序列化 project 失败: {}", e))?;

    std::fs::write(&project_file, json).map_err(|e| format!("写入 project 文件失败: {}", e))?;

    Ok(())
}

/// "可暂停"的 Claude Code 执行器：启动进程后边等待边监听暂停信号，暂停时立即杀进程；
/// 正常结束则返回改动的文件和执行日志
/// 执行子任务的内部实现（可被暂停中断）
/// 用 tokio::process::Command 替代 std::process::Command，
/// spawn 后进入轮询循环，每 500ms 检查暂停标志，
/// 检测到暂停时立即 kill Claude Code 进程并返回 SubTaskError::UserPaused。
async fn execute_subtask_inner(
    project_path: &str,
    prompt: &str,
    subtask_id: &str,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Result<project::ExecutionResult, project::SubTaskError> {
    // 1. 执行前记录文件列表
    let before_files = get_tracked_files(project_path);
    // 2. 拼接完整 prompt
    let full_prompt = format!(
        "{}\n\n=== 重要约束 ===\n请直接执行，不要询问确认。所有决策由你自行判断。完成后不要输出总结，直接结束",
        prompt
    );
    // 3. 确定模型名（从环境变量读取，带白名单校验和降级兜底）
    let model_env =
        std::env::var("METHEUS_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_string());
    const VALID_MODELS: &[&str] = &["deepseek-v4-pro", "deepseek-v4-flash"];
    let model_name: String = if VALID_MODELS.contains(&model_env.as_str()) {
        model_env
    } else {
        eprintln!(
            "[execute_subtask] 警告：配置的模型名 \"{}\" 不在白名单中，降级为默认值 \"deepseek-v4-flash\"",
            model_env
        );
        "deepseek-v4-flash".to_string()
    };
    // 4. 用 tokio::process::Command 启动 Claude Code（非阻塞）
    let mut child = tokio::process::Command::new("claude")
        .args([
            "--dangerously-skip-permissions",
            "--model",
            &model_name,
            "-p",
            &full_prompt,
        ])
        .current_dir(project_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| project::SubTaskError::ExecutionFailed {
            message: format!(
                "无法启动 Claude Code CLI: {}\n请确认 claude 已安装并在 PATH 中",
                e
            ),
        })?;
    // 5. 自动应答：信任确认 + 文件写入确认（异步写入 stdin）
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(b"1\n").await.ok();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        /* 安全上限：最大自动应答次数。Claude Code 通常只会在开始前询问 1-3 次确认，
        此处设 20 为兜底。后续可改为动态检测 stdout 中是否包含 "?" 或 "确认" 等
        提示语来决定是否需要继续应答。 */
        const MAX_AUTO_CONFIRM: u32 = 20;
        for _ in 0..MAX_AUTO_CONFIRM {
            stdin.write_all(b"yes\n").await.ok();
        }
        // stdin 在这里 drop，关闭管道
    }
    // 6. 轮询等待进程结束，期间检查暂停标志和超时
    let start_time = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // 进程已结束 → 读取 stdout/stderr
                let output = child.wait_with_output().await.map_err(|e| {
                    project::SubTaskError::ExecutionFailed {
                        message: format!("读取 Claude Code 输出失败: {}", e),
                    }
                })?;
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let success = status.success();
                // 获取改动文件列表
                let after_files = get_tracked_files(project_path);
                let file_changes = if success {
                    detect_changes(&before_files, &after_files, project_path)
                } else {
                    vec![]
                };
                let error_log = if success {
                    String::new()
                } else {
                    format!(
                        "Claude Code 执行失败 (exit code: {:?})\nstderr:\n{}",
                        status.code(),
                        stderr
                    )
                };
                let combined_output = format!(
                    "=== 执行日志 ===\n小阶段 ID：{}\n\n=== 提示词 ===\n{}\n\n=== stdout ===\n{}\n=== stderr ===\n{}",
                    subtask_id, full_prompt, stdout, stderr
                );
                return Ok(project::ExecutionResult {
                    success,
                    output: combined_output,
                    error_log,
                    file_changes,
                });
            }
            Ok(None) => {
                // 进程还在运行 → 检查暂停标志
                let paused = {
                    let guard = state.lock().await;
                    guard
                        .as_ref()
                        .map_or(false, |s| s.status == PipelineStatus::Paused)
                };
                if paused {
                    // 用户点了暂停 → 强制终止 Claude Code
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(project::SubTaskError::UserPaused);
                }
                // 检查整体超时
                if start_time.elapsed() > std::time::Duration::from_secs(CLAUDE_CODE_TIMEOUT_SECS) {
                    eprintln!(
                        "[execute_subtask_inner] 子任务 {} 执行超时（已运行 {:.0}s，上限 {}s），强制终止",
                        subtask_id,
                        start_time.elapsed().as_secs(),
                        CLAUDE_CODE_TIMEOUT_SECS
                    );
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(project::SubTaskError::Timeout);
                }
                // 没暂停也没超时 → 等 500ms 再检查
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(e) => {
                let _ = child.start_kill();
                return Err(project::SubTaskError::ExecutionFailed {
                    message: format!("Claude Code 进程异常: {}", e),
                });
            }
        }
    }
}

/// Tauri command 壳：前端调用入口，内部委托给 execute_subtask_inner。
/// 前端直接调时没有暂停状态，传一个临时空 state。
#[tauri::command]
async fn execute_subtask(
    project_path: String,
    prompt: String,
    subtask_id: String,
    _milestone_id: String,
    _mid_stage_id: String,
) -> Result<project::ExecutionResult, String> {
    // 前端直接调用时，没有流水线上下文，传空 state
    let dummy_state = Arc::new(Mutex::new(None));
    execute_subtask_inner(&project_path, &prompt, &subtask_id, dummy_state)
        .await
        .map_err(|e| match e {
            project::SubTaskError::UserPaused => "用户暂停".to_string(),
            project::SubTaskError::ExecutionFailed { message } => message,
            project::SubTaskError::Timeout => "执行超时".to_string(),
        })
}

/// excute_subtask辅助函数
/// 扫描项目目录，返回所有文件路径列表（跳过 .git / node_modules / target）
/// 对比执行前后的文件列表，返回新增文件（相对路径）
fn detect_changes(before: &[String], after: &[String], project_path: &str) -> Vec<String> {
    let mut changes = Vec::new();

    // 检测新增文件（after 中有，before 中没有）
    for file in after {
        if !before.contains(file) {
            // 转换为相对路径
            if let Ok(relative) = std::path::Path::new(file).strip_prefix(project_path) {
                changes.push(relative.to_string_lossy().to_string());
            } else {
                changes.push(file.clone());
            }
        }
    }

    changes
}
/// 调用方（如 check_subtask）
///    ↓
/// run_test_command("cargo", &["test"], "/project", 120)  → 得到 (code, stdout, stderr)
///    ↓
/// summarize_test_output(code, &stdout, &stderr)           → 得到精简摘要
///    ↓
/// format_test_result("cargo test", code, &summary)        → 得到最终字符串
///    ↓
/// 返回给 AI 或前端
/// 模拟一个"测试工程师"角色：
/// 自动检查当前项目里所有改动的代码，判断是否达到了子任务的目标，并返回测试结果（通过/问题/建议）
/// 扫描项目目录，递归返回所有文件路径列表（跳过 .git / node_modules / target）
fn get_tracked_files(project_path: &str) -> Vec<String> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(project_path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path().to_string_lossy().to_string();
        // 跳过 .git / node_modules / target 目录及其内部
        if path.contains("/.git/")
            || path.contains("/node_modules/")
            || path.contains("/target/")
            || path.ends_with("/.git")
            || path.ends_with("/node_modules")
            || path.ends_with("/target")
        {
            continue;
        }
        // 只记录文件
        if entry.file_type().is_file() {
            files.push(path);
        }
    }
    files.sort();
    files
}
/// 执行测试命令，带超时控制（spawn + try_wait 轮询）
/// 返回: (exit_code, stdout, stderr)
/// 测试辅助函数
fn run_test_command(
    cmd: &str,
    args: &[&str],
    cwd: &str,
    timeout_secs: u64,
) -> Result<(i32, String, String), String> {
    use std::io::Read;
    // 创建子进程，以便捕获输出
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动进程 '{}': {}", cmd, e))?;
    // 记录开始时间
    let start = std::time::Instant::now();
    // 进入循环
    loop {
        // 检查进程是否结束（非阻塞）
        match child.try_wait() {
            // 如果已结束（Ok(Some(status))）：读取 stdout/stderr 剩余内容，返回 (exit_code, stdout, stderr)
            Ok(Some(status)) => {
                let mut out_str = String::new();
                let mut err_str = String::new();
                if let Some(mut out) = child.stdout.take() {
                    out.read_to_string(&mut out_str).ok();
                }
                if let Some(mut err) = child.stderr.take() {
                    err.read_to_string(&mut err_str).ok();
                }
                let code = status.code().unwrap_or(-1);
                return Ok((code, out_str, err_str));
            }
            // 如果还在运行（Ok(None)）：检查是否超时，若超时则 child.kill() 并返回错误；否则休眠 500 毫秒后继续轮询
            Ok(None) => {
                if start.elapsed() > std::time::Duration::from_secs(timeout_secs) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("测试超时（超过 {} 秒），已强制终止", timeout_secs));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            // 出错（Err(e)）：终止进程并返回错误
            Err(e) => {
                let _ = child.kill();
                return Err(format!("进程异常: {}", e));
            }
        }
    }
}

/// 从测试输出中提取关键信息
/// 通过 → 截取最后 500 字符
/// 失败 → 截取最后 3000 字符 + 提取含失败关键词的行
fn summarize_test_output(exit_code: i32, stdout: &str, stderr: &str) -> String {
    let combined = format!("{}{}", stdout, stderr);
    // 如果测试通过（exit_code == 0）：只保留最后 500 个字符
    if exit_code == 0 {
        // 通过：保留最后 500 字符
        if combined.len() > 500 {
            let suffix: String = combined
                .chars()
                .rev()
                .take(500)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!(
                "…(省略前面 {} 字符)…\n\n{}",
                combined.chars().count().saturating_sub(500),
                suffix
            )
        } else {
            combined
        }
    // 如果测试失败（exit_code != 0）
    } else {
        // 失败：搜索关键词，截取失败点附近的上下文
        let keywords = ["FAIL", "Error", "失败", "error", "panic", "Exception"];

        // 从末尾向前搜索关键词，取最靠近末尾的匹配位置
        let mut best_pos: Option<usize> = None;
        for kw in &keywords {
            if let Some(pos) = combined.rfind(kw) {
                match best_pos {
                    None => best_pos = Some(pos),
                    Some(current) if pos > current => best_pos = Some(pos),
                    _ => {}
                }
            }
        }

        match best_pos {
            Some(kw_byte_pos) => {
                // 将字节偏移转换为字符索引
                let kw_char_idx = combined[..kw_byte_pos].chars().count();
                let total_chars = combined.chars().count();
                let start_char = kw_char_idx.saturating_sub(500);
                let end_char = (kw_char_idx + 500).min(total_chars);
                let snippet: String = combined
                    .chars()
                    .skip(start_char)
                    .take(end_char - start_char)
                    .collect();
                format!("退出码: {}\n\n{}", exit_code, snippet)
            }
            None => {
                // 回退：未找到关键词，截取最后 3000 字符
                let tail: String = if combined.len() > 3000 {
                    combined
                        .chars()
                        .rev()
                        .take(3000)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect()
                } else {
                    combined
                };
                format!("退出码: {}\n\n{}", exit_code, tail)
            }
        }
    }
}

/// 格式化测试结果
fn format_test_result(_label: &str, command: &str, exit_code: i32, summary: &str) -> String {
    let status = if exit_code == 0 {
        "✅ 通过"
    } else {
        "❌ 失败"
    };
    format!(
        "测试命令: {}\n状态: {} (exit code: {})\n\n输出:\n{}",
        command, status, exit_code, summary
    )
}

/// 检查 stderr 是否表明测试未配置（而非测试失败）
fn is_test_not_configured(stderr: &str, stdout: &str) -> bool {
    let combined = format!("{}{}", stderr, stdout);
    combined.contains("missing script: test")
        || combined.contains("No tests found")
        || combined.contains("no test specified")
        || combined.contains("No test files found")
}

/// 测试
#[tauri::command]
async fn check_subtask(
    project_path: &str,
    subtask_goal: &str,
    _subtask_id: &str,
    _milestone_id: &str,
    _mid_stage_id: &str,
) -> Result<project::TestResult, String> {
    // 1.尝试 git diff --name-only 获取改动文件
    let files: Vec<String> = {
        let git_result = std::process::Command::new("git")
            .args(["diff", "--name-only"])
            .current_dir(&project_path)
            .output();

        match git_result {
            Ok(output) if output.status.success() => {
                // git 命令成功，解析 stdout
                let changed = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<String>>();
                if !changed.is_empty() {
                    changed
                } else {
                    // git diff 成功但无变更（工作区干净），降级走文件系统
                    vec![]
                }
            }
            _ => {
                // git 命令失败（非仓库/未安装git/其他错误），降级走文件系统
                vec![]
            }
        }
    };

    // 2.如果 git diff 没能拿到文件列表，降级：扫描项目目录中的源文件
    let files = if files.is_empty() {
        walkdir::WalkDir::new(&project_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let path = e.path().strip_prefix(&project_path).ok()?;
                let ext = path.extension()?.to_str()?;
                // 只收集常见源代码文件
                match ext {
                    "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp"
                    | "h" | "hpp" | "cs" | "rb" | "php" | "swift" | "kt" | "scala" | "vue"
                    | "svelte" | "html" | "css" | "scss" | "json" | "yaml" | "yml" | "toml"
                    | "md" | "txt" => Some(path.to_string_lossy().to_string()),
                    _ => None,
                }
            })
            .collect::<Vec<String>>()
    } else {
        files
    };

    // 3.遍历每个文件，读取内容（限制总大小防止爆 token）
    let mut file_contents = String::new();
    const MAX_CONTENT_BYTES: usize = 30_000; // 约 7500 个中文字符
    for file in &files {
        if file_contents.len() >= MAX_CONTENT_BYTES {
            break;
        }
        let content = std::fs::read_to_string(std::path::Path::new(&project_path).join(file))
            .unwrap_or_default();
        let truncated = if content.len() > 4000 {
            let prefix: String = content.chars().take(1000).collect();
            format!(
                "{}...(省略后续 {} 字符)",
                prefix,
                content.chars().count().saturating_sub(1000)
            )
        } else {
            content
        };
        file_contents.push_str(&format!("\n=== {} ===\n{}\n", file, truncated));
    }
    // ===== 真测试：检测项目类型，执行对应的测试命令 =====
    let test_output: Option<String> = {
        let project_root = std::path::Path::new(project_path);

        // 优先检测自定义测试命令文件 .metheus-test
        let metheus_test_file = project_root.join(".metheus-test");
        if metheus_test_file.exists() {
            match std::fs::read_to_string(&metheus_test_file) {
                Ok(contents) => {
                    let cmd_line = contents.trim().to_string();
                    if cmd_line.is_empty() || cmd_line.starts_with('#') {
                        eprintln!("[check_subtask] .metheus-test 为空或注释，跳过");
                        None
                    } else {
                        let parts: Vec<&str> = cmd_line.split_whitespace().collect();
                        let cmd = parts[0];
                        let cmd_args = &parts[1..];
                        eprintln!("[check_subtask] 使用自定义测试命令: {}", cmd_line);
                        match run_test_command(cmd, cmd_args, project_path, 300) {
                            Ok((code, stdout, stderr)) => {
                                let summary = summarize_test_output(code, &stdout, &stderr);
                                Some(format_test_result("自定义测试", &cmd_line, code, &summary))
                            }
                            Err(e) => Some(format!("自定义测试执行失败：{}", e)),
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[check_subtask] 读取 .metheus-test 失败: {}", e);
                    None
                }
            }
        } else if project_root.join("package.json").exists() {
            // JS/TS 项目：自动识别包管理器
            let pm = if project_root.join("pnpm-lock.yaml").exists() {
                "pnpm"
            } else if project_root.join("yarn.lock").exists() {
                "yarn"
            } else {
                "npm"
            };
            let label = format!("{} test", pm);
            match run_test_command(pm, &["test"], project_path, 300) {
                Ok((code, stdout, stderr)) => {
                    if code != 0 && is_test_not_configured(&stderr, &stdout) {
                        let stderr_preview: String = stderr.chars().take(200).collect();
                        Some(format!(
                            "测试命令: {}\n状态: ⚠️ 未配置测试用例（{} 返回：{}）\n\n该项目未配置测试用例，请仅基于代码审查判定，不要将此视为测试失败。",
                            label, pm, stderr_preview
                        ))
                    } else {
                        let summary = summarize_test_output(code, &stdout, &stderr);
                        Some(format_test_result(&label, &label, code, &summary))
                    }
                }
                Err(e) => Some(format!("{} test 执行失败（测试环境未配置）：{}", pm, e)),
            }
        } else if project_root.join("Cargo.toml").exists() {
            // Rust 项目
            match run_test_command("cargo", &["test"], project_path, 600) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result(
                        "cargo test",
                        "cargo test",
                        code,
                        &summary,
                    ))
                }
                Err(e) => Some(format!("cargo test 执行失败：{}", e)),
            }
        } else if project_root.join("go.mod").exists() {
            // Go 项目
            match run_test_command("go", &["test", "./..."], project_path, 300) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result(
                        "go test",
                        "go test ./...",
                        code,
                        &summary,
                    ))
                }
                Err(e) => Some(format!("go test 执行失败：{}", e)),
            }
        } else if project_root.join("pyproject.toml").exists()
            || project_root.join("setup.py").exists()
            || project_root.join("setup.cfg").exists()
        {
            // Python 项目：先检测 pytest 是否可用
            let (cmd, args): (&str, Vec<&str>) = if std::process::Command::new("python")
                .args(["-m", "pytest", "--version"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                ("python", vec!["-m", "pytest"])
            } else {
                ("python", vec!["-m", "unittest", "discover"])
            };
            let label = if args.contains(&"pytest") {
                "pytest"
            } else {
                "unittest"
            };
            let full_cmd = format!("{} {}", cmd, args.join(" "));
            let args_slice: Vec<&str> = args.iter().map(|s| *s).collect();
            match run_test_command(cmd, &args_slice, project_path, 300) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result(label, &full_cmd, code, &summary))
                }
                Err(e) => Some(format!("{} 执行失败：{}", label, e)),
            }
        } else if project_root.join("CMakeLists.txt").exists() {
            // C++ 项目
            match run_test_command("ctest", &[], project_path, 300) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result("ctest", "ctest", code, &summary))
                }
                Err(e) => Some(format!("ctest 执行失败：{}", e)),
            }
        } else if project_root.join("pom.xml").exists() {
            // Java Maven
            match run_test_command("mvn", &["test"], project_path, 600) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result("mvn test", "mvn test", code, &summary))
                }
                Err(e) => Some(format!("mvn test 执行失败：{}", e)),
            }
        } else if project_root.join("build.gradle").exists()
            || project_root.join("build.gradle.kts").exists()
        {
            // Java Gradle
            let gradle_cmd = if cfg!(windows) {
                "gradlew.bat"
            } else {
                "./gradlew"
            };
            match run_test_command(gradle_cmd, &["test"], project_path, 600) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result(
                        "gradle test",
                        "gradle test",
                        code,
                        &summary,
                    ))
                }
                Err(e) => Some(format!("gradle test 执行失败：{}", e)),
            }
        } else {
            eprintln!("[check_subtask] 未检测到已知测试框架，跳过真测试");
            None
        }
    };
    // Mock 版本 -> 3.4.1c改动
    // 构建测试工程师prompt 的 user_message
    // eprintln!("[check_subtask] 测试结果注入完成, test_output 长度: {}",
    //     test_output.as_ref().map(|s| s.len()).unwrap_or(0));
    // 构造子任务目标描述（注入给测试工程师 AI）
    let goal_section = if subtask_goal.is_empty() {
        "## 子任务目标\n（未提供子任务目标描述，请仅根据代码变更做通用质量检查）\n\n".to_string()
    } else {
        let truncated: String = subtask_goal.chars().take(2000).collect();
        let suffix = if subtask_goal.chars().count() > 2000 { "…（已截断）" } else { "" };
        format!(
            "## 子任务目标\n{}\n{}\n请根据以上目标，检查下列代码变更是否完整、正确地实现了该目标。\n\n",
            truncated, suffix
        )
    };
    let user_message = if let Some(ref test_result) = test_output {
        format!(
            "{}请检查以下代码改动。\n\n## 自动化测试结果\n项目自动化测试已执行，结果如下：\n\n{}\n\n---\n\n## 改动文件列表（共 {} 个文件）\n{}\n\n## 改动文件内容\n{}",
            goal_section,
            test_result,
            files.len(),
            files.join("\n"),
            file_contents
        )
    } else {
        format!(
            "{}请检查以下代码改动：\n\n## 改动文件列表（共 {} 个文件）\n{}\n\n## 改动文件内容\n{}",
            goal_section,
            files.len(),
            files.join("\n"),
            file_contents
        )
    };
    //     test_output.as_ref().map(|s| s.len()).unwrap_or(0));
    // 调用 AI（强制 JSON 模式）
    let mut diagnosis_warnings: Vec<String> = Vec::new();
    let raw_reply = call_deepseek_api_json(TEST_PROMPT, &user_message).await
        .unwrap_or_else(|e| {
            eprintln!("[check_subtask] AI API 调用失败：{}，返回兜底 JSON", e);
            diagnosis_warnings.push(format!("AI API 调用失败：{}", e));
            r#"{"passed": false, "issues": ["AI API 调用失败"], "suggestion": ""}"#.to_string()
        });
    // 兜底：如果 AI 返回空数组 []，自动转换为测试通过
    let raw_reply = if raw_reply.trim() == "[]" {
        eprintln!("[check_subtask] AI 返回空数组 []，自动转换为测试通过");
        diagnosis_warnings.push("AI 返回空数组，自动判定为通过".to_string());
        r#"{"passed": true, "issues": [], "suggestion": ""}"#.to_string()
    } else {
        raw_reply
    };
    // 解析 JSON 响应（带兜底）
    let test_result: project::TestResult = match parse_json_with_retry::<project::TestResult>(&raw_reply).await {
        Ok(mut result) => {
            result.warnings.extend(diagnosis_warnings);
            result
        }
        Err(e) => {
            eprintln!("[check_subtask] TestResult JSON 解析失败：{}，使用默认失败结果", e);
            let preview: String = raw_reply.chars().take(200).collect();
            diagnosis_warnings.push(format!(
                "TestResult JSON 解析失败：{}。原始内容（前200字符）：{}",
                e, preview
            ));
            project::TestResult {
                passed: false,
                issues: vec![format!("AI 返回格式异常，解析失败：{}。原始内容（前200字符）：{}", e, preview)],
                suggestion: "AI 返回格式异常，请人工审查".to_string(),
                warnings: diagnosis_warnings,
            }
        }
    };
    Ok(test_result)
}

/// 生成下一个子任务的提示词
#[tauri::command]
async fn generate_next_prompt(
    mid_stage_title: String,
    mid_stage_description: String,
    previous_subtask_title: String,
    previous_subtask_result: String,
    file_changes: Vec<String>,
    test_result: String,
    is_retry: bool,
    retry_reason: String,
) -> Result<project::GeneratedSubtask, String> {
    // 构建开发工程师 prompt 的 user_message
    let user_message = format!(
        "## 当前中阶段\n标题：{}\n描述：{}\n\n## 上一个小阶段\n标题：{}\n执行结果：{}\n\n## 改动文件\n{}\n\n## 测试结果\n{}\n\n## 是否重试\n{}\n\n## 打回原因\n{}\n\n## 项目技术栈约束\n本项目是一个 Tauri 桌面应用，必须遵守以下技术栈约束：\n- 后端使用 Rust 语言，文件位于 src-tauri/src/ 目录\n- 前端使用 React + TypeScript，文件位于 src/ 目录\n- 不使用任何数据库（无 MySQL、MongoDB、PostgreSQL、SQLite 等）\n- 不使用任何 ORM（无 Mongoose、Prisma、TypeORM 等）\n- 不使用 Express、Koa、Next.js 等服务端框架\n- 如果提示词需要创建文件，文件路径必须在上述目录范围内\n- 提示词中不得出现与项目技术栈无关的技术名词",
        mid_stage_title,
        mid_stage_description,
        previous_subtask_title,
        previous_subtask_result,
        file_changes.join("\n"),
        test_result,
        if is_retry { "是" } else { "否" },
        retry_reason
    );
    // 调用 AI
    let reply = call_deepseek_api_json(TECH_PROMPT, &user_message).await?;
    // 解析 JSON 响应
    let generated: project::GeneratedSubtask = parse_json_with_retry(&reply)
        .await
        .map_err(|e| format!("解析生成结果失败：{}", e))?;
    Ok(generated)
}

/// 从硬盘加载项目数据（Tauri 命令）
#[tauri::command]
async fn get_project(project_name: String) -> Result<project::Project, String> {
    let name = if project_name.is_empty() {
        "我的游戏".to_string()
    } else {
        project_name
    };
    match load_project(&name) {
        Ok(project) => Ok(project),
        Err(_) => {
            // 文件不存在时返回默认空项目，不报错
            Ok(project::Project::new(&name))
        }
    }
}

/// 3.3 执行引擎流水线
/// 根据传入的中阶段信息和子任务列表，启动一个后台任务，逐个执行这些子任务，并实时更新执行状态（运行中、成功、失败等）
/// 启动后台流水线执行一组子任务，立即返回成功，执行进度保存在全局状态中，前端可查询。
#[tauri::command]
async fn start_execution(
    state: tauri::State<'_, AppState>,
    project_id: String,
    project_path: String,
    mid_stage_id: String,
    mid_stage_title: String,
    mid_stage_description: String,
    subtasks_json: String,
) -> Result<(), String> {
    // 前置校验：项目路径有效性
    let path_check = check_project_path(&project_path);
    if !path_check.is_valid {
        return Err(format!(
            "项目目录无效，无法启动执行：{}",
            path_check.error_message
        ));
    }
    // 自动初始化 git 仓库（路径有效但不是 git 仓库时）
    if !path_check.is_git_repo {
        let init = std::process::Command::new("git")
            .args(["init"])
            .current_dir(&project_path)
            .output()
            .map_err(|e| format!("{}：git 命令不可用 — {}", GIT_INIT_FAILED, e))?;
        if !init.status.success() {
            let stderr = String::from_utf8_lossy(&init.stderr);
            let truncated: String = stderr.chars().take(200).collect();
            return Err(format!("{}：{}", GIT_INIT_FAILED, truncated));
        }
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&project_path)
            .output()
            .map_err(|e| format!("git add 失败：{}", e))?;
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", GIT_AUTO_INIT_COMMIT_MSG])
            .current_dir(&project_path)
            .output()
            .map_err(|e| format!("git commit 失败：{}", e))?;
    }
    // 解析子任务列表：把 subtasks_json 转成 Rust 结构体 Vec<Subtask>
    let subtasks: Vec<project::Subtask> =
        serde_json::from_str(&subtasks_json).map_err(|e| format!("解析小阶段列表失败：{}", e))?;
    if subtasks.is_empty() {
        return Err("子任务列表为空，请先生成执行计划".to_string());
    }
    let pipeline_state = state.pipeline_state.clone();
    // 初始化状态 在全局共享状态中创建一个 PipelineState，
    // 记录当前阶段 ID、总任务数、每个子任务的状态（等待/执行中/成功/失败）、当前日志等
    {
        let mut guard = pipeline_state.lock().await;
        *guard = Some(PipelineState {
            mid_stage_id: mid_stage_id.clone(),
            status: PipelineStatus::Running,
            current_subtask_index: 0,
            total_subtasks: subtasks.len(),
            subtask_statuses: subtasks
                .iter()
                .map(|s| SubtaskStatusItem {
                    subtask_id: s.id.clone(),
                    title: s.title.clone(),
                    status: "waiting".to_string(),
                    test_result: None,
                    retry_count: 0,
                })
                .collect(),
            current_log: "🚀 流水线已启动".to_string(),
            last_error: None,
        });
    }
    // 启动后台任务：
    // 用 tokio::spawn 启动一个异步任务，调用 execute_mid_stage_pipeline 真正去执行这些子任务
    tokio::spawn(async move {
        let result = execute_mid_stage_pipeline(
            project_id.clone(),
            mid_stage_id.clone(),
            project_path,
            subtasks,
            mid_stage_title,
            mid_stage_description,
            pipeline_state.clone(),
        )
        .await;
        if let Err(e) = result {
            let mut guard = pipeline_state.lock().await;
            // 捕获失败：如果后台任务执行失败，会将全局状态中的流水线标记为 Failed，并记录错误日志
            if let Some(s) = guard.as_mut() {
                s.status = PipelineStatus::Failed;
                s.last_error = Some(e.clone());
                s.current_log = format!("❌ 流水线失败: {}", e);
            }
        }
    });
    // 不等待结果直接返回：
    // 函数立即返回 Ok(())，后台任务继续运行。这样前端调用后不会卡住
    Ok(())
}
/// 这个函数就是一条自动流水线：
/// 逐个执行子任务，每个子任务最多重试 3 次，期间可以暂停/恢复，并实时更新进度面板。
/// 1. 初始化（准备空账本）
///   ↓
/// 2. 循环每个子任务（洗菜 → 切菜 → 炒菜）
///   ├─ 2.1 更新状态：开始做这道菜
///   ├─ 2.2 内层重试循环（最多 3 次）
///   │    ├─ 暂停检查（如果老板喊停，就等待恢复）
///   │    ├─ 生成提示词（告诉厨师下一步做什么）
///   │    ├─ 执行子任务（厨师做菜）
///   │    ├─ 运行测试（质检员尝菜）
///   │    ├─ 如果通过 → 记录成功，跳出重试循环
///   │    └─ 如果失败 → 重试次数+1，继续重试
///   └─ 2.3 更新状态：这道菜完成
///   ↓
/// 3. 全部子任务完成 → 更新最终状态为"完成"
#[tauri::command]
async fn get_execution_status(
    state: tauri::State<'_, AppState>,
) -> Result<Option<PipelineState>, String> {
    let guard = state.pipeline_state.lock().await;
    Ok(guard.clone())
}
/// 暂停流水线执行
#[tauri::command]
async fn pause_execution(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.pipeline_state.lock().await;
    match guard.as_mut() {
        Some(s) if s.status == PipelineStatus::Running => {
            s.status = PipelineStatus::Paused;
            s.current_log = "⏸ 已暂停".to_string();
            Ok(())
        }
        _ => Err("当前没有正在执行的流水线".to_string()),
    }
}

/// 恢复流水线执行
#[tauri::command]
async fn resume_execution(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.pipeline_state.lock().await;
    match guard.as_mut() {
        Some(s) if s.status == PipelineStatus::Paused => {
            s.status = PipelineStatus::Running;
            s.current_log = "▶ 已恢复".to_string();
            Ok(())
        }
        _ => Err("当前没有已暂停的流水线".to_string()),
    }
}

/// 停止流水线执行
#[tauri::command]
async fn stop_execution(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.pipeline_state.lock().await;
    match guard.as_mut() {
        Some(s) => {
            s.status = PipelineStatus::Failed;
            s.last_error = Some("用户手动停止".to_string());
            s.current_log = "⏹ 已停止".to_string();
            Ok(())
        }
        None => {
            // 没有活跃执行，幂等返回成功
            Ok(())
        }
    }
}

/// 按顺序执行一组子任务（subtasks），每个子任务可能重试最多 3 次，全部通过后标记中阶段完成；
/// 过程中实时更新全局状态（进度、日志、测试结果），并支持暂停/恢复。
async fn execute_mid_stage_pipeline(
    project_id: String,
    mid_stage_id: String,
    project_path: String,
    mut subtasks: Vec<project::Subtask>,
    mid_stage_title: String,
    mid_stage_description: String,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Result<(), String> {
    // 初始化变量
    let mut previous_result = String::new();
    let mut previous_title = String::new();
    let mut file_changes: Vec<String> = vec![];
    let mut last_test_result = String::new();
    let mut mid_stage_version = String::new();

    // 提前从 project 文件中提取 mid_stage_version（避免只在函数末尾获取）
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let project_file = std::path::Path::new(&home)
            .join(".metheus")
            .join(format!("{}.json", project_id));
        if let Ok(content) = std::fs::read_to_string(&project_file) {
            if let Ok(project) = serde_json::from_str::<project::Project>(&content) {
                for milestone in &project.milestones {
                    for mid_stage in &milestone.mid_stages {
                        if mid_stage.id == mid_stage_id {
                            mid_stage_version = mid_stage.version.clone();
                            break;
                        }
                    }
                    if !mid_stage_version.is_empty() {
                        break;
                    }
                }
            }
        }
    }

    for i in 0..subtasks.len() {
        let subtask_title = subtasks[i].title.clone();
        let subtask_id = subtasks[i].id.clone();
        let mut retry_count = 0u32;
        let max_retries = 3u32;
        // 更新状态
        // 标记当前子任务为 "executing"，更新 current_log
        {
            let mut guard = state.lock().await;
            if let Some(s) = guard.as_mut() {
                s.current_subtask_index = i;
                if i > 0 {
                    s.subtask_statuses[i - 1].status = "passed".to_string();
                }
                s.subtask_statuses[i].status = "executing".to_string();
                s.current_log =
                    format!("▶ 执行中 ({}/{})：{}", i + 1, subtasks.len(), subtask_title);
            }
        }
        while retry_count < max_retries {
            // 暂停检查
            // 如果全局状态变成 Paused，就循环等待直到恢复
            {
                let guard = state.lock().await;
                if let Some(s) = guard.as_ref() {
                    if s.status == PipelineStatus::Paused {
                        drop(guard);
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            let guard2 = state.lock().await;
                            if let Some(s2) = guard2.as_ref() {
                                if s2.status != PipelineStatus::Paused {
                                    // 恢复后立即检查：如果被标记为 Failed，直接终止流水线
                                    if s2.status == PipelineStatus::Failed {
                                        return Err("流水线已被取消".to_string());
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            // 生成 prompt：根据上一步结果、文件变更、测试结果等，调用 generate_next_prompt 得到下一步的标题和指令
            let generated = generate_next_prompt(
                mid_stage_title.clone(),
                mid_stage_description.clone(),
                previous_title.clone(),
                previous_result.clone(),
                file_changes.clone(),
                last_test_result.clone(),
                retry_count > 0,
                if retry_count > 0 {
                    last_test_result.clone()
                } else {
                    String::new()
                },
            )
            .await?;
            // 更新日志
            {
                let mut guard = state.lock().await;
                if let Some(s) = guard.as_mut() {
                    s.current_log = format!("⚙️ {}", generated.title);
                }
            }
            // 执行子任务（可被暂停中断）
            let exec_result = match execute_subtask_inner(
                &project_path,
                &generated.prompt,
                &subtask_id,
                state.clone(),
            )
            .await
            {
                Ok(r) => r,
                Err(project::SubTaskError::UserPaused) => {
                    // 用户暂停：Claude Code 已被 kill，进入等待恢复循环
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        let guard = state.lock().await;
                        if let Some(s) = guard.as_ref() {
                            if s.status != PipelineStatus::Paused {
                                // 已恢复（或已取消）
                                if s.status == PipelineStatus::Failed {
                                    return Err("流水线已被取消".to_string());
                                }
                                break;
                            }
                        }
                    }
                    // 恢复后重新执行当前子任务（不增加 retry_count）
                    continue;
                }
                Err(e) => {
                    let msg = match e {
                        project::SubTaskError::ExecutionFailed { message } => message,
                        project::SubTaskError::Timeout => "执行超时".to_string(),
                        _ => format!("{:?}", e),
                    };
                    return Err(format!("Claude Code 执行失败：{}", msg));
                }
            };
            subtasks[i].execution_result = Some(exec_result.clone());
            file_changes = exec_result.file_changes.clone();
            // 记录执行结果到日志
            {
                let mut guard = state.lock().await;
                if let Some(s) = guard.as_mut() {
                    if exec_result.success {
                        s.current_log = format!(
                            "✅ 完成: {} (变更 {} 个文件)",
                            subtask_title,
                            file_changes.len()
                        );
                    } else {
                        s.current_log = format!(
                            "❌ 失败: {} — {}",
                            subtask_title,
                            exec_result.error_log.chars().take(100).collect::<String>()
                        );
                    }
                }
            }
            // Claude Code 进程失败 -> 不进入重试循环，直接中止
            if !exec_result.success {
                return Err(format!("Claude Code 执行失败：{}", exec_result.error_log));
            }
            // 运行测试
            // 调用 check_subtask，得到 test.passed
            {
                let mut guard = state.lock().await;
                if let Some(s) = guard.as_mut() {
                    s.subtask_statuses[i].status = "testing".to_string();
                    s.current_log = format!("🔍 测试: {}", subtask_title);
                }
            }
            // 检查
            let test = match check_subtask(
                &project_path,
                &generated.prompt,
                &subtask_id,
                &subtask_title,
                &mid_stage_id,
            )
            .await
            {
                Ok(t) => t,
                Err(err) => project::TestResult {
                    passed: false,
                    issues: vec![format!("测试服务不可用: {}", err)],
                    suggestion: "请手动检查".to_string(),
                    warnings: vec![],
                },
            };
            last_test_result = if test.passed {
                "通过".to_string()
            } else if test.issues.is_empty() {
                "不通过（测试工程师未提供具体问题）".to_string()
            } else {
                let issues_text = test.issues.iter()
                    .map(|issue| format!("- {}", issue))
                    .collect::<Vec<_>>()
                    .join("\n");
                let full = format!("不通过。具体问题：\n{}", issues_text);
                // 防止 retry_reason 过长，截断到 1000 字符
                if full.chars().count() > 1000 {
                    format!("{}…（已截断）", full.chars().take(1000).collect::<String>())
                } else {
                    full
                }
            };
            if test.passed {
                {
                    let mut guard = state.lock().await;
                    if let Some(s) = guard.as_mut() {
                        s.subtask_statuses[i].status = "passed".to_string();
                        s.subtask_statuses[i].test_result = Some(test.clone());
                        s.current_log = format!("✅ 通过: {}", subtask_title);
                    }
                }
                previous_result = "通过".to_string();
                previous_title = subtask_title.clone();
                subtasks[i].test_result = Some(test);
                subtasks[i].retry_count = retry_count;

                // === 宪法更新链 ===
                let mut constitution_updated = false;
                let mut old_constitution = String::new();
                // 步骤 1：获取 git diff
                match std::process::Command::new("git")
                    .args(["diff", "HEAD"])
                    .current_dir(&project_path)
                    .output()
                {
                    Ok(output) => {
                        let diff_stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        // 步骤 2：提取变更摘要
                        let diff_summary = extract_diff_summary(&diff_stdout);
                        // 步骤 3：检查是否有实际变更
                        if diff_summary.new_files.is_empty()
                            && diff_summary.modified_files.is_empty()
                            && diff_summary.deleted_files.is_empty()
                            && diff_summary.new_functions.is_empty()
                            && diff_summary.modified_functions.is_empty()
                            && diff_summary.deleted_functions.is_empty()
                            && diff_summary.changed_dependencies.is_empty()
                        {
                            eprintln!("[constitution] 宪法更新跳过（无变更）");
                        } else {
                            // 步骤 4：读取当前 CONSTITUTION.md
                            let constitution_path =
                                std::path::Path::new(&project_path).join("CONSTITUTION.md");
                            let constitution_content =
                                std::fs::read_to_string(&constitution_path).unwrap_or_default();
                            old_constitution = constitution_content.clone();
                            // 步骤 5：调用 update_constitution
                            match update_constitution(constitution_content.clone(), diff_summary)
                                .await
                            {
                                Ok(updated_content) => {
                                    // 步骤 6：写回 CONSTITUTION.md
                                    if let Err(e) =
                                        std::fs::write(&constitution_path, &updated_content)
                                    {
                                        eprintln!(
                                            "[constitution] 写入 CONSTITUTION.md 失败：{}",
                                            e
                                        );
                                    } else {
                                        constitution_updated = true;
                                        // 步骤 6b：检查是否需要剪枝
                                        // 提取第 2 部分，超过阈值则触发 compact_constitution
                                        if let Some(part2_start) =
                                            updated_content.find("## 第 2 部分")
                                        {
                                            let part2 = &updated_content[part2_start..];
                                            if estimate_tokens(part2) > COMPACTION_TRIGGER_TOKENS {
                                                match compact_constitution(updated_content.clone())
                                                    .await
                                                {
                                                    Ok(compacted) => {
                                                        if let Err(e) = std::fs::write(
                                                            &constitution_path,
                                                            &compacted,
                                                        ) {
                                                            eprintln!(
                                                                "[constitution] 写入剪枝后宪法失败：{}",
                                                                e
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        eprintln!(
                                                            "[constitution] 宪法剪枝失败，保留膨胀版本：{}",
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[constitution] 宪法更新失败：{}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[constitution] git diff 失败，跳过宪法更新：{}", e);
                    }
                }

                // === git_save_subtask ===
                match git_save_subtask(
                    project_path.clone(),
                    (i + 1) as u32,
                    mid_stage_version.clone(),
                    subtask_title.clone(),
                )
                .await
                {
                    Ok(tag_name) => {
                        subtasks[i].auto_tag = Some(tag_name);
                    }
                    Err(e) => {
                        eprintln!("[constitution] git_save_subtask 失败：{}", e);
                        // 如果宪法在此次流水线中被更新过，回退宪法到更新前的内容
                        if constitution_updated {
                            let constitution_path =
                                std::path::Path::new(&project_path).join("CONSTITUTION.md");
                            if let Err(e2) = std::fs::write(&constitution_path, &old_constitution) {
                                eprintln!(
                                    "[constitution] 宪法回退写入也失败，宪法可能处于不一致状态：{}",
                                    e2
                                );
                            } else {
                                eprintln!(
                                    "[constitution] git_save_subtask 失败，宪法已回退到更新前状态"
                                );
                            }
                        }
                    }
                }

                break;
            } else {
                retry_count += 1;
                subtasks[i].retry_count = retry_count;
                {
                    let mut guard = state.lock().await;
                    if let Some(s) = guard.as_mut() {
                        s.subtask_statuses[i].status = "retrying".to_string();
                        s.subtask_statuses[i].retry_count = retry_count;
                        s.current_log =
                            format!("🔄 重试 {}/{}: {}", retry_count, max_retries, subtask_title);
                    }
                }
                if retry_count >= max_retries {
                    return Err(format!(
                        "小阶段「{}」重试 {} 次仍未通过",
                        subtask_title, max_retries
                    ));
                }
            }
        }
    }
    // 全部完成
    // 更新状态为 "passed"，记录测试结果，break 出 while 循环
    {
        let mut guard = state.lock().await;
        if let Some(s) = guard.as_mut() {
            s.status = PipelineStatus::Completed;
            if let Some(last) = s.subtask_statuses.last_mut() {
                last.status = "passed".to_string();
            }
            s.current_log = "✅ 所有小阶段执行完成！".to_string();
        }
    }
    // 写回 project 文件
    // 流水线跑完后，找到项目文件里对应的那个"中阶段"（MidStage），
    // 把每个子任务的执行结果、测试结果、重试次数填进去，
    // 然后把中阶段状态改成 "completed"，最后保存文件
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let project_file = std::path::Path::new(&home)
        .join(".metheus")
        .join(format!("{}.json", project_id));
    if let Ok(content) = std::fs::read_to_string(&project_file) {
        if let Ok(mut project) = serde_json::from_str::<project::Project>(&content) {
            // 找到对应的 MidStage，更新结果
            for milestone in &mut project.milestones {
                for mid_stage in &mut milestone.mid_stages {
                    if mid_stage.id == mid_stage_id {
                        mid_stage_version = mid_stage.version.clone();
                        for (i, subtask) in mid_stage.subtasks.iter_mut().enumerate() {
                            if i < subtasks.len() {
                                subtask.execution_result = subtasks[i].execution_result.clone();
                                subtask.test_result = subtasks[i].test_result.clone();
                                subtask.retry_count = subtasks[i].retry_count;
                                subtask.auto_tag = subtasks[i].auto_tag.clone();
                            }
                        }
                        mid_stage.status = project::MidStageStatus::Completed;
                        break;
                    }
                }
            }
            // 保存
            if let Ok(json) = serde_json::to_string_pretty(&project) {
                let _ = std::fs::write(&project_file, json);
            }
        }
    }
    // === 全部小阶段完成，自动 Git 存档 ===
    let tag_name = format!("metheus/{}", mid_stage_version);
    git_save_node(
        project_path.to_string(),
        mid_stage_version.clone(),
        mid_stage_title.to_string(),
    )
    .await?;
    save_tag_to_mid_stage(&project_id, &mid_stage_id, &tag_name)?;
    Ok(())
}

/// 通用的 DeepSeek 调用函数，
/// 给它系统提示词和用户消息，它返回 AI 生成的文本（JSON 格式），并处理所有网络和解析错误。
// ===== 纯文本对话用（不强制 JSON） =====
async fn call_deepseek_api(system_prompt: &str, user_message: &str) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, false, 0.1).await
}

// ===== 结构化输出用（强制 JSON） =====
async fn call_deepseek_api_json(system_prompt: &str, user_message: &str) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, true, 0.5).await
}

// ===== 内部实现 =====
async fn call_deepseek_api_inner(
    system_prompt: &str,
    user_message: &str,
    force_json: bool,
    temperature: f64,
) -> Result<String, String> {
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!(
                "[call_deepseek_api_inner] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端",
                e
            );
            reqwest::Client::new()
        });

    let mut messages: Vec<serde_json::Value> = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt
        }));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": user_message
    }));

    let mut body = serde_json::json!({
        "model": "deepseek-v4-flash",
        "messages": messages,
        "temperature": temperature,
    });

    if force_json {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }

    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;

    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let reply = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复异常".to_string())?
        .to_string();

    Ok(reply)
}

/// 清洗 AI 返回的文本，提取出纯净的 JSON 字符串
/// 处理三种干扰：
///   1. Markdown 代码块包裹（```json ... ```）
///   2. 礼貌前缀（"好的，以下是JSON："）
///   3. 末尾多余文字
fn sanitize_json_response(raw: &str) -> String {
    let text = raw.trim();
    // 第一层：处理 Markdown 代码块包裹
    let text = if text.starts_with("```") {
        // 跳过第一行（可能是```json 或 ```）
        let after_first_newline = text.find("\n").map(|i| &text[i + 1..]).unwrap_or(text);
        // 找到最后一个```, 截断到它之前
        match after_first_newline.rfind("\n```") {
            Some(pos) => &after_first_newline[..pos],
            None => after_first_newline,
        }
    } else {
        text
    };
    // 第二层: 找到第一个 [ 或 {
    let start = text.find('[').or_else(|| text.find('{')).unwrap_or(0);
    // 第三层：用括号计数器找到匹配的闭合位置
    // 使用字节迭代器：{ } [ ] 都是 ASCII 单字节字符，byte_offset 与 start（字节索引）单位一致
    let end = {
        let mut depth: i32 = 0;
        let mut found_end = text.len();
        for (byte_offset, byte) in text[start..].bytes().enumerate() {
            match byte {
                b'{' | b'[' => depth += 1,
                b'}' | b']' => {
                    depth -= 1;
                    if depth == 0 {
                        found_end = start + byte_offset + 1; // 同为字节索引，相加正确
                        break;
                    }
                }
                _ => {}
            }
        }
        found_end
    };
    let result = text[start..end].to_string();
    let result = result.trim();
    if result.is_empty() {
        eprintln!("[sanitize_json_response] 清洗后为空字符串，返回兜底 JSON 对象");
        SANITIZE_FALLBACK_JSON.to_string()
    } else {
        result.to_string()
    }
}

/// 带重试的 JSON 解析
/// 第 1 次：sanitize → 直接解析
/// 第 2 次：把错误发给 AI 修正 → sanitize → 解析
/// 第 3 次：再次发给 AI 修正（附"最后一次机会"）→ 解析
/// 三次全失败则返回错误
async fn parse_json_with_retry<T: serde::de::DeserializeOwned + Default>(
    response_text: &str,
) -> Result<T, String> {
    // 第一次尝试：直接 sanitize + 解析
    let cleaned = sanitize_json_response(response_text);
    match serde_json::from_str::<T>(&cleaned) {
        Ok(value) => return Ok(value),
        Err(first_err) => {
            eprintln!("[parse_json_with_retry] 第一次解析失败：{}", first_err);
        }
    }
    // 第二次尝试：请 AI 修正 JSON
    let system_prompt = "你是一个 JSON 修复工具。用户会给你一段有格式错误的 JSON 文本和一个解析错误信息。请输出修正后的合法 JSON。只输出 JSON，不要 Markdown 包裹，不要任何解释文字。";
    let user_message = format!(
        "以下 JSON 解析失败。\n\n错误信息：\n解析失败，请检查 JSON 格式是否正确。\n\n原始内容：\n{}\n\n请修正后重新输出，只输出 JSON，不要任何其他内容。",
        cleaned
    );
    match call_deepseek_api_inner(system_prompt, &user_message, false, 0.5).await {
        Ok(reply) => {
            let cleaned2 = sanitize_json_response(&reply);
            match serde_json::from_str::<T>(&cleaned2) {
                Ok(value) => return Ok(value),
                Err(second_err) => {
                    eprintln!("[parse_json_with_retry] 第2次解析失败：{}", second_err);
                }
            }
        }
        Err(e) => {
            eprintln!("[parse_json_with_retry] AI 修正失败：{}", e);
        }
    }
    // 第三次尝试：最后机会
    let user_message_last = format!(
        "以下 JSON 解析仍然失败，这是最后一次修正机会。\n\n原始内容：\n{}\n\n请修正后只输出 JSON，不要任何其他内容。如果仍无法修正，请输出一个空 JSON 对象 {{}}。",
        cleaned
    );
    match call_deepseek_api_inner(system_prompt, &user_message_last, false, 0.5).await {
        Ok(reply) => {
            let cleaned3 = sanitize_json_response(&reply);
            match serde_json::from_str::<T>(&cleaned3) {
                Ok(value) => Ok(value),
                Err(final_err) => {
                    let preview: String = cleaned3.chars().take(200).collect();
                    let original_preview: String = response_text.chars().take(200).collect();
                    eprintln!(
                        "[parse_json_with_retry] 第 3 次解析仍然失败：{}，返回默认值。\
                         AI 修正后内容（前200字符）：{}；原始响应（前200字符）：{}",
                        final_err,
                        preview,
                        original_preview
                    );
                    Ok(T::default())
                }
            }
        }
        Err(e) => {
            let original_preview: String = response_text.chars().take(200).collect();
            eprintln!(
                "[parse_json_with_retry] AI 修正请求失败（第 3 次）：{}，返回默认值。原始响应（前200字符）：{}",
                e,
                original_preview
            );
            Ok(T::default())
        }
    }
}

///包装器，前端传来的JSON项目数据保存
#[tauri::command]
async fn persist_project(project_json: String) -> Result<String, String> {
    //前端发来的 JSON 字符串转成 Project 对象
    let project: project::Project =
        serde_json::from_str(&project_json).map_err(|e| format!("解析项目失败：{}", e))?;
    //调用已有的保存函数，把项目写入文件
    save_project(&project)?;
    Ok("保存成功".to_string())
}

/// 审批命令
/// 根据 project_id 和 mid_stage_id，找到对应的中阶段，把它的状态改为 "approved"，然后保存回文件
#[tauri::command]
async fn approve_mid_stage(project_id: String, mid_stage_id: String) -> Result<String, String> {
    // 1. 获取home 目录
    let app_dir = dirs::home_dir().ok_or("无法获取 home 目录".to_string())?;
    // 2. 构造项目文件路径
    let project_file = app_dir
        .join(".metheus")
        .join(format!("{}.json", project_id));
    // 3. 读取文件内容
    let content =
        std::fs::read_to_string(&project_file).map_err(|e| format!("读取项目文件失败：{}", e))?;
    // 4. 解析为 Project 结构
    let mut project: project::Project =
        serde_json::from_str(&content).map_err(|e| format!("解析项目文件失败：{}", e))?;
    // 5. 双层循环查找并批准 mid_stage
    let mut found = false;
    let mut current_milestone_index = 0;
    for (mi, milestone) in project.milestones.iter().enumerate() {
        for mid_stage in &milestone.mid_stages {
            if mid_stage.id == mid_stage_id {
                current_milestone_index = mi;
                found = true;
                break;
            }
        }
        if found {
            break;
        }
    }
    if !found {
        return Err("未找到指定的中阶段".to_string());
    }
    // 将当前 mid_stage 标记为 Approved
    {
        let milestone = &mut project.milestones[current_milestone_index];
        for mid_stage in &mut milestone.mid_stages {
            if mid_stage.id == mid_stage_id {
                mid_stage.status = project::MidStageStatus::Approved;
                break;
            }
        }
    }
    // 6. 查找下一个可推进的中阶段（在当前 milestone 内）
    let mut next_mid_stage_id: Option<String> = None;
    let mut next_milestone_id: Option<String> = None;
    let mut project_completed = false;

    {
        let milestone = &project.milestones[current_milestone_index];
        let mut found_current = false;
        for mid_stage in &milestone.mid_stages {
            if mid_stage.id == mid_stage_id {
                found_current = true;
                continue;
            }
            if found_current
                && (mid_stage.status == project::MidStageStatus::Pending
                    || mid_stage.status == project::MidStageStatus::Ready)
            {
                next_mid_stage_id = Some(mid_stage.id.clone());
                next_milestone_id = Some(milestone.id.clone());
                break;
            }
        }
    }

    // 如果当前 milestone 内没有下一个 mid_stage，标记当前 milestone 为 Completed
    if next_mid_stage_id.is_none() {
        project.milestones[current_milestone_index].status = project::MilestoneStatus::Completed;

        // 查找下一个 Pending/Ready 的大阶段
        for mi in (current_milestone_index + 1)..project.milestones.len() {
            let ms = &project.milestones[mi];
            if ms.status == project::MilestoneStatus::Pending
                || ms.status == project::MilestoneStatus::InProgress
            {
                // 找到下一个大阶段，将其第一个 mid_stage 设为 Ready
                if let Some(first_mid) = ms.mid_stages.first() {
                    next_mid_stage_id = Some(first_mid.id.clone());
                    next_milestone_id = Some(ms.id.clone());
                    break;
                }
            }
        }

        // 如果仍然没有找到下一个，标记项目完成
        if next_mid_stage_id.is_none() {
            project.status = project::ProjectStatus::Completed;
            project_completed = true;
        }
    }

    // 将找到的下一中阶段设为 Ready
    if let Some(ref next_mid_id) = next_mid_stage_id {
        for milestone in &mut project.milestones {
            for mid_stage in &mut milestone.mid_stages {
                if mid_stage.id == *next_mid_id {
                    mid_stage.status = project::MidStageStatus::Ready;
                    break;
                }
            }
        }
    }

    // 序列化回 JSON 并写回文件
    let json = serde_json::to_string_pretty(&project).map_err(|e| format!("序列化失败：{}", e))?;
    std::fs::write(&project_file, json).map_err(|e| format!("保存失败：{}", e))?;

    // 构造返回值
    let result = serde_json::json!({
        "next_milestone_id": next_milestone_id,
        "next_mid_stage_id": next_mid_stage_id,
        "project_completed": project_completed,
    });
    Ok(result.to_string())
}
/// 拒绝指定的中阶段：把它的状态改成 "rejected"，然后保存回项目文件
#[tauri::command]
async fn reject_mid_stage(project_id: String, mid_stage_id: String) -> Result<(), String> {
    let app_dir = dirs::home_dir().ok_or("无法获取 home 目录".to_string())?;
    let project_path = app_dir
        .join(".metheus")
        .join(format!("{}.json", project_id));
    let content =
        std::fs::read_to_string(&project_path).map_err(|e| format!("读取项目文件失败: {}", e))?;
    let mut project: project::Project =
        serde_json::from_str(&content).map_err(|e| format!("解析项目文件失败: {}", e))?;
    let mut found = false;
    for milestone in &mut project.milestones {
        for mid_stage in &mut milestone.mid_stages {
            if mid_stage.id == mid_stage_id {
                mid_stage.status = project::MidStageStatus::Rejected;
                found = true;
                break;
            }
        }
        if found {
            break;
        }
    }
    if !found {
        return Err("未找到指定的中阶段".to_string());
    }
    let json = serde_json::to_string_pretty(&project).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(&project_path, json).map_err(|e| format!("保存失败: {}", e))?;
    Ok(())
}

/// 校验 AI 更新的宪法内容是否合法
///
/// 检查三个维度：
/// 1. 第 1 部分是否被修改（防 AI 越界修改）
/// 2. 第 2 部分结构是否完整
/// 3. 返回内容是否为空或过短
fn validate_constitution_update(before: &str, after: &str) -> ValidationResult {
    // 第 1 层：空内容检查
    if after.trim().len() < 100 {
        return ValidationResult::Empty(format!("返回内容仅 {} 字符，过短", after.trim().len()));
    }

    // 提取"更新前"第 1 部分
    fn extract_part1(text: &str) -> Option<&str> {
        let start = text.find("## 第 1 部分")?;
        let after_start = &text[start..];
        let end = after_start.find("## 第 2 部分")?;
        Some(&after_start[..end])
    }

    let before_part1 = extract_part1(before);
    let after_part1 = extract_part1(after);

    // 第 2 层：第 1 部分比对
    match (before_part1, after_part1) {
        (Some(b), Some(a)) => {
            // 标准化：统一换行、去除首尾空白
            let norm_b: String = b
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            let norm_a: String = a
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n");

            if norm_b != norm_a {
                // 构造差异描述
                let diff_desc = if norm_b.len().abs_diff(norm_a.len()) > 100 {
                    format!(
                        "第 1 部分长度变化：{} → {} 字符",
                        norm_b.len(),
                        norm_a.len()
                    )
                } else {
                    // 找第一个不同的字符位置
                    let mut diff_pos: usize = 0;
                    for (cb, ca) in norm_b.chars().zip(norm_a.chars()) {
                        if cb != ca {
                            break;
                        }
                        diff_pos += 1;
                    }
                    let ctx_start = diff_pos.saturating_sub(30);
                    format!(
                        "第 1 部分在偏移 {} 处出现差异：...{}...",
                        diff_pos,
                        &norm_a[ctx_start..norm_a.len().min(ctx_start + 200)]
                    )
                };
                return ValidationResult::Part1Modified(diff_desc);
            }
        }
        (Some(_), None) => {
            return ValidationResult::Part1Modified("AI 返回中缺少第 1 部分".to_string());
        }
        (None, Some(_)) => {
            // 之前没有第 1 部分（首次）——放行，由调用方处理
        }
        (None, None) => {
            // 都没有第 1 部分——放行
        }
    }

    // 第 3 层：第 2 部分结构检查
    match after.find("## 第 2 部分") {
        Some(pos) => {
            let part2 = &after[pos..];
            // 检查是否至少有一个 ### 子标题
            if !part2.contains("###") {
                return ValidationResult::StructureDamaged(
                    "第 2 部分缺少子标题（###）".to_string(),
                );
            }
        }
        None => {
            return ValidationResult::StructureDamaged("缺少第 2 部分标记".to_string());
        }
    }

    ValidationResult::Passed
}

/// 兜底机械更新：不调用 AI，直接将 DiffSummary 的信息追加到宪法第 2 部分
///
/// 在 AI 连续失败时的降级方案。在「变更历史」段落标注 [机械更新]。
fn mechanical_update_constitution(
    current_constitution: &str,
    diff: &project::DiffSummary,
) -> Result<String, String> {
    let mut result = current_constitution.to_string();
    let timestamp = chrono::Utc::now()
        .format("%Y-%m-%d %H:%M:%S UTC")
        .to_string();

    // 确保第 2 部分存在
    if !result.contains("## 第 2 部分") {
        result.push_str("\n\n## 第 2 部分：项目当前状态\n");
    }

    // 确保三个子段落存在
    let ensure_section = |text: &mut String, section_title: &str| {
        if !text.contains(section_title) {
            // 在 "## 第 2 部分" 之后插入
            if let Some(pos) = text.find("## 第 2 部分") {
                let insert_pos = text[pos..]
                    .find('\n')
                    .map(|n| pos + n + 1)
                    .unwrap_or(text.len());
                text.insert_str(insert_pos, &format!("\n{}\n", section_title));
            }
        }
    };
    ensure_section(&mut result, "### 项目结构");
    ensure_section(&mut result, "### 函数/接口定义");
    ensure_section(&mut result, "### 变更历史");

    // 处理新增文件
    for f in &diff.new_files {
        let entry = format!("\n- [新增] {}", f);
        if !result.contains(&entry) {
            if let Some(section_pos) = result.find("### 项目结构") {
                // 在项目结构段落末尾插入
                let next_section = result[section_pos..].find("\n###").map(|p| section_pos + p);
                match next_section {
                    Some(ins_pos) => result.insert_str(ins_pos, &entry),
                    None => result.push_str(&entry),
                }
            }
        }
    }

    // 处理删除文件
    for f in &diff.deleted_files {
        let search = format!("- [新增] {}", f);
        let replace_with = format!("- [已删除] {}", f);
        if result.contains(&search) {
            result = result.replace(&search, &replace_with);
        } else {
            let entry = format!("\n- [已删除] {}", f);
            if !result.contains(&entry) {
                if let Some(section_pos) = result.find("### 项目结构") {
                    let next_section = result[section_pos..].find("\n###").map(|p| section_pos + p);
                    match next_section {
                        Some(ins_pos) => result.insert_str(ins_pos, &entry),
                        None => result.push_str(&entry),
                    }
                }
            }
        }
    }

    // 处理修改文件
    for f in &diff.modified_files {
        let search_new = format!("- [新增] {}", f);
        let search_del = format!("- [已删除] {}", f);
        let _marker = format!("- {}（已修改）", f);
        if !result.contains("（已修改）") && !result.contains(f) {
            // 找到该文件的条目，追加修改标记
            let file_entry = format!("- {}", f);
            if let Some(entry_pos) = result.find(&file_entry) {
                let line_end = result[entry_pos..]
                    .find('\n')
                    .unwrap_or(result[entry_pos..].len());
                let existing = &result[entry_pos..entry_pos + line_end];
                if !existing.contains("（已修改）") {
                    let new_entry = format!("- {}（已修改）", f);
                    result.replace_range(entry_pos..entry_pos + line_end, &new_entry);
                }
            } else {
                let entry = format!("\n- {}（已修改）", f);
                result.push_str(&entry);
            }
        }
        // 移除可能触发的 unused warning
        let _ = search_new;
        let _ = search_del;
    }

    // 处理新增函数
    for func in &diff.new_functions {
        let entry = format!("\n- [新增] {}", func);
        if !result.contains(&entry) {
            if let Some(section_pos) = result.find("### 函数/接口定义") {
                let next_section = result[section_pos..].find("\n###").map(|p| section_pos + p);
                match next_section {
                    Some(ins_pos) => result.insert_str(ins_pos, &entry),
                    None => result.push_str(&entry),
                }
            }
        }
    }

    // 处理删除函数
    for func in &diff.deleted_functions {
        let search = format!("- [新增] {}", func);
        let replace_with = format!("- [已删除] {}", func);
        if result.contains(&search) {
            result = result.replace(&search, &replace_with);
        } else {
            let entry = format!("\n- [已删除] {}", func);
            if !result.contains(&entry) {
                if let Some(section_pos) = result.find("### 函数/接口定义") {
                    let next_section = result[section_pos..].find("\n###").map(|p| section_pos + p);
                    match next_section {
                        Some(ins_pos) => result.insert_str(ins_pos, &entry),
                        None => result.push_str(&entry),
                    }
                }
            }
        }
    }

    // 追加变更历史条目
    let history_entry = format!(
        "\n- [机械更新] 小阶段自动更新，AI 更新失败后降级处理 — {}",
        timestamp
    );
    if let Some(section_pos) = result.find("### 变更历史") {
        let next_section = result[section_pos..].find("\n###").map(|p| section_pos + p);
        match next_section {
            Some(ins_pos) => result.insert_str(ins_pos, &history_entry),
            None => result.push_str(&history_entry),
        }
    } else {
        result.push_str(&history_entry);
    }

    Ok(result)
}

/// 宪法更新主函数
///
/// 接收当前宪法全文和变更摘要，调用 AI 更新第 2 部分。
/// 流程：检查 → AI 调用 → 校验 → 重试 → 兜底
#[tauri::command]
async fn update_constitution(
    constitution_content: String,
    diff_summary: project::DiffSummary,
) -> Result<String, String> {
    // 第一步：所有字段为空 → 跳过 AI 调用
    if diff_summary.new_files.is_empty()
        && diff_summary.modified_files.is_empty()
        && diff_summary.deleted_files.is_empty()
        && diff_summary.new_functions.is_empty()
        && diff_summary.modified_functions.is_empty()
        && diff_summary.deleted_functions.is_empty()
        && diff_summary.changed_dependencies.is_empty()
    {
        return Ok(constitution_content);
    }

    // 第二步：构造 user message
    let mut change_desc = String::new();
    if !diff_summary.new_files.is_empty() {
        change_desc.push_str("### 新增文件\n");
        for f in &diff_summary.new_files {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.modified_files.is_empty() {
        change_desc.push_str("### 修改文件\n");
        for f in &diff_summary.modified_files {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.deleted_files.is_empty() {
        change_desc.push_str("### 删除文件\n");
        for f in &diff_summary.deleted_files {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.new_functions.is_empty() {
        change_desc.push_str("### 新增函数\n");
        for f in &diff_summary.new_functions {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.modified_functions.is_empty() {
        change_desc.push_str("### 修改函数\n");
        for f in &diff_summary.modified_functions {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.deleted_functions.is_empty() {
        change_desc.push_str("### 删除函数\n");
        for f in &diff_summary.deleted_functions {
            change_desc.push_str(&format!("- {}\n", f));
        }
    }
    if !diff_summary.changed_dependencies.is_empty() {
        change_desc.push_str("### 依赖变更\n");
        for d in &diff_summary.changed_dependencies {
            change_desc.push_str(&format!("- {}\n", d));
        }
    }

    let user_message = format!(
        "【当前宪法】\n{}\n\n【本次变更】\n{}\n\n严格约束：你只能修改第 2 部分。第 1 部分一个字都不要动。",
        constitution_content, change_desc
    );

    // 第三步：调用 AI（Flash 模型，低 temperature，纯文本模式）
    let ai_result = match call_deepseek_api_inner(
        CONSTITUTION_UPDATE_PROMPT,
        &user_message,
        false,
        0.1,
    )
    .await
    {
        Ok(reply) => reply,
        Err(e) => {
            // AI 调用失败 → 直接兜底
            eprintln!("[constitution] AI 调用失败，降级为机械更新：{}", e);
            return mechanical_update_constitution(&constitution_content, &diff_summary);
        }
    };

    // 第四步：校验
    let validation = validate_constitution_update(&constitution_content, &ai_result);
    match validation {
        ValidationResult::Passed => {
            eprintln!("[constitution] 宪法更新成功");
            return Ok(ai_result);
        }
        ref result @ _ => {
            let err_desc = match result {
                ValidationResult::Part1Modified(desc) => desc.clone(),
                ValidationResult::StructureDamaged(desc) => desc.clone(),
                ValidationResult::Empty(desc) => desc.clone(),
                ValidationResult::Passed => unreachable!(),
            };
            eprintln!("[constitution] 第一次校验不通过：{}，进入重试", err_desc);

            // 第五步：重试
            let retry_message = format!(
                "{}\n\n你上一次更新宪法时出现了以下错误：{}\n请修正后重新输出。务必严格遵守约束：只修改第 2 部分。",
                user_message, err_desc
            );

            match call_deepseek_api_inner(CONSTITUTION_UPDATE_PROMPT, &retry_message, false, 0.1)
                .await
            {
                Ok(retry_reply) => {
                    let validation2 =
                        validate_constitution_update(&constitution_content, &retry_reply);
                    match validation2 {
                        ValidationResult::Passed => {
                            eprintln!("[constitution] 宪法更新成功（重试后）");
                            return Ok(retry_reply);
                        }
                        ref result2 @ _ => {
                            let err_desc2 = match result2 {
                                ValidationResult::Part1Modified(desc) => desc.clone(),
                                ValidationResult::StructureDamaged(desc) => desc.clone(),
                                ValidationResult::Empty(desc) => desc.clone(),
                                ValidationResult::Passed => unreachable!(),
                            };
                            eprintln!(
                                "[constitution] 宪法更新降级（机械更新），原因：{}",
                                err_desc2
                            );
                            return mechanical_update_constitution(
                                &constitution_content,
                                &diff_summary,
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[constitution] AI 调用失败，降级为机械更新：{}", e);
                    return mechanical_update_constitution(&constitution_content, &diff_summary);
                }
            }
        }
    }
}

/// 估算文本的 token 数量
///
/// 中文字符按 1.0 token，ASCII 可打印字符按 0.25 token，其他按 0.5 token。
/// 纯计算函数，无 I/O。
fn estimate_tokens(text: &str) -> f64 {
    let mut tokens = 0.0;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c.is_ascii_punctuation() || c == ' ' || c == '\n' {
            tokens += 0.25;
        } else if matches!(c,
            '\u{4e00}'..='\u{9fff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{f900}'..='\u{faff}'
            | '\u{20000}'..='\u{2a6df}'
            | '\u{2a700}'..='\u{2b73f}'
            | '\u{2b740}'..='\u{2b81f}'
            | '\u{2b820}'..='\u{2ceaf}'
            | '\u{3000}'..='\u{303f}'
        ) {
            tokens += 1.0;
        } else {
            tokens += 0.5;
        }
    }
    tokens
}

/// 宪法剪枝主函数
///
/// 接收当前宪法全文，当第 2 部分超过阈值时调用 AI 压缩。
/// 流程：阈值检查 → AI 调用 → 校验 → 重试 → 返回
/// 与 update_constitution 不同：剪枝失败不降级为机械更新，而是保留膨胀版本。
const COMPACTION_TRIGGER_TOKENS: f64 = 3000.0;

#[tauri::command]
async fn compact_constitution(constitution_content: String) -> Result<String, String> {
    // 第一步：提取第 2 部分
    let part2_start = match constitution_content.find("## 第 2 部分") {
        Some(pos) => pos,
        None => {
            eprintln!("[constitution] 宪法中缺少第 2 部分，跳过剪枝");
            return Ok(constitution_content);
        }
    };
    let part2 = &constitution_content[part2_start..];

    // 第二步：阈值检查（基于 token 估算）
    let estimated_tokens = estimate_tokens(part2);
    if estimated_tokens < COMPACTION_TRIGGER_TOKENS {
        eprintln!(
            "[constitution] 宪法第 2 部分未超过阈值（估算 {:.0} < {:.0} token），跳过剪枝",
            estimated_tokens, COMPACTION_TRIGGER_TOKENS
        );
        return Ok(constitution_content);
    }

    // 第三步：构造 AI 调用消息
    let user_message = format!(
        "【当前宪法】\n{}\n\n【压缩指令】\n\
        压缩第 2 部分，操作规则：\n\
        1. 保留最新的项目结构（文件树）\n\
        2. 保留所有仍然有效的函数/接口定义（删除已被后续覆盖的过时条目）\n\
        3. 如果旧函数名已被新函数替代，只保留最新的函数定义\n\
        4. 变更历史：保留最近 5 条完整记录，更早的合并为一行概述\n\
        5. 保持 Markdown 结构和标题层级不变\n\
        6. 压缩后第 2 部分的目标：约 1500 token\n\
        7. 直接输出完整的 CONSTITUTION.md 文件内容\n\
        \n严格约束：你只能修改第 2 部分。第 1 部分一个字都不要动。",
        constitution_content
    );

    // 第四步：调用 AI
    let ai_result =
        match call_deepseek_api_inner(COMPACT_CONSTITUTION_PROMPT, &user_message, false, 0.1).await
        {
            Ok(reply) => reply,
            Err(e) => {
                eprintln!("[constitution] 宪法剪枝 AI 调用失败：{}，保留膨胀版本", e);
                return Err(format!("AI 调用失败：{}", e));
            }
        };

    // 第五步：校验
    let validation = validate_constitution_update(&constitution_content, &ai_result);
    match validation {
        ValidationResult::Passed => {
            eprintln!("[constitution] 宪法剪枝成功");
            return Ok(ai_result);
        }
        ref result @ _ => {
            let err_desc = match result {
                ValidationResult::Part1Modified(desc) => desc.clone(),
                ValidationResult::StructureDamaged(desc) => desc.clone(),
                ValidationResult::Empty(desc) => desc.clone(),
                ValidationResult::Passed => unreachable!(),
            };
            eprintln!(
                "[constitution] 宪法剪枝第一次校验不通过：{}，进入重试",
                err_desc
            );

            // 第六步：重试（仅 1 次）
            let retry_message = format!(
                "{}\n\n你上一次剪枝宪法时出现了以下错误：{}\n请修正后重新输出。务必严格遵守约束：只修改第 2 部分。",
                user_message, err_desc
            );

            match call_deepseek_api_inner(COMPACT_CONSTITUTION_PROMPT, &retry_message, false, 0.1)
                .await
            {
                Ok(retry_reply) => {
                    let validation2 =
                        validate_constitution_update(&constitution_content, &retry_reply);
                    match validation2 {
                        ValidationResult::Passed => {
                            eprintln!("[constitution] 宪法剪枝成功（重试后）");
                            return Ok(retry_reply);
                        }
                        ref result2 @ _ => {
                            let err_desc2 = match result2 {
                                ValidationResult::Part1Modified(desc) => desc.clone(),
                                ValidationResult::StructureDamaged(desc) => desc.clone(),
                                ValidationResult::Empty(desc) => desc.clone(),
                                ValidationResult::Passed => unreachable!(),
                            };
                            eprintln!("[constitution] 宪法剪枝失败，保留膨胀版本：{}", err_desc2);
                            return Err(format!("剪枝校验两次不通过：{}", err_desc2));
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[constitution] 宪法剪枝重试 AI 调用失败：{}，保留膨胀版本",
                        e
                    );
                    return Err(format!("重试 AI 调用失败：{}", e));
                }
            }
        }
    }
}

/// 读取项目目录下的 CONSTITUTION.md 文件，返回完整内容。
/// 文件不存在或为空时返回友好提示（Ok），而非报错。
#[tauri::command]
async fn read_constitution(project_path: String) -> Result<String, String> {
    use std::fs;
    use std::path::Path;

    let file_path = Path::new(&project_path).join("CONSTITUTION.md");

    match fs::read(&file_path) {
        Ok(bytes) => {
            if bytes.is_empty() {
                return Ok("项目宪法为空。".to_string());
            }
            match String::from_utf8(bytes) {
                Ok(content) => Ok(content),
                Err(_) => Err("项目宪法文件编码异常，无法读取。".to_string()),
            }
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok("项目宪法尚未生成，请先完成一个阶段的任务。".to_string())
            } else {
                Err(format!("项目宪法文件读取失败：{}", e))
            }
        }
    }
}

/// 获取宪法摘要信息
///
/// 从 CONSTITUTION.md 第 2 部分中提取项目状态快照，包括：
/// - 项目结构简述
/// - 公开函数数量
/// - 最近变更列表（最多 5 条）
/// - 第 2 部分的 token 估算值
/// 宪法不存在或缺少第 2 部分时返回空字段结构体，不报错。
#[tauri::command]
async fn get_constitution_summary(
    project_path: String,
) -> Result<project::ConstitutionSummary, String> {
    use std::fs;
    use std::path::Path;

    let empty_summary = project::ConstitutionSummary {
        structure_description: String::new(),
        function_count: 0,
        recent_changes: vec![],
        total_tokens: 0.0,
    };

    // 读取 CONSTITUTION.md
    let file_path = Path::new(&project_path).join("CONSTITUTION.md");
    let content = match fs::read_to_string(&file_path) {
        Ok(c) => c,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                return Ok(empty_summary);
            }
            eprintln!("[get_constitution_summary] 读取宪法文件失败: {}", e);
            return Ok(empty_summary);
        }
    };

    if content.trim().is_empty() {
        return Ok(empty_summary);
    }

    // 定位第 2 部分
    let part2_start = match content.find("## 第 2 部分") {
        Some(pos) => pos,
        None => {
            eprintln!("[get_constitution_summary] 宪法中缺少第 2 部分");
            return Ok(empty_summary);
        }
    };
    let part2 = &content[part2_start..];

    // 辅助函数：提取子标题之间的文本内容
    fn extract_section(text: &str, heading: &str, next_headings: &[&str]) -> String {
        let start = match text.find(heading) {
            Some(pos) => pos + heading.len(),
            None => return String::new(),
        };
        let section = &text[start..];

        // 找到下一个最近的标题（### 或 ##）
        let mut end = section.len();
        for h in next_headings {
            if let Some(pos) = section.find(h) {
                if pos < end {
                    end = pos;
                }
            }
        }
        // 也查找任何 ### 标题
        if let Some(pos) = section.find("\n### ") {
            if pos < end {
                end = pos;
            }
        }
        section[..end].trim().to_string()
    }

    // 提取 structure_description：第 2 部分中的第一个 ### 子标题内容
    // 蓝图要求从 "### 项目结构" 提取
    let structure_description = extract_section(
        part2,
        "### 项目结构",
        &["### 函数/接口定义", "### 变更历史"],
    );

    // 解析 function_count：从 "### 函数/接口定义" 统计含 ( 的行
    let func_section = extract_section(
        part2,
        "### 函数/接口定义",
        &["### 变更历史", "### 项目结构"],
    );
    let function_count: u32 = func_section
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            (trimmed.starts_with("- [新增]")
                || trimmed.starts_with("- fn ")
                || trimmed.starts_with("- function ")
                || trimmed.starts_with("- pub "))
                && trimmed.contains('(')
        })
        .count() as u32;

    // 解析 recent_changes：从 "### 变更历史" 提取以 "- " 开头的行，最多 5 条
    let changes_section = extract_section(
        part2,
        "### 变更历史",
        &["### 项目结构", "### 函数/接口定义"],
    );
    let recent_changes: Vec<String> = changes_section
        .lines()
        .filter(|line| line.trim().starts_with("- "))
        .take(5)
        .map(|line| line.trim().trim_start_matches("- ").to_string())
        .collect();

    // 计算 total_tokens：对第 2 部分全文调用 estimate_tokens
    let total_tokens = estimate_tokens(part2);

    Ok(project::ConstitutionSummary {
        structure_description,
        function_count,
        recent_changes,
        total_tokens,
    })
}

/// 3.3 执行状态结构体
// ========== Phase 3 新增：执行状态结构体 ==========

/// 宪法更新校验结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationResult {
    /// 校验通过
    Passed,
    /// 第 1 部分被修改，携带差异描述
    Part1Modified(String),
    /// 第 2 部分结构损坏，携带错误描述
    StructureDamaged(String),
    /// 返回内容为空或过短，携带原因描述
    Empty(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PipelineStatus {
    Idle,
    Running,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskStatusItem {
    pub subtask_id: String,
    pub title: String,
    pub status: String,
    pub test_result: Option<project::TestResult>,
    pub retry_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineState {
    pub mid_stage_id: String,
    pub status: PipelineStatus,
    pub current_subtask_index: usize,
    pub total_subtasks: usize,
    pub subtask_statuses: Vec<SubtaskStatusItem>,
    pub current_log: String,
    pub last_error: Option<String>,
}

pub struct AppState {
    pub pipeline_state: Arc<Mutex<Option<PipelineState>>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_env();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            pipeline_state: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            send_message,
            get_project,
            chat_with_role,
            generate_version_plan,
            approve_version_plan,
            persist_project,
            generate_milestones,
            regenerate_milestones_with_feedback,
            generate_mid_stages,
            execute_subtask,
            check_subtask,
            generate_next_prompt,
            start_execution,
            get_execution_status,
            pause_execution,
            resume_execution,
            stop_execution,
            approve_mid_stage,
            reject_mid_stage,
            git_save_node,
            git_save_subtask,
            git_rollback_to_mid_stage,
            git_rollback_to_subtask,
            update_constitution,
            compact_constitution,
            read_constitution,
            get_git_tags_summary,
            get_current_diff,
            validate_project_path,
            get_project_files,
            get_constitution_summary
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
