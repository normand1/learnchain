use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    markdown_rules,
    session_sources::{Session, SessionEvent, SessionEventKind},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionAnalytics {
    pub total_tool_calls: u32,
    pub successful_tool_calls: u32,
    pub failed_tool_calls: u32,
    pub unknown_outcome_tool_calls: u32,
    pub mcp_tool_calls: u32,
    pub external_lookup_calls: u32,
    pub adjust_course_count: u32,
    pub external_resources: Vec<ExternalResourceRef>,
    pub adjustments: Vec<AdjustmentMarker>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalResourceKind {
    Web,
    Mcp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExternalResourceRef {
    pub kind: ExternalResourceKind,
    pub tool_name: String,
    pub label: String,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjustmentKind {
    PostFailurePivot,
    RetryWithDifferentArguments,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdjustmentMarker {
    pub kind: AdjustmentKind,
    pub from_tool_name: String,
    pub to_tool_name: String,
    pub from_argument_summary: Option<String>,
    pub to_argument_summary: Option<String>,
}

impl Default for ExternalResourceRef {
    fn default() -> Self {
        Self {
            kind: ExternalResourceKind::Web,
            tool_name: String::new(),
            label: String::new(),
            count: 0,
        }
    }
}

impl Default for AdjustmentMarker {
    fn default() -> Self {
        Self {
            kind: AdjustmentKind::PostFailurePivot,
            from_tool_name: String::new(),
            to_tool_name: String::new(),
            from_argument_summary: None,
            to_argument_summary: None,
        }
    }
}

impl SessionAnalytics {
    pub fn is_empty(&self) -> bool {
        self.total_tool_calls == 0
            && self.successful_tool_calls == 0
            && self.failed_tool_calls == 0
            && self.unknown_outcome_tool_calls == 0
            && self.mcp_tool_calls == 0
            && self.external_lookup_calls == 0
            && self.adjust_course_count == 0
            && self.external_resources.is_empty()
            && self.adjustments.is_empty()
    }
}

#[derive(Debug)]
struct ToolCallRecord<'a> {
    tool_name: &'a str,
    call_id: Option<&'a str>,
    normalized_arguments: Option<String>,
    argument_summary: Option<String>,
}

pub fn analyze(session: &Session) -> SessionAnalytics {
    let mut analytics = SessionAnalytics::default();
    let mut tool_calls = Vec::new();
    let mut call_ids_to_calls: HashMap<&str, usize> = HashMap::new();
    let mut results_by_call_id: HashMap<&str, &SessionEvent> = HashMap::new();
    let mut external_resources: BTreeMap<(ExternalResourceKind, String, String), u32> =
        BTreeMap::new();

    for event in &session.events {
        if event.event_kind == SessionEventKind::ToolCall {
            analytics.total_tool_calls += 1;

            if let Some(tool_name) = event.tool_name.as_deref() {
                if tool_name.starts_with("mcp__") {
                    analytics.mcp_tool_calls += 1;
                }

                if is_external_lookup_tool(tool_name) {
                    analytics.external_lookup_calls += 1;
                    let kind = if tool_name.starts_with("mcp__") {
                        ExternalResourceKind::Mcp
                    } else {
                        ExternalResourceKind::Web
                    };
                    let label = external_resource_label(tool_name, event.arguments.as_deref());
                    *external_resources
                        .entry((kind, tool_name.to_string(), label))
                        .or_default() += 1;
                }
            }

            let record = ToolCallRecord {
                tool_name: event.tool_name.as_deref().unwrap_or(&event.payload_type),
                call_id: event.call_id.as_deref(),
                normalized_arguments: normalize_arguments(event.arguments.as_deref()),
                argument_summary: summarize_tool_context(
                    event.tool_name.as_deref().unwrap_or(&event.payload_type),
                    event.arguments.as_deref(),
                ),
            };
            let index = tool_calls.len();
            if let Some(call_id) = record.call_id {
                call_ids_to_calls.insert(call_id, index);
            }
            tool_calls.push(record);
        } else if event.event_kind == SessionEventKind::ToolResult
            && let Some(call_id) = event.call_id.as_deref()
        {
            results_by_call_id.entry(call_id).or_insert(event);
        }
    }

    for call in &tool_calls {
        let Some(call_id) = call.call_id else {
            analytics.unknown_outcome_tool_calls += 1;
            continue;
        };

        match results_by_call_id.get(call_id) {
            Some(result) if is_successful_result(result) => analytics.successful_tool_calls += 1,
            Some(result) if is_problematic_result(result) => analytics.failed_tool_calls += 1,
            Some(_) | None => analytics.unknown_outcome_tool_calls += 1,
        }
    }

    let mut adjustments = Vec::new();
    for (index, event) in session.events.iter().enumerate() {
        if event.event_kind != SessionEventKind::ToolResult || !is_problematic_result(event) {
            continue;
        }

        let Some(call_id) = event.call_id.as_deref() else {
            continue;
        };
        let Some(source_index) = call_ids_to_calls.get(call_id).copied() else {
            continue;
        };
        let Some(next_call) = next_tool_call_after(&session.events, index + 1) else {
            continue;
        };

        let source_call = &tool_calls[source_index];
        let next_tool_name = next_call
            .tool_name
            .as_deref()
            .unwrap_or(&next_call.payload_type);

        if source_call.tool_name != next_tool_name {
            adjustments.push(AdjustmentMarker {
                kind: AdjustmentKind::PostFailurePivot,
                from_tool_name: source_call.tool_name.to_string(),
                to_tool_name: next_tool_name.to_string(),
                from_argument_summary: source_call.argument_summary.clone(),
                to_argument_summary: summarize_tool_context(
                    next_tool_name,
                    next_call.arguments.as_deref(),
                ),
            });
            continue;
        }

        let next_args = normalize_arguments(next_call.arguments.as_deref());
        if source_call.normalized_arguments != next_args {
            adjustments.push(AdjustmentMarker {
                kind: AdjustmentKind::RetryWithDifferentArguments,
                from_tool_name: source_call.tool_name.to_string(),
                to_tool_name: next_tool_name.to_string(),
                from_argument_summary: source_call.argument_summary.clone(),
                to_argument_summary: summarize_tool_context(
                    next_tool_name,
                    next_call.arguments.as_deref(),
                ),
            });
        }
    }

    analytics.adjust_course_count = adjustments.len() as u32;
    analytics.adjustments = adjustments;
    analytics.external_resources = external_resources
        .into_iter()
        .map(|((kind, tool_name, label), count)| ExternalResourceRef {
            kind,
            tool_name,
            label,
            count,
        })
        .collect();

    analytics
}

fn next_tool_call_after(events: &[SessionEvent], start_index: usize) -> Option<&SessionEvent> {
    events[start_index..]
        .iter()
        .find(|event| event.event_kind == SessionEventKind::ToolCall)
}

fn is_successful_result(event: &SessionEvent) -> bool {
    event
        .result_metadata
        .as_ref()
        .and_then(|metadata| metadata.exit_code)
        == Some(0)
        && !markdown_rules::includes_problematic_tool_result(event)
}

fn is_problematic_result(event: &SessionEvent) -> bool {
    event
        .result_metadata
        .as_ref()
        .and_then(|metadata| metadata.exit_code)
        .is_some_and(|code| code != 0)
        || markdown_rules::includes_problematic_tool_result(event)
}

fn is_external_lookup_tool(tool_name: &str) -> bool {
    tool_name.starts_with("web.") || tool_name.starts_with("mcp__")
}

pub(crate) fn describe_adjustment(adjustment: &AdjustmentMarker) -> String {
    let description = match adjustment.kind {
        AdjustmentKind::PostFailurePivot => "pivot after failure",
        AdjustmentKind::RetryWithDifferentArguments => "changed args after failure",
    };
    let mut line = format!(
        "{} -> {} ({})",
        adjustment.from_tool_name, adjustment.to_tool_name, description
    );

    match (
        adjustment.from_argument_summary.as_deref(),
        adjustment.to_argument_summary.as_deref(),
    ) {
        (Some(from), Some(to)) => {
            line.push_str(&format!(": {} -> {}", from, to));
        }
        (Some(from), None) => {
            line.push_str(&format!(": {}", from));
        }
        (None, Some(to)) => {
            line.push_str(&format!(": {}", to));
        }
        (None, None) => {}
    }

    line
}

fn external_resource_label(tool_name: &str, arguments: Option<&str>) -> String {
    if tool_name.starts_with("mcp__") {
        return mcp_label(tool_name, arguments);
    }

    let parsed = arguments.and_then(parse_json_value);
    if let Some(value) = parsed.as_ref() {
        if let Some(query) = first_string_for_keys(value, &["q", "url", "pattern", "location"]) {
            return query;
        }
        if let Some(domains) = first_array_for_key(value, "domains") {
            return format!("domains: {}", domains.join(", "));
        }
        if let Some(ticker) =
            first_string_for_keys(value, &["ticker", "team", "opponent", "ref_id"])
        {
            return ticker;
        }
    }

    arguments
        .map(compact_text)
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| tool_name.to_string())
}

fn mcp_label(tool_name: &str, arguments: Option<&str>) -> String {
    let mut segments = tool_name.split("__");
    let _ = segments.next();
    let server = segments.next().unwrap_or("unknown");
    let tool = segments.next().unwrap_or("tool");
    let base = format!("{server}/{tool}");

    let Some(arguments) = arguments else {
        return base;
    };

    let summary = compact_argument_summary(arguments);
    if summary.is_empty() {
        base
    } else {
        format!("{base} ({summary})")
    }
}

fn compact_argument_summary(arguments: &str) -> String {
    let Some(value) = parse_json_value(arguments) else {
        return compact_text(arguments);
    };

    if let Some(query) = first_string_for_keys(
        &value,
        &[
            "query",
            "q",
            "url",
            "libraryId",
            "path",
            "location",
            "filePath",
        ],
    ) {
        return compact_text(&query);
    }

    compact_text(&canonical_json(&value))
}

fn summarize_tool_context(tool_name: &str, arguments: Option<&str>) -> Option<String> {
    let arguments = arguments?;
    let parsed = parse_json_value(arguments);

    let summary = match tool_name {
        "exec_command" | "shell" => parsed
            .as_ref()
            .and_then(exec_command_summary)
            .unwrap_or_else(|| compact_text(arguments)),
        "write_stdin" => parsed
            .as_ref()
            .and_then(write_stdin_summary)
            .unwrap_or_else(|| compact_text(arguments)),
        "request_user_input" => parsed
            .as_ref()
            .and_then(request_user_input_summary)
            .unwrap_or_else(|| compact_text(arguments)),
        _ if is_external_lookup_tool(tool_name) => {
            external_resource_label(tool_name, Some(arguments))
        }
        _ => compact_argument_summary(arguments),
    };

    if summary.trim().is_empty() {
        None
    } else {
        Some(summary)
    }
}

fn exec_command_summary(value: &Value) -> Option<String> {
    let map = value.as_object()?;
    let command = map
        .get("cmd")
        .map(command_value_summary)
        .or_else(|| map.get("command").map(command_value_summary))
        .filter(|summary| !summary.is_empty())?;

    let mut parts = vec![format!("cmd={command}")];
    if let Some(workdir) = map.get("workdir").and_then(|value| value.as_str()) {
        let workdir = compact_text(workdir);
        if !workdir.is_empty() {
            parts.push(format!("workdir={workdir}"));
        }
    }
    Some(parts.join(" | "))
}

fn command_value_summary(value: &Value) -> String {
    match value {
        Value::Array(items) => compact_text(
            &items
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        Value::String(text) => compact_text(text),
        other => compact_text(&canonical_json(other)),
    }
}

fn write_stdin_summary(value: &Value) -> Option<String> {
    let map = value.as_object()?;
    let session_id = map
        .get("session_id")
        .and_then(|value| value.as_i64())
        .map(|value| value.to_string())
        .or_else(|| {
            map.get("session_id")
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        })?;
    let chars = map
        .get("chars")
        .and_then(|value| value.as_str())
        .map(compact_text)
        .filter(|value| !value.is_empty());

    Some(match chars {
        Some(chars) => format!("session_id={session_id} | chars={chars}"),
        None => format!("session_id={session_id} | poll"),
    })
}

fn request_user_input_summary(value: &Value) -> Option<String> {
    let map = value.as_object()?;
    let questions = map.get("questions")?.as_array()?;
    let headers = questions
        .iter()
        .filter_map(|question| question.as_object())
        .filter_map(|question| question.get("header").and_then(|value| value.as_str()))
        .collect::<Vec<_>>();

    if headers.is_empty() {
        Some(format!("questions={}", questions.len()))
    } else {
        Some(format!(
            "questions={} ({})",
            questions.len(),
            compact_text(&headers.join(", "))
        ))
    }
}

fn normalize_arguments(arguments: Option<&str>) -> Option<String> {
    let args = arguments?;
    let parsed = parse_json_value(args)?;
    Some(normalize_argument_value(&parsed))
}

fn normalize_argument_value(value: &Value) -> String {
    if let Value::Object(map) = value
        && let Some(command) = map.get("command")
    {
        let mut normalized = Map::new();
        normalized.insert("command".to_string(), normalize_command_value(command));
        if let Some(workdir) = map.get("workdir") {
            normalized.insert("workdir".to_string(), workdir.clone());
        }
        return canonical_json(&Value::Object(normalized));
    }

    canonical_json(value)
}

fn normalize_command_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.clone()),
        Value::String(text) => Value::String(text.trim().to_string()),
        other => other.clone(),
    }
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, child) in map {
                sorted.insert(key, canonical_json(child));
            }
            let mut parts = Vec::with_capacity(sorted.len());
            for (key, child) in sorted {
                parts.push(format!(
                    "{}:{}",
                    serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()),
                    child
                ));
            }
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let values = items.iter().map(canonical_json).collect::<Vec<_>>();
            format!("[{}]", values.join(","))
        }
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn parse_json_value(value: &str) -> Option<Value> {
    serde_json::from_str(value).ok()
}

fn compact_text(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() > 80 {
        format!("{}...", &collapsed[..77])
    } else {
        collapsed
    }
}

fn first_string_for_keys(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = first_string_for_key(value, key) {
            return Some(found);
        }
    }
    None
}

fn first_string_for_key(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(string) = map.get(key).and_then(|value| value.as_str()) {
                return Some(string.to_string());
            }
            for child in map.values() {
                if let Some(found) = first_string_for_key(child, key) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items
            .iter()
            .find_map(|item| first_string_for_key(item, key)),
        _ => None,
    }
}

fn first_array_for_key(value: &Value, key: &str) -> Option<Vec<String>> {
    match value {
        Value::Object(map) => {
            if let Some(items) = map.get(key).and_then(|value| value.as_array()) {
                let strings = items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>();
                if !strings.is_empty() {
                    return Some(strings);
                }
            }
            for child in map.values() {
                if let Some(found) = first_array_for_key(child, key) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| first_array_for_key(item, key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::session_sources::{SessionEventKind, ToolResultMetadata};

    fn session_with_events(events: Vec<SessionEvent>) -> Session {
        let mut session = Session {
            id: "session-1".to_string(),
            date: "2026-03-08".to_string(),
            timestamp: "2026-03-08T12:00:00Z".to_string(),
            cwd: "/workspace".to_string(),
            summary: "summary".to_string(),
            first_user_prompt: None,
            source_file: PathBuf::from("/tmp/session.jsonl"),
            source_label: "Codex CLI".to_string(),
            analytics: SessionAnalytics::default(),
            events,
        };
        session.analytics = analyze(&session);
        session
    }

    fn tool_call(
        timestamp: &str,
        call_id: &str,
        tool_name: &str,
        arguments: Option<&str>,
    ) -> SessionEvent {
        SessionEvent {
            timestamp: timestamp.to_string(),
            payload_type: "function_call".to_string(),
            event_kind: SessionEventKind::ToolCall,
            call_id: Some(call_id.to_string()),
            tool_name: Some(tool_name.to_string()),
            arguments: arguments.map(ToString::to_string),
            output: None,
            result_metadata: None,
            content_texts: Vec::new(),
        }
    }

    fn tool_result(
        timestamp: &str,
        call_id: &str,
        exit_code: Option<i32>,
        output: Option<&str>,
    ) -> SessionEvent {
        SessionEvent {
            timestamp: timestamp.to_string(),
            payload_type: "function_call_output".to_string(),
            event_kind: SessionEventKind::ToolResult,
            call_id: Some(call_id.to_string()),
            tool_name: None,
            arguments: None,
            output: output.map(ToString::to_string),
            result_metadata: Some(ToolResultMetadata {
                exit_code,
                duration_seconds: Some(0.2),
            }),
            content_texts: Vec::new(),
        }
    }

    #[test]
    fn clean_zero_exit_code_counts_as_success() {
        let session = session_with_events(vec![
            tool_call(
                "1",
                "call-1",
                "shell",
                Some(r#"{"command":["bash","-lc","ls"]}"#),
            ),
            tool_result("2", "call-1", Some(0), Some("ok")),
        ]);

        assert_eq!(session.analytics.total_tool_calls, 1);
        assert_eq!(session.analytics.successful_tool_calls, 1);
        assert_eq!(session.analytics.failed_tool_calls, 0);
    }

    #[test]
    fn non_zero_exit_code_counts_as_failure() {
        let session = session_with_events(vec![
            tool_call("1", "call-1", "shell", Some(r#"{"command":"ls"}"#)),
            tool_result("2", "call-1", Some(1), Some("failed")),
        ]);

        assert_eq!(session.analytics.failed_tool_calls, 1);
    }

    #[test]
    fn operation_not_permitted_counts_as_failure_even_with_zero_exit_code() {
        let session = session_with_events(vec![
            tool_call("1", "call-1", "shell", Some(r#"{"command":"ls"}"#)),
            tool_result(
                "2",
                "call-1",
                Some(0),
                Some("/bin/ps: Operation not permitted"),
            ),
        ]);

        assert_eq!(session.analytics.failed_tool_calls, 1);
    }

    #[test]
    fn missing_result_counts_as_unknown() {
        let session = session_with_events(vec![tool_call(
            "1",
            "call-1",
            "shell",
            Some(r#"{"command":"ls"}"#),
        )]);

        assert_eq!(session.analytics.unknown_outcome_tool_calls, 1);
    }

    #[test]
    fn mcp_calls_are_counted() {
        let session = session_with_events(vec![
            tool_call(
                "1",
                "call-1",
                "mcp__context7__query-docs",
                Some(r#"{"libraryId":"/mongodb/docs","query":"auth"}"#),
            ),
            tool_result("2", "call-1", Some(0), Some("ok")),
        ]);

        assert_eq!(session.analytics.mcp_tool_calls, 1);
        assert_eq!(session.analytics.external_lookup_calls, 1);
    }

    #[test]
    fn web_search_records_external_resource() {
        let session = session_with_events(vec![
            tool_call(
                "1",
                "call-1",
                "web.search_query",
                Some(r#"{"search_query":[{"q":"rust iterators"}]}"#),
            ),
            tool_result("2", "call-1", Some(0), Some("ok")),
        ]);

        assert_eq!(session.analytics.external_lookup_calls, 1);
        assert_eq!(session.analytics.external_resources.len(), 1);
        assert_eq!(
            session.analytics.external_resources[0].label,
            "rust iterators"
        );
    }

    #[test]
    fn repeated_external_lookups_increment_count() {
        let session = session_with_events(vec![
            tool_call(
                "1",
                "call-1",
                "web.search_query",
                Some(r#"{"search_query":[{"q":"rust iterators"}]}"#),
            ),
            tool_result("2", "call-1", Some(0), Some("ok")),
            tool_call(
                "3",
                "call-2",
                "web.search_query",
                Some(r#"{"search_query":[{"q":"rust iterators"}]}"#),
            ),
            tool_result("4", "call-2", Some(0), Some("ok")),
        ]);

        assert_eq!(session.analytics.external_resources.len(), 1);
        assert_eq!(session.analytics.external_resources[0].count, 2);
    }

    #[test]
    fn changed_shell_args_after_failure_counts_as_adjustment() {
        let session = session_with_events(vec![
            tool_call(
                "1",
                "call-1",
                "exec_command",
                Some(r#"{"cmd":"ls missing","workdir":"/workspace"}"#),
            ),
            tool_result("2", "call-1", Some(1), Some("no file")),
            tool_call(
                "3",
                "call-2",
                "exec_command",
                Some(r#"{"cmd":"ls src","workdir":"/workspace"}"#),
            ),
        ]);

        assert_eq!(session.analytics.adjust_course_count, 1);
        assert_eq!(
            session.analytics.adjustments[0].kind,
            AdjustmentKind::RetryWithDifferentArguments
        );
        assert_eq!(
            session.analytics.adjustments[0]
                .from_argument_summary
                .as_deref(),
            Some("cmd=ls missing | workdir=/workspace")
        );
        assert_eq!(
            session.analytics.adjustments[0]
                .to_argument_summary
                .as_deref(),
            Some("cmd=ls src | workdir=/workspace")
        );
    }

    #[test]
    fn pivot_after_failure_counts_as_adjustment() {
        let session = session_with_events(vec![
            tool_call(
                "1",
                "call-1",
                "exec_command",
                Some(r#"{"cmd":"cat missing","workdir":"/workspace"}"#),
            ),
            tool_result("2", "call-1", Some(1), Some("missing")),
            tool_call(
                "3",
                "call-2",
                "web.search_query",
                Some(r#"{"search_query":[{"q":"rust cat file"}]}"#),
            ),
        ]);

        assert_eq!(session.analytics.adjust_course_count, 1);
        assert_eq!(
            session.analytics.adjustments[0].kind,
            AdjustmentKind::PostFailurePivot
        );
        assert_eq!(
            session.analytics.adjustments[0]
                .to_argument_summary
                .as_deref(),
            Some("rust cat file")
        );
        assert_eq!(
            describe_adjustment(&session.analytics.adjustments[0]),
            "exec_command -> web.search_query (pivot after failure): cmd=cat missing | workdir=/workspace -> rust cat file"
        );
    }

    #[test]
    fn identical_retry_after_failure_is_not_counted_as_adjustment() {
        let session = session_with_events(vec![
            tool_call("1", "call-1", "shell", Some(r#"{"command":"ls missing"}"#)),
            tool_result("2", "call-1", Some(1), Some("missing")),
            tool_call("3", "call-2", "shell", Some(r#"{"command":"ls missing"}"#)),
        ]);

        assert_eq!(session.analytics.adjust_course_count, 0);
    }
}
