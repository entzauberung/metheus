use crate::api::call_deepseek_api_inner;
use crate::constants::SANITIZE_FALLBACK_JSON;

/// 清洗 AI 返回的文本，提取出纯净的 JSON 字符串
/// 处理三种干扰：
///   1. Markdown 代码块包裹（```json ... ```）
///   2. 礼貌前缀（"好的，以下是JSON："）
///   3. 末尾多余文字
pub(crate) fn sanitize_json_response(raw: &str) -> String {
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
    // 第二层: 找到第一个 { 或 [（取最早出现的位置）
    let brace_pos = text.find('{');
    let bracket_pos = text.find('[');
    let start = match (brace_pos, bracket_pos) {
        (Some(b), Some(sq)) => b.min(sq),
        (Some(b), None) => b,
        (None, Some(sq)) => sq,
        (None, None) => 0,
    };
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
pub(crate) async fn parse_json_with_retry<T: serde::de::DeserializeOwned>(
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
                        "[parse_json_with_retry] 第 3 次解析仍然失败：{}。\
                         AI 修正后内容（前200字符）：{}；原始响应（前200字符）：{}",
                        final_err, preview, original_preview
                    );
                    Err(format!(
                        "JSON 解析失败（3 次重试均失败）：{}。AI 修正后内容：{}...",
                        final_err, preview
                    ))
                }
            }
        }
        Err(e) => {
            let original_preview: String = response_text.chars().take(200).collect();
            eprintln!(
                "[parse_json_with_retry] AI 修正请求失败（第 3 次）：{}。原始响应（前200字符）：{}",
                e, original_preview
            );
            Err(format!("AI 修正请求在 3 次重试后仍然失败：{}", e))
        }
    }
}
