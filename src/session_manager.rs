use crate::config::SessionSourceKind;
use crate::session_sources::{
    ClaudeCodeSource, CodexCliSource, MultiSessionLoad, Session, SessionLoad, SessionSource,
    append_error, merge_errors,
};
use chrono::{DateTime, Local};
use std::path::{Path, PathBuf};

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
        self.load_events_internal(now, None, None, false)
    }

    pub fn load_new_events(
        &self,
        last_file: Option<&Path>,
        last_timestamp: Option<&str>,
    ) -> SessionLoad {
        let now = Local::now();
        self.load_events_internal(now, last_file, last_timestamp, true)
    }

    pub fn load_all_sessions(&self) -> MultiSessionLoad {
        let now = Local::now();
        let mut all_sessions: Vec<Session> = Vec::new();
        let mut aggregated_error: Option<String> = None;

        for source in &self.sources {
            let load = source.load_all(now);
            all_sessions.extend(load.sessions);
            if let Some(err) = load.error {
                append_error(&mut aggregated_error, format!("{}: {}", load.source, err));
            }
        }

        // Sort all sessions by timestamp, most recent first
        all_sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        MultiSessionLoad {
            source: "all".to_string(),
            sessions: all_sessions,
            error: aggregated_error,
        }
    }

    fn load_events_internal(
        &self,
        now: DateTime<Local>,
        last_file: Option<&Path>,
        last_timestamp: Option<&str>,
        only_new: bool,
    ) -> SessionLoad {
        let mut aggregated_error: Option<String> = None;
        let mut fallback: Option<SessionLoad> = None;

        for source in &self.sources {
            let mut load = if only_new {
                source.load_since(now, last_file, last_timestamp)
            } else {
                source.load(now)
            };
            let current_error = load.error.take();
            let has_results = if only_new {
                !load.events.is_empty()
            } else {
                load.has_results()
            };
            if has_results {
                load.error = merge_errors(current_error, aggregated_error);
                return load;
            }

            if let Some(err) = current_error {
                append_error(&mut aggregated_error, format!("{}: {}", load.source, err));
            }

            if fallback.is_none() {
                fallback = Some(load);
            }
        }

        let mut load = fallback.unwrap_or_else(|| SessionLoad::empty(now, "unknown".to_string()));
        let current_error = load.error.take();
        load.error = merge_errors(current_error, aggregated_error);
        if only_new && load.events.is_empty() {
            append_error(
                &mut load.error,
                "No new session events available yet. Run another coding session before generating a new quiz.".to_string(),
            );
        }
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

impl Default for SessionManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_sources::is_timestamp_newer;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn load_new_events_returns_entries_after_marker() {
        let temp = tempdir().unwrap();
        let year_dir = temp.path().join("2025");
        let day_dir = year_dir.join("10").join("05");
        fs::create_dir_all(&day_dir).unwrap();
        let log_path = day_dir.join("session.jsonl");

        let initial_lines = [
            r#"{"timestamp":"2025-10-05T10:00:00Z","payload":{"type":"function_call","call_id":"call-1","arguments":{"cmd":"ls"}}}"#,
            r#"{"timestamp":"2025-10-05T10:05:00Z","payload":{"type":"function_call_output","call_id":"call-1","output":{"stdout":"done"}}}"#,
        ];
        let mut file = File::create(&log_path).unwrap();
        for line in &initial_lines {
            writeln!(file, "{}", line).unwrap();
        }
        drop(file);

        let manager = SessionManager::builder()
            .with_codex_cli_root(temp.path())
            .build();

        let initial = manager.load_today_events();
        assert_eq!(initial.events.len(), 2);

        let baseline = initial
            .events
            .last()
            .expect("expected events")
            .timestamp
            .clone();
        let latest_file = initial.latest_file.clone();

        let no_new = manager.load_new_events(latest_file.as_deref(), Some(baseline.as_str()));
        assert!(no_new.events.is_empty());
        assert!(matches!(
            no_new.error.as_ref(),
            Some(message) if message.contains("No new session events")
        ));

        let mut append_file = OpenOptions::new().append(true).open(&log_path).unwrap();
        writeln!(
            append_file,
            "{}",
            r#"{"timestamp":"2025-10-05T10:10:00Z","payload":{"type":"function_call","call_id":"call-2","arguments":{"cmd":"pwd"}}}"#
        )
        .unwrap();
        drop(append_file);

        let new_load = manager.load_new_events(latest_file.as_deref(), Some(baseline.as_str()));
        assert_eq!(new_load.events.len(), 1);
        let new_event = new_load.events.first().unwrap();
        assert_eq!(new_event.call_id.as_deref(), Some("call-2"));
        assert!(is_timestamp_newer(&new_event.timestamp, &baseline));
    }
}
