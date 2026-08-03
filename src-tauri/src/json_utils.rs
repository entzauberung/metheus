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

const SCHEMA_REPAIR_SYSTEM_PROMPT: &str = "你是确定性的 JSON 协议修复器。只能修复用户提供的 JSON，使其符合给定契约；不得补写代码事实、验收结论或证据编号。只输出一个 JSON 对象，不要 Markdown 或解释。";

pub(crate) fn schema_repair_user_message(
    response_text: &str,
    schema_contract: &str,
    error_path: &str,
    expected: &str,
    actual: &str,
) -> String {
    let cleaned = sanitize_json_response(response_text);
    format!(
        "以下审查 JSON 未通过协议校验。只修复格式和字段类型，不得改变语义或添加原文中不存在的事实。\n\nJSON Schema 契约：\n{schema_contract}\n\n真实校验错误：\n- 字段路径：{error_path}\n- 期望类型或枚举：{expected}\n- 实际类型：{actual}\n\n待修复 JSON：\n{cleaned}\n\n只输出修复后的 JSON 对象。"
    )
}

pub(crate) async fn repair_json_once_with_contract(
    response_text: &str,
    schema_contract: &str,
    error_path: &str,
    expected: &str,
    actual: &str,
) -> Result<String, crate::api::ApiRequestError> {
    repair_json_once_with_contract_and_context(
        response_text,
        schema_contract,
        error_path,
        expected,
        actual,
        crate::cost_ledger::ModelCallContext::default(),
    )
    .await
}

pub(crate) async fn repair_json_once_with_contract_and_context(
    response_text: &str,
    schema_contract: &str,
    error_path: &str,
    expected: &str,
    actual: &str,
    mut context: crate::cost_ledger::ModelCallContext,
) -> Result<String, crate::api::ApiRequestError> {
    let user_message =
        schema_repair_user_message(response_text, schema_contract, error_path, expected, actual);
    context.purpose = Some(crate::cost_ledger::ModelCallPurpose::SchemaRepair);
    let response = crate::api::call_deepseek_api_inner_typed_with_context(
        SCHEMA_REPAIR_SYSTEM_PROMPT,
        &user_message,
        true,
        0.0,
        context.clone(),
    )
    .await?;
    crate::cost_ledger::mark_call_outcome_best_effort(
        &context.project_name,
        &response.metadata.call_id,
        crate::cost_ledger::ModelCallOutcome {
            produced_change: true,
            ..Default::default()
        },
    );
    Ok(response.content)
}

pub(crate) const MAX_CONTRACT_REPAIR_ATTEMPTS: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonExpectedType {
    Object,
    Array,
    String,
    Boolean,
    Number,
    StringArray,
}

impl JsonExpectedType {
    fn label(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Array => "array",
            Self::String => "string",
            Self::Boolean => "boolean",
            Self::Number => "number",
            Self::StringArray => "string array",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonFieldContract {
    pub(crate) path: &'static str,
    pub(crate) expected_type: JsonExpectedType,
    pub(crate) allowed_values: &'static [&'static str],
    pub(crate) required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonTargetContract {
    pub(crate) name: &'static str,
    pub(crate) fields: &'static [JsonFieldContract],
}

impl JsonTargetContract {
    pub(crate) fn describe(&self) -> String {
        let fields = self
            .fields
            .iter()
            .map(|field| {
                let values = if field.allowed_values.is_empty() {
                    String::new()
                } else {
                    format!("；合法枚举：{}", field.allowed_values.join("、"))
                };
                format!(
                    "- {}：{}；{}{}",
                    field.path,
                    field.expected_type.label(),
                    if field.required { "必填" } else { "可选" },
                    values
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("目标结构：{}\n{}", self.name, fields)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JsonParseErrorKind {
    InvalidJson,
    MissingField,
    TypeMismatch,
    InvalidEnum,
    DeserializeFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonParseErrorDetail {
    pub(crate) kind: JsonParseErrorKind,
    pub(crate) path: String,
    pub(crate) expected: String,
    pub(crate) actual: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonRepairAttemptRecord {
    pub(crate) attempt: usize,
    pub(crate) before: JsonParseErrorDetail,
    pub(crate) after: Option<JsonParseErrorDetail>,
    pub(crate) made_progress: bool,
    pub(crate) deterministic_normalization_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonProtocolFailure {
    pub(crate) contract_name: String,
    pub(crate) final_error: JsonParseErrorDetail,
    pub(crate) repair_attempts: Vec<JsonRepairAttemptRecord>,
    pub(crate) no_progress: bool,
}

impl std::fmt::Display for JsonProtocolFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "JSON 协议 {} 校验失败：字段 {} 预期 {}，实际为 {}{}",
            self.contract_name,
            self.final_error.path,
            self.final_error.expected,
            self.final_error.actual,
            if self.no_progress {
                "；单次契约修复无进展，已停止重复请求"
            } else {
                ""
            }
        )
    }
}

const PLAN_PATCH_FIELDS: &[JsonFieldContract] = &[
    JsonFieldContract {
        path: "$",
        expected_type: JsonExpectedType::Object,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.implementation_guidance",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.context_summary",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.evidence_files",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.dependency_notes",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: false,
    },
    JsonFieldContract {
        path: "$.rationale",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: false,
    },
];

pub(crate) const PLAN_PATCH_JSON_CONTRACT: JsonTargetContract = JsonTargetContract {
    name: "PlanPatchOutput",
    fields: PLAN_PATCH_FIELDS,
};

const MILESTONE_CHECK_FIELDS: &[JsonFieldContract] = &[
    JsonFieldContract {
        path: "$",
        expected_type: JsonExpectedType::Object,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.passed",
        expected_type: JsonExpectedType::Boolean,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.summary",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.omissions",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.overlaps",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.out_of_scope",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.ordering_issues",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.suggestions",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
];

pub(crate) const MILESTONE_CHECK_JSON_CONTRACT: JsonTargetContract = JsonTargetContract {
    name: "MilestoneCheckResult",
    fields: MILESTONE_CHECK_FIELDS,
};

const MID_STAGE_CHECK_FIELDS: &[JsonFieldContract] = &[
    JsonFieldContract {
        path: "$",
        expected_type: JsonExpectedType::Object,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.passed",
        expected_type: JsonExpectedType::Boolean,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.summary",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.omissions",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.overlaps",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.ordering_issues",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.suggestions",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
];

pub(crate) const MID_STAGE_CHECK_JSON_CONTRACT: JsonTargetContract = JsonTargetContract {
    name: "MidStageCheckResult",
    fields: MID_STAGE_CHECK_FIELDS,
};

const EXECUTION_PLAN_CHECK_FIELDS: &[JsonFieldContract] = &[
    JsonFieldContract {
        path: "$",
        expected_type: JsonExpectedType::Object,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.passed",
        expected_type: JsonExpectedType::Boolean,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.summary",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.omissions",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.out_of_scope",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.not_executable",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.suggestions",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
];

pub(crate) const EXECUTION_PLAN_CHECK_JSON_CONTRACT: JsonTargetContract = JsonTargetContract {
    name: "ExecutionPlanCheckResult",
    fields: EXECUTION_PLAN_CHECK_FIELDS,
};

const QA_RESULT_FIELDS: &[JsonFieldContract] = &[
    JsonFieldContract {
        path: "$",
        expected_type: JsonExpectedType::Object,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.passed",
        expected_type: JsonExpectedType::Boolean,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.reason",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.details",
        expected_type: JsonExpectedType::Array,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.details[].issue_type",
        expected_type: JsonExpectedType::String,
        allowed_values: &["遗漏", "多余", "偏离", "未知"],
        required: true,
    },
    JsonFieldContract {
        path: "$.details[].description",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.details[].related_requirement",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.attention_points",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.checked_at",
        expected_type: JsonExpectedType::String,
        allowed_values: &[],
        required: true,
    },
    JsonFieldContract {
        path: "$.warnings",
        expected_type: JsonExpectedType::StringArray,
        allowed_values: &[],
        required: false,
    },
];

pub(crate) const QA_RESULT_JSON_CONTRACT: JsonTargetContract = JsonTargetContract {
    name: "QAResult",
    fields: QA_RESULT_FIELDS,
};

fn json_value_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn type_matches(value: &serde_json::Value, expected: JsonExpectedType) -> bool {
    match expected {
        JsonExpectedType::Object => value.is_object(),
        JsonExpectedType::Array => value.is_array(),
        JsonExpectedType::String => value.is_string(),
        JsonExpectedType::Boolean => value.is_boolean(),
        JsonExpectedType::Number => value.is_number(),
        JsonExpectedType::StringArray => value
            .as_array()
            .is_some_and(|items| items.iter().all(serde_json::Value::is_string)),
    }
}

fn validate_contract_value(
    value: &serde_json::Value,
    contract: &JsonTargetContract,
) -> Result<(), JsonParseErrorDetail> {
    for field in contract.fields {
        validate_contract_field(value, field)?;
    }
    Ok(())
}

fn validate_contract_field(
    root: &serde_json::Value,
    field: &JsonFieldContract,
) -> Result<(), JsonParseErrorDetail> {
    if field.path == "$" {
        return validate_contract_leaf(root, field, "$".to_string());
    }
    let segments = field
        .path
        .strip_prefix("$.")
        .unwrap_or(field.path)
        .split('.')
        .collect::<Vec<_>>();
    validate_contract_segments(root, &segments, field, "$".to_string())
}

fn validate_contract_segments(
    current: &serde_json::Value,
    segments: &[&str],
    field: &JsonFieldContract,
    current_path: String,
) -> Result<(), JsonParseErrorDetail> {
    let segment = segments[0];
    let array_items = segment.ends_with("[]");
    let key = segment.trim_end_matches("[]");
    let Some(next) = current.as_object().and_then(|object| object.get(key)) else {
        return if field.required {
            Err(JsonParseErrorDetail {
                kind: JsonParseErrorKind::MissingField,
                path: format!("{current_path}.{key}"),
                expected: field.expected_type.label().to_string(),
                actual: "missing".to_string(),
            })
        } else {
            Ok(())
        };
    };
    let next_path = format!("{current_path}.{key}");
    if array_items {
        let Some(items) = next.as_array() else {
            return Err(JsonParseErrorDetail {
                kind: JsonParseErrorKind::TypeMismatch,
                path: next_path,
                expected: "array".to_string(),
                actual: json_value_kind(next).to_string(),
            });
        };
        for (index, item) in items.iter().enumerate() {
            let item_path = format!("{next_path}[{index}]");
            if segments.len() == 1 {
                validate_contract_leaf(item, field, item_path)?;
            } else {
                validate_contract_segments(item, &segments[1..], field, item_path)?;
            }
        }
        Ok(())
    } else if segments.len() == 1 {
        validate_contract_leaf(next, field, next_path)
    } else {
        validate_contract_segments(next, &segments[1..], field, next_path)
    }
}

fn validate_contract_leaf(
    value: &serde_json::Value,
    field: &JsonFieldContract,
    path: String,
) -> Result<(), JsonParseErrorDetail> {
    if !type_matches(value, field.expected_type) {
        return Err(JsonParseErrorDetail {
            kind: JsonParseErrorKind::TypeMismatch,
            path,
            expected: field.expected_type.label().to_string(),
            actual: json_value_kind(value).to_string(),
        });
    }
    if !field.allowed_values.is_empty() {
        let actual = value.as_str().unwrap_or_default();
        if !field.allowed_values.contains(&actual) {
            return Err(JsonParseErrorDetail {
                kind: JsonParseErrorKind::InvalidEnum,
                path,
                expected: format!("枚举 {}", field.allowed_values.join("、")),
                actual: actual.to_string(),
            });
        }
    }
    Ok(())
}

fn parse_contract_value(
    raw: &str,
    contract: &JsonTargetContract,
) -> Result<serde_json::Value, JsonParseErrorDetail> {
    let cleaned = sanitize_json_response(raw);
    let value = serde_json::from_str::<serde_json::Value>(&cleaned).map_err(|error| {
        JsonParseErrorDetail {
            kind: JsonParseErrorKind::InvalidJson,
            path: "$".to_string(),
            expected: "valid JSON".to_string(),
            actual: format!(
                "syntax error at line {} column {}",
                error.line(),
                error.column()
            ),
        }
    })?;
    validate_contract_value(&value, contract)?;
    Ok(value)
}

fn deserialize_contract_value<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
    contract: &JsonTargetContract,
) -> Result<T, JsonParseErrorDetail> {
    serde_json::from_value(value).map_err(|error| JsonParseErrorDetail {
        kind: JsonParseErrorKind::DeserializeFailure,
        path: "$".to_string(),
        expected: contract.name.to_string(),
        actual: error.to_string(),
    })
}

fn normalize_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn edit_distance(left: &str, right: &str) -> usize {
    let mut previous = (0..=right.chars().count()).collect::<Vec<_>>();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut current = vec![left_index + 1];
        for (right_index, right_char) in right.chars().enumerate() {
            current.push(std::cmp::min(
                std::cmp::min(current[right_index] + 1, previous[right_index + 1] + 1),
                previous[right_index] + usize::from(left_char != right_char),
            ));
        }
        previous = current;
    }
    previous.last().copied().unwrap_or(0)
}

fn normalize_enum(value: &str, allowed: &[&str]) -> Option<String> {
    let normalized = normalize_token(value);
    if let Some(exact) = allowed
        .iter()
        .find(|candidate| normalize_token(candidate) == normalized)
    {
        return Some((*exact).to_string());
    }
    let mut ranked = allowed
        .iter()
        .map(|candidate| {
            (
                edit_distance(&normalized, &normalize_token(candidate)),
                *candidate,
            )
        })
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(distance, _)| *distance);
    if ranked.first().is_some_and(|(distance, _)| *distance <= 2)
        && (ranked.len() == 1 || ranked[0].0 < ranked[1].0)
    {
        return Some(ranked[0].1.to_string());
    }
    allowed
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case("unknown") || **candidate == "未知")
        .map(|candidate| (*candidate).to_string())
}

fn normalize_contract_value(value: &mut serde_json::Value, contract: &JsonTargetContract) -> bool {
    let mut changed = false;
    for field in contract.fields {
        if field.path == "$" {
            continue;
        }
        normalize_contract_field(value, field, &mut changed);
    }
    changed
}

fn normalize_contract_field(
    root: &mut serde_json::Value,
    field: &JsonFieldContract,
    changed: &mut bool,
) {
    let segments = field
        .path
        .strip_prefix("$.")
        .unwrap_or(field.path)
        .split('.')
        .collect::<Vec<_>>();
    normalize_contract_segments(root, &segments, field, changed);
}

fn normalize_contract_segments(
    current: &mut serde_json::Value,
    segments: &[&str],
    field: &JsonFieldContract,
    changed: &mut bool,
) {
    let segment = segments[0];
    let array_items = segment.ends_with("[]");
    let key = segment.trim_end_matches("[]");
    let Some(next) = current
        .as_object_mut()
        .and_then(|object| object.get_mut(key))
    else {
        return;
    };
    if array_items {
        if let Some(items) = next.as_array_mut() {
            for item in items {
                if segments.len() == 1 {
                    normalize_contract_leaf(item, field, changed);
                } else {
                    normalize_contract_segments(item, &segments[1..], field, changed);
                }
            }
        }
    } else if segments.len() == 1 {
        normalize_contract_leaf(next, field, changed);
    } else {
        normalize_contract_segments(next, &segments[1..], field, changed);
    }
}

fn normalize_contract_leaf(
    value: &mut serde_json::Value,
    field: &JsonFieldContract,
    changed: &mut bool,
) {
    match field.expected_type {
        JsonExpectedType::String if !value.is_string() => {
            if let Ok(rendered) = serde_json::to_string(value) {
                *value = serde_json::Value::String(rendered);
                *changed = true;
            }
        }
        JsonExpectedType::StringArray => {
            if value.is_string() {
                *value = serde_json::Value::Array(vec![value.clone()]);
                *changed = true;
            } else if let Some(items) = value.as_array_mut() {
                for item in items {
                    if !item.is_string() {
                        if let Ok(rendered) = serde_json::to_string(item) {
                            *item = serde_json::Value::String(rendered);
                            *changed = true;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    if !field.allowed_values.is_empty() {
        if let Some(actual) = value.as_str() {
            if !field.allowed_values.contains(&actual) {
                if let Some(normalized) = normalize_enum(actual, field.allowed_values) {
                    *value = serde_json::Value::String(normalized);
                    *changed = true;
                }
            }
        }
    }
}

async fn parse_json_with_contract_using<T, F, Fut>(
    response_text: &str,
    contract: &JsonTargetContract,
    repair: F,
) -> Result<T, JsonProtocolFailure>
where
    T: serde::de::DeserializeOwned,
    F: FnOnce(String, String, JsonParseErrorDetail) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let initial_error = match parse_contract_value(response_text, contract)
        .and_then(|value| deserialize_contract_value(value, contract))
    {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let repaired = repair(
        response_text.to_string(),
        contract.describe(),
        initial_error.clone(),
    )
    .await
    .map_err(|actual| JsonProtocolFailure {
        contract_name: contract.name.to_string(),
        final_error: JsonParseErrorDetail {
            kind: JsonParseErrorKind::InvalidJson,
            path: initial_error.path.clone(),
            expected: initial_error.expected.clone(),
            actual,
        },
        repair_attempts: vec![JsonRepairAttemptRecord {
            attempt: 1,
            before: initial_error.clone(),
            after: None,
            made_progress: false,
            deterministic_normalization_applied: false,
        }],
        no_progress: true,
    })?;
    let repaired_result = parse_contract_value(&repaired, contract)
        .and_then(|value| deserialize_contract_value(value, contract));
    let repaired_error = match repaired_result {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let no_progress = repaired_error == initial_error;
    let mut normalization_applied = false;
    if let Ok(mut value) =
        serde_json::from_str::<serde_json::Value>(&sanitize_json_response(&repaired))
    {
        normalization_applied = normalize_contract_value(&mut value, contract);
        if normalization_applied && validate_contract_value(&value, contract).is_ok() {
            if let Ok(parsed) = deserialize_contract_value(value, contract) {
                return Ok(parsed);
            }
        }
    }
    Err(JsonProtocolFailure {
        contract_name: contract.name.to_string(),
        final_error: repaired_error.clone(),
        repair_attempts: vec![JsonRepairAttemptRecord {
            attempt: MAX_CONTRACT_REPAIR_ATTEMPTS,
            before: initial_error,
            after: Some(repaired_error),
            made_progress: !no_progress,
            deterministic_normalization_applied: normalization_applied,
        }],
        no_progress,
    })
}

pub(crate) async fn parse_json_with_contract<T: serde::de::DeserializeOwned>(
    response_text: &str,
    contract: &JsonTargetContract,
) -> Result<T, JsonProtocolFailure> {
    parse_json_with_contract_and_context(
        response_text,
        contract,
        crate::cost_ledger::ModelCallContext::default(),
    )
    .await
}

pub(crate) async fn parse_json_with_contract_and_context<T: serde::de::DeserializeOwned>(
    response_text: &str,
    contract: &JsonTargetContract,
    context: crate::cost_ledger::ModelCallContext,
) -> Result<T, JsonProtocolFailure> {
    parse_json_with_contract_using(
        response_text,
        contract,
        |raw, description, error| async move {
            repair_json_once_with_contract_and_context(
                &raw,
                &description,
                &error.path,
                &error.expected,
                &error.actual,
                context,
            )
            .await
            .map_err(|repair_error| repair_error.diagnostic_summary().to_string())
        },
    )
    .await
}

/// 带重试的 JSON 解析
/// 第 1 次：sanitize → 直接解析
/// 第 2 次：把错误发给 AI 修正 → sanitize → 解析
/// 第 3 次：再次发给 AI 修正（附"最后一次机会"）→ 解析
/// 三次全失败则返回错误
pub(crate) async fn parse_json_with_retry<T: serde::de::DeserializeOwned>(
    response_text: &str,
) -> Result<T, String> {
    parse_json_with_retry_and_context(response_text, None).await
}

pub(crate) async fn parse_json_with_retry_with_context<T: serde::de::DeserializeOwned>(
    response_text: &str,
    mut context: crate::cost_ledger::ModelCallContext,
) -> Result<T, String> {
    context.purpose = Some(crate::cost_ledger::ModelCallPurpose::SchemaRepair);
    parse_json_with_retry_and_context(response_text, Some(context)).await
}

async fn parse_json_with_retry_and_context<T: serde::de::DeserializeOwned>(
    response_text: &str,
    context: Option<crate::cost_ledger::ModelCallContext>,
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
    let second = match context.clone() {
        Some(context) => crate::api::call_deepseek_api_inner_with_context(
            system_prompt,
            &user_message,
            false,
            0.5,
            context,
        )
        .await
        .map(|response| (response.content, Some(response.metadata.call_id))),
        None => call_deepseek_api_inner(system_prompt, &user_message, false, 0.5)
            .await
            .map(|reply| (reply, None)),
    };
    match second {
        Ok((reply, call_id)) => {
            let cleaned2 = sanitize_json_response(&reply);
            match serde_json::from_str::<T>(&cleaned2) {
                Ok(value) => {
                    if let (Some(context), Some(call_id)) = (context.as_ref(), call_id.as_deref()) {
                        crate::cost_ledger::mark_call_outcome_best_effort(
                            &context.project_name,
                            call_id,
                            crate::cost_ledger::ModelCallOutcome {
                                produced_change: true,
                                ..Default::default()
                            },
                        );
                    }
                    return Ok(value);
                }
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
    let third = match context.clone() {
        Some(context) => crate::api::call_deepseek_api_inner_with_context(
            system_prompt,
            &user_message_last,
            false,
            0.5,
            context,
        )
        .await
        .map(|response| (response.content, Some(response.metadata.call_id))),
        None => call_deepseek_api_inner(system_prompt, &user_message_last, false, 0.5)
            .await
            .map(|reply| (reply, None)),
    };
    match third {
        Ok((reply, call_id)) => {
            let cleaned3 = sanitize_json_response(&reply);
            match serde_json::from_str::<T>(&cleaned3) {
                Ok(value) => {
                    if let (Some(context), Some(call_id)) = (context.as_ref(), call_id.as_deref()) {
                        crate::cost_ledger::mark_call_outcome_best_effort(
                            &context.project_name,
                            call_id,
                            crate::cost_ledger::ModelCallOutcome {
                                produced_change: true,
                                ..Default::default()
                            },
                        );
                    }
                    Ok(value)
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_repair_prompt_contains_exact_contract_and_error_path() {
        let message = schema_repair_user_message(
            r#"{"review_issues":[{"actual":{"found":false}}]}"#,
            crate::prompts::REVIEW_SCHEMA_CONTRACT,
            "$.review_issues[0].actual",
            "string、number、boolean 或 simple object",
            "object",
        );
        assert!(message.contains(crate::prompts::REVIEW_SCHEMA_CONTRACT));
        assert!(message.contains("$.review_issues[0].actual"));
        assert!(message.contains("string、number、boolean 或 simple object"));
        assert!(message.contains("实际类型：object"));
        assert!(!message.contains("请检查 JSON 格式是否正确"));
    }

    #[test]
    fn review_prompt_and_repair_use_the_same_schema_contract() {
        assert!(crate::prompts::TEST_PROMPT.contains(crate::prompts::REVIEW_SCHEMA_CONTRACT));
    }

    #[tokio::test]
    async fn generic_json_parser_still_accepts_valid_planning_json_without_repair() {
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct PlanningOutput {
            title: String,
        }

        let parsed: PlanningOutput = parse_json_with_retry(r#"{"title":"stable"}"#)
            .await
            .expect("valid planning JSON should parse directly");
        assert_eq!(
            parsed,
            PlanningOutput {
                title: "stable".to_string()
            }
        );
    }

    #[tokio::test]
    async fn runtime_fault_json_contract_compresses_object_string_after_one_repair() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        let raw = r#"{
            "implementation_guidance":{"action":"rebuild","scope":"current"},
            "context_summary":"context",
            "evidence_files":["src/lib.rs"],
            "dependency_notes":"keep dependencies",
            "rationale":"avoid retry"
        }"#;
        let parsed: crate::plan_calibration::PlanPatchOutput = parse_json_with_contract_using(
            raw,
            &PLAN_PATCH_JSON_CONTRACT,
            move |original, description, error| {
                observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    assert!(description.contains("$.implementation_guidance：string；必填"));
                    assert_eq!(error.path, "$.implementation_guidance");
                    assert_eq!(error.expected, "string");
                    assert_eq!(error.actual, "object");
                    Ok(original)
                }
            },
        )
        .await
        .expect("deterministic normalization should compress the object");

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(parsed.implementation_guidance.contains("rebuild"));
    }

    #[tokio::test]
    async fn check_convergence_stage_contracts_accept_declared_shapes_without_repair() {
        let cases = [
            (
                r#"{"passed":true,"summary":"ok","omissions":[],"overlaps":[],"out_of_scope":[],"ordering_issues":[],"suggestions":[]}"#,
                &MILESTONE_CHECK_JSON_CONTRACT,
            ),
            (
                r#"{"passed":true,"summary":"ok","omissions":[],"overlaps":[],"ordering_issues":[],"suggestions":[]}"#,
                &MID_STAGE_CHECK_JSON_CONTRACT,
            ),
            (
                r#"{"passed":true,"summary":"ok","omissions":[],"out_of_scope":[],"not_executable":[],"suggestions":[]}"#,
                &EXECUTION_PLAN_CHECK_JSON_CONTRACT,
            ),
        ];

        for (raw, contract) in cases {
            let parsed: serde_json::Value = parse_json_with_contract_using(
                raw,
                contract,
                |_original, _description, _error| async move {
                    Err("valid check JSON must not request repair".to_string())
                },
            )
            .await
            .expect("declared stage check shape should parse");
            assert_eq!(parsed["passed"], true);
        }
    }

    #[tokio::test]
    async fn check_convergence_contract_repairs_then_compresses_string_positions() {
        #[derive(Debug, serde::Deserialize)]
        struct CheckOutput {
            summary: String,
            omissions: Vec<String>,
        }

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        let raw = r#"{
            "passed":false,
            "summary":{"status":"review"},
            "omissions":[{"issue":"missing artifact"}],
            "out_of_scope":[],
            "not_executable":[],
            "suggestions":[]
        }"#;
        let parsed: CheckOutput = parse_json_with_contract_using(
            raw,
            &EXECUTION_PLAN_CHECK_JSON_CONTRACT,
            move |original, _description, _error| {
                observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move { Ok(original) }
            },
        )
        .await
        .expect("one repair followed by deterministic compression should recover the shape");

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(parsed.summary.contains("review"));
        assert!(parsed.omissions[0].contains("missing artifact"));
    }

    #[tokio::test]
    async fn check_convergence_protocol_failure_remains_distinct_from_quality_failure() {
        let failure = parse_json_with_contract_using::<serde_json::Value, _, _>(
            r#"{"passed":"no","summary":"bad protocol","omissions":[],"out_of_scope":[],"not_executable":[],"suggestions":[]}"#,
            &EXECUTION_PLAN_CHECK_JSON_CONTRACT,
            |original, _description, _error| async move { Ok(original) },
        )
        .await
        .expect_err("an unrepaired boolean mismatch must stay a protocol failure");

        assert_eq!(failure.contract_name, "ExecutionPlanCheckResult");
        assert_eq!(failure.final_error.path, "$.passed");
        assert!(failure.no_progress);
    }

    #[tokio::test]
    async fn runtime_fault_json_contract_normalizes_unknown_enum_safely() {
        let raw = r#"{
            "passed":false,
            "reason":"needs review",
            "details":[{"issue_type":"other-kind","description":"d","related_requirement":"r"}],
            "attention_points":[],
            "checked_at":"",
            "warnings":[]
        }"#;
        let parsed: crate::project::QAResult = parse_json_with_contract_using(
            raw,
            &QA_RESULT_JSON_CONTRACT,
            |original, _description, error| async move {
                assert_eq!(error.path, "$.details[0].issue_type");
                assert_eq!(error.kind, JsonParseErrorKind::InvalidEnum);
                Ok(original)
            },
        )
        .await
        .expect("unknown enum should normalize to the explicit safe value");

        assert_eq!(parsed.details[0].issue_type, "未知");
    }

    #[tokio::test]
    async fn runtime_fault_json_contract_stops_after_same_error_without_progress() {
        #[derive(Debug, serde::Deserialize)]
        struct StrictOutput {
            passed: bool,
        }
        const FIELDS: &[JsonFieldContract] = &[
            JsonFieldContract {
                path: "$",
                expected_type: JsonExpectedType::Object,
                allowed_values: &[],
                required: true,
            },
            JsonFieldContract {
                path: "$.passed",
                expected_type: JsonExpectedType::Boolean,
                allowed_values: &[],
                required: true,
            },
        ];
        const CONTRACT: JsonTargetContract = JsonTargetContract {
            name: "StrictOutput",
            fields: FIELDS,
        };
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        let failure = parse_json_with_contract_using::<StrictOutput, _, _>(
            r#"{"passed":"yes"}"#,
            &CONTRACT,
            move |original, _description, _error| {
                observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move { Ok(original) }
            },
        )
        .await
        .unwrap_err();

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(failure.no_progress);
        assert_eq!(failure.repair_attempts.len(), MAX_CONTRACT_REPAIR_ATTEMPTS);
        assert!(!failure.repair_attempts[0].made_progress);
        assert_eq!(failure.final_error.path, "$.passed");
        let _ = StrictOutput { passed: false }.passed;
    }
}
