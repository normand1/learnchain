use color_eyre::eyre::{Context, Result, eyre};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::PathBuf,
    sync::{OnceLock, RwLock},
};

/// Globally accessible application configuration values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_max_events_value")]
    pub default_max_events: usize,
    #[serde(default = "default_min_quiz_questions_value")]
    pub min_quiz_questions: usize,
    #[serde(default = "default_session_source_kind")]
    pub session_source: SessionSourceKind,
    #[serde(default = "default_write_output_artifacts_value")]
    pub write_output_artifacts: bool,
    #[serde(default = "default_openai_model_kind")]
    pub openai_model: OpenAiModelKind,
    #[serde(default)]
    pub openai_api_key: String,
}

impl AppConfig {
    fn normalize(&mut self) {
        if self.default_max_events == 0 {
            self.default_max_events = DEFAULT_MAX_EVENTS;
        }
        if self.min_quiz_questions == 0 {
            self.min_quiz_questions = DEFAULT_MIN_QUIZ_QUESTIONS;
        }
    }

    pub fn system_prompt(&self) -> String {
        SYSTEM_PROMPT_TEMPLATE.replace("{MIN_QUIZ_QUESTIONS}", &self.min_quiz_questions.to_string())
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_max_events: DEFAULT_MAX_EVENTS,
            min_quiz_questions: DEFAULT_MIN_QUIZ_QUESTIONS,
            session_source: default_session_source_kind(),
            write_output_artifacts: default_write_output_artifacts_value(),
            openai_model: default_openai_model_kind(),
            openai_api_key: String::new(),
        }
    }
}

const DEFAULT_MAX_EVENTS: usize = 15;
const DEFAULT_MIN_QUIZ_QUESTIONS: usize = 5;
const fn default_session_source_kind() -> SessionSourceKind {
    SessionSourceKind::Codex
}
const fn default_write_output_artifacts_value() -> bool {
    false
}
const fn default_openai_model_kind() -> OpenAiModelKind {
    OpenAiModelKind::Gpt5Mini
}
const SYSTEM_PROMPT_TEMPLATE: &str = r#"You are a precise curriculum planner that helps the student learn about coding concepts.
You will produce a quiz that will teach the user about a coding concept based on the provided context.
You should base each quiz item on the provided context to help the student learn new language features or concepts.
All context examples include bash scripts. The contents of the bash updates are what should be considered for quiz updates.
Example full bash script json:
```
{'command':['bash','-lc','apply_patch <<'PATCH'
*** Begin Patch
*** Update File: src/ai_manager.rs
@@
-    #[serde(default)]
-    pub knowledge_type_language: Option<String>,
+    #[serde(default)]
+    pub knowledge_type_language: String,
 }
*** End Patch
PATCH
'],'workdir':'/Users/davidnorman/learnchain'}
```
Example subset of what should actually be considered for learning content:
```
Update File: src/ai_manager.rs
@@
-    #[serde(default)]
-    pub knowledge_type_language: Option<String>,
+    #[serde(default)]
+    pub knowledge_type_language: String,
 }
```
All questions should be language specific and should not quiz based on implementation of the specific program.
You should return a minimum of {MIN_QUIZ_QUESTIONS} quiz questions.
Return JSON that strictly matches the provided schema."#;

const CONFIG_FILE_PATH: &str = "config/app_config.toml";

static APP_CONFIG: OnceLock<RwLock<AppConfig>> = OnceLock::new();

fn config_lock() -> &'static RwLock<AppConfig> {
    APP_CONFIG.get_or_init(|| RwLock::new(AppConfig::default()))
}

/// Attempt to load configuration from disk. If loading fails, the in-memory config will be reset to defaults
/// and the error will be returned for the caller to surface if desired.
pub fn initialize() -> Result<()> {
    match load_config_from_disk() {
        Ok(config) => {
            let lock = config_lock();
            *lock.write().expect("config lock poisoned") = config;
            Ok(())
        }
        Err(err) => {
            let lock = config_lock();
            *lock.write().expect("config lock poisoned") = AppConfig::default();
            Err(err)
        }
    }
}

/// Retrieve a clone of the current configuration.
pub fn current() -> AppConfig {
    config_lock().read().expect("config lock poisoned").clone()
}

/// Convenience accessor for the configured `default_max_events` value.
pub fn default_max_events() -> usize {
    config_lock()
        .read()
        .expect("config lock poisoned")
        .default_max_events
}

/// Convenience accessor for the configured system prompt.
pub fn system_prompt() -> String {
    config_lock()
        .read()
        .expect("config lock poisoned")
        .system_prompt()
}

/// Apply the provided mutation to the in-memory configuration and persist the result to disk.
pub fn update<F>(mutator: F) -> Result<AppConfig>
where
    F: FnOnce(&mut AppConfig),
{
    let lock = config_lock();
    let mut config = lock.write().expect("config lock poisoned");
    mutator(&mut config);
    config.normalize();
    save_config_to_disk(&config)?;
    Ok(config.clone())
}

/// Absolute path to the configuration file used for persistence.
pub fn config_file_path() -> PathBuf {
    PathBuf::from(CONFIG_FILE_PATH)
}

fn load_config_from_disk() -> Result<AppConfig> {
    let path = config_file_path();
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let mut config: AppConfig = toml::from_str(&contents)
                .wrap_err_with(|| format!("failed to parse configuration at {}", path.display()))?;
            config.normalize();
            Ok(config)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(err) => Err(eyre!(format!(
            "failed to read configuration at {}: {}",
            path.display(),
            err
        ))),
    }
}

fn save_config_to_disk(config: &AppConfig) -> Result<()> {
    let path = config_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).wrap_err_with(|| {
            format!(
                "failed to create configuration directory {}",
                parent.display()
            )
        })?;
    }
    let serialized =
        toml::to_string_pretty(config).wrap_err("failed to serialize configuration to TOML")?;
    fs::write(&path, serialized)
        .wrap_err_with(|| format!("failed to write configuration to {}", path.display()))
}

const fn default_max_events_value() -> usize {
    DEFAULT_MAX_EVENTS
}

const fn default_min_quiz_questions_value() -> usize {
    DEFAULT_MIN_QUIZ_QUESTIONS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSourceKind {
    Codex,
    ClaudeCode,
}

impl SessionSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex CLI",
            Self::ClaudeCode => "Claude Code",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Codex => Self::ClaudeCode,
            Self::ClaudeCode => Self::Codex,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Codex => Self::ClaudeCode,
            Self::ClaudeCode => Self::Codex,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigField {
    MaxEvents,
    MinQuiz,
    SessionSource,
    OutputArtifacts,
    OpenAiModel,
    OpenAiKey,
}

#[derive(Debug, Clone)]
pub struct ConfigForm {
    pub(crate) max_events: usize,
    pub(crate) min_quiz_questions: usize,
    pub(crate) session_source: SessionSourceKind,
    pub(crate) write_output_artifacts: bool,
    pub(crate) openai_model: OpenAiModelKind,
    pub(crate) openai_api_key: String,
    editing_openai_key: bool,
    openai_key_buffer: String,
    field: ConfigField,
    pub(crate) dirty: bool,
    pub(crate) status: Option<String>,
}

impl ConfigForm {
    pub(crate) fn from_config(config: AppConfig) -> Self {
        Self {
            max_events: config.default_max_events,
            min_quiz_questions: config.min_quiz_questions,
            session_source: config.session_source,
            write_output_artifacts: config.write_output_artifacts,
            openai_model: config.openai_model,
            openai_api_key: config.openai_api_key,
            editing_openai_key: false,
            openai_key_buffer: String::new(),
            field: ConfigField::MaxEvents,
            dirty: false,
            status: None,
        }
    }

    pub(crate) fn selected_index(&self) -> usize {
        self.field.index()
    }

    pub(crate) fn select_next(&mut self) {
        self.field = self.field.next();
    }

    pub(crate) fn select_previous(&mut self) {
        self.field = self.field.previous();
    }

    pub(crate) fn adjust_current(&mut self, delta: isize) {
        if delta == 0 {
            return;
        }

        if matches!(self.field, ConfigField::SessionSource) {
            let updated = if delta > 0 {
                self.session_source.next()
            } else {
                self.session_source.previous()
            };
            if updated != self.session_source {
                self.session_source = updated;
                self.dirty = true;
                self.status = None;
            }
            return;
        }

        if matches!(self.field, ConfigField::OutputArtifacts) {
            let updated = !self.write_output_artifacts;
            if updated != self.write_output_artifacts {
                self.write_output_artifacts = updated;
                self.dirty = true;
                self.status = None;
            }
            return;
        }

        if matches!(self.field, ConfigField::OpenAiModel) {
            let updated = if delta > 0 {
                self.openai_model.next()
            } else {
                self.openai_model.previous()
            };
            if updated != self.openai_model {
                self.openai_model = updated;
                self.dirty = true;
                self.status = None;
            }
            return;
        }

        if matches!(self.field, ConfigField::OpenAiKey) {
            return;
        }

        let (value, minimum) = match self.field {
            ConfigField::MaxEvents => (&mut self.max_events, 1),
            ConfigField::MinQuiz => (&mut self.min_quiz_questions, 1),
            ConfigField::SessionSource
            | ConfigField::OutputArtifacts
            | ConfigField::OpenAiModel
            | ConfigField::OpenAiKey => {
                unreachable!()
            }
        };

        let current = *value as isize;
        let min_value = minimum as isize;
        let updated = (current + delta).max(min_value) as usize;

        if updated != *value {
            *value = updated;
            self.dirty = true;
            self.status = None;
        }
    }

    pub(crate) fn apply_saved(&mut self, config: AppConfig) {
        self.max_events = config.default_max_events;
        self.min_quiz_questions = config.min_quiz_questions;
        self.session_source = config.session_source;
        self.write_output_artifacts = config.write_output_artifacts;
        self.openai_model = config.openai_model;
        self.openai_api_key = config.openai_api_key;
        self.editing_openai_key = false;
        self.openai_key_buffer.clear();
        self.dirty = false;
        self.status = None;
    }

    pub(crate) fn set_status<S: Into<String>>(&mut self, status: S) {
        self.status = Some(status.into());
    }

    pub(crate) fn is_openai_key_selected(&self) -> bool {
        matches!(self.field, ConfigField::OpenAiKey)
    }

    pub(crate) fn is_editing_openai_key(&self) -> bool {
        self.editing_openai_key
    }

    pub(crate) fn start_editing_openai_key(&mut self) {
        self.editing_openai_key = true;
        self.openai_key_buffer = self.openai_api_key.clone();
        self.status = Some("Editing OpenAI API key (Enter to save, Esc to cancel)".to_string());
    }

    pub(crate) fn cancel_openai_key_edit(&mut self) {
        self.editing_openai_key = false;
        self.openai_key_buffer.clear();
        self.status = Some("Cancelled OpenAI API key edit.".to_string());
    }

    pub(crate) fn apply_openai_key_edit(&mut self) {
        let new_value = self.openai_key_buffer.trim().to_string();
        if new_value != self.openai_api_key {
            self.openai_api_key = new_value;
            self.dirty = true;
            self.status = Some("Updated OpenAI API key.".to_string());
        } else {
            self.status = Some("OpenAI API key unchanged.".to_string());
        }
        self.editing_openai_key = false;
        self.openai_key_buffer.clear();
    }

    pub(crate) fn backspace_openai_key(&mut self) {
        self.openai_key_buffer.pop();
        self.status = Some("Editing OpenAI API key...".to_string());
    }

    pub(crate) fn push_openai_key_char(&mut self, ch: char) {
        self.openai_key_buffer.push(ch);
        self.status = Some("Editing OpenAI API key...".to_string());
    }

    pub(crate) fn masked_openai_key(&self) -> String {
        mask_secret(&self.openai_api_key)
    }

    pub(crate) fn masked_openai_key_buffer(&self) -> String {
        mask_secret(&self.openai_key_buffer)
    }
}

fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        return "<not set>".to_string();
    }
    let len = value.chars().count();
    if len <= 4 {
        "****".to_string()
    } else {
        let suffix: String = value
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{}{}", "*".repeat(len.saturating_sub(4)), suffix)
    }
}

impl ConfigField {
    fn index(self) -> usize {
        match self {
            Self::MaxEvents => 0,
            Self::MinQuiz => 1,
            Self::SessionSource => 2,
            Self::OutputArtifacts => 3,
            Self::OpenAiModel => 4,
            Self::OpenAiKey => 5,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::MaxEvents => Self::MinQuiz,
            Self::MinQuiz => Self::SessionSource,
            Self::SessionSource => Self::OutputArtifacts,
            Self::OutputArtifacts => Self::OpenAiModel,
            Self::OpenAiModel => Self::OpenAiKey,
            Self::OpenAiKey => Self::MaxEvents,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::MaxEvents => Self::OpenAiKey,
            Self::MinQuiz => Self::MaxEvents,
            Self::SessionSource => Self::MinQuiz,
            Self::OutputArtifacts => Self::SessionSource,
            Self::OpenAiModel => Self::OutputArtifacts,
            Self::OpenAiKey => Self::OpenAiModel,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenAiModelKind {
    Gpt5Mini,
    Gpt5,
}

impl OpenAiModelKind {
    pub fn as_model_name(self) -> &'static str {
        match self {
            Self::Gpt5Mini => "gpt-5-mini",
            Self::Gpt5 => "gpt-5",
        }
    }

    pub fn label(self) -> &'static str {
        self.as_model_name()
    }

    pub fn next(self) -> Self {
        match self {
            Self::Gpt5Mini => Self::Gpt5,
            Self::Gpt5 => Self::Gpt5Mini,
        }
    }

    pub fn previous(self) -> Self {
        self.next()
    }
}
