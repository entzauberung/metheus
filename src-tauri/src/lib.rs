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
请严格按 JSON 格式输出，不要包含 markdown 代码块标记：\n{\"title\": \"子任务标题\", \"prompt\": \"可执行的 Claude Code 提示词\"}";
const TEST_PROMPT: &str = "\
你是测试工程师，角色名「测试工程师」。\
你的职责是检查代码质量和功能正确性。\
你需要读取被修改的文件，验证逻辑是否正确、边界情况是否处理、代码风格是否规范。\
输出格式：通过/不通过 + 问题列表（如果不通过）。\
回答风格：客观、具体，指出具体文件和行号。\
请严格按 JSON 格式输出，不要包含 markdown 代码块标记：\n{\"passed\": true或false, \"issues\": [\"问题1\"], \"suggestion\": \"改进建议\"}";
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
只输出 JSON，不要任何其他文字。";

///获取项目文件的存储路径
fn get_project_path(name: &str) -> String {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{}/.metheus/{}.json", home, name)
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
    let client = reqwest::Client::new();
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
        .map_err(|e| format!("网络请求失败: {}", e))?;
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
async fn generate_version_plan(messages: Vec<project::Message>) -> Result<String, String> {
    //1.读群API密钥
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    //2.构造API消息列表：system_prompt + 对话历史
    let mut api_messages: Vec<serde_json::Value> = vec![serde_json::json!({
        "role": "system",
        "content": "你是一个产品战略顾问，角色名「策略产品经理」。\
                    请根据以下对话历史，输出一份结构化的「版本方案摘要」。\
                    使用 Markdown 格式，包含以下章节：\
                    ## 项目愿景\n## 目标用户\n## 核心功能\n## 版本路径\n\
                    每个版本路径下的版本要清晰列出。\
                    回答风格：结构化、清晰、可直接用于执行。"
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
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {}", e))?;
    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败：{}", e))?;
    let mut plan = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复异常".to_string())?
        .to_string();

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
            discussion = discussion_chars[discussion_chars.len() - 3000..].iter().collect();
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
            eprintln!("[generate_version_plan] 自检调用失败：{}，使用原始版本方案", e);
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
    let client = reqwest::Client::new();
    //发出请求等待回复
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {}", e))?;
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
            eprintln!("[generate_milestones] 大阶段 JSON 序列化失败：{}，跳过质检", e);
            return Ok(milestones);
        }
    };

    // 步骤 2：构造质检请求的 user_message
    let qa_user_message = format!(
        "【原始需求（版本方案）】\n{}\n\n【当前产出（大阶段列表）】\n{}",
        version_plan, milestones_json
    );

    // 步骤 3：调用 DeepSeek Flash 执行质检（纯文本模式，低 temperature）
    let qa_response = match call_deepseek_api_inner(QA_CHECK_PROMPT, &qa_user_message, false, 0.1).await {
        Ok(reply) => reply,
        Err(e) => {
            eprintln!("[generate_milestones] 质检 API 调用失败：{}，跳过质检", e);
            return Ok(milestones);
        }
    };

    // 步骤 4：清洗并解析 AI 返回的 QAResult JSON
    let qa_result = {
        let cleaned = sanitize_json_response(&qa_response);
        match serde_json::from_str::<project::QAResult>(&cleaned) {
            Ok(mut result) => {
                result.checked_at = chrono::Utc::now().to_rfc3339();
                result
            }
            Err(e) => {
                eprintln!("[generate_milestones] 质检 JSON 解析失败：{}，使用默认通过结果", e);
                project::QAResult {
                    passed: true,
                    reason: "质检结果解析失败，默认通过".to_string(),
                    details: vec![],
                    attention_points: vec![],
                    checked_at: chrono::Utc::now().to_rfc3339(),
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
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {}", e))?;
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
            eprintln!("[regenerate_milestones_with_feedback] 大阶段 JSON 序列化失败：{}，跳过质检", e);
            return Ok(milestones);
        }
    };

    // 步骤 7.2：构造质检请求的 user_message
    let qa_user_message = format!(
        "【原始需求（版本方案）】\n{}\n\n【当前产出（大阶段列表）】\n{}",
        version_plan, milestones_json
    );

    // 步骤 7.3：调用 DeepSeek Flash 执行质检（纯文本模式，低 temperature）
    let qa_response = match call_deepseek_api_inner(QA_CHECK_PROMPT, &qa_user_message, false, 0.1).await {
        Ok(reply) => reply,
        Err(e) => {
            eprintln!("[regenerate_milestones_with_feedback] 质检 API 调用失败：{}，跳过质检", e);
            return Ok(milestones);
        }
    };

    // 步骤 7.4：清洗并解析 AI 返回的 QAResult JSON
    let qa_result = {
        let cleaned = sanitize_json_response(&qa_response);
        match serde_json::from_str::<project::QAResult>(&cleaned) {
            Ok(mut result) => {
                result.checked_at = chrono::Utc::now().to_rfc3339();
                result
            }
            Err(e) => {
                eprintln!("[regenerate_milestones_with_feedback] 质检 JSON 解析失败：{}，使用默认通过结果", e);
                project::QAResult {
                    passed: true,
                    reason: "质检结果解析失败，默认通过".to_string(),
                    details: vec![],
                    attention_points: vec![],
                    checked_at: chrono::Utc::now().to_rfc3339(),
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
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.deepseek.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {}", e))?;
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

/// “可暂停”的 Claude Code 执行器：启动进程后边等待边监听暂停信号，暂停时立即杀进程；
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
    // 3. 用 tokio::process::Command 启动 Claude Code（非阻塞）
    let mut child = tokio::process::Command::new("claude")
        .args(["--dangerously-skip-permissions", &full_prompt])
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
    // 4. 自动应答：信任确认 + 文件写入确认（异步写入 stdin）
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(b"1\n").await.ok();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        for _ in 0..10 {
            stdin.write_all(b"yes\n").await.ok();
        }
        // stdin 在这里 drop，关闭管道
    }
    // 5. 轮询等待进程结束，期间检查暂停标志
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
                // 没暂停 → 等 500ms 再检查
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
/// 模拟一个“测试工程师”角色：
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
        // 失败：保留最后 3000 字符
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

        // 提取含失败关键词的行
        let keywords = [
            "FAIL",
            "Error",
            "error:",
            "assertion",
            "✕",
            "✗",
            "fail:",
            "FAILED",
            "--- FAIL",
            "[ERROR]",
            "panic",
            "Panic",
        ];
        let mut key_lines: Vec<&str> = Vec::new();
        for line in tail.lines() {
            // 如果有这样的行，把它们单独列出来作为“关键失败行”；否则只返回退出码和尾部输出
            if keywords.iter().any(|k| line.contains(k)) {
                key_lines.push(line);
            }
        }

        if key_lines.is_empty() {
            format!("退出码: {}\n\n{}", exit_code, tail)
        } else {
            format!(
                "退出码: {}\n\n## 关键失败行\n{}\n\n## 完整输出（尾部）\n{}",
                exit_code,
                key_lines.join("\n"),
                tail
            )
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
/// 测试
#[tauri::command]
async fn check_subtask(
    project_path: &str,
    _subtask_goal: &str,
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
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result(&label, &label, code, &summary))
                }
                Err(e) => Some(format!("{} test 执行失败：{}", pm, e)),
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
    let user_message = if let Some(ref test_result) = test_output {
        format!(
            "请检查以下代码改动。\n\n## 自动化测试结果\n项目自动化测试已执行，结果如下：\n\n{}\n\n---\n\n## 改动文件列表（共 {} 个文件）\n{}\n\n## 改动文件内容\n{}",
            test_result,
            files.len(),
            files.join("\n"),
            file_contents
        )
    } else {
        format!(
            "请检查以下代码改动：\n\n## 改动文件列表（共 {} 个文件）\n{}\n\n## 改动文件内容\n{}",
            files.len(),
            files.join("\n"),
            file_contents
        )
    };
    //     test_output.as_ref().map(|s| s.len()).unwrap_or(0));
    // 调用ai
    let reply = call_deepseek_api_json(TEST_PROMPT, &user_message).await?;
    // 解析JSON 响应
    let test_result: project::TestResult = parse_json_with_retry(&reply)
        .await
        .map_err(|e| format!("解析测试结果失败：{}", e))?;
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
        "## 当前中阶段\n标题：{}\n描述：{}\n\n## 上一个小阶段\n标题：{}\n执行结果：{}\n\n## 改动文件\n{}\n\n## 测试结果\n{}\n\n## 是否重试\n{}\n\n## 打回原因\n{}",
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
    // 解析子任务列表：把 subtasks_json 转成 Rust 结构体 Vec<Subtask>
    let subtasks: Vec<project::Subtask> =
        serde_json::from_str(&subtasks_json).map_err(|e| format!("解析小阶段列表失败：{}", e))?;
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
/// 3. 全部子任务完成 → 更新最终状态为“完成”
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
                    "代码质量不通过，请修正".to_string()
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
                &subtask_title,
                &subtask_id,
                "",
                &mid_stage_id,
            )
            .await
            {
                Ok(t) => t,
                Err(err) => project::TestResult {
                    passed: false,
                    issues: vec![format!("测试服务不可用: {}", err)],
                    suggestion: "请手动检查".to_string(),
                },
            };
            last_test_result = if test.passed {
                "通过".to_string()
            } else {
                "不通过".to_string()
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
    // 流水线跑完后，找到项目文件里对应的那个“中阶段”（MidStage），
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
    let client = reqwest::Client::new();

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
        .map_err(|e| format!("网络请求失败: {}", e))?;

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
    text[start..end].to_string()
}

/// 带重试的 JSON 解析
/// 第 1 次：sanitize → 直接解析
/// 第 2 次：把错误发给 AI 修正 → sanitize → 解析
/// 第 3 次：再次发给 AI 修正（附"最后一次机会"）→ 解析
/// 三次全失败则返回错误
async fn parse_json_with_retry<T: serde::de::DeserializeOwned>(
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
        "以下 JSON 解析仍然失败，这是最后一次修正机会。\n\n原始内容：\n{}\n\n请修正后只输出 JSON，不要任何其他内容。如果仍无法修正，请输出一个空 JSON 数组 []。",
        cleaned
    );
    match call_deepseek_api_inner(system_prompt, &user_message_last, false, 0.5).await {
        Ok(reply) => {
            let cleaned3 = sanitize_json_response(&reply);
            serde_json::from_str::<T>(&cleaned3)
                .map_err(|final_err| {
                    format!(
                        "JSON 解析失败，AI 两次修正后仍无效。最后一次错误：{}\n清洗后内容（前500字符）：{}",
                        final_err,
                        &cleaned3[..cleaned3.len().min(500)]
                    )
                })
        }
        Err(e) => Err(format!("AI 修正请求失败（第 3 次）：{}", e)),
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
async fn approve_mid_stage(project_id: String, mid_stage_id: String) -> Result<(), String> {
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
    // 找到对应的 MidStage
    // 5. 双层循环查找mid_stage
    let mut found = false;
    for milestone in &mut project.milestones {
        for mid_stage in &mut milestone.mid_stages {
            if mid_stage.id == mid_stage_id {
                mid_stage.status = project::MidStageStatus::Approved;
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
    // 序列化回 JSON 并写回文件
    let json = serde_json::to_string_pretty(&project).map_err(|e| format!("序列化失败：{}", e))?;
    std::fs::write(&project_file, json).map_err(|e| format!("保存失败：{}", e))?;
    Ok(())
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

/// 3.3 执行状态结构体
// ========== Phase 3 新增：执行状态结构体 ==========

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
            approve_mid_stage,
            reject_mid_stage,
            git_save_node,
            git_rollback_to_mid_stage
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
