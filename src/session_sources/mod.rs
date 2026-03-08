pub mod claude;
pub mod codex;

pub use claude::ClaudeCodeSource;
pub use codex::CodexCliSource;

use chrono::{DateTime, Local, NaiveDateTime};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::session_analytics::{self, SessionAnalytics};

#[derive(Debug)]
pub struct SessionLoad {
    pub source: String,
    pub session_date: String,
    pub session_dir: PathBuf,
    pub latest_file: Option<PathBuf>,
    pub events: Vec<SessionEvent>,
    pub error: Option<String>,
}

impl SessionLoad {
    pub(crate) fn empty(now: DateTime<Local>, source: String) -> Self {
        let session_date = now.format("%Y-%m-%d").to_string();
        Self {
            source,
            session_date,
            session_dir: PathBuf::new(),
            latest_file: None,
            events: Vec::new(),
            error: None,
        }
    }

    pub(crate) fn has_results(&self) -> bool {
        self.latest_file.is_some() || !self.events.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct SessionEvent {
    pub timestamp: String,
    pub payload_type: String,
    pub event_kind: SessionEventKind,
    pub call_id: Option<String>,
    pub tool_name: Option<String>,
    pub arguments: Option<String>,
    pub output: Option<String>,
    pub result_metadata: Option<ToolResultMetadata>,
    pub content_texts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub date: String,
    pub timestamp: String,
    pub cwd: String,
    pub summary: String,
    pub first_user_prompt: Option<String>,
    pub source_file: PathBuf,
    pub source_label: String,
    pub analytics: SessionAnalytics,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SessionEventKind {
    UserPrompt,
    ToolCall,
    ToolResult,
    AgentReasoning,
    Context,
    Metric,
    Message,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ToolResultMetadata {
    pub exit_code: Option<i32>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionFileMetadata {
    pub session_id: Option<String>,
    pub timestamp: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug)]
pub struct MultiSessionLoad {
    pub source: String,
    pub sessions: Vec<Session>,
    pub error: Option<String>,
}

impl SessionEvent {
    pub(crate) fn format_value(value: Value) -> String {
        match value {
            Value::String(raw) => Self::decode_output_string(&raw),
            other => other.to_string(),
        }
    }

    fn decode_output_string(raw: &str) -> String {
        if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
            match parsed {
                Value::Object(map) => {
                    if let Some(inner) = map.get("output") {
                        match inner {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        }
                    } else {
                        Value::Object(map).to_string()
                    }
                }
                Value::String(text) => text,
                other => other.to_string(),
            }
        } else if let Ok(unescaped) = serde_json::from_str::<String>(raw) {
            unescaped
        } else {
            raw.to_string()
        }
    }
}

fn parse_timestamp_micros(value: &str) -> Option<i128> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.timestamp_micros() as i128);
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(naive.and_utc().timestamp_micros() as i128);
    }
    None
}

pub(crate) fn is_timestamp_newer(current: &str, baseline: &str) -> bool {
    match (
        parse_timestamp_micros(current),
        parse_timestamp_micros(baseline),
    ) {
        (Some(current_ts), Some(baseline_ts)) => current_ts > baseline_ts,
        _ => current > baseline,
    }
}

pub trait SessionSource {
    fn label(&self) -> &str;
    fn session_dir(&self, now: DateTime<Local>) -> PathBuf;
    fn find_latest_file(&self, session_dir: &Path) -> (Option<PathBuf>, Option<String>);
    fn parse_events(&self, path: &Path) -> (Vec<SessionEvent>, Option<String>);

    fn load(&self, now: DateTime<Local>) -> SessionLoad {
        let session_dir = self.session_dir(now);
        let session_date = now.format("%Y-%m-%d").to_string();
        let (latest_file, entry_error) = self.find_latest_file(&session_dir);
        let (events, parse_error) = match latest_file.as_ref() {
            Some(path) => self.parse_events(path),
            None => (Vec::new(), None),
        };

        SessionLoad {
            source: self.label().to_string(),
            session_date,
            session_dir,
            latest_file,
            events,
            error: merge_errors(entry_error, parse_error),
        }
    }

    fn load_since(
        &self,
        now: DateTime<Local>,
        _last_file: Option<&Path>,
        last_timestamp: Option<&str>,
    ) -> SessionLoad {
        let mut load = self.load(now);
        if let Some(marker) = last_timestamp {
            load.events
                .retain(|event| is_timestamp_newer(&event.timestamp, marker));
        }
        load
    }

    /// Find all session files in the source directory
    fn find_all_files(&self, session_dir: &Path) -> (Vec<PathBuf>, Option<String>) {
        // Default: return only latest file for backward compatibility
        let (latest, error) = self.find_latest_file(session_dir);
        (latest.into_iter().collect(), error)
    }

    /// Load all sessions from all available files
    fn load_all(&self, now: DateTime<Local>) -> MultiSessionLoad {
        let session_dir = self.session_dir(now);
        let (files, file_error) = self.find_all_files(&session_dir);

        let mut all_sessions = Vec::new();
        let mut aggregated_error = file_error;

        for file_path in files {
            let (events, parse_error) = self.parse_events(&file_path);
            if let Some(err) = parse_error {
                append_error(&mut aggregated_error, err);
            }
            let sessions = group_events_by_session(events, &file_path, self.label());
            all_sessions.extend(sessions);
        }

        // Sort all sessions by timestamp, most recent first
        all_sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        MultiSessionLoad {
            source: self.label().to_string(),
            sessions: all_sessions,
            error: aggregated_error,
        }
    }
}

pub(crate) fn merge_errors(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(mut first), Some(second)) => {
            first.push_str(" | ");
            first.push_str(&second);
            Some(first)
        }
        (Some(first), None) => Some(first),
        (None, Some(second)) => Some(second),
        (None, None) => None,
    }
}

pub(crate) fn append_error(slot: &mut Option<String>, message: String) {
    if let Some(existing) = slot {
        existing.push_str(" | ");
        existing.push_str(&message);
    } else {
        *slot = Some(message);
    }
}

pub fn group_events_by_session(
    events: Vec<SessionEvent>,
    source_file: &Path,
    source_label: &str,
) -> Vec<Session> {
    group_events_by_session_with_metadata(events, source_file, None, source_label)
}

pub fn group_events_by_session_with_metadata(
    events: Vec<SessionEvent>,
    source_file: &Path,
    metadata: Option<&SessionFileMetadata>,
    source_label: &str,
) -> Vec<Session> {
    let mut session_map: HashMap<String, Vec<SessionEvent>> = HashMap::new();
    let fallback_session_id = metadata
        .and_then(|metadata| metadata.session_id.as_deref())
        .unwrap_or("unknown")
        .to_string();

    for event in events {
        let session_id = event
            .content_texts
            .iter()
            .find(|s| s.starts_with("session: "))
            .map(|s| s.trim_start_matches("session: ").to_string())
            .unwrap_or_else(|| fallback_session_id.clone());

        session_map.entry(session_id).or_default().push(event);
    }

    if session_map.is_empty()
        && let Some(metadata) = metadata
        && metadata.session_id.is_some()
    {
        session_map.insert(fallback_session_id, Vec::new());
    }

    let mut sessions: Vec<Session> = session_map
        .into_iter()
        .map(|(id, events)| {
            let timestamp = events
                .first()
                .map(|e| e.timestamp.clone())
                .or_else(|| metadata.and_then(|metadata| metadata.timestamp.clone()))
                .unwrap_or_default();
            let date = derive_date_from_timestamp(&timestamp);
            let first_user_prompt = find_first_user_prompt(&events);
            let summary = derive_session_summary(&events, first_user_prompt.as_deref());
            let mut session = Session {
                id,
                date,
                timestamp,
                cwd: metadata
                    .and_then(|metadata| metadata.cwd.clone())
                    .unwrap_or_else(|| derive_session_cwd(&events)),
                summary,
                first_user_prompt,
                source_file: source_file.to_path_buf(),
                source_label: source_label.to_string(),
                analytics: SessionAnalytics::default(),
                events,
            };
            session.analytics = session_analytics::analyze(&session);
            session
        })
        .collect();

    // Sort by timestamp, most recent first
    sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    sessions
}

fn find_first_user_prompt(events: &[SessionEvent]) -> Option<String> {
    for event in events {
        // Check for user_prompt type (from Claude Code initial messages)
        // or messages with "role: user" in content_texts
        let is_user_message = event.event_kind == SessionEventKind::UserPrompt
            || event.payload_type == "user_prompt"
            || event.content_texts.iter().any(|t| t == "role: user");

        if is_user_message {
            // Find the actual text content (not the metadata lines)
            for text in &event.content_texts {
                if !text.starts_with("role: ")
                    && !text.starts_with("cwd: ")
                    && !text.starts_with("branch: ")
                    && !text.starts_with("session: ")
                    && !text.starts_with("model: ")
                    && !text.starts_with("tool: ")
                    && !text.is_empty()
                {
                    return Some(text.clone());
                }
            }
        }
    }
    None
}

fn derive_date_from_timestamp(timestamp: &str) -> String {
    if timestamp.len() >= 10 {
        timestamp[..10].to_string()
    } else {
        "unknown".to_string()
    }
}

fn derive_session_cwd(events: &[SessionEvent]) -> String {
    for event in events {
        if let Some(cwd) = event
            .content_texts
            .iter()
            .find(|text| text.starts_with("cwd: "))
            .map(|text| text.trim_start_matches("cwd: ").to_string())
        {
            return cwd;
        }
    }

    "Unknown".to_string()
}

fn derive_session_summary(events: &[SessionEvent], first_user_prompt: Option<&str>) -> String {
    // Use the first user prompt if available
    if let Some(prompt) = first_user_prompt {
        return prompt.to_string();
    }

    // Try to find the first tool name (Claude Code uses "tool_use:", Codex uses "function_call")
    for event in events {
        if let Some(tool_name) = event.payload_type.strip_prefix("tool_use: ") {
            return format!("Started with {}", tool_name);
        }
        if event.event_kind == SessionEventKind::ToolCall || event.payload_type == "function_call" {
            // For Codex, try to extract command from arguments
            if let Some(args) = &event.arguments {
                if let Some(cmd) = extract_command_from_args(args) {
                    return format!("Started with: {}", cmd);
                }
            }
            return "CLI session".to_string();
        }
    }
    // Fallback - don't include event count since it's shown separately
    "Session".to_string()
}

fn extract_command_from_args(args: &str) -> Option<String> {
    // Try to extract "command" field from JSON-like arguments
    let parsed = serde_json::from_str::<serde_json::Value>(args).ok()?;
    let cmd_value = parsed.get("command")?;

    let cmd_str = match cmd_value {
        // String command (simple case)
        serde_json::Value::String(s) => s.clone(),
        // Array command like ["bash", "-lc", "ls"] - extract the actual command
        serde_json::Value::Array(arr) => {
            // Find the last element that looks like an actual command
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.starts_with('-') && *s != "bash" && *s != "sh")
                .last()
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
        }
        _ => return None,
    };

    // Truncate long commands
    Some(if cmd_str.len() > 30 {
        format!("{}...", &cmd_str[..30])
    } else {
        cmd_str
    })
}
