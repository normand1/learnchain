use crate::{
    llm::{
        DeepDiveArtifactMetadata, DeepDiveDocument, DeepDiveHistoryEntry,
        StructuredLearningResponse,
    },
    markdown_rules::MarkdownRules,
    session_sources::SessionEvent,
};
use chrono::{DateTime, Utc};
use serde_json::from_str;
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

const DEEP_DIVE_DIRNAME: &str = "deep-dives";
const DEEP_DIVE_ARTIFACT_TYPE: &str = "session_deep_dive";
const LEARNING_RESPONSE_PREFIX: &str = "learning-response-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryArtifactKind {
    DeepDive,
    Quiz,
}

impl LibraryArtifactKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::DeepDive => "Deep Dive",
            Self::Quiz => "Quiz",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LearningArtifactHistoryEntry {
    pub file_modified_at: Option<String>,
    pub session_date: String,
    pub knowledge_group_count: usize,
    pub question_count: usize,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum LibraryArtifactEntry {
    DeepDive(Box<DeepDiveHistoryEntry>),
    Quiz(LearningArtifactHistoryEntry),
}

impl LibraryArtifactEntry {
    pub fn sort_key(&self) -> String {
        match self {
            Self::DeepDive(entry) => history_sort_key(entry),
            Self::Quiz(entry) => entry.file_modified_at.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug)]
pub struct OutputManager {
    root: PathBuf,
}

#[derive(Debug)]
pub struct SummaryArtifact {
    pub path: Option<PathBuf>,
    pub content: String,
    pub error: Option<String>,
}

impl Default for OutputManager {
    fn default() -> Self {
        Self::with_root("output")
    }
}

impl OutputManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_root<P: Into<PathBuf>>(root: P) -> Self {
        Self { root: root.into() }
    }

    pub fn write_markdown_summary(
        &self,
        events: &[SessionEvent],
        session_date: &str,
        latest_file: Option<&Path>,
        persist: bool,
    ) -> SummaryArtifact {
        let mut error: Option<String> = None;
        let mut target_path: Option<PathBuf> = None;

        if persist {
            match self.output_directory() {
                Ok(dir) => {
                    if let Err(err) = fs::create_dir_all(&dir) {
                        error = Some(format!("{}: {}", dir.display(), err));
                    } else {
                        let filename = latest_file
                            .and_then(|path| path.file_stem())
                            .and_then(|stem| stem.to_str())
                            .map(|stem| format!("{stem}.md"))
                            .unwrap_or_else(|| format!("session-{}.md", session_date));

                        let mut candidate = dir;
                        candidate.push(filename);
                        target_path = Some(candidate);
                    }
                }
                Err(err) => {
                    error = Some(err);
                }
            }
        }

        let mut document = format!("# Session Output - {}\n\n", session_date);
        let mut had_content = false;
        let rules = MarkdownRules::default();
        let selected_events = rules.select_events(events);
        for event in &selected_events {
            had_content = true;
            document.push_str(&format!(
                "## {} - {}\n\n",
                event.timestamp, event.payload_type
            ));
            for text in &event.content_texts {
                document.push_str(text);
                document.push_str("\n\n");
            }
            let arguments_text = event
                .arguments
                .as_ref()
                .filter(|value| !value.trim().is_empty());
            let output_text = event
                .output
                .as_ref()
                .filter(|value| !value.trim().is_empty());

            if event.payload_type == "function_call" {
                if let Some(arguments) = arguments_text {
                    document.push_str("Arguments:\n");
                    document.push_str(arguments);
                    document.push_str("\n\n");
                } else if let Some(output) = output_text {
                    document.push_str("Output:\n");
                    document.push_str(output);
                    document.push_str("\n\n");
                }
            } else if let Some(output) = output_text {
                document.push_str("Output:\n");
                document.push_str(output);
                document.push_str("\n\n");
            }
        }

        if !had_content {
            document.push_str("_No event content, arguments, or output available._\n");
        } else if selected_events.len() == rules.max_events()
            && selected_events.len() < events.len()
        {
            document.push_str(&format!(
                "_Limited to the first {} matching events._\n",
                rules.max_events()
            ));
        }

        let mut written_path = None;
        if let Some(path) = target_path {
            match fs::write(&path, &document) {
                Ok(_) => {
                    written_path = Some(path);
                }
                Err(err) => {
                    error = Some(format!("{}: {}", path.display(), err));
                }
            }
        }

        SummaryArtifact {
            path: written_path,
            content: document,
            error,
        }
    }

    pub fn output_directory(&self) -> Result<PathBuf, String> {
        if self.root.is_absolute() {
            return Ok(self.root.clone());
        }

        match env::current_dir() {
            Ok(mut dir) => {
                dir.push(&self.root);
                Ok(dir)
            }
            Err(err) => Err(format!("failed to resolve current directory: {}", err)),
        }
    }

    pub fn deep_dive_directory(&self) -> Result<PathBuf, String> {
        let mut dir = self.output_directory()?;
        dir.push(DEEP_DIVE_DIRNAME);
        Ok(dir)
    }

    pub fn write_deep_dive_markdown(
        &self,
        metadata: &DeepDiveArtifactMetadata,
        markdown: &str,
    ) -> Result<DeepDiveDocument, String> {
        let dir = self.deep_dive_directory()?;
        fs::create_dir_all(&dir)
            .map_err(|err| format!("failed to create {}: {}", dir.display(), err))?;

        let filename = format!(
            "deep-dive-{}-{}.md",
            sanitize_filename(&metadata.generated_at),
            sanitize_filename(&metadata.session_id)
        );
        let path = dir.join(filename);
        let contents = format!(
            "+++\n{}+++\n\n{}",
            toml::to_string(metadata)
                .map_err(|err| format!("failed to serialize deep-dive metadata: {}", err))?,
            markdown
        );

        fs::write(&path, contents)
            .map_err(|err| format!("failed to write deep dive to {}: {}", path.display(), err))?;

        Ok(DeepDiveDocument {
            metadata: metadata.clone(),
            markdown: markdown.to_string(),
            path,
        })
    }

    pub fn list_deep_dive_artifacts(&self) -> Result<Vec<DeepDiveHistoryEntry>, String> {
        let dir = self.deep_dive_directory()?;
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        let read_dir = fs::read_dir(&dir)
            .map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
        for entry in read_dir {
            let entry = entry
                .map_err(|err| format!("failed to read entry in {}: {}", dir.display(), err))?;
            let path = entry.path();
            if !is_markdown(&path) {
                continue;
            }

            let metadata = entry.metadata().map_err(|err| {
                format!("failed to read metadata for {}: {}", path.display(), err)
            })?;
            let modified = metadata.modified().ok();
            let document = self.read_deep_dive_markdown(&path).unwrap_or_else(|_| {
                let fallback = fallback_deep_dive_metadata(&path, modified);
                DeepDiveDocument {
                    metadata: fallback,
                    markdown: String::new(),
                    path: path.clone(),
                }
            });
            entries.push(DeepDiveHistoryEntry {
                file_modified_at: modified.and_then(format_system_time),
                metadata: document.metadata,
                path,
            });
        }

        entries.sort_by(|a, b| {
            let left = history_sort_key(a);
            let right = history_sort_key(b);
            right.cmp(&left)
        });
        Ok(entries)
    }

    pub fn list_learning_artifacts(&self) -> Result<Vec<LearningArtifactHistoryEntry>, String> {
        let dir = self.output_directory()?;
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        let read_dir = fs::read_dir(&dir)
            .map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
        for entry in read_dir {
            let entry = entry
                .map_err(|err| format!("failed to read entry in {}: {}", dir.display(), err))?;
            let path = entry.path();
            if !is_learning_artifact(&path) {
                continue;
            }

            let metadata = entry.metadata().map_err(|err| {
                format!("failed to read metadata for {}: {}", path.display(), err)
            })?;
            let modified = metadata.modified().ok();
            let response = self.read_learning_response(&path).ok();
            let (knowledge_group_count, question_count) = response
                .as_ref()
                .map(|response| {
                    let group_count = response.response.len();
                    let question_count =
                        response.response.iter().map(|group| group.quiz.len()).sum();
                    (group_count, question_count)
                })
                .unwrap_or((0, 0));

            entries.push(LearningArtifactHistoryEntry {
                file_modified_at: modified.and_then(format_system_time),
                session_date: derive_learning_session_date(&path),
                knowledge_group_count,
                question_count,
                path,
            });
        }

        entries.sort_by(|a, b| {
            let left = a.file_modified_at.clone().unwrap_or_default();
            let right = b.file_modified_at.clone().unwrap_or_default();
            right.cmp(&left)
        });
        Ok(entries)
    }

    pub fn list_library_artifacts(&self) -> Result<Vec<LibraryArtifactEntry>, String> {
        let mut entries: Vec<LibraryArtifactEntry> = self
            .list_deep_dive_artifacts()?
            .into_iter()
            .map(|entry| LibraryArtifactEntry::DeepDive(Box::new(entry)))
            .collect();
        entries.extend(
            self.list_learning_artifacts()?
                .into_iter()
                .map(LibraryArtifactEntry::Quiz),
        );
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.sort_key()));
        Ok(entries)
    }

    pub fn read_deep_dive_markdown(&self, path: &Path) -> Result<DeepDiveDocument, String> {
        let contents = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        let (metadata, markdown) = parse_deep_dive_contents(path, &contents)?;
        Ok(DeepDiveDocument {
            metadata,
            markdown,
            path: path.to_path_buf(),
        })
    }

    pub fn read_learning_response(
        &self,
        path: &Path,
    ) -> Result<StructuredLearningResponse, String> {
        let contents = fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
        from_str(&contents).map_err(|err| {
            format!(
                "failed to parse learning response at {}: {}",
                path.display(),
                err
            )
        })
    }
}

fn parse_deep_dive_contents(
    path: &Path,
    contents: &str,
) -> Result<(DeepDiveArtifactMetadata, String), String> {
    let trimmed = contents.strip_prefix("+++\n");
    if let Some(remainder) = trimmed
        && let Some((front_matter, markdown)) = remainder.split_once("\n+++\n")
    {
        let mut metadata: DeepDiveArtifactMetadata =
            toml::from_str(front_matter).map_err(|err| {
                format!(
                    "failed to parse deep-dive front matter in {}: {}",
                    path.display(),
                    err
                )
            })?;
        if metadata.artifact_type.trim().is_empty() {
            metadata.artifact_type = DEEP_DIVE_ARTIFACT_TYPE.to_string();
        }
        return Ok((metadata, markdown.to_string()));
    }

    Ok((
        fallback_deep_dive_metadata(
            path,
            path.metadata().ok().and_then(|meta| meta.modified().ok()),
        ),
        contents.to_string(),
    ))
}

fn fallback_deep_dive_metadata(
    path: &Path,
    modified: Option<SystemTime>,
) -> DeepDiveArtifactMetadata {
    DeepDiveArtifactMetadata {
        artifact_type: DEEP_DIVE_ARTIFACT_TYPE.to_string(),
        title: path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Deep Dive")
            .to_string(),
        generated_at: modified.and_then(format_system_time).unwrap_or_default(),
        session_source: String::new(),
        session_id: String::new(),
        session_timestamp: String::new(),
        session_date: String::new(),
        project_name: String::new(),
        project_cwd: String::new(),
        source_file: String::new(),
        referenced_url_count: 0,
        reviewed_url_count: 0,
    }
}

fn format_system_time(time: SystemTime) -> Option<String> {
    let timestamp: DateTime<Utc> = time.into();
    Some(timestamp.to_rfc3339())
}

fn history_sort_key(entry: &DeepDiveHistoryEntry) -> String {
    if !entry.metadata.generated_at.trim().is_empty() {
        return entry.metadata.generated_at.clone();
    }
    entry.file_modified_at.clone().unwrap_or_default()
}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("md")
    )
}

fn is_learning_artifact(path: &Path) -> bool {
    let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    filename.starts_with(LEARNING_RESPONSE_PREFIX)
        && matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some(ext) if ext.eq_ignore_ascii_case("json")
        )
}

fn derive_learning_session_date(path: &Path) -> String {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return String::new();
    };
    let Some(remainder) = stem.strip_prefix(LEARNING_RESPONSE_PREFIX) else {
        return String::new();
    };
    remainder.split('-').take(3).collect::<Vec<_>>().join("-")
}

fn sanitize_filename(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch,
            _ => '-',
        })
        .collect();
    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "artifact".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_replaces_special_characters() {
        assert_eq!(
            sanitize_filename("2026-03-06T10:00:00Z"),
            "2026-03-06T10-00-00Z"
        );
        assert_eq!(sanitize_filename(""), "artifact");
    }
}
