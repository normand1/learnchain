use crate::config::SessionSourceKind;
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
    fn empty(now: DateTime<Local>, source: String) -> Self {
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

    fn has_results(&self) -> bool {
        self.latest_file.is_some() || !self.events.is_empty()
    }
}

#[derive(Debug)]
pub struct SessionEvent {
    pub timestamp: String,
    pub payload_type: String,
    pub call_id: Option<String>,
    pub arguments: Option<String>,
    pub output: Option<String>,
    pub content_texts: Vec<String>,
}

impl SessionEvent {
    fn format_value(value: Value) -> String {
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
}

pub struct SessionManager {
    sources: Vec<Box<dyn SessionSource>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self::builder().with_codex_cli_source().build()
    }

    pub fn from_source(source: SessionSourceKind) -> Self {
        let builder = match source {
            SessionSourceKind::Codex => SessionManager::builder().with_codex_cli_source(),
            SessionSourceKind::ClaudeCode => SessionManager::builder().with_claude_code_source(),
        };
        builder.build()
    }

    #[allow(dead_code)]
    pub fn with_root<P: Into<PathBuf>>(root: P) -> Self {
        Self::builder().with_codex_cli_root(root).build()
    }

    pub fn builder() -> SessionManagerBuilder {
        SessionManagerBuilder::new()
    }

    pub fn load_today_events(&self) -> SessionLoad {
        let now = Local::now();
        self.load_events_for(now)
    }

    fn load_events_for(&self, now: DateTime<Local>) -> SessionLoad {
        let mut aggregated_error: Option<String> = None;
        let mut fallback: Option<SessionLoad> = None;

        for source in &self.sources {
            let mut load = source.load(now.clone());
            if load.has_results() {
                let current_error = load.error.take();
                load.error = merge_errors(current_error, aggregated_error);
                return load;
            }

            if let Some(err) = load.error.take() {
                append_error(&mut aggregated_error, format!("{}: {}", load.source, err));
            }

            if fallback.is_none() {
                fallback = Some(load);
            }
        }

        let mut load =
            fallback.unwrap_or_else(|| SessionLoad::empty(now.clone(), "unknown".to_string()));
        let current_error = load.error.take();
        load.error = merge_errors(current_error, aggregated_error);
        load
    }
}

pub struct SessionManagerBuilder {
    sources: Vec<Box<dyn SessionSource>>,
}

impl SessionManagerBuilder {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn add_source<S>(mut self, source: S) -> Self
    where
        S: SessionSource + 'static,
    {
        self.sources.push(Box::new(source));
        self
    }

    pub fn with_codex_cli_source(mut self) -> Self {
        self.sources.push(Box::new(CodexCliSource::default()));
        self
    }

    pub fn with_claude_code_source(mut self) -> Self {
        self.sources.push(Box::new(ClaudeCodeSource::default()));
        self
    }

    #[allow(dead_code)]
    pub fn with_codex_cli_root<P: Into<PathBuf>>(mut self, root: P) -> Self {
        self.sources
            .push(Box::new(CodexCliSource::with_root(root.into())));
        self
    }

    pub fn build(mut self) -> SessionManager {
        if self.sources.is_empty() {
            self.sources.push(Box::new(CodexCliSource::default()));
        }
        SessionManager {
            sources: self.sources,
        }
    }
}

struct CodexCliSource {
    label: String,
    root_dir: PathBuf,
}

impl CodexCliSource {
    fn default() -> Self {
        Self::with_root(default_session_root())
    }

    fn with_root(root_dir: PathBuf) -> Self {
        Self {
            label: "Codex CLI".to_string(),
            root_dir,
        }
    }
}

impl SessionSource for CodexCliSource {
    fn load(&self, now: DateTime<Local>) -> SessionLoad {
        let (latest_file, traversal_error) = self.find_latest_recursively(&self.root_dir);
        let mut session_dir = self.root_dir.clone();
        let mut session_date = now.format("%Y-%m-%d").to_string();

        let (events, parse_error) = match latest_file.as_ref() {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    session_dir = parent.to_path_buf();
                }
                session_date = derive_codex_session_date(path).unwrap_or(session_date);
                parse_codex_session_file(path)
            }
            None => (Vec::new(), None),
        };

        SessionLoad {
            source: self.label.clone(),
            session_date,
            session_dir,
            latest_file,
            events,
            error: merge_errors(traversal_error, parse_error),
        }
    }

    fn label(&self) -> &str {
        &self.label
    }

    fn session_dir(&self, now: DateTime<Local>) -> PathBuf {
        let mut session_dir = self.root_dir.clone();
        session_dir.push(now.format("%Y").to_string());
        session_dir.push(now.format("%m").to_string());
        session_dir.push(now.format("%d").to_string());
        session_dir
    }

    fn find_latest_file(&self, session_dir: &Path) -> (Option<PathBuf>, Option<String>) {
        let mut entry_error: Option<String> = None;
        let latest_file = match fs::read_dir(session_dir) {
            Ok(entries) => {
                let mut latest: Option<(SystemTime, PathBuf)> = None;
                for entry in entries {
                    match entry {
                        Ok(entry) => match entry.metadata() {
                            Ok(metadata) => {
                                let path = entry.path();
                                if !is_codex_session_log_file(&path, &metadata) {
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
                                        session_dir.display(),
                                        entry.file_name().to_string_lossy(),
                                        err
                                    ),
                                );
                            }
                        },
                        Err(err) => {
                            append_error(
                                &mut entry_error,
                                format!("{}: {}", session_dir.display(), err),
                            );
                        }
                    }
                }
                latest.map(|(_, path)| path)
            }
            Err(err) => {
                let path_str = session_dir.display().to_string();
                return (None, Some(format!("{}: {}", path_str, err)));
            }
        };

        (latest_file, entry_error)
    }

    fn parse_events(&self, path: &Path) -> (Vec<SessionEvent>, Option<String>) {
        parse_codex_session_file(path)
    }
}

impl CodexCliSource {
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
                                    if !is_codex_session_log_file(&path, &metadata) {
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
}

struct ClaudeCodeSource {
    label: String,
    root_dir: PathBuf,
}

impl ClaudeCodeSource {
    fn default() -> Self {
        Self::with_root(default_claude_projects_root())
    }

    fn with_root(root_dir: PathBuf) -> Self {
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

    fn parse_events(&self, path: &Path) -> (Vec<SessionEvent>, Option<String>) {
        parse_claude_session_file(path)
    }
}

fn default_session_root() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("~"))
        .join(".codex")
        .join("sessions")
}

fn default_claude_projects_root() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("~"))
        .join(".claude")
        .join("projects")
}

fn merge_errors(a: Option<String>, b: Option<String>) -> Option<String> {
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

fn append_error(slot: &mut Option<String>, message: String) {
    if let Some(existing) = slot {
        existing.push_str(" | ");
        existing.push_str(&message);
    } else {
        *slot = Some(message);
    }
}

fn parse_codex_session_file(path: &Path) -> (Vec<SessionEvent>, Option<String>) {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => return (Vec::new(), Some(format!("{}: {}", path.display(), err))),
    };

    let reader = BufReader::new(file);
    let mut events = Vec::new();
    let mut issues: Vec<String> = Vec::new();

    for (idx, line) in reader.lines().enumerate() {
        match line {
            Ok(content) => {
                if content.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<RawEvent>(&content) {
                    Ok(raw) => {
                        if let Some(payload) = raw.payload {
                            let RawPayload {
                                payload_type,
                                call_id,
                                output,
                                arguments,
                                content,
                            } = payload;
                            if let Some(payload_type) = payload_type {
                                if is_relevant_payload_type(payload_type.as_str()) {
                                    let timestamp =
                                        raw.timestamp.unwrap_or_else(|| "<unknown>".to_string());
                                    let formatted_output = output.map(SessionEvent::format_value);
                                    let formatted_arguments =
                                        arguments.map(SessionEvent::format_value);
                                    let content_texts = content
                                        .unwrap_or_default()
                                        .into_iter()
                                        .filter_map(|fragment| fragment.text)
                                        .collect();

                                    events.push(SessionEvent {
                                        timestamp,
                                        payload_type,
                                        call_id,
                                        arguments: formatted_arguments,
                                        output: formatted_output,
                                        content_texts,
                                    });
                                }
                            }
                        }
                    }
                    Err(err) => issues.push(format!("{}:#{}: {}", path.display(), idx + 1, err)),
                }
            }
            Err(err) => {
                return (
                    events,
                    Some(format!("{} (line {}): {}", path.display(), idx + 1, err)),
                );
            }
        }
    }

    let error = if issues.is_empty() {
        None
    } else {
        Some(issues.join(" | "))
    };

    (events, error)
}

fn is_relevant_payload_type(payload_type: &str) -> bool {
    matches!(payload_type, "function_call" | "function_call_output")
}

fn is_codex_session_log_file(path: &Path, metadata: &Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("jsonl")
    )
}

fn derive_codex_session_date(path: &Path) -> Option<String> {
    let day = path.parent()?.file_name()?.to_str()?.to_string();
    let month = path.parent()?.parent()?.file_name()?.to_str()?.to_string();
    let year = path
        .parent()?
        .parent()?
        .parent()?
        .file_name()?
        .to_str()?
        .to_string();

    if year.len() == 4 && month.len() == 2 && day.len() == 2 {
        return Some(format!("{}-{}-{}", year, month, day));
    }

    file_modified_date(path)
}

fn file_modified_date(path: &Path) -> Option<String> {
    let metadata = path.metadata().ok()?;
    let modified = metadata.modified().ok()?;
    let datetime: DateTime<Local> = DateTime::<Local>::from(modified);
    Some(datetime.format("%Y-%m-%d").to_string())
}

fn parse_claude_session_file(path: &Path) -> (Vec<SessionEvent>, Option<String>) {
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
                    if let Some(contents) = message.content {
                        for content in contents {
                            if !content.is_relevant() {
                                continue;
                            }
                            let payload_type = content.payload_label();
                            let call_id = content.id.clone().or_else(|| base_call_id.clone());
                            let arguments = content.input.clone().map(SessionEvent::format_value);
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
struct RawEvent {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    _event_type: Option<String>,
    payload: Option<RawPayload>,
}

#[derive(Debug, Deserialize)]
struct RawPayload {
    #[serde(rename = "type")]
    payload_type: Option<String>,
    call_id: Option<String>,
    output: Option<Value>,
    arguments: Option<Value>,
    content: Option<Vec<ContentFragment>>,
}

#[derive(Debug, Deserialize)]
struct ContentFragment {
    text: Option<String>,
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
    content: Option<Vec<ClaudeContent>>,
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
        matches!(self.content_type.as_deref(), Some("tool_use"))
    }

    fn payload_label(&self) -> String {
        match (self.content_type.as_deref(), self.name.as_deref()) {
            (Some(content_type), Some(name)) => format!("{}: {}", content_type, name),
            (Some(content_type), None) => content_type.to_string(),
            _ => "tool_use".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn fixture_path(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    #[test]
    fn parse_codex_fixture_extracts_function_events() {
        let path = fixture_path("test_fixtures/codex_events_sample.jsonl");
        let (events, error) = parse_codex_session_file(&path);

        assert!(error.is_none(), "unexpected parse error: {:?}", error);
        assert_eq!(events.len(), 12, "expected function call entries");

        let first = &events[0];
        assert_eq!(first.payload_type, "function_call");
        assert_eq!(
            first.call_id.as_deref(),
            Some("call_o6cPedcTIBUW6VtobSubFUQS")
        );
        let arguments = first
            .arguments
            .as_deref()
            .expect("function call should include arguments");
        assert!(arguments.contains("\"command\""));
        assert!(arguments.contains("\"ls\""));

        let second = &events[1];
        assert_eq!(second.payload_type, "function_call_output");
        assert_eq!(
            second.call_id.as_deref(),
            Some("call_o6cPedcTIBUW6VtobSubFUQS")
        );
        let output = second.output.as_deref().expect("output should be present");
        assert!(output.contains("AGENTS.md"));
        assert!(output.contains("Cargo.toml"));
    }

    #[test]
    fn parse_claude_fixture_extracts_tool_use_entries() {
        let path = fixture_path("test_fixtures/claude_code_events_sample.jsonl");
        let (events, error) = parse_claude_session_file(&path);

        assert!(
            matches!(error.as_ref(), Some(message) if message.contains("invalid type")),
            "expected schema mismatch warning containing 'invalid type', got {:?}",
            error
        );
        assert_eq!(events.len(), 3, "expected tool_use items only");

        let first = &events[0];
        assert_eq!(first.payload_type, "tool_use: LS");
        assert_eq!(
            first.call_id.as_deref(),
            Some("toolu_01QDbFXvHxuhvTaNYopFubX2")
        );
        let args = first
            .arguments
            .as_deref()
            .expect("tool use should include arguments");
        assert!(args.contains("\"path\""));
        assert!(args.contains("learnchain"));
        assert!(
            first
                .content_texts
                .iter()
                .any(|line| line.contains("tool: LS"))
        );
        assert!(
            first
                .content_texts
                .iter()
                .any(|line| line.contains("session: 5d33cbd0-0d2f-4085-876f-40361797613e"))
        );
        assert!(
            first
                .content_texts
                .iter()
                .any(|line| line.contains("model: claude-sonnet-4-20250514"))
        );

        let last = events.last().expect("expected at least one event");
        assert!(last.payload_type.starts_with("tool_use: Read"));
        assert!(
            last.content_texts
                .iter()
                .any(|line| line.contains("tool: Read"))
        );
    }
}
