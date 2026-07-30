use crate::project;
use serde_json::{Map, Value};
use std::fmt;
use std::future::Future;

const MAX_DIAGNOSTIC_TEXT_CHARS: usize = 500;
const MAX_SIMPLE_OBJECT_FIELDS: usize = 12;

#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct ModelCriterionReview {
    #[serde(default)]
    pub(crate) criterion_index: u32,
    #[serde(default)]
    pub(crate) conclusion: project::CriterionReviewConclusion,
    #[serde(default)]
    pub(crate) confidence: f64,
    #[serde(default)]
    pub(crate) evidence_block_ids: Vec<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct ModelReviewIssue {
    #[serde(default)]
    pub(crate) criterion_index: Option<u32>,
    #[serde(default)]
    pub(crate) criterion: String,
    #[serde(default)]
    pub(crate) file: String,
    #[serde(default)]
    pub(crate) expected: String,
    #[serde(default)]
    pub(crate) actual: String,
    #[serde(default)]
    pub(crate) suggested_change: String,
    #[serde(default)]
    pub(crate) confidence: f64,
    #[serde(default)]
    pub(crate) severity: Option<project::ReviewIssueSeverity>,
    #[serde(default)]
    pub(crate) evidence_block_ids: Vec<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct ModelReviewResponse {
    #[serde(default)]
    pub(crate) passed: bool,
    #[serde(default)]
    pub(crate) issues: Vec<String>,
    #[serde(default)]
    pub(crate) suggestion: String,
    #[serde(default)]
    pub(crate) review_issues: Vec<ModelReviewIssue>,
    #[serde(default)]
    pub(crate) warnings: Vec<String>,
    #[serde(default)]
    pub(crate) criterion_reviews: Option<Vec<ModelCriterionReview>>,
}

#[derive(Debug)]
pub(crate) struct NormalizedReviewResponse {
    pub(crate) response: ModelReviewResponse,
    pub(crate) normalized_field_count: u32,
    pub(crate) protocol_repair_attempted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewProtocolError {
    pub(crate) kind: project::ReviewFailureKind,
    pub(crate) path: String,
    pub(crate) expected: String,
    pub(crate) actual: String,
    pub(crate) protocol_repair_attempted: bool,
}

impl fmt::Display for ReviewProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "审查协议字段 {} 预期 {}，实际为 {}",
            self.path, self.expected, self.actual
        )
    }
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn field_error(path: impl Into<String>, expected: &str, value: &Value) -> ReviewProtocolError {
    ReviewProtocolError {
        kind: project::ReviewFailureKind::FieldTypeMismatch,
        path: path.into(),
        expected: expected.to_string(),
        actual: value_kind(value).to_string(),
        protocol_repair_attempted: false,
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut rendered = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        rendered.push_str("...");
    }
    rendered
}

fn is_simple_value(value: &Value, nested: bool) -> bool {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
        Value::Array(items) => {
            items.len() <= MAX_SIMPLE_OBJECT_FIELDS
                && items.iter().all(|item| {
                    matches!(
                        item,
                        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
                    )
                })
        }
        Value::Object(fields) => {
            !nested
                && fields.len() <= MAX_SIMPLE_OBJECT_FIELDS
                && fields.values().all(|value| is_simple_value(value, true))
        }
    }
}

fn sensitive_key(key: &str) -> bool {
    matches!(
        normalized_token(key).as_str(),
        "api_key" | "apikey" | "authorization" | "token" | "secret" | "password"
    )
}

fn redact_sensitive_fields(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if sensitive_key(key) {
                    *value = Value::String("[REDACTED]".to_string());
                } else {
                    redact_sensitive_fields(value);
                }
            }
        }
        Value::Array(items) => {
            for value in items {
                redact_sensitive_fields(value);
            }
        }
        _ => {}
    }
}

fn diagnostic_text(value: Value, path: &str) -> Result<(String, bool), ReviewProtocolError> {
    match value {
        Value::String(text) => Ok((
            truncate_chars(text.trim(), MAX_DIAGNOSTIC_TEXT_CHARS),
            false,
        )),
        Value::Number(number) => Ok((number.to_string(), true)),
        Value::Bool(value) => Ok((value.to_string(), true)),
        Value::Null => Ok((String::new(), true)),
        Value::Object(fields) => {
            let mut value = Value::Object(fields);
            if !is_simple_value(&value, false) {
                return Err(field_error(
                    path,
                    "string、number、boolean 或 simple object",
                    &value,
                ));
            }
            redact_sensitive_fields(&mut value);
            let rendered = serde_json::to_string(&value)
                .map_err(|_| field_error(path, "可压缩为诊断文本的 simple object", &value))?;
            Ok((truncate_chars(&rendered, MAX_DIAGNOSTIC_TEXT_CHARS), true))
        }
        other => Err(field_error(
            path,
            "string、number、boolean 或 simple object",
            &other,
        )),
    }
}

fn normalize_text_field(
    object: &mut Map<String, Value>,
    field: &str,
    path: &str,
    normalized: &mut u32,
) -> Result<(), ReviewProtocolError> {
    let Some(value) = object.remove(field) else {
        return Ok(());
    };
    let (text, changed) = diagnostic_text(value, path)?;
    *normalized += u32::from(changed);
    object.insert(field.to_string(), Value::String(text));
    Ok(())
}

fn normalize_strict_string_field(
    object: &mut Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<(), ReviewProtocolError> {
    if let Some(value) = object.get(field) {
        if !value.is_string() {
            return Err(field_error(path, "string", value));
        }
    }
    Ok(())
}

fn normalize_text_list(
    object: &mut Map<String, Value>,
    field: &str,
    path: &str,
    normalized: &mut u32,
) -> Result<(), ReviewProtocolError> {
    let Some(value) = object.remove(field) else {
        return Ok(());
    };
    let items = match value {
        Value::Null => {
            *normalized += 1;
            Vec::new()
        }
        Value::String(text) => {
            *normalized += 1;
            vec![Value::String(truncate_chars(
                text.trim(),
                MAX_DIAGNOSTIC_TEXT_CHARS,
            ))]
        }
        Value::Array(items) => items,
        other => return Err(field_error(path, "string 或 string/object array", &other)),
    };
    let mut rendered = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let (text, changed) = diagnostic_text(item, &item_path)?;
        *normalized += u32::from(changed);
        rendered.push(Value::String(text));
    }
    object.insert(field.to_string(), Value::Array(rendered));
    Ok(())
}

fn normalize_index(
    object: &mut Map<String, Value>,
    field: &str,
    path: &str,
    optional: bool,
    normalized: &mut u32,
) -> Result<(), ReviewProtocolError> {
    let Some(value) = object.remove(field) else {
        return Ok(());
    };
    if optional && value.is_null() {
        object.insert(field.to_string(), Value::Null);
        return Ok(());
    }
    let index = match &value {
        Value::Number(number) => number.as_u64().and_then(|value| u32::try_from(value).ok()),
        Value::String(text) => {
            *normalized += 1;
            text.trim().parse::<u32>().ok()
        }
        _ => None,
    }
    .ok_or_else(|| field_error(path, "非负整数或整数字符串", &value))?;
    object.insert(field.to_string(), Value::Number(index.into()));
    Ok(())
}

fn normalize_confidence(
    object: &mut Map<String, Value>,
    path: &str,
    normalized: &mut u32,
) -> Result<(), ReviewProtocolError> {
    let Some(value) = object.remove("confidence") else {
        return Ok(());
    };
    let confidence = value
        .as_f64()
        .ok_or_else(|| field_error(path, "0.0 到 1.0 之间的 number", &value))?;
    let clamped = confidence.clamp(0.0, 1.0);
    if clamped != confidence {
        *normalized += 1;
    }
    let number = serde_json::Number::from_f64(clamped)
        .ok_or_else(|| field_error(path, "有限 number", &value))?;
    object.insert("confidence".to_string(), Value::Number(number));
    Ok(())
}

fn normalized_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn normalize_severity(
    object: &mut Map<String, Value>,
    path: &str,
    normalized: &mut u32,
) -> Result<(), ReviewProtocolError> {
    let Some(value) = object.remove("severity") else {
        return Ok(());
    };
    if value.is_null() {
        object.insert("severity".to_string(), Value::Null);
        return Ok(());
    }
    let raw = value
        .as_str()
        .ok_or_else(|| field_error(path, "Blocking、Warning 或 Suggestion", &value))?;
    let canonical = match normalized_token(raw).as_str() {
        "blocking" | "blocker" | "blocked" | "critical" | "error" | "high" | "阻断" => "Blocking",
        "warning" | "warn" | "medium" | "警告" => "Warning",
        "suggestion" | "suggest" | "info" | "advisory" | "improvement" | "建议" => "Suggestion",
        _ => return Err(field_error(path, "Blocking、Warning 或 Suggestion", &value)),
    };
    *normalized += u32::from(raw != canonical);
    object.insert("severity".to_string(), Value::String(canonical.to_string()));
    Ok(())
}

fn normalize_conclusion(
    object: &mut Map<String, Value>,
    path: &str,
    normalized: &mut u32,
) -> Result<(), ReviewProtocolError> {
    let Some(value) = object.remove("conclusion") else {
        return Ok(());
    };
    let raw = value.as_str().ok_or_else(|| {
        field_error(
            path,
            "Satisfied、Unsatisfied 或 EvidenceInsufficient",
            &value,
        )
    })?;
    let canonical = match normalized_token(raw).as_str() {
        "satisfied" | "pass" | "passed" | "success" | "met" | "满足" | "通过" => "Satisfied",
        "unsatisfied" | "fail" | "failed" | "unmet" | "不满足" | "未通过" => "Unsatisfied",
        "evidenceinsufficient"
        | "evidence_insufficient"
        | "insufficient"
        | "unknown"
        | "not_enough_evidence"
        | "证据不足" => "EvidenceInsufficient",
        _ => {
            return Err(field_error(
                path,
                "Satisfied、Unsatisfied 或 EvidenceInsufficient",
                &value,
            ))
        }
    };
    *normalized += u32::from(raw != canonical);
    object.insert(
        "conclusion".to_string(),
        Value::String(canonical.to_string()),
    );
    Ok(())
}

fn normalize_evidence_ids(
    object: &mut Map<String, Value>,
    path: &str,
    normalized: &mut u32,
) -> Result<(), ReviewProtocolError> {
    let Some(value) = object.remove("evidence_block_ids") else {
        return Ok(());
    };
    let items = match value {
        Value::Null => {
            *normalized += 1;
            Vec::new()
        }
        Value::String(value) => {
            *normalized += 1;
            vec![Value::String(value)]
        }
        Value::Array(items) => items,
        other => return Err(field_error(path, "string 或 string array", &other)),
    };
    let original_len = items.len();
    let retained = items
        .into_iter()
        .filter(|item| item.is_string())
        .collect::<Vec<_>>();
    *normalized += (original_len - retained.len()) as u32;
    object.insert("evidence_block_ids".to_string(), Value::Array(retained));
    Ok(())
}

fn normalize_review_issue(
    value: &mut Value,
    path: &str,
    normalized: &mut u32,
) -> Result<(), ReviewProtocolError> {
    if !value.is_object() {
        return Err(field_error(path, "object", value));
    }
    let object = value.as_object_mut().expect("object type checked above");
    normalize_index(
        object,
        "criterion_index",
        &format!("{path}.criterion_index"),
        true,
        normalized,
    )?;
    normalize_strict_string_field(object, "criterion", &format!("{path}.criterion"))?;
    normalize_strict_string_field(object, "file", &format!("{path}.file"))?;
    normalize_text_field(object, "expected", &format!("{path}.expected"), normalized)?;
    normalize_text_field(object, "actual", &format!("{path}.actual"), normalized)?;
    normalize_text_field(
        object,
        "suggested_change",
        &format!("{path}.suggested_change"),
        normalized,
    )?;
    normalize_confidence(object, &format!("{path}.confidence"), normalized)?;
    normalize_severity(object, &format!("{path}.severity"), normalized)?;
    normalize_evidence_ids(object, &format!("{path}.evidence_block_ids"), normalized)
}

fn normalize_criterion_review(
    value: &mut Value,
    path: &str,
    normalized: &mut u32,
) -> Result<(), ReviewProtocolError> {
    if !value.is_object() {
        return Err(field_error(path, "object", value));
    }
    let object = value.as_object_mut().expect("object type checked above");
    normalize_index(
        object,
        "criterion_index",
        &format!("{path}.criterion_index"),
        false,
        normalized,
    )?;
    normalize_conclusion(object, &format!("{path}.conclusion"), normalized)?;
    normalize_confidence(object, &format!("{path}.confidence"), normalized)?;
    normalize_evidence_ids(object, &format!("{path}.evidence_block_ids"), normalized)
}

fn normalize_object_array(
    object: &mut Map<String, Value>,
    field: &str,
    path: &str,
    normalized: &mut u32,
    normalize_item: fn(&mut Value, &str, &mut u32) -> Result<(), ReviewProtocolError>,
) -> Result<(), ReviewProtocolError> {
    let Some(value) = object.remove(field) else {
        return Ok(());
    };
    let mut items = match value {
        Value::Null => {
            *normalized += 1;
            Vec::new()
        }
        Value::Array(items) => items,
        other => return Err(field_error(path, "object array", &other)),
    };
    for (index, item) in items.iter_mut().enumerate() {
        normalize_item(item, &format!("{path}[{index}]"), normalized)?;
    }
    object.insert(field.to_string(), Value::Array(items));
    Ok(())
}

pub(crate) fn parse_review_response(
    raw: &str,
) -> Result<NormalizedReviewResponse, ReviewProtocolError> {
    if raw.trim().is_empty() {
        return Err(ReviewProtocolError {
            kind: project::ReviewFailureKind::EmptyResponse,
            path: "$".to_string(),
            expected: "non-empty JSON object".to_string(),
            actual: "empty response".to_string(),
            protocol_repair_attempted: false,
        });
    }
    let cleaned = crate::json_utils::sanitize_json_response(raw);
    let mut value =
        serde_json::from_str::<Value>(&cleaned).map_err(|error| ReviewProtocolError {
            kind: project::ReviewFailureKind::InvalidJson,
            path: "$".to_string(),
            expected: "valid JSON object".to_string(),
            actual: format!(
                "JSON syntax error at line {} column {}",
                error.line(),
                error.column()
            ),
            protocol_repair_attempted: false,
        })?;
    if !value.is_object() {
        return Err(field_error("$", "JSON object", &value));
    }
    let object = value.as_object_mut().expect("object type checked above");
    let mut normalized = 0;
    if let Some(passed) = object.get("passed") {
        if !passed.is_boolean() {
            return Err(field_error("$.passed", "boolean", passed));
        }
    }
    normalize_text_list(object, "issues", "$.issues", &mut normalized)?;
    normalize_text_field(object, "suggestion", "$.suggestion", &mut normalized)?;
    normalize_text_list(object, "warnings", "$.warnings", &mut normalized)?;
    normalize_object_array(
        object,
        "review_issues",
        "$.review_issues",
        &mut normalized,
        normalize_review_issue,
    )?;
    normalize_object_array(
        object,
        "criterion_reviews",
        "$.criterion_reviews",
        &mut normalized,
        normalize_criterion_review,
    )?;
    let response = serde_json::from_value::<ModelReviewResponse>(value).map_err(|error| {
        ReviewProtocolError {
            kind: project::ReviewFailureKind::FieldTypeMismatch,
            path: "$".to_string(),
            expected: "review response schema".to_string(),
            actual: format!("strict schema mismatch: {error}"),
            protocol_repair_attempted: false,
        }
    })?;
    Ok(NormalizedReviewResponse {
        response,
        normalized_field_count: normalized,
        protocol_repair_attempted: false,
    })
}

pub(crate) async fn parse_review_response_with_repair_and_progress(
    raw: &str,
    progress: Option<&(dyn Fn(project::VerificationStage) + Send + Sync)>,
) -> Result<NormalizedReviewResponse, ReviewProtocolError> {
    parse_review_response_with_repair_and_progress_with_context(
        raw,
        progress,
        crate::cost_ledger::ModelCallContext::default(),
    )
    .await
}

pub(crate) async fn parse_review_response_with_repair_and_progress_with_context(
    raw: &str,
    progress: Option<&(dyn Fn(project::VerificationStage) + Send + Sync)>,
    context: crate::cost_ledger::ModelCallContext,
) -> Result<NormalizedReviewResponse, ReviewProtocolError> {
    parse_review_response_with_repair_using(raw, progress, |response_text, error| async move {
        crate::json_utils::repair_json_once_with_contract_and_context(
            &response_text,
            crate::prompts::REVIEW_SCHEMA_CONTRACT,
            &error.path,
            &error.expected,
            &error.actual,
            context,
        )
        .await
        .map_err(|repair_error| ReviewProtocolError {
            kind: repair_error.review_failure_kind(),
            path: error.path.clone(),
            expected: error.expected.clone(),
            actual: format!(
                "protocol repair request failed: {}",
                truncate_chars(repair_error.diagnostic_summary(), 300)
            ),
            protocol_repair_attempted: true,
        })
    })
    .await
}

async fn parse_review_response_with_repair_using<F, Fut>(
    raw: &str,
    progress: Option<&(dyn Fn(project::VerificationStage) + Send + Sync)>,
    repair: F,
) -> Result<NormalizedReviewResponse, ReviewProtocolError>
where
    F: FnOnce(String, ReviewProtocolError) -> Fut,
    Fut: Future<Output = Result<String, ReviewProtocolError>>,
{
    if let Some(progress) = progress {
        progress(project::VerificationStage::DeterministicNormalization);
    }
    let initial_error = match parse_review_response(raw) {
        Ok(response) => return Ok(response),
        Err(error) => error,
    };
    if initial_error.kind == project::ReviewFailureKind::EmptyResponse {
        return Err(initial_error);
    }
    if let Some(progress) = progress {
        progress(project::VerificationStage::ProtocolRepair);
    }
    let repaired = repair(raw.to_string(), initial_error.clone()).await?;
    match parse_review_response(&repaired) {
        Ok(mut response) => {
            response.protocol_repair_attempted = true;
            Ok(response)
        }
        Err(mut error) => {
            error.protocol_repair_attempted = true;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn normalizes_common_review_type_drift_deterministically() -> Result<(), String> {
        let normalized = parse_review_response(
            r#"{
                "passed": false,
                "issues": "需要核对",
                "suggestion": {"action":"add guard","line":12},
                "warnings": [{"code":"W1","message":"格式漂移"}],
                "criterion_reviews": [{
                    "criterion_index": "1",
                    "conclusion": "passed",
                    "confidence": 1.4,
                    "evidence_block_ids": ["E001", 2]
                }],
                "review_issues": [{
                    "criterion_index": "1",
                    "criterion": "guard exists",
                    "file": "src/lib.rs",
                    "expected": true,
                    "actual": {"found":false},
                    "suggested_change": 1,
                    "confidence": -0.2,
                    "severity": "critical",
                    "evidence_block_ids": "E001"
                }]
            }"#,
        )
        .map_err(|error| error.to_string())?;
        assert!(normalized.normalized_field_count >= 10);
        assert_eq!(normalized.response.issues, vec!["需要核对"]);
        assert!(normalized.response.suggestion.contains("add guard"));
        assert_eq!(
            normalized.response.criterion_reviews.as_ref().unwrap()[0].conclusion,
            project::CriterionReviewConclusion::Satisfied
        );
        let issue = &normalized.response.review_issues[0];
        assert_eq!(issue.expected, "true");
        assert_eq!(issue.actual, r#"{"found":false}"#);
        assert_eq!(issue.suggested_change, "1");
        assert_eq!(issue.confidence, 0.0);
        assert_eq!(issue.severity, Some(project::ReviewIssueSeverity::Blocking));
        assert_eq!(issue.evidence_block_ids, vec!["E001"]);
        Ok(())
    }

    #[test]
    fn redacts_sensitive_values_from_object_diagnostics() -> Result<(), String> {
        let normalized = parse_review_response(
            r#"{"suggestion":{"action":"retry","authorization":"Bearer private","meta":["safe"]}}"#,
        )
        .map_err(|error| error.to_string())?;
        assert!(normalized.response.suggestion.contains("retry"));
        assert!(normalized.response.suggestion.contains("[REDACTED]"));
        assert!(!normalized.response.suggestion.contains("private"));
        Ok(())
    }

    #[test]
    fn rejects_complex_objects_with_an_exact_field_path() {
        let error = parse_review_response(
            r#"{"review_issues":[{"actual":{"nested":{"value":"unsafe"}}}]}"#,
        )
        .unwrap_err();
        assert_eq!(error.kind, project::ReviewFailureKind::FieldTypeMismatch);
        assert_eq!(error.path, "$.review_issues[0].actual");
        assert!(error.expected.contains("simple object"));
    }

    #[test]
    fn rejects_non_string_evidence_container() {
        let error = parse_review_response(
            r#"{"criterion_reviews":[{"criterion_index":1,"evidence_block_ids":{"id":"E001"}}]}"#,
        )
        .unwrap_err();
        assert_eq!(error.path, "$.criterion_reviews[0].evidence_block_ids");
    }

    #[tokio::test]
    async fn protocol_repair_is_attempted_only_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let error = parse_review_response_with_repair_using(
            r#"{"passed":"yes"}"#,
            None,
            move |_raw, first_error| {
                observed.fetch_add(1, Ordering::SeqCst);
                async move {
                    assert_eq!(first_error.path, "$.passed");
                    Ok(r#"{"passed":"still wrong"}"#.to_string())
                }
            },
        )
        .await
        .unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(error.protocol_repair_attempted);
        assert_eq!(error.path, "$.passed");
    }

    #[tokio::test]
    async fn validation_protocol_progress_reports_normalization_and_schema_repair_stages() {
        let stages = Arc::new(Mutex::new(Vec::new()));
        let observed = stages.clone();
        let reporter = move |stage| observed.lock().unwrap().push(stage);
        let normalized = parse_review_response_with_repair_using(
            r#"{"passed":"yes"}"#,
            Some(&reporter),
            |_raw, _error| async move { Ok(r#"{"passed":true}"#.to_string()) },
        )
        .await
        .unwrap();

        assert!(normalized.response.passed);
        assert_eq!(
            *stages.lock().unwrap(),
            vec![
                project::VerificationStage::DeterministicNormalization,
                project::VerificationStage::ProtocolRepair,
            ]
        );
    }

    #[tokio::test]
    async fn empty_response_skips_protocol_repair() {
        let error = parse_review_response_with_repair_using("", None, |_raw, _error| async move {
            panic!("empty responses must be reviewed again, not synthesized")
        })
        .await
        .unwrap_err();
        assert_eq!(error.kind, project::ReviewFailureKind::EmptyResponse);
        assert!(!error.protocol_repair_attempted);
    }
}
