use super::{
    MultiSessionLoad, Session, SessionEvent, SessionFileMetadata, SessionLoad, SessionSource,
    append_error, group_events_by_session_with_metadata, merge_errors,
};
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

#[derive(Debug, Clone, Default)]
pub(crate) struct CodexSessionFileMetadata {
    pub id: String,
    pub timestamp: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedCodexSessionFile {
    pub metadata: Option<CodexSessionFileMetadata>,
    pub events: Vec<SessionEvent>,
    pub error: Option<String>,
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

    pub(crate) fn load_latest_session(&self) -> Result<Session, String> {
        let (latest_file, traversal_error) = self.find_latest_recursively(&self.root_dir);
        let path = latest_file.ok_or_else(|| {
            traversal_error.unwrap_or_else(|| {
                format!(
                    "No Codex session files were found in {}.",
                    self.root_dir.display()
                )
            })
        })?;
        self.load_session_from_path(&path)
    }

    pub(crate) fn load_session_by_id(&self, session_id: &str) -> Result<Session, String> {
        let (path, traversal_error) =
            self.find_file_by_session_id_recursively(&self.root_dir, session_id);
        let path = path.ok_or_else(|| {
            traversal_error.unwrap_or_else(|| {
                format!("No Codex session file matched session id '{}'.", session_id)
            })
        })?;
        self.load_session_from_path(&path)
    }

    pub(crate) fn load_session_from_path(&self, path: &Path) -> Result<Session, String> {
        let parsed = parse_codex_session_file_with_metadata(path);
        let metadata = parsed.metadata.clone().ok_or_else(|| {
            format!(
                "Codex session metadata was missing from {}.",
                path.display()
            )
        })?;
        if let Some(error) = parsed.error.clone() {
            return Err(error);
        }

        let session_metadata = SessionFileMetadata {
            session_id: Some(metadata.id),
            timestamp: metadata.timestamp,
            cwd: metadata.cwd,
        };
        let sessions =
            group_events_by_session_with_metadata(parsed.events, path, Some(&session_metadata));
        sessions
            .into_iter()
            .next()
            .ok_or_else(|| format!("No Codex session content was found in {}.", path.display()))
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
                let parsed = parse_codex_session_file_with_metadata(path);
                (parsed.events, parsed.error)
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

    fn load_all(&self, now: DateTime<Local>) -> MultiSessionLoad {
        let session_dir = self.session_dir(now);
        let (files, file_error) = self.find_all_files(&session_dir);

        let mut all_sessions = Vec::new();
        let mut aggregated_error = file_error;

        for file_path in files {
            let parsed = parse_codex_session_file_with_metadata(&file_path);
            if let Some(err) = parsed.error {
                append_error(&mut aggregated_error, err);
            }

            let metadata = parsed
                .metadata
                .as_ref()
                .map(|metadata| SessionFileMetadata {
                    session_id: Some(metadata.id.clone()),
                    timestamp: metadata.timestamp.clone(),
                    cwd: metadata.cwd.clone(),
                });
            let sessions =
                group_events_by_session_with_metadata(parsed.events, &file_path, metadata.as_ref());
            all_sessions.extend(sessions);
        }

        all_sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        MultiSessionLoad {
            source: self.label().to_string(),
            sessions: all_sessions,
            error: aggregated_error,
        }
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

    fn find_file_by_session_id_recursively(
        &self,
        root: &Path,
        session_id: &str,
    ) -> (Option<PathBuf>, Option<String>) {
        let mut entry_error: Option<String> = None;
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
                                    match read_codex_session_file_metadata(&path) {
                                        Ok(Some(metadata)) if metadata.id == session_id => {
                                            return (Some(path), entry_error);
                                        }
                                        Ok(_) => {}
                                        Err(err) => append_error(&mut entry_error, err),
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
                Err(err) => append_error(&mut entry_error, format!("{}: {}", dir.display(), err)),
            }
        }

        (None, entry_error)
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
    let parsed = parse_codex_session_file_with_metadata(path);
    (parsed.events, parsed.error)
}

pub(crate) fn parse_codex_session_file_with_metadata(path: &Path) -> ParsedCodexSessionFile {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => {
            return ParsedCodexSessionFile {
                metadata: None,
                events: Vec::new(),
                error: Some(format!("{}: {}", path.display(), err)),
            };
        }
    };

    let reader = BufReader::new(file);
    let mut events = Vec::new();
    let mut issues: Vec<String> = Vec::new();
    let mut metadata: Option<CodexSessionFileMetadata> = None;

    for (idx, line) in reader.lines().enumerate() {
        match line {
            Ok(content) => {
                if content.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<RawEventEnvelope>(&content) {
                    Ok(raw) => {
                        if raw.event_type.as_deref() == Some("session_meta") {
                            if let Some(payload) = raw.payload {
                                match serde_json::from_value::<RawSessionMeta>(payload) {
                                    Ok(session_meta) => {
                                        metadata = Some(CodexSessionFileMetadata {
                                            id: session_meta.id,
                                            timestamp: session_meta.timestamp,
                                            cwd: session_meta.cwd,
                                        });
                                    }
                                    Err(err) => issues.push(format!(
                                        "{}:#{}: failed to parse session metadata: {}",
                                        path.display(),
                                        idx + 1,
                                        err
                                    )),
                                }
                            }
                            continue;
                        }

                        if let Some(payload) = raw
                            .payload
                            .and_then(|payload| serde_json::from_value::<RawPayload>(payload).ok())
                        {
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
                return ParsedCodexSessionFile {
                    metadata,
                    events,
                    error: Some(format!("{} (line {}): {}", path.display(), idx + 1, err)),
                };
            }
        }
    }

    let error = if issues.is_empty() {
        None
    } else {
        Some(issues.join(" | "))
    };

    ParsedCodexSessionFile {
        metadata,
        events,
        error,
    }
}

fn read_codex_session_file_metadata(
    path: &Path,
) -> Result<Option<CodexSessionFileMetadata>, String> {
    let file = File::open(path).map_err(|err| format!("{}: {}", path.display(), err))?;
    let reader = BufReader::new(file);

    for (idx, line) in reader.lines().enumerate() {
        let content =
            line.map_err(|err| format!("{} (line {}): {}", path.display(), idx + 1, err))?;
        if content.trim().is_empty() {
            continue;
        }

        let raw = serde_json::from_str::<RawEventEnvelope>(&content)
            .map_err(|err| format!("{}:#{}: {}", path.display(), idx + 1, err))?;
        if raw.event_type.as_deref() == Some("session_meta") {
            return raw
                .payload
                .map(|payload| {
                    serde_json::from_value::<RawSessionMeta>(payload)
                        .map(|metadata| CodexSessionFileMetadata {
                            id: metadata.id,
                            timestamp: metadata.timestamp,
                            cwd: metadata.cwd,
                        })
                        .map_err(|err| {
                            format!(
                                "{}:#{}: failed to parse session metadata: {}",
                                path.display(),
                                idx + 1,
                                err
                            )
                        })
                })
                .transpose();
        }
    }

    Ok(None)
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
struct RawEventEnvelope {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    event_type: Option<String>,
    payload: Option<Value>,
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
struct RawSessionMeta {
    id: String,
    timestamp: Option<String>,
    cwd: Option<String>,
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
    fn parse_codex_fixture_extracts_session_metadata() {
        let path = fixture_path("test_fixtures/codex_events_sample.jsonl");
        let parsed = parse_codex_session_file_with_metadata(&path);

        let metadata = parsed.metadata.expect("expected session metadata");
        assert_eq!(metadata.id, "0199969f-2c8a-7b70-8d01-2926e17d1fd5");
        assert_eq!(metadata.cwd.as_deref(), Some("/workspace/learnchain"));
        assert_eq!(
            metadata.timestamp.as_deref(),
            Some("2025-09-29T17:57:18.091Z")
        );
    }

    #[test]
    fn codex_session_extracts_command_summary() {
        use crate::session_sources::{SessionFileMetadata, group_events_by_session_with_metadata};

        let path = fixture_path("test_fixtures/codex_events_sample.jsonl");
        let parsed = parse_codex_session_file_with_metadata(&path);
        let metadata = parsed
            .metadata
            .as_ref()
            .map(|metadata| SessionFileMetadata {
                session_id: Some(metadata.id.clone()),
                timestamp: metadata.timestamp.clone(),
                cwd: metadata.cwd.clone(),
            });

        let sessions =
            group_events_by_session_with_metadata(parsed.events, &path, metadata.as_ref());
        assert!(!sessions.is_empty(), "expected at least one session");

        let session = &sessions[0];
        assert_eq!(session.id, "0199969f-2c8a-7b70-8d01-2926e17d1fd5");
        assert_eq!(session.cwd, "/workspace/learnchain");
        // Should extract "ls" from the command array ["bash", "-lc", "ls"]
        assert!(
            session.summary.contains("ls") || session.summary.contains("Started with"),
            "expected summary to contain command, got: {}",
            session.summary
        );
    }

    #[test]
    fn codex_source_loads_session_by_id() {
        let source = CodexCliSource::with_root(
            fixture_path("test_fixtures")
                .parent()
                .expect("fixture root")
                .join("test_fixtures"),
        );

        let session = source
            .load_session_by_id("0199969f-2c8a-7b70-8d01-2926e17d1fd5")
            .expect("expected session");
        assert_eq!(session.id, "0199969f-2c8a-7b70-8d01-2926e17d1fd5");
        assert_eq!(session.cwd, "/workspace/learnchain");
        assert_eq!(
            session.source_file,
            fixture_path("test_fixtures/codex_events_sample.jsonl")
        );
    }
}
