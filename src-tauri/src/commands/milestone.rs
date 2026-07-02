use std::env;
use crate::project;

#[tauri::command]
pub(crate) async fn generate_milestones(
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
        crate::prompts::PM_PROMPT, mode
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
        .timeout(std::time::Duration::from_secs(crate::constants::DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!(
                "[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端",
                e
            );
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
                    crate::constants::DEEPSEEK_API_TIMEOUT_SECS
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
    let raw_milestones: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
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
        match crate::api::call_deepseek_api_inner(crate::prompts::QA_CHECK_PROMPT, &qa_user_message, false, 0.1).await {
            Ok(reply) => reply,
            Err(e) => {
                eprintln!("[generate_milestones] 质检 API 调用失败：{}，跳过质检", e);
                return Ok(milestones);
            }
        };

    // 步骤 4：清洗并解析 AI 返回的 QAResult JSON
    let qa_result = {
        let cleaned = crate::json_utils::sanitize_json_response(&qa_response);
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
                        reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案"
                            .to_string(),
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
pub(crate) async fn regenerate_milestones_with_feedback(
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
        crate::prompts::PM_PROMPT, mode
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
        .timeout(std::time::Duration::from_secs(crate::constants::DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!(
                "[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端",
                e
            );
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
                    crate::constants::DEEPSEEK_API_TIMEOUT_SECS
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
    let raw_milestones: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
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
        match crate::api::call_deepseek_api_inner(crate::prompts::QA_CHECK_PROMPT, &qa_user_message, false, 0.1).await {
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
        let cleaned = crate::json_utils::sanitize_json_response(&qa_response);
        // 兜底：AI 有时返回空数组 [] 而非对象，直接走降级
        if cleaned == "[]" {
            eprintln!(
                "[regenerate_milestones_with_feedback] 质检 AI 返回空数组 []，使用兜底不通过结果"
            );
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
                        reason: "质检结果解析失败，请人工审查大阶段列表是否对齐版本方案"
                            .to_string(),
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
pub(crate) async fn generate_mid_stages(
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
        crate::prompts::DOMAIN_LEAD_PROMPT, mode
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
        .timeout(std::time::Duration::from_secs(crate::constants::DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|e| {
            eprintln!(
                "[metheus] 构造带超时的 HTTP 客户端失败：{}，降级使用无超时客户端",
                e
            );
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
                    crate::constants::DEEPSEEK_API_TIMEOUT_SECS
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
    let raw_mid_stages: Vec<serde_json::Value> = crate::json_utils::parse_json_with_retry(&content)
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




/// 校验项目路径的前端可调用命令
/// 生成下一个子任务的提示词
#[tauri::command]
pub(crate) async fn generate_next_prompt(
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
    let reply = crate::api::call_deepseek_api_json(crate::prompts::TECH_PROMPT, &user_message).await?;
    // 解析 JSON 响应
    let generated: project::GeneratedSubtask = crate::json_utils::parse_json_with_retry(&reply)
        .await
        .map_err(|e| format!("解析生成结果失败：{}", e))?;
    Ok(generated)
}
