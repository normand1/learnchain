use super::{SessionEvent, SessionLoad, SessionSource, append_error, merge_errors};
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

pub struct CodexCliSource {
    label: String,
    root_dir: PathBuf,
}

impl CodexCliSource {
    pub(crate) fn default() -> Self {
        Self::with_root(default_session_root())
    }

    pub(crate) fn with_root(root_dir: PathBuf) -> Self {
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

    fn find_all_files(&self, _session_dir: &Path) -> (Vec<PathBuf>, Option<String>) {
        // For Codex, search from the root directory recursively
        if !self.root_dir.exists() {
            let message = format!("{}: directory not found", self.root_dir.display());
            return (Vec::new(), Some(message));
        }
        // Limit to 50 most recent files to avoid loading too much data
        self.find_all_files_recursively(&self.root_dir, 50)
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
                                    if !is_codex_session_log_file(&path, &metadata) {
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

fn default_session_root() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("~"))
        .join(".codex")
        .join("sessions")
}

pub(crate) fn parse_codex_session_file(path: &Path) -> (Vec<SessionEvent>, Option<String>) {
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
                            if let Some(payload_type) = payload_type
                                && is_relevant_payload_type(payload_type.as_str())
                            {
                                let timestamp =
                                    raw.timestamp.unwrap_or_else(|| "<unknown>".to_string());
                                let formatted_output = output.map(SessionEvent::format_value);
                                let formatted_arguments = arguments.map(SessionEvent::format_value);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
    fn codex_session_extracts_command_summary() {
        use crate::session_sources::group_events_by_session;

        let path = fixture_path("test_fixtures/codex_events_sample.jsonl");
        let (events, _) = parse_codex_session_file(&path);

        let sessions = group_events_by_session(events, &path);
        assert!(!sessions.is_empty(), "expected at least one session");

        let session = &sessions[0];
        // Should extract "ls" from the command array ["bash", "-lc", "ls"]
        assert!(
            session.summary.contains("ls") || session.summary.contains("Started with"),
            "expected summary to contain command, got: {}",
            session.summary
        );
    }
}
