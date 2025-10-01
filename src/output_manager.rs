use crate::{markdown_rules::MarkdownRules, session_manager::SessionEvent};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

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
}
