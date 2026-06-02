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
你只输出 JSON 数组，不要包含 markdown 代码块标记。";
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
    let plan = response_data["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("AI回复异常".to_string())?
        .to_string();
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
    //（上面的文字）JSON数组转化为Rust数组
    let raw_milestones: Vec<serde_json::Value> =
        serde_json::from_str(&content).map_err(|e| format!("解析JSON 失败：{}", e))?;
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
            //创建空的可变的字符串
            git_commit_hash: "".to_string(),
        });
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
) -> Result<Vec<project::MidStage>, String> {
    // 1. 读取 API 密钥
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    // 2. 构造 system prompt
    let system_prompt = format!(
        "{}\n\n当前项目模式：{}。请根据版本方案，将大阶段拆解为 3-6 个中阶段。\
         每个中阶段是一个垂直切片。",
        DOMAIN_LEAD_PROMPT, mode
    );
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
    // 5. 解析 JSON
    let raw_mid_stages: Vec<serde_json::Value> =
        serde_json::from_str(&content).map_err(|e| format!("解析JSON失败: {}", e))?;
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
        });
    }
    Ok(mid_stages)
}

///根据传入的提示词，在项目文件夹创建一个“执行标记”文件，证明这个子任务被调用过
#[tauri::command]
async fn execute_subtask(
    project_path: String,
    prompt: String,
    subtask_id: String,
    _milestone_id: String,
    _mid_stage_id: String,
) -> Result<project::ExecutionResult, String> {
    // Mock (假装执行) 版本：在 project_path 下创建一个文件证明执行过
    // 构造文件路径
    let mock_path = std::path::Path::new(&project_path).join("metheus_executed.txt");
    // 准备文件内容format!,并写入文件（std::fs）
    std::fs::write(
        &mock_path,
        format!(
            "Prompt: {}\nSubtask: {}\nTime: {}",
            prompt,
            subtask_id,
            //计算时间戳
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs()
        ),
    )
    .map_err(|e| format!("写入失败：{}", e))?;
    // 返回成功结果
    Ok(project::ExecutionResult {
        success: true,
        output: "Mock: 文件已创建".to_string(),
        error_log: String::new(),
        file_changes: vec!["metheus_executed.txt".to_string()],
    })
}

/// 模拟一个“测试工程师”角色：
/// 自动检查当前项目里所有改动的代码，判断是否达到了子任务的目标，并返回测试结果（通过/问题/建议）
#[tauri::command]
async fn check_subtask(
    project_path: String,
    _subtask_id: String,
    subtask_goal: String,
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
            format!(
                "{}...(省略后续 {} 字符)",
                &content[..4000],
                content.len() - 4000
            )
        } else {
            content
        };
        file_contents.push_str(&format!("\n=== {} ===\n{}\n", file, truncated));
    }
    // Mock 版本 -> 3.4.1c改动
    // 构建测试工程师prompt 的 user_message
    let user_message = format!(
        "## 小阶段目标\n{}\n\n## 改动文件列表\n{}\n\n## 文件内容\n{}",
        subtask_goal,
        files.join("\n"),
        file_contents
    );
    // 调用ai
    let reply = call_deepseek_api_json(TEST_PROMPT, &user_message).await?;
    // 解析JSON 响应
    let test_result: project::TestResult = serde_json::from_str(&reply)
        .map_err(|e| format!("解析测试结果失败：{}, 原始响应：{}", e, reply))?;
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
    let generated: project::GeneratedSubtask = serde_json::from_str(&reply)
        .map_err(|e| format!("解析生成结果失败: {}，原始响应: {}", e, reply))?;
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
            // 执行子任务
            // 调用 execute_subtask，返回文件变更等
            let exec_result = execute_subtask(
                project_path.clone(),
                generated.prompt.clone(),
                subtask_id.clone(),
                String::new(),
                String::new(),
            )
            .await?;
            file_changes = exec_result.file_changes.clone();
            subtasks[i].execution_result = Some(exec_result);
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
                project_path.clone(),
                subtask_id.clone(),
                subtask_title.clone(),
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
    Ok(())
}

/// 通用的 DeepSeek 调用函数，
/// 给它系统提示词和用户消息，它返回 AI 生成的文本（JSON 格式），并处理所有网络和解析错误。
// ===== 纯文本对话用（不强制 JSON） =====
async fn call_deepseek_api(system_prompt: &str, user_message: &str) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, false).await
}

// ===== 结构化输出用（强制 JSON） =====
async fn call_deepseek_api_json(system_prompt: &str, user_message: &str) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, true).await
}

// ===== 内部实现 =====
async fn call_deepseek_api_inner(
    system_prompt: &str,
    user_message: &str,
    force_json: bool,
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
        "temperature": 0.5,
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
    let json = serde_json::to_string(&project).map_err(|e| format!("序列化失败：{}", e))?;
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
            generate_mid_stages,
            execute_subtask,
            check_subtask,
            generate_next_prompt,
            start_execution,
            get_execution_status,
            pause_execution,
            resume_execution,
            approve_mid_stage,
            reject_mid_stage
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
