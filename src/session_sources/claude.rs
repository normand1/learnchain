use super::{SessionEvent, SessionSource, append_error};
use chrono::{DateTime, Local};
use serde::Deserialize;
use serde_json::Value;
use std::{
    env,
    fs::{self, File, Metadata},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct ClaudeCodeSource {
    label: String,
    root_dir: PathBuf,
}

impl ClaudeCodeSource {
    pub(crate) fn default() -> Self {
        Self::with_root(default_claude_projects_root())
    }

    #[allow(dead_code)]
    pub(crate) fn with_root(root_dir: PathBuf) -> Self {
        Self {
            label: "Claude Code".to_string(),
            root_dir,
        }
    }

    fn find_latest_recursively(&self, root: &Path) -> (Option<PathBuf>, Option<String>) {
        let mut entry_error: Option<String> = None;
        let mut latest: Option<(SystemTime, PathBuf)> = None;
        let mut stack = vec![root.to_path_buf()];

        while let Some(dir) = stack.pop() {
            match fs::read_dir(&dir) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(entry) => match entry.metadata() {
                                Ok(metadata) => {
                                    let path = entry.path();
                                    if metadata.is_dir() {
                                        stack.push(path);
                                        continue;
                                    }
                                    if !is_claude_session_log_file(&path, &metadata) {
                                        continue;
                                    }
                                    let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
                                    let replace = latest
                                        .as_ref()
                                        .map(|(time, _)| modified > *time)
                                        .unwrap_or(true);
                                    if replace {
                                        latest = Some((modified, path));
                                    }
                                }
                                Err(err) => {
                                    append_error(
                                        &mut entry_error,
                                        format!(
                                            "{} ({}): {}",
                                            dir.display(),
                                            entry.file_name().to_string_lossy(),
                                            err
                                        ),
                                    );
                                }
                            },
                            Err(err) => {
                                append_error(
                                    &mut entry_error,
                                    format!("{}: {}", dir.display(), err),
                                );
                            }
                        }
                    }
                }
                Err(err) => {
                    append_error(&mut entry_error, format!("{}: {}", dir.display(), err));
                }
            }
        }

        (latest.map(|(_, path)| path), entry_error)
    }

    fn find_all_files_recursively(
        &self,
        root: &Path,
        limit: usize,
    ) -> (Vec<PathBuf>, Option<String>) {
        let mut entry_error: Option<String> = None;
        let mut all_files: Vec<(SystemTime, PathBuf)> = Vec::new();
        let mut stack = vec![root.to_path_buf()];

        while let Some(dir) = stack.pop() {
            match fs::read_dir(&dir) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(entry) => match entry.metadata() {
                                Ok(metadata) => {
                                    let path = entry.path();
                                    if metadata.is_dir() {
                                        stack.push(path);
                                        continue;
                                    }
                                    if !is_claude_session_log_file(&path, &metadata) {
                                        continue;
                                    }
                                    let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
                                    all_files.push((modified, path));
                                }
                                Err(err) => {
                                    append_error(
                                        &mut entry_error,
                                        format!(
                                            "{} ({}): {}",
                                            dir.display(),
                                            entry.file_name().to_string_lossy(),
                                            err
                                        ),
                                    );
                                }
                            },
                            Err(err) => {
                                append_error(
                                    &mut entry_error,
                                    format!("{}: {}", dir.display(), err),
                                );
                            }
                        }
                    }
                }
                Err(err) => {
                    append_error(&mut entry_error, format!("{}: {}", dir.display(), err));
                }
            }
        }

        // Sort by modification time, most recent first
        all_files.sort_by(|a, b| b.0.cmp(&a.0));

        // Limit to specified count
        let files: Vec<PathBuf> = all_files
            .into_iter()
            .take(limit)
            .map(|(_, path)| path)
            .collect();

        (files, entry_error)
    }
}

impl SessionSource for ClaudeCodeSource {
    fn label(&self) -> &str {
        &self.label
    }

    fn session_dir(&self, _now: DateTime<Local>) -> PathBuf {
        self.root_dir.clone()
    }

    fn find_latest_file(&self, session_dir: &Path) -> (Option<PathBuf>, Option<String>) {
        if !session_dir.exists() {
            let message = format!("{}: directory not found", session_dir.display());
            return (None, Some(message));
        }
        self.find_latest_recursively(session_dir)
    }

    fn find_all_files(&self, session_dir: &Path) -> (Vec<PathBuf>, Option<String>) {
        if !session_dir.exists() {
            let message = format!("{}: directory not found", session_dir.display());
            return (Vec::new(), Some(message));
        }
        // Limit to 50 most recent files to avoid loading too much data
        self.find_all_files_recursively(session_dir, 50)
    }

    fn parse_events(&self, path: &Path) -> (Vec<SessionEvent>, Option<String>) {
        parse_claude_session_file(path)
    }
}

fn default_claude_projects_root() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("~"))
        .join(".claude")
        .join("projects")
}

pub(crate) fn parse_claude_session_file(path: &Path) -> (Vec<SessionEvent>, Option<String>) {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => return (Vec::new(), Some(format!("{}: {}", path.display(), err))),
    };

    let reader = BufReader::new(file);
    let mut events = Vec::new();
    let mut issues: Vec<String> = Vec::new();

    for (idx, line) in reader.lines().enumerate() {
        let content = match line {
            Ok(line) => line,
            Err(err) => {
                return (
                    events,
                    Some(format!("{} (line {}): {}", path.display(), idx + 1, err)),
                );
            }
        };

        if content.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<ClaudeRawEvent>(&content) {
            Ok(raw) => {
                let timestamp = raw.timestamp.unwrap_or_else(|| "<unknown>".to_string());
                let cwd = raw.cwd;
                let session_id = raw.session_id;
                let git_branch = raw.git_branch;
                if let Some(message) = raw.message {
                    let base_call_id = message.id.clone();
                    let model = message.model.clone();
                    let role = message.role.clone();

                    match message.content {
                        // Handle simple string content (initial user messages)
                        ClaudeMessageContent::Text(text) => {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                let mut content_texts = vec![trimmed.to_string()];
                                if let Some(role) = role.as_deref() {
                                    content_texts.push(format!("role: {}", role));
                                }
                                if let Some(ref cwd_value) = cwd {
                                    content_texts.push(format!("cwd: {}", cwd_value));
                                }
                                if let Some(ref branch) = git_branch {
                                    content_texts.push(format!("branch: {}", branch));
                                }
                                if let Some(ref session) = session_id {
                                    content_texts.push(format!("session: {}", session));
                                }

                                events.push(SessionEvent {
                                    timestamp: timestamp.clone(),
                                    payload_type: "user_prompt".to_string(),
                                    call_id: base_call_id,
                                    arguments: None,
                                    output: None,
                                    content_texts,
                                });
                            }
                        }
                        // Handle array of content blocks (assistant messages)
                        ClaudeMessageContent::Blocks(contents) => {
                            for content in contents {
                                if !content.is_relevant() {
                                    continue;
                                }
                                let payload_type = content.payload_label();
                                let call_id = content.id.clone().or_else(|| base_call_id.clone());
                                let arguments =
                                    content.input.clone().map(SessionEvent::format_value);
                                let mut content_texts = Vec::new();
                                if let Some(name) = content.name.as_deref() {
                                    content_texts.push(format!("tool: {}", name));
                                }
                                if let Some(text) = content
                                    .text
                                    .as_deref()
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                {
                                    content_texts.push(text.to_string());
                                }
                                if let Some(role) = role.as_deref() {
                                    content_texts.push(format!("role: {}", role));
                                }
                                if let Some(ref cwd_value) = cwd {
                                    content_texts.push(format!("cwd: {}", cwd_value));
                                }
                                if let Some(ref branch) = git_branch {
                                    content_texts.push(format!("branch: {}", branch));
                                }
                                if let Some(ref session) = session_id {
                                    content_texts.push(format!("session: {}", session));
                                }
                                if let Some(model) = model.as_deref() {
                                    content_texts.push(format!("model: {}", model));
                                }

                                events.push(SessionEvent {
                                    timestamp: timestamp.clone(),
                                    payload_type,
                                    call_id,
                                    arguments,
                                    output: None,
                                    content_texts,
                                });
                            }
                        }
                        ClaudeMessageContent::None => {}
                    }
                }
            }
            Err(err) => issues.push(format!("{}:#{}: {}", path.display(), idx + 1, err)),
        }
    }

    let error = if issues.is_empty() {
        None
    } else {
        Some(issues.join(" | "))
    };

    (events, error)
}

fn is_claude_session_log_file(path: &Path, metadata: &Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("jsonl")
            || ext.eq_ignore_ascii_case("json")
    )
}

#[derive(Debug, Deserialize)]
struct ClaudeRawEvent {
    timestamp: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    message: Option<ClaudeMessage>,
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    role: Option<String>,
    model: Option<String>,
    #[serde(default)]
    content: ClaudeMessageContent,
}

/// Message content can be either a simple string (user messages) or an array of content blocks
#[derive(Debug, Default)]
enum ClaudeMessageContent {
    #[default]
    None,
    Text(String),
    Blocks(Vec<ClaudeContent>),
}

impl<'de> serde::Deserialize<'de> for ClaudeMessageContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, Visitor};

        struct ContentVisitor;

        impl<'de> Visitor<'de> for ContentVisitor {
            type Value = ClaudeMessageContent;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string or array of content blocks")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(ClaudeMessageContent::Text(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(ClaudeMessageContent::Text(value))
            }

            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let blocks = Vec::deserialize(de::value::SeqAccessDeserializer::new(seq))?;
                Ok(ClaudeMessageContent::Blocks(blocks))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(ClaudeMessageContent::None)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(ClaudeMessageContent::None)
            }
        }

        deserializer.deserialize_any(ContentVisitor)
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeContent {
    #[serde(rename = "type")]
    content_type: Option<String>,
    id: Option<String>,
    name: Option<String>,
    text: Option<String>,
    input: Option<Value>,
}

impl ClaudeContent {
    fn is_relevant(&self) -> bool {
        matches!(
            self.content_type.as_deref(),
            Some("tool_use") | Some("text")
        )
    }

    fn payload_label(&self) -> String {
        match (self.content_type.as_deref(), self.name.as_deref()) {
            (Some(content_type), Some(name)) => format!("{}: {}", content_type, name),
            (Some(content_type), None) => content_type.to_string(),
            _ => "unknown".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    #[test]
    fn parse_claude_fixture_extracts_tool_use_entries() {
        let path = fixture_path("test_fixtures/claude_code_events_sample.jsonl");
        let (events, error) = parse_claude_session_file(&path);

        // No errors expected with updated parser
        assert!(error.is_none(), "unexpected parse error: {:?}", error);

        // Find events by type
        let tool_use_events: Vec<_> = events
            .iter()
            .filter(|e| e.payload_type.starts_with("tool_use:"))
            .collect();
        let text_events: Vec<_> = events.iter().filter(|e| e.payload_type == "text").collect();
        let user_prompt_events: Vec<_> = events
            .iter()
            .filter(|e| e.payload_type == "user_prompt")
            .collect();

        assert_eq!(tool_use_events.len(), 3, "expected 3 tool_use items");
        assert!(!text_events.is_empty(), "expected at least one text event");
        assert!(
            !user_prompt_events.is_empty(),
            "expected at least one user_prompt event"
        );

        // Verify the initial user prompt was captured
        let first_user_prompt = user_prompt_events[0];
        assert!(
            first_user_prompt
                .content_texts
                .iter()
                .any(|t| t.contains("AGENTS.md")),
            "expected user prompt to contain 'AGENTS.md'"
        );

        // Verify tool_use events
        let first_tool_use = tool_use_events[0];
        assert_eq!(first_tool_use.payload_type, "tool_use: LS");
        assert_eq!(
            first_tool_use.call_id.as_deref(),
            Some("toolu_01QDbFXvHxuhvTaNYopFubX2")
        );
        let args = first_tool_use
            .arguments
            .as_deref()
            .expect("tool use should include arguments");
        assert!(args.contains("\"path\""));
        assert!(args.contains("learnchain"));
        assert!(
            first_tool_use
                .content_texts
                .iter()
                .any(|line| line.contains("tool: LS"))
        );
        assert!(
            first_tool_use
                .content_texts
                .iter()
                .any(|line| line.contains("session: 5d33cbd0-0d2f-4085-876f-40361797613e"))
        );
        assert!(
            first_tool_use
                .content_texts
                .iter()
                .any(|line| line.contains("model: claude-sonnet-4-20250514"))
        );

        let last_tool_use = tool_use_events
            .last()
            .expect("expected at least one tool_use event");
        assert!(last_tool_use.payload_type.starts_with("tool_use: Read"));
        assert!(
            last_tool_use
                .content_texts
                .iter()
                .any(|line| line.contains("tool: Read"))
        );
    }
}
