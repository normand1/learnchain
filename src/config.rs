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
    // AI Provider selection
    #[serde(default)]
    pub ai_provider: AiProvider,
    // OpenAI settings
    #[serde(default = "default_openai_model_kind")]
    pub openai_model: OpenAiModelKind,
    #[serde(default)]
    pub openai_api_key: String,
    // Anthropic settings
    #[serde(default)]
    pub anthropic_model: AnthropicModelKind,
    #[serde(default)]
    pub anthropic_api_key: String,
    // OpenRouter settings
    #[serde(default)]
    pub openrouter_model: String,
    #[serde(default)]
    pub openrouter_api_key: String,
    // Sampling percentage for quiz generation (1-100)
    #[serde(default = "default_sampling_percentage")]
    pub sampling_percentage: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLlmConfig {
    pub provider: AiProvider,
    pub model_name: String,
    pub model_label: String,
    pub api_key: String,
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

    pub fn resolved_llm(&self) -> ResolvedLlmConfig {
        match self.ai_provider {
            AiProvider::OpenAI => ResolvedLlmConfig {
                provider: self.ai_provider,
                model_name: self.openai_model.as_model_name().to_string(),
                model_label: self.openai_model.label().to_string(),
                api_key: self.openai_api_key.clone(),
            },
            AiProvider::Anthropic => ResolvedLlmConfig {
                provider: self.ai_provider,
                model_name: self.anthropic_model.as_model_name().to_string(),
                model_label: self.anthropic_model.label().to_string(),
                api_key: self.anthropic_api_key.clone(),
            },
            AiProvider::OpenRouter => ResolvedLlmConfig {
                provider: self.ai_provider,
                model_name: self.openrouter_model.clone(),
                model_label: if self.openrouter_model.trim().is_empty() {
                    "<not set>".to_string()
                } else {
                    self.openrouter_model.clone()
                },
                api_key: self.openrouter_api_key.clone(),
            },
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_max_events: DEFAULT_MAX_EVENTS,
            min_quiz_questions: DEFAULT_MIN_QUIZ_QUESTIONS,
            session_source: default_session_source_kind(),
            write_output_artifacts: default_write_output_artifacts_value(),
            ai_provider: AiProvider::default(),
            openai_model: default_openai_model_kind(),
            openai_api_key: String::new(),
            anthropic_model: AnthropicModelKind::default(),
            anthropic_api_key: String::new(),
            openrouter_model: String::new(),
            openrouter_api_key: String::new(),
            sampling_percentage: default_sampling_percentage(),
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
const fn default_sampling_percentage() -> u8 {
    10
}
const SYSTEM_PROMPT_TEMPLATE: &str = r#"You are a precise curriculum planner that helps the student learn coding concepts from the provided session summary.
Use the session summary as the source of truth and produce a structured learning response using the fields provided by the calling system.
Base each quiz item on the relevant code changes or concepts in the summary so the student learns language features, libraries, frameworks, tools, or APIs that appeared in the session.
When the summary includes bash scripts, focus on the actual file/content changes represented inside those scripts rather than the wrapper shell command itself.
Example full bash script json:
```
{'command':['bash','-lc','apply_patch <<'PATCH'
*** Begin Patch
*** Update File: src/llm/types.rs
@@
+#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
+#[serde(deny_unknown_fields)]
+pub struct QuizOption {
+    #[serde(default)]
+    pub selection: String,
 }
*** End Patch
PATCH
'],'workdir':'/Users/davidnorman/learnchain'}
```
Example subset of what should actually be considered for learning content:
```
Update File: src/llm/types.rs
@@
+#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
+#[serde(deny_unknown_fields)]
+pub struct QuizOption {
+    #[serde(default)]
+    pub selection: String,
 }
```
All questions should be language specific and should not quiz the user on the implementation details of this application itself.
Return a minimum of {MIN_QUIZ_QUESTIONS} quiz questions overall.
Fill every required field in the structured response."#;

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
#[allow(dead_code)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    #[default]
    OpenAI,
    Anthropic,
    OpenRouter,
}

impl AiProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::OpenRouter => "OpenRouter",
        }
    }

    pub fn missing_key_help(self) -> &'static str {
        match self {
            Self::OpenAI => {
                "OpenAI API key not configured. Open the Config view (select \"OpenAI API key\" and press Enter) or run `learnchain --set-openai-key <your-key>` to add it."
            }
            Self::Anthropic => {
                "Anthropic API key not configured. Open the Config view or run `learnchain --set-anthropic-key <your-key>` to add it."
            }
            Self::OpenRouter => {
                "OpenRouter API key not configured. Open the Config view or run `learnchain --set-openrouter-key <your-key>` to add it."
            }
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::OpenAI => Self::Anthropic,
            Self::Anthropic => Self::OpenRouter,
            Self::OpenRouter => Self::OpenAI,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::OpenAI => Self::OpenRouter,
            Self::Anthropic => Self::OpenAI,
            Self::OpenRouter => Self::Anthropic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigField {
    MaxEvents,
    MinQuiz,
    SamplingPercentage,
    SessionSource,
    OutputArtifacts,
    AiProvider,
    OpenAiModel,
    OpenAiKey,
    AnthropicModel,
    AnthropicKey,
    OpenRouterModel,
    OpenRouterKey,
}

#[derive(Debug, Clone)]
pub struct ConfigForm {
    pub(crate) max_events: usize,
    pub(crate) min_quiz_questions: usize,
    pub(crate) sampling_percentage: u8,
    pub(crate) session_source: SessionSourceKind,
    pub(crate) write_output_artifacts: bool,
    // Provider selection
    pub(crate) ai_provider: AiProvider,
    // OpenAI
    pub(crate) openai_model: OpenAiModelKind,
    pub(crate) openai_api_key: String,
    editing_openai_key: bool,
    openai_key_buffer: String,
    // Anthropic
    pub(crate) anthropic_model: AnthropicModelKind,
    pub(crate) anthropic_api_key: String,
    editing_anthropic_key: bool,
    anthropic_key_buffer: String,
    // OpenRouter
    pub(crate) openrouter_model: String,
    pub(crate) openrouter_api_key: String,
    editing_openrouter_key: bool,
    openrouter_key_buffer: String,
    editing_openrouter_model: bool,
    openrouter_model_buffer: String,
    // Form state
    pub(crate) field: ConfigField,
    pub(crate) dirty: bool,
    pub(crate) status: Option<String>,
}

impl ConfigForm {
    pub(crate) fn from_config(config: AppConfig) -> Self {
        Self {
            max_events: config.default_max_events,
            min_quiz_questions: config.min_quiz_questions,
            sampling_percentage: config.sampling_percentage,
            session_source: config.session_source,
            write_output_artifacts: config.write_output_artifacts,
            ai_provider: config.ai_provider,
            openai_model: config.openai_model,
            openai_api_key: config.openai_api_key,
            editing_openai_key: false,
            openai_key_buffer: String::new(),
            anthropic_model: config.anthropic_model,
            anthropic_api_key: config.anthropic_api_key,
            editing_anthropic_key: false,
            anthropic_key_buffer: String::new(),
            openrouter_model: config.openrouter_model,
            openrouter_api_key: config.openrouter_api_key,
            editing_openrouter_key: false,
            openrouter_key_buffer: String::new(),
            editing_openrouter_model: false,
            openrouter_model_buffer: String::new(),
            field: ConfigField::MaxEvents,
            dirty: false,
            status: None,
        }
    }

    pub(crate) fn resolved_llm(&self) -> ResolvedLlmConfig {
        match self.ai_provider {
            AiProvider::OpenAI => ResolvedLlmConfig {
                provider: self.ai_provider,
                model_name: self.openai_model.as_model_name().to_string(),
                model_label: self.openai_model.label().to_string(),
                api_key: self.openai_api_key.clone(),
            },
            AiProvider::Anthropic => ResolvedLlmConfig {
                provider: self.ai_provider,
                model_name: self.anthropic_model.as_model_name().to_string(),
                model_label: self.anthropic_model.label().to_string(),
                api_key: self.anthropic_api_key.clone(),
            },
            AiProvider::OpenRouter => ResolvedLlmConfig {
                provider: self.ai_provider,
                model_name: self.openrouter_model.clone(),
                model_label: if self.openrouter_model.trim().is_empty() {
                    "<not set>".to_string()
                } else {
                    self.openrouter_model.clone()
                },
                api_key: self.openrouter_api_key.clone(),
            },
        }
    }

    /// Returns the list of fields visible based on the current provider selection.
    pub(crate) fn visible_fields(&self) -> Vec<ConfigField> {
        let mut fields = vec![
            ConfigField::MaxEvents,
            ConfigField::MinQuiz,
            ConfigField::SamplingPercentage,
            ConfigField::SessionSource,
            ConfigField::OutputArtifacts,
            ConfigField::AiProvider,
        ];

        match self.ai_provider {
            AiProvider::OpenAI => {
                fields.push(ConfigField::OpenAiModel);
                fields.push(ConfigField::OpenAiKey);
            }
            AiProvider::Anthropic => {
                fields.push(ConfigField::AnthropicModel);
                fields.push(ConfigField::AnthropicKey);
            }
            AiProvider::OpenRouter => {
                fields.push(ConfigField::OpenRouterModel);
                fields.push(ConfigField::OpenRouterKey);
            }
        }

        fields
    }

    pub(crate) fn selected_index(&self) -> usize {
        let visible = self.visible_fields();
        visible.iter().position(|f| *f == self.field).unwrap_or(0)
    }

    pub(crate) fn select_next(&mut self) {
        let visible = self.visible_fields();
        let current_index = visible.iter().position(|f| *f == self.field).unwrap_or(0);
        let next_index = (current_index + 1) % visible.len();
        self.field = visible[next_index];
    }

    pub(crate) fn select_previous(&mut self) {
        let visible = self.visible_fields();
        let current_index = visible.iter().position(|f| *f == self.field).unwrap_or(0);
        let prev_index = if current_index == 0 {
            visible.len() - 1
        } else {
            current_index - 1
        };
        self.field = visible[prev_index];
    }

    pub(crate) fn adjust_current(&mut self, delta: isize) {
        if delta == 0 {
            return;
        }

        match self.field {
            ConfigField::SessionSource => {
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
            }
            ConfigField::OutputArtifacts => {
                self.write_output_artifacts = !self.write_output_artifacts;
                self.dirty = true;
                self.status = None;
            }
            ConfigField::AiProvider => {
                let updated = if delta > 0 {
                    self.ai_provider.next()
                } else {
                    self.ai_provider.previous()
                };
                if updated != self.ai_provider {
                    self.ai_provider = updated;
                    self.dirty = true;
                    self.status = None;
                    // Reset field to AiProvider when provider changes to avoid pointing to hidden field
                    self.field = ConfigField::AiProvider;
                }
            }
            ConfigField::OpenAiModel => {
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
            }
            ConfigField::AnthropicModel => {
                let updated = if delta > 0 {
                    self.anthropic_model.next()
                } else {
                    self.anthropic_model.previous()
                };
                if updated != self.anthropic_model {
                    self.anthropic_model = updated;
                    self.dirty = true;
                    self.status = None;
                }
            }
            ConfigField::OpenAiKey
            | ConfigField::AnthropicKey
            | ConfigField::OpenRouterKey
            | ConfigField::OpenRouterModel => {
                // These fields require text editing, not adjustment
            }
            ConfigField::MaxEvents | ConfigField::MinQuiz => {
                let (value, minimum) = match self.field {
                    ConfigField::MaxEvents => (&mut self.max_events, 1),
                    ConfigField::MinQuiz => (&mut self.min_quiz_questions, 1),
                    _ => unreachable!(),
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
            ConfigField::SamplingPercentage => {
                let current = self.sampling_percentage as isize;
                let updated = (current + delta).clamp(1, 100) as u8;

                if updated != self.sampling_percentage {
                    self.sampling_percentage = updated;
                    self.dirty = true;
                    self.status = None;
                }
            }
        }
    }

    pub(crate) fn apply_saved(&mut self, config: AppConfig) {
        self.max_events = config.default_max_events;
        self.min_quiz_questions = config.min_quiz_questions;
        self.sampling_percentage = config.sampling_percentage;
        self.session_source = config.session_source;
        self.write_output_artifacts = config.write_output_artifacts;
        self.ai_provider = config.ai_provider;
        self.openai_model = config.openai_model;
        self.openai_api_key = config.openai_api_key;
        self.editing_openai_key = false;
        self.openai_key_buffer.clear();
        self.anthropic_model = config.anthropic_model;
        self.anthropic_api_key = config.anthropic_api_key;
        self.editing_anthropic_key = false;
        self.anthropic_key_buffer.clear();
        self.openrouter_model = config.openrouter_model;
        self.openrouter_api_key = config.openrouter_api_key;
        self.editing_openrouter_key = false;
        self.openrouter_key_buffer.clear();
        self.editing_openrouter_model = false;
        self.openrouter_model_buffer.clear();
        self.dirty = false;
        self.status = None;
    }

    pub(crate) fn set_status<S: Into<String>>(&mut self, status: S) {
        self.status = Some(status.into());
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

    // Anthropic key editing methods
    pub(crate) fn is_editing_anthropic_key(&self) -> bool {
        self.editing_anthropic_key
    }

    pub(crate) fn start_editing_anthropic_key(&mut self) {
        self.editing_anthropic_key = true;
        self.anthropic_key_buffer = self.anthropic_api_key.clone();
        self.status = Some("Editing Anthropic API key (Enter to save, Esc to cancel)".to_string());
    }

    pub(crate) fn cancel_anthropic_key_edit(&mut self) {
        self.editing_anthropic_key = false;
        self.anthropic_key_buffer.clear();
        self.status = Some("Cancelled Anthropic API key edit.".to_string());
    }

    pub(crate) fn apply_anthropic_key_edit(&mut self) {
        let new_value = self.anthropic_key_buffer.trim().to_string();
        if new_value != self.anthropic_api_key {
            self.anthropic_api_key = new_value;
            self.dirty = true;
            self.status = Some("Updated Anthropic API key.".to_string());
        } else {
            self.status = Some("Anthropic API key unchanged.".to_string());
        }
        self.editing_anthropic_key = false;
        self.anthropic_key_buffer.clear();
    }

    pub(crate) fn backspace_anthropic_key(&mut self) {
        self.anthropic_key_buffer.pop();
        self.status = Some("Editing Anthropic API key...".to_string());
    }

    pub(crate) fn push_anthropic_key_char(&mut self, ch: char) {
        self.anthropic_key_buffer.push(ch);
        self.status = Some("Editing Anthropic API key...".to_string());
    }

    pub(crate) fn masked_anthropic_key(&self) -> String {
        mask_secret(&self.anthropic_api_key)
    }

    pub(crate) fn masked_anthropic_key_buffer(&self) -> String {
        mask_secret(&self.anthropic_key_buffer)
    }

    // OpenRouter key editing methods
    pub(crate) fn is_editing_openrouter_key(&self) -> bool {
        self.editing_openrouter_key
    }

    pub(crate) fn start_editing_openrouter_key(&mut self) {
        self.editing_openrouter_key = true;
        self.openrouter_key_buffer = self.openrouter_api_key.clone();
        self.status = Some("Editing OpenRouter API key (Enter to save, Esc to cancel)".to_string());
    }

    pub(crate) fn cancel_openrouter_key_edit(&mut self) {
        self.editing_openrouter_key = false;
        self.openrouter_key_buffer.clear();
        self.status = Some("Cancelled OpenRouter API key edit.".to_string());
    }

    pub(crate) fn apply_openrouter_key_edit(&mut self) {
        let new_value = self.openrouter_key_buffer.trim().to_string();
        if new_value != self.openrouter_api_key {
            self.openrouter_api_key = new_value;
            self.dirty = true;
            self.status = Some("Updated OpenRouter API key.".to_string());
        } else {
            self.status = Some("OpenRouter API key unchanged.".to_string());
        }
        self.editing_openrouter_key = false;
        self.openrouter_key_buffer.clear();
    }

    pub(crate) fn backspace_openrouter_key(&mut self) {
        self.openrouter_key_buffer.pop();
        self.status = Some("Editing OpenRouter API key...".to_string());
    }

    pub(crate) fn push_openrouter_key_char(&mut self, ch: char) {
        self.openrouter_key_buffer.push(ch);
        self.status = Some("Editing OpenRouter API key...".to_string());
    }

    pub(crate) fn masked_openrouter_key(&self) -> String {
        mask_secret(&self.openrouter_api_key)
    }

    pub(crate) fn masked_openrouter_key_buffer(&self) -> String {
        mask_secret(&self.openrouter_key_buffer)
    }

    // OpenRouter model editing methods (free-text input)
    pub(crate) fn is_editing_openrouter_model(&self) -> bool {
        self.editing_openrouter_model
    }

    pub(crate) fn start_editing_openrouter_model(&mut self) {
        self.editing_openrouter_model = true;
        self.openrouter_model_buffer = self.openrouter_model.clone();
        self.status = Some("Editing OpenRouter model (Enter to save, Esc to cancel)".to_string());
    }

    pub(crate) fn cancel_openrouter_model_edit(&mut self) {
        self.editing_openrouter_model = false;
        self.openrouter_model_buffer.clear();
        self.status = Some("Cancelled OpenRouter model edit.".to_string());
    }

    pub(crate) fn apply_openrouter_model_edit(&mut self) {
        let new_value = self.openrouter_model_buffer.trim().to_string();
        if new_value != self.openrouter_model {
            self.openrouter_model = new_value;
            self.dirty = true;
            self.status = Some("Updated OpenRouter model.".to_string());
        } else {
            self.status = Some("OpenRouter model unchanged.".to_string());
        }
        self.editing_openrouter_model = false;
        self.openrouter_model_buffer.clear();
    }

    pub(crate) fn backspace_openrouter_model(&mut self) {
        self.openrouter_model_buffer.pop();
        self.status = Some("Editing OpenRouter model...".to_string());
    }

    pub(crate) fn push_openrouter_model_char(&mut self, ch: char) {
        self.openrouter_model_buffer.push(ch);
        self.status = Some("Editing OpenRouter model...".to_string());
    }

    pub(crate) fn openrouter_model_buffer(&self) -> &str {
        &self.openrouter_model_buffer
    }

    /// Returns true if any text field is currently being edited.
    pub(crate) fn is_editing_text_field(&self) -> bool {
        self.editing_openai_key
            || self.editing_anthropic_key
            || self.editing_openrouter_key
            || self.editing_openrouter_model
    }

    /// Returns the currently selected field.
    pub(crate) fn current_field(&self) -> ConfigField {
        self.field
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AnthropicModelKind {
    #[default]
    ClaudeSonnet4,
    ClaudeOpus4,
    ClaudeSonnet35,
}

impl AnthropicModelKind {
    pub fn as_model_name(self) -> &'static str {
        match self {
            Self::ClaudeSonnet4 => "claude-sonnet-4-20250514",
            Self::ClaudeOpus4 => "claude-opus-4-20250514",
            Self::ClaudeSonnet35 => "claude-3-5-sonnet-20241022",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeSonnet4 => "Claude Sonnet 4",
            Self::ClaudeOpus4 => "Claude Opus 4",
            Self::ClaudeSonnet35 => "Claude 3.5 Sonnet",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::ClaudeSonnet4 => Self::ClaudeOpus4,
            Self::ClaudeOpus4 => Self::ClaudeSonnet35,
            Self::ClaudeSonnet35 => Self::ClaudeSonnet4,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::ClaudeSonnet4 => Self::ClaudeSonnet35,
            Self::ClaudeOpus4 => Self::ClaudeSonnet4,
            Self::ClaudeSonnet35 => Self::ClaudeOpus4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_config_resolved_llm_for_openai() {
        let config = AppConfig {
            ai_provider: AiProvider::OpenAI,
            openai_model: OpenAiModelKind::Gpt5,
            openai_api_key: "sk-openai".to_string(),
            ..AppConfig::default()
        };

        let resolved = config.resolved_llm();
        assert_eq!(resolved.provider, AiProvider::OpenAI);
        assert_eq!(resolved.model_name, "gpt-5");
        assert_eq!(resolved.model_label, "gpt-5");
        assert_eq!(resolved.api_key, "sk-openai");
        assert!(
            AiProvider::OpenAI
                .missing_key_help()
                .contains("--set-openai-key")
        );
    }

    #[test]
    fn app_config_resolved_llm_for_anthropic() {
        let config = AppConfig {
            ai_provider: AiProvider::Anthropic,
            anthropic_model: AnthropicModelKind::ClaudeOpus4,
            anthropic_api_key: "sk-anthropic".to_string(),
            ..AppConfig::default()
        };

        let resolved = config.resolved_llm();
        assert_eq!(resolved.provider, AiProvider::Anthropic);
        assert_eq!(resolved.model_name, "claude-opus-4-20250514");
        assert_eq!(resolved.model_label, "Claude Opus 4");
        assert_eq!(resolved.api_key, "sk-anthropic");
        assert!(
            AiProvider::Anthropic
                .missing_key_help()
                .contains("--set-anthropic-key")
        );
    }

    #[test]
    fn config_form_resolved_llm_for_openrouter() {
        let mut form = ConfigForm::from_config(AppConfig::default());
        form.ai_provider = AiProvider::OpenRouter;
        form.openrouter_model = "openrouter/model".to_string();
        form.openrouter_api_key = "sk-openrouter".to_string();

        let resolved = form.resolved_llm();
        assert_eq!(resolved.provider, AiProvider::OpenRouter);
        assert_eq!(resolved.model_name, "openrouter/model");
        assert_eq!(resolved.model_label, "openrouter/model");
        assert_eq!(resolved.api_key, "sk-openrouter");
        assert!(
            AiProvider::OpenRouter
                .missing_key_help()
                .contains("--set-openrouter-key")
        );
    }
}
