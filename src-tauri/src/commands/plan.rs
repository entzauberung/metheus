use std::env;
use crate::project;


#[tauri::command]
pub(crate) async fn generate_version_plan(
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
            crate::prompts::CONSTITUTION_PART1_PROMPT
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
    match crate::api::call_deepseek_api(crate::prompts::SELF_CHECK_PROMPT, &self_check_user_message).await {
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
pub(crate) async fn approve_version_plan(
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
    crate::save_project(&project)?;
    Ok("批准成功".to_string())
}
