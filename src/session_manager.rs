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
pub struct SessionManager {
    root_dir: PathBuf,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::with_root(default_session_root())
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_root<P: Into<PathBuf>>(root: P) -> Self {
        Self {
            root_dir: root.into(),
        }
    }

    pub fn load_today_events(&self) -> SessionLoad {
        let now = Local::now();
        self.load_events_for(now)
    }

    fn load_events_for(&self, now: DateTime<Local>) -> SessionLoad {
        let session_date = now.format("%Y-%m-%d").to_string();

        let mut session_dir = self.root_dir.clone();
        session_dir.push(now.format("%Y").to_string());
        session_dir.push(now.format("%m").to_string());
        session_dir.push(now.format("%d").to_string());

        let (latest_file, entry_error) = self.find_latest_file(&session_dir);
        let (events, parse_error) = match latest_file.as_ref() {
            Some(path) => Self::parse_session_file(path),
            None => (Vec::new(), None),
        };

        let error = merge_errors(entry_error, parse_error);

        SessionLoad {
            session_date,
            session_dir,
            latest_file,
            events,
            error,
        }
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
                                if !Self::is_session_log_file(&path, &metadata) {
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

    fn parse_session_file(path: &Path) -> (Vec<SessionEvent>, Option<String>) {
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
                                    if Self::is_relevant_payload_type(payload_type.as_str()) {
                                        let timestamp = raw
                                            .timestamp
                                            .unwrap_or_else(|| "<unknown>".to_string());
                                        let formatted_output =
                                            output.map(SessionEvent::format_value);
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
                        Err(err) => {
                            issues.push(format!("{}:#{}: {}", path.display(), idx + 1, err))
                        }
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

    fn is_session_log_file(path: &Path, metadata: &Metadata) -> bool {
        if !metadata.is_file() {
            return false;
        }
        matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some(ext) if ext.eq_ignore_ascii_case("jsonl")
        )
    }
}

fn default_session_root() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("~"))
        .join(".codex")
        .join("sessions")
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

#[derive(Debug)]
pub struct SessionLoad {
    pub session_date: String,
    pub session_dir: PathBuf,
    pub latest_file: Option<PathBuf>,
    pub events: Vec<SessionEvent>,
    pub error: Option<String>,
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
