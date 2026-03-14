use color_eyre::eyre::{Context, Result, eyre};
use serde::{Deserialize, Serialize};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
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
    #[serde(default)]
    pub document_repository: DocumentRepositoryKind,
    #[serde(default)]
    pub document_repository_target: String,
    #[serde(default)]
    pub notion_api_token: String,
    #[serde(default = "default_learnchain_site_url")]
    pub learnchain_site_url: String,
    #[serde(default)]
    pub learnchain_email: String,
    #[serde(default)]
    pub learnchain_access_token: String,
    #[serde(default)]
    pub learnchain_refresh_token: String,
    #[serde(default)]
    pub learnchain_password: String,
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
    #[serde(default)]
    pub deep_dive_sections: DeepDiveSectionsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DeepDiveSectionsConfig {
    pub session_metadata: bool,
    pub goal: bool,
    pub accomplishments: bool,
    pub interesting_learnings: bool,
    pub teaching_narrative: bool,
    pub reviewed_external_sources: bool,
    pub referenced_urls: bool,
}

impl DeepDiveSectionsConfig {
    pub const fn total_count() -> usize {
        7
    }

    pub fn enabled_count(&self) -> usize {
        [
            self.session_metadata,
            self.goal,
            self.accomplishments,
            self.interesting_learnings,
            self.teaching_narrative,
            self.reviewed_external_sources,
            self.referenced_urls,
        ]
        .into_iter()
        .filter(|enabled| *enabled)
        .count()
    }
}

impl Default for DeepDiveSectionsConfig {
    fn default() -> Self {
        Self {
            session_metadata: true,
            goal: true,
            accomplishments: true,
            interesting_learnings: true,
            teaching_narrative: true,
            reviewed_external_sources: true,
            referenced_urls: true,
        }
    }
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
        self.normalize_document_repository();
        self.learnchain_site_url = normalize_learnchain_site_url(&self.learnchain_site_url);
        self.learnchain_email = self.learnchain_email.trim().to_string();
    }

    fn normalize_document_repository(&mut self) {
        if self.document_repository != DocumentRepositoryKind::None {
            self.document_repository_target = self.document_repository_target.trim().to_string();
            return;
        }

        let trimmed = self.document_repository_target.trim();
        let Some((provider, target)) = trimmed.split_once(':') else {
            self.document_repository_target = trimmed.to_string();
            return;
        };

        if provider.trim().eq_ignore_ascii_case("notion") {
            self.document_repository = DocumentRepositoryKind::Notion;
            self.document_repository_target = target.trim().to_string();
        } else {
            self.document_repository_target = trimmed.to_string();
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
            AiProvider::CodexCli => ResolvedLlmConfig {
                provider: self.ai_provider,
                model_name: "codex-exec".to_string(),
                model_label: "CLI default".to_string(),
                api_key: String::new(),
            },
            AiProvider::ClaudeCodeCli => ResolvedLlmConfig {
                provider: self.ai_provider,
                model_name: "claude-code-print".to_string(),
                model_label: "CLI default".to_string(),
                api_key: String::new(),
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
            document_repository: DocumentRepositoryKind::default(),
            document_repository_target: String::new(),
            notion_api_token: String::new(),
            learnchain_site_url: default_learnchain_site_url(),
            learnchain_email: String::new(),
            learnchain_access_token: String::new(),
            learnchain_refresh_token: String::new(),
            learnchain_password: String::new(),
            ai_provider: AiProvider::default(),
            openai_model: default_openai_model_kind(),
            openai_api_key: String::new(),
            anthropic_model: AnthropicModelKind::default(),
            anthropic_api_key: String::new(),
            openrouter_model: String::new(),
            openrouter_api_key: String::new(),
            sampling_percentage: default_sampling_percentage(),
            deep_dive_sections: DeepDiveSectionsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocumentRepositoryKind {
    #[default]
    None,
    Notion,
    LearnChain,
}

impl DocumentRepositoryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Notion => "Notion",
            Self::LearnChain => "LearnChain",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::None => Self::Notion,
            Self::Notion => Self::LearnChain,
            Self::LearnChain => Self::None,
        }
    }

    pub fn previous(self) -> Self {
        self.next()
    }
}

const DEFAULT_MAX_EVENTS: usize = 15;
const DEFAULT_MIN_QUIZ_QUESTIONS: usize = 5;
pub(crate) const LEARNCHAIN_DEFAULT_SITE_URL: &str = "https://learnchain.co";
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
fn default_learnchain_site_url() -> String {
    LEARNCHAIN_DEFAULT_SITE_URL.to_string()
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
'],'workdir':'/workspace/learnchain'}
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

const CONFIG_FILE_NAME: &str = "app_config.toml";
const LEGACY_CONFIG_FILE_PATH: &str = "config/app_config.toml";

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

/// Primary path used for configuration persistence.
pub fn config_file_path() -> PathBuf {
    global_config_file_path().unwrap_or_else(legacy_config_file_path)
}

fn load_config_from_disk() -> Result<AppConfig> {
    let path = config_file_path();
    let legacy_path = legacy_config_file_path();
    load_config_from_paths(&path, &legacy_path)
}

fn load_config_from_paths(path: &Path, legacy_path: &Path) -> Result<AppConfig> {
    if let Some(config) = load_config_from_path(&path)? {
        return Ok(config);
    }

    if legacy_path != path {
        if let Some(config) = load_config_from_path(&legacy_path)? {
            // Best-effort migration so future launches outside the original cwd keep working.
            let _ = save_config_to_path(&path, &config);
            return Ok(config);
        }
    }

    Ok(AppConfig::default())
}

fn save_config_to_disk(config: &AppConfig) -> Result<()> {
    let path = config_file_path();
    save_config_to_path(&path, config)
}

fn load_config_from_path(path: &Path) -> Result<Option<AppConfig>> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let mut config: AppConfig = toml::from_str(&contents)
                .wrap_err_with(|| format!("failed to parse configuration at {}", path.display()))?;
            config.normalize();
            Ok(Some(config))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(eyre!(format!(
            "failed to read configuration at {}: {}",
            path.display(),
            err
        ))),
    }
}

fn save_config_to_path(path: &Path, config: &AppConfig) -> Result<()> {
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
    fs::write(path, serialized)
        .wrap_err_with(|| format!("failed to write configuration to {}", path.display()))
}

fn legacy_config_file_path() -> PathBuf {
    PathBuf::from(LEGACY_CONFIG_FILE_PATH)
}

fn global_config_file_path() -> Option<PathBuf> {
    global_config_dir().map(|dir| dir.join(CONFIG_FILE_NAME))
}

fn global_config_dir() -> Option<PathBuf> {
    resolve_global_config_dir(
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        env::var_os("APPDATA").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    )
}

#[cfg(target_os = "macos")]
fn resolve_global_config_dir(
    _xdg_config_home: Option<PathBuf>,
    _appdata: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    home.map(|home| {
        home.join("Library")
            .join("Application Support")
            .join("learnchain")
    })
}

#[cfg(target_os = "windows")]
fn resolve_global_config_dir(
    _xdg_config_home: Option<PathBuf>,
    appdata: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    appdata
        .or_else(|| home.map(|home| home.join("AppData").join("Roaming")))
        .map(|dir| dir.join("LearnChain"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn resolve_global_config_dir(
    xdg_config_home: Option<PathBuf>,
    _appdata: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    xdg_config_home
        .or_else(|| home.map(|home| home.join(".config")))
        .map(|dir| dir.join("learnchain"))
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
    CodexCli,
    ClaudeCodeCli,
}

impl AiProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::OpenRouter => "OpenRouter",
            Self::CodexCli => "Codex CLI",
            Self::ClaudeCodeCli => "Claude Code CLI",
        }
    }

    pub fn setup_help(self) -> &'static str {
        match self {
            Self::OpenAI => {
                "OpenAI API key not configured. Open the Config view (select \"OpenAI API key\" and press Enter) or run `learnchain config set openai-key <your-key>` to add it."
            }
            Self::Anthropic => {
                "Anthropic API key not configured. Open the Config view or run `learnchain config set anthropic-key <your-key>` to add it."
            }
            Self::OpenRouter => {
                "OpenRouter API key not configured. Open the Config view or run `learnchain config set openrouter-key <your-key>` to add it."
            }
            Self::CodexCli => {
                "Codex CLI is not available. Ensure `codex` is installed and authenticated in your shell."
            }
            Self::ClaudeCodeCli => {
                "Claude Code CLI is not available. Ensure `claude` is installed and authenticated in your shell."
            }
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::OpenAI => Self::Anthropic,
            Self::Anthropic => Self::OpenRouter,
            Self::OpenRouter => Self::CodexCli,
            Self::CodexCli => Self::ClaudeCodeCli,
            Self::ClaudeCodeCli => Self::OpenAI,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::OpenAI => Self::ClaudeCodeCli,
            Self::Anthropic => Self::OpenAI,
            Self::OpenRouter => Self::Anthropic,
            Self::CodexCli => Self::OpenRouter,
            Self::ClaudeCodeCli => Self::CodexCli,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigField {
    MaxEvents,
    MinQuiz,
    SamplingPercentage,
    SessionSource,
    DeepDiveSessionMetadata,
    DeepDiveGoal,
    DeepDiveAccomplishments,
    DeepDiveInterestingLearnings,
    DeepDiveTeachingNarrative,
    DeepDiveReviewedExternalSources,
    DeepDiveReferencedUrls,
    OutputArtifacts,
    DocumentRepository,
    DocumentRepositoryTarget,
    NotionApiToken,
    LearnChainSiteUrl,
    LearnChainEmail,
    LearnChainAuthCode,
    AiProvider,
    OpenAiModel,
    OpenAiKey,
    AnthropicModel,
    AnthropicKey,
    OpenRouterModel,
    OpenRouterKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigSection {
    Session,
    DeepDive,
    Export,
    Ai,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigNavigationFocus {
    Section,
    Step,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LearnChainAuthFeedback {
    Neutral,
    Success,
    Failure,
}

impl ConfigSection {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Session => "Session Setup",
            Self::DeepDive => "Deep Dive Document",
            Self::Export => "Export & Publishing",
            Self::Ai => "AI Generation",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Session => {
                "Choose what session data LearnChain loads and how much context it keeps."
            }
            Self::DeepDive => {
                "Toggle the optional sections LearnChain includes when it renders a generated deep dive. The embedded quiz is always included."
            }
            Self::Export => "Control saved artifacts and optional document repository publishing.",
            Self::Ai => {
                "Pick the provider, model, and credentials used for quizzes and deep dives."
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigForm {
    pub(crate) max_events: usize,
    pub(crate) min_quiz_questions: usize,
    pub(crate) sampling_percentage: u8,
    pub(crate) session_source: SessionSourceKind,
    pub(crate) deep_dive_sections: DeepDiveSectionsConfig,
    pub(crate) write_output_artifacts: bool,
    pub(crate) document_repository: DocumentRepositoryKind,
    pub(crate) document_repository_target: String,
    pub(crate) notion_api_token: String,
    pub(crate) learnchain_site_url: String,
    pub(crate) learnchain_email: String,
    pub(crate) learnchain_access_token: String,
    pub(crate) learnchain_refresh_token: String,
    pub(crate) learnchain_password: String,
    editing_document_repository_target: bool,
    document_repository_target_buffer: String,
    editing_notion_api_token: bool,
    notion_api_token_buffer: String,
    editing_learnchain_site_url: bool,
    learnchain_site_url_buffer: String,
    pub(crate) learnchain_auth_code: String,
    editing_learnchain_auth_code: bool,
    learnchain_auth_code_buffer: String,
    learnchain_auth_feedback: LearnChainAuthFeedback,
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
    selected_section: ConfigSection,
    navigation_focus: ConfigNavigationFocus,
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
            deep_dive_sections: config.deep_dive_sections,
            write_output_artifacts: config.write_output_artifacts,
            document_repository: config.document_repository,
            document_repository_target: config.document_repository_target,
            notion_api_token: config.notion_api_token,
            learnchain_site_url: config.learnchain_site_url,
            learnchain_email: config.learnchain_email,
            learnchain_access_token: config.learnchain_access_token,
            learnchain_refresh_token: config.learnchain_refresh_token,
            learnchain_password: config.learnchain_password,
            editing_document_repository_target: false,
            document_repository_target_buffer: String::new(),
            editing_notion_api_token: false,
            notion_api_token_buffer: String::new(),
            editing_learnchain_site_url: false,
            learnchain_site_url_buffer: String::new(),
            learnchain_auth_code: String::new(),
            editing_learnchain_auth_code: false,
            learnchain_auth_code_buffer: String::new(),
            learnchain_auth_feedback: LearnChainAuthFeedback::Neutral,
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
            selected_section: ConfigSection::Session,
            navigation_focus: ConfigNavigationFocus::Section,
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
            AiProvider::CodexCli => ResolvedLlmConfig {
                provider: self.ai_provider,
                model_name: "codex-exec".to_string(),
                model_label: "CLI default".to_string(),
                api_key: String::new(),
            },
            AiProvider::ClaudeCodeCli => ResolvedLlmConfig {
                provider: self.ai_provider,
                model_name: "claude-code-print".to_string(),
                model_label: "CLI default".to_string(),
                api_key: String::new(),
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
            ConfigField::DeepDiveSessionMetadata,
            ConfigField::DeepDiveGoal,
            ConfigField::DeepDiveAccomplishments,
            ConfigField::DeepDiveInterestingLearnings,
            ConfigField::DeepDiveTeachingNarrative,
            ConfigField::DeepDiveReviewedExternalSources,
            ConfigField::DeepDiveReferencedUrls,
            ConfigField::OutputArtifacts,
            ConfigField::DocumentRepository,
        ];

        if self.document_repository != DocumentRepositoryKind::None {
            match self.document_repository {
                DocumentRepositoryKind::Notion => {
                    fields.push(ConfigField::DocumentRepositoryTarget);
                    fields.push(ConfigField::NotionApiToken);
                }
                DocumentRepositoryKind::LearnChain => {
                    fields.push(ConfigField::LearnChainSiteUrl);
                    fields.push(ConfigField::LearnChainEmail);
                    fields.push(ConfigField::LearnChainAuthCode);
                }
                DocumentRepositoryKind::None => {}
            }
        }

        fields.push(ConfigField::AiProvider);

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
            AiProvider::CodexCli | AiProvider::ClaudeCodeCli => {}
        }

        fields
    }

    pub(crate) fn section_for_field(field: ConfigField) -> ConfigSection {
        match field {
            ConfigField::MaxEvents
            | ConfigField::MinQuiz
            | ConfigField::SamplingPercentage
            | ConfigField::SessionSource => ConfigSection::Session,
            ConfigField::DeepDiveSessionMetadata
            | ConfigField::DeepDiveGoal
            | ConfigField::DeepDiveAccomplishments
            | ConfigField::DeepDiveInterestingLearnings
            | ConfigField::DeepDiveTeachingNarrative
            | ConfigField::DeepDiveReviewedExternalSources
            | ConfigField::DeepDiveReferencedUrls => ConfigSection::DeepDive,
            ConfigField::OutputArtifacts
            | ConfigField::DocumentRepository
            | ConfigField::DocumentRepositoryTarget
            | ConfigField::NotionApiToken
            | ConfigField::LearnChainSiteUrl
            | ConfigField::LearnChainEmail
            | ConfigField::LearnChainAuthCode => ConfigSection::Export,
            ConfigField::AiProvider
            | ConfigField::OpenAiModel
            | ConfigField::OpenAiKey
            | ConfigField::AnthropicModel
            | ConfigField::AnthropicKey
            | ConfigField::OpenRouterModel
            | ConfigField::OpenRouterKey => ConfigSection::Ai,
        }
    }

    pub(crate) fn visible_fields_in_section(&self, section: ConfigSection) -> Vec<ConfigField> {
        self.visible_fields()
            .into_iter()
            .filter(|field| Self::section_for_field(*field) == section)
            .collect()
    }

    pub(crate) fn selected_section(&self) -> ConfigSection {
        self.selected_section
    }

    pub(crate) fn is_section_navigation_focused(&self) -> bool {
        self.navigation_focus == ConfigNavigationFocus::Section
    }

    pub(crate) fn is_step_navigation_focused(&self) -> bool {
        self.navigation_focus == ConfigNavigationFocus::Step
    }

    pub(crate) fn focus_section_navigation(&mut self) {
        self.navigation_focus = ConfigNavigationFocus::Section;
    }

    pub(crate) fn focus_step_navigation(&mut self) {
        self.navigation_focus = ConfigNavigationFocus::Step;
        self.sync_field_to_selected_section();
    }

    pub(crate) fn selected_index_in_section(&self) -> usize {
        let visible = self.visible_fields_in_section(self.selected_section);
        visible
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0)
    }

    pub(crate) fn select_next(&mut self) {
        match self.navigation_focus {
            ConfigNavigationFocus::Section => {
                let sections = [
                    ConfigSection::Session,
                    ConfigSection::DeepDive,
                    ConfigSection::Export,
                    ConfigSection::Ai,
                ];
                let current_index = sections
                    .iter()
                    .position(|section| *section == self.selected_section)
                    .unwrap_or(0);
                let next_index = (current_index + 1) % sections.len();
                self.selected_section = sections[next_index];
                self.sync_field_to_selected_section();
            }
            ConfigNavigationFocus::Step => {
                let visible = self.visible_fields_in_section(self.selected_section);
                let current_index = visible.iter().position(|f| *f == self.field).unwrap_or(0);
                let next_index = (current_index + 1) % visible.len();
                self.field = visible[next_index];
            }
        }
    }

    pub(crate) fn select_previous(&mut self) {
        match self.navigation_focus {
            ConfigNavigationFocus::Section => {
                let sections = [
                    ConfigSection::Session,
                    ConfigSection::DeepDive,
                    ConfigSection::Export,
                    ConfigSection::Ai,
                ];
                let current_index = sections
                    .iter()
                    .position(|section| *section == self.selected_section)
                    .unwrap_or(0);
                let prev_index = if current_index == 0 {
                    sections.len() - 1
                } else {
                    current_index - 1
                };
                self.selected_section = sections[prev_index];
                self.sync_field_to_selected_section();
            }
            ConfigNavigationFocus::Step => {
                let visible = self.visible_fields_in_section(self.selected_section);
                let current_index = visible.iter().position(|f| *f == self.field).unwrap_or(0);
                let prev_index = if current_index == 0 {
                    visible.len() - 1
                } else {
                    current_index - 1
                };
                self.field = visible[prev_index];
            }
        }
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
            ConfigField::DeepDiveSessionMetadata
            | ConfigField::DeepDiveGoal
            | ConfigField::DeepDiveAccomplishments
            | ConfigField::DeepDiveInterestingLearnings
            | ConfigField::DeepDiveTeachingNarrative
            | ConfigField::DeepDiveReviewedExternalSources
            | ConfigField::DeepDiveReferencedUrls => {
                let value = match self.field {
                    ConfigField::DeepDiveSessionMetadata => {
                        &mut self.deep_dive_sections.session_metadata
                    }
                    ConfigField::DeepDiveGoal => &mut self.deep_dive_sections.goal,
                    ConfigField::DeepDiveAccomplishments => {
                        &mut self.deep_dive_sections.accomplishments
                    }
                    ConfigField::DeepDiveInterestingLearnings => {
                        &mut self.deep_dive_sections.interesting_learnings
                    }
                    ConfigField::DeepDiveTeachingNarrative => {
                        &mut self.deep_dive_sections.teaching_narrative
                    }
                    ConfigField::DeepDiveReviewedExternalSources => {
                        &mut self.deep_dive_sections.reviewed_external_sources
                    }
                    ConfigField::DeepDiveReferencedUrls => {
                        &mut self.deep_dive_sections.referenced_urls
                    }
                    _ => unreachable!(),
                };
                *value = !*value;
                self.dirty = true;
                self.status = None;
            }
            ConfigField::DocumentRepository => {
                let updated = if delta > 0 {
                    self.document_repository.next()
                } else {
                    self.document_repository.previous()
                };
                if updated != self.document_repository {
                    self.document_repository = updated;
                    if self.document_repository == DocumentRepositoryKind::LearnChain
                        && self.learnchain_site_url.trim().is_empty()
                    {
                        self.learnchain_site_url = default_learnchain_site_url();
                    }
                    self.clear_learnchain_auth_feedback();
                    self.dirty = true;
                    self.status = match self.document_repository {
                        DocumentRepositoryKind::LearnChain if !self.has_learnchain_auth() => Some(
                            learnchain_authorization_help_message(&self.learnchain_site_url),
                        ),
                        _ => None,
                    };
                    self.field = ConfigField::DocumentRepository;
                }
            }
            ConfigField::DocumentRepositoryTarget => {
                // This field requires text editing, not adjustment.
            }
            ConfigField::NotionApiToken => {
                // This field requires text editing, not adjustment.
            }
            ConfigField::LearnChainSiteUrl
            | ConfigField::LearnChainEmail
            | ConfigField::LearnChainAuthCode => {
                // These fields require text editing, not adjustment.
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
                    self.status = match updated {
                        AiProvider::CodexCli => Some(codex_cli_config_help_message().to_string()),
                        AiProvider::ClaudeCodeCli => {
                            Some(claude_code_cli_config_help_message().to_string())
                        }
                        _ => None,
                    };
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

        self.sync_field_to_selected_section();
    }

    pub(crate) fn apply_saved(&mut self, config: AppConfig) {
        self.max_events = config.default_max_events;
        self.min_quiz_questions = config.min_quiz_questions;
        self.sampling_percentage = config.sampling_percentage;
        self.session_source = config.session_source;
        self.deep_dive_sections = config.deep_dive_sections;
        self.write_output_artifacts = config.write_output_artifacts;
        self.document_repository = config.document_repository;
        self.document_repository_target = config.document_repository_target;
        self.notion_api_token = config.notion_api_token;
        self.learnchain_site_url = config.learnchain_site_url;
        self.learnchain_email = config.learnchain_email;
        self.learnchain_access_token = config.learnchain_access_token;
        self.learnchain_refresh_token = config.learnchain_refresh_token;
        self.learnchain_password = config.learnchain_password;
        self.editing_document_repository_target = false;
        self.document_repository_target_buffer.clear();
        self.editing_notion_api_token = false;
        self.notion_api_token_buffer.clear();
        self.editing_learnchain_site_url = false;
        self.learnchain_site_url_buffer.clear();
        self.learnchain_auth_code.clear();
        self.editing_learnchain_auth_code = false;
        self.learnchain_auth_code_buffer.clear();
        self.learnchain_auth_feedback = LearnChainAuthFeedback::Neutral;
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
        self.sync_field_to_selected_section();
        self.dirty = false;
        self.status = None;
    }

    pub(crate) fn set_status<S: Into<String>>(&mut self, status: S) {
        self.status = Some(status.into());
    }

    pub(crate) fn learnchain_auth_feedback(&self) -> LearnChainAuthFeedback {
        self.learnchain_auth_feedback
    }

    pub(crate) fn mark_learnchain_auth_success(&mut self) {
        self.learnchain_auth_feedback = LearnChainAuthFeedback::Success;
    }

    pub(crate) fn mark_learnchain_auth_failure(&mut self) {
        self.learnchain_auth_feedback = LearnChainAuthFeedback::Failure;
    }

    pub(crate) fn clear_learnchain_auth_feedback(&mut self) {
        self.learnchain_auth_feedback = LearnChainAuthFeedback::Neutral;
    }

    pub(crate) fn is_editing_openai_key(&self) -> bool {
        self.editing_openai_key
    }

    pub(crate) fn is_editing_document_repository_target(&self) -> bool {
        self.editing_document_repository_target
    }

    pub(crate) fn is_notion_target_selected(&self) -> bool {
        self.document_repository == DocumentRepositoryKind::Notion
    }

    pub(crate) fn is_learnchain_selected(&self) -> bool {
        self.document_repository == DocumentRepositoryKind::LearnChain
    }

    pub(crate) fn has_learnchain_session(&self) -> bool {
        !self.learnchain_refresh_token.trim().is_empty()
            || !self.learnchain_access_token.trim().is_empty()
    }

    pub(crate) fn has_learnchain_auth(&self) -> bool {
        self.has_learnchain_session()
            || (!self.learnchain_email.trim().is_empty()
                && !self.learnchain_password.trim().is_empty())
    }

    pub(crate) fn start_editing_document_repository_target(&mut self) {
        self.editing_document_repository_target = true;
        self.document_repository_target_buffer = self.document_repository_target.clone();
        self.status = Some(match self.document_repository {
            DocumentRepositoryKind::Notion => {
                "Editing Notion destination. Enter the database/page ID or full URL (Enter to save, Esc to cancel).".to_string()
            }
            DocumentRepositoryKind::LearnChain => {
                "Editing document repository target (Enter to save, Esc to cancel).".to_string()
            }
            DocumentRepositoryKind::None => {
                "Editing document repository target (Enter to save, Esc to cancel).".to_string()
            }
        });
    }

    pub(crate) fn cancel_document_repository_target_edit(&mut self) {
        self.editing_document_repository_target = false;
        self.document_repository_target_buffer.clear();
        self.status = Some("Cancelled document repository target edit.".to_string());
    }

    pub(crate) fn apply_document_repository_target_edit(&mut self) {
        let new_value = self.document_repository_target_buffer.trim().to_string();
        if new_value != self.document_repository_target {
            self.document_repository_target = new_value;
            self.dirty = true;
            self.status = Some(match self.document_repository {
                DocumentRepositoryKind::Notion => "Updated Notion destination.".to_string(),
                DocumentRepositoryKind::LearnChain => {
                    "Updated document repository target.".to_string()
                }
                DocumentRepositoryKind::None => "Updated document repository target.".to_string(),
            });
        } else {
            self.status = Some(match self.document_repository {
                DocumentRepositoryKind::Notion => "Notion destination unchanged.".to_string(),
                DocumentRepositoryKind::LearnChain => {
                    "Document repository target unchanged.".to_string()
                }
                DocumentRepositoryKind::None => "Document repository target unchanged.".to_string(),
            });
        }
        self.editing_document_repository_target = false;
        self.document_repository_target_buffer.clear();
    }

    pub(crate) fn backspace_document_repository_target(&mut self) {
        self.document_repository_target_buffer.pop();
        self.status = Some(match self.document_repository {
            DocumentRepositoryKind::Notion => "Editing Notion destination...".to_string(),
            DocumentRepositoryKind::LearnChain => {
                "Editing document repository target...".to_string()
            }
            DocumentRepositoryKind::None => "Editing document repository target...".to_string(),
        });
    }

    pub(crate) fn push_document_repository_target_char(&mut self, ch: char) {
        self.document_repository_target_buffer.push(ch);
        self.status = Some(match self.document_repository {
            DocumentRepositoryKind::Notion => "Editing Notion destination...".to_string(),
            DocumentRepositoryKind::LearnChain => {
                "Editing document repository target...".to_string()
            }
            DocumentRepositoryKind::None => "Editing document repository target...".to_string(),
        });
    }

    pub(crate) fn document_repository_target_buffer(&self) -> &str {
        &self.document_repository_target_buffer
    }

    pub(crate) fn is_editing_notion_api_token(&self) -> bool {
        self.editing_notion_api_token
    }

    pub(crate) fn start_editing_notion_api_token(&mut self) {
        self.editing_notion_api_token = true;
        self.notion_api_token_buffer = self.notion_api_token.clone();
        self.status = Some("Editing Notion API token (Enter to save, Esc to cancel)".to_string());
    }

    pub(crate) fn cancel_notion_api_token_edit(&mut self) {
        self.editing_notion_api_token = false;
        self.notion_api_token_buffer.clear();
        self.status = Some("Cancelled Notion API token edit.".to_string());
    }

    pub(crate) fn apply_notion_api_token_edit(&mut self) {
        let new_value = self.notion_api_token_buffer.trim().to_string();
        if new_value != self.notion_api_token {
            self.notion_api_token = new_value;
            self.dirty = true;
            self.status = Some("Updated Notion API token.".to_string());
        } else {
            self.status = Some("Notion API token unchanged.".to_string());
        }
        self.editing_notion_api_token = false;
        self.notion_api_token_buffer.clear();
    }

    pub(crate) fn backspace_notion_api_token(&mut self) {
        self.notion_api_token_buffer.pop();
        self.status = Some("Editing Notion API token...".to_string());
    }

    pub(crate) fn push_notion_api_token_char(&mut self, ch: char) {
        self.notion_api_token_buffer.push(ch);
        self.status = Some("Editing Notion API token...".to_string());
    }

    pub(crate) fn masked_notion_api_token(&self) -> String {
        mask_secret(&self.notion_api_token)
    }

    pub(crate) fn masked_notion_api_token_buffer(&self) -> String {
        mask_secret(&self.notion_api_token_buffer)
    }

    pub(crate) fn is_editing_learnchain_site_url(&self) -> bool {
        self.editing_learnchain_site_url
    }

    pub(crate) fn start_editing_learnchain_site_url(&mut self) {
        self.editing_learnchain_site_url = true;
        self.learnchain_site_url_buffer = self.learnchain_site_url.clone();
        self.status =
            Some("Editing LearnChain site URL (Enter to save, Esc to cancel)".to_string());
    }

    pub(crate) fn cancel_learnchain_site_url_edit(&mut self) {
        self.editing_learnchain_site_url = false;
        self.learnchain_site_url_buffer.clear();
        self.status = Some("Cancelled LearnChain site URL edit.".to_string());
    }

    pub(crate) fn apply_learnchain_site_url_edit(&mut self) {
        let requested_default = self.learnchain_site_url_buffer.trim().is_empty();
        let new_value = normalize_learnchain_site_url(&self.learnchain_site_url_buffer);
        if new_value != self.learnchain_site_url {
            self.learnchain_site_url = new_value;
            self.clear_learnchain_auth_feedback();
            self.dirty = true;
            self.status = Some(if requested_default {
                format!(
                    "Reset LearnChain site URL to the default ({}). Dashboard: {}",
                    LEARNCHAIN_DEFAULT_SITE_URL,
                    learnchain_dashboard_url(&self.learnchain_site_url)
                )
            } else {
                format!(
                    "Updated LearnChain site URL. Dashboard: {}",
                    learnchain_dashboard_url(&self.learnchain_site_url)
                )
            });
        } else if requested_default {
            self.status = Some(format!(
                "LearnChain site URL already matches the default ({}).",
                LEARNCHAIN_DEFAULT_SITE_URL
            ));
        } else {
            self.status = Some("LearnChain site URL unchanged.".to_string());
        }
        self.editing_learnchain_site_url = false;
        self.learnchain_site_url_buffer.clear();
    }

    pub(crate) fn backspace_learnchain_site_url(&mut self) {
        self.learnchain_site_url_buffer.pop();
        self.status = Some("Editing LearnChain site URL...".to_string());
    }

    pub(crate) fn push_learnchain_site_url_char(&mut self, ch: char) {
        self.learnchain_site_url_buffer.push(ch);
        self.status = Some("Editing LearnChain site URL...".to_string());
    }

    pub(crate) fn learnchain_site_url_buffer(&self) -> &str {
        &self.learnchain_site_url_buffer
    }

    pub(crate) fn is_editing_learnchain_auth_code(&self) -> bool {
        self.editing_learnchain_auth_code
    }

    pub(crate) fn start_editing_learnchain_auth_code(&mut self) {
        self.editing_learnchain_auth_code = true;
        self.learnchain_auth_code_buffer = self.learnchain_auth_code.clone();
        self.status =
            Some("Editing LearnChain login code (Enter to save, Esc to cancel)".to_string());
    }

    pub(crate) fn cancel_learnchain_auth_code_edit(&mut self) {
        self.editing_learnchain_auth_code = false;
        self.learnchain_auth_code_buffer.clear();
        self.status = Some("Cancelled LearnChain login code edit.".to_string());
    }

    pub(crate) fn apply_learnchain_auth_code_edit(&mut self) {
        let new_value = self.learnchain_auth_code_buffer.trim().to_uppercase();
        if new_value != self.learnchain_auth_code {
            self.learnchain_auth_code = new_value;
            self.clear_learnchain_auth_feedback();
            self.dirty = true;
            self.status = Some("Updated LearnChain login code.".to_string());
        } else {
            self.status = Some("LearnChain login code unchanged.".to_string());
        }
        self.editing_learnchain_auth_code = false;
        self.learnchain_auth_code_buffer.clear();
    }

    pub(crate) fn backspace_learnchain_auth_code(&mut self) {
        self.learnchain_auth_code_buffer.pop();
        self.status = Some("Editing LearnChain login code...".to_string());
    }

    pub(crate) fn push_learnchain_auth_code_char(&mut self, ch: char) {
        self.learnchain_auth_code_buffer
            .push(ch.to_ascii_uppercase());
        self.status = Some("Editing LearnChain login code...".to_string());
    }

    pub(crate) fn learnchain_auth_code_buffer(&self) -> &str {
        &self.learnchain_auth_code_buffer
    }

    pub(crate) fn clear_learnchain_session(&mut self) {
        self.learnchain_email.clear();
        self.learnchain_access_token.clear();
        self.learnchain_refresh_token.clear();
        self.learnchain_password.clear();
        self.learnchain_auth_code.clear();
        self.learnchain_auth_code_buffer.clear();
        self.editing_learnchain_auth_code = false;
        self.clear_learnchain_auth_feedback();
        self.dirty = true;
        self.status = Some("Cleared LearnChain account authorization.".to_string());
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
        self.editing_document_repository_target
            || self.editing_notion_api_token
            || self.editing_learnchain_site_url
            || self.editing_learnchain_auth_code
            || self.editing_openai_key
            || self.editing_anthropic_key
            || self.editing_openrouter_key
            || self.editing_openrouter_model
    }

    /// Returns the currently selected field.
    pub(crate) fn current_field(&self) -> ConfigField {
        self.field
    }

    fn sync_field_to_selected_section(&mut self) {
        let visible = self.visible_fields_in_section(self.selected_section);
        if !visible.contains(&self.field)
            && let Some(first) = visible.first().copied()
        {
            self.field = first;
        }
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

pub(crate) fn validate_document_repository_target(
    repository: DocumentRepositoryKind,
    value: &str,
) -> std::result::Result<(), String> {
    if repository == DocumentRepositoryKind::None {
        return Ok(());
    }

    if repository == DocumentRepositoryKind::LearnChain {
        return Ok(());
    }

    if value.trim().is_empty() {
        return Err(match repository {
            DocumentRepositoryKind::Notion => {
                "For Notion, enter the destination database/page ID or the full Notion URL."
                    .to_string()
            }
            DocumentRepositoryKind::LearnChain => {
                "Document repository target cannot be empty.".to_string()
            }
            DocumentRepositoryKind::None => {
                "Document repository target cannot be empty.".to_string()
            }
        });
    }

    Ok(())
}

pub(crate) fn notion_token_help_message() -> &'static str {
    "Notion export requires a Notion API token. Select \"Notion API token\" and press Enter. In Notion, create an internal integration, copy its token, and connect that integration to the target database."
}

pub(crate) fn codex_cli_config_help_message() -> &'static str {
    "Codex CLI uses your existing codex login and default model/profile. LearnChain will pass only the prepared prompt and output schema."
}

pub(crate) fn claude_code_cli_config_help_message() -> &'static str {
    "Claude Code CLI uses your existing claude login and default model/profile. LearnChain will pass the prepared prompt, system instructions, and output schema."
}

pub(crate) fn validate_learnchain_site_url(value: &str) -> std::result::Result<(), String> {
    let normalized = normalize_learnchain_site_url(value);
    let url = reqwest::Url::parse(&normalized).map_err(|_| {
        "Enter a valid LearnChain URL like https://learnchain.co or http://localhost:3000."
            .to_string()
    })?;

    match url.scheme() {
        "http" | "https" => Ok(()),
        _ => Err("LearnChain URL must start with http:// or https://.".to_string()),
    }
}

pub(crate) fn learnchain_signup_url(site_url: &str) -> String {
    format!(
        "{}/login",
        normalize_learnchain_site_url(site_url).trim_end_matches('/')
    )
}

pub(crate) fn learnchain_dashboard_url(site_url: &str) -> String {
    format!(
        "{}/dashboard",
        normalize_learnchain_site_url(site_url).trim_end_matches('/')
    )
}

pub(crate) fn learnchain_authorization_help_message(site_url: &str) -> String {
    format!(
        "LearnChain upload requires authorization. Sign in at {}, generate a CLI login code, then paste it into LearnChain login code here.",
        learnchain_dashboard_url(site_url)
    )
}

fn normalize_learnchain_site_url(value: &str) -> String {
    let trimmed = value.trim();
    let resolved = if trimmed.is_empty() {
        LEARNCHAIN_DEFAULT_SITE_URL
    } else {
        trimmed
    };
    let resolved = if resolved.starts_with("localhost:")
        || resolved.starts_with("127.0.0.1:")
        || resolved.starts_with("0.0.0.0:")
    {
        format!("http://{}", resolved)
    } else {
        resolved.to_string()
    };

    if let Ok(mut url) = reqwest::Url::parse(&resolved) {
        let host = url.host_str().unwrap_or_default();
        if matches!(host, "learnchain.co" | "www.learnchain.co") {
            let _ = url.set_scheme("https");
            let _ = url.set_host(Some("learnchain.co"));
            return url.to_string().trim_end_matches('/').to_string();
        }
    }

    resolved.trim_end_matches('/').to_string()
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
    use tempfile::tempdir;

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
                .setup_help()
                .contains("config set openai-key")
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
                .setup_help()
                .contains("config set anthropic-key")
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
                .setup_help()
                .contains("config set openrouter-key")
        );
    }

    #[test]
    fn app_config_resolved_llm_for_codex_cli() {
        let config = AppConfig {
            ai_provider: AiProvider::CodexCli,
            ..AppConfig::default()
        };

        let resolved = config.resolved_llm();
        assert_eq!(resolved.provider, AiProvider::CodexCli);
        assert_eq!(resolved.model_name, "codex-exec");
        assert_eq!(resolved.model_label, "CLI default");
        assert!(resolved.api_key.is_empty());
        assert!(AiProvider::CodexCli.setup_help().contains("Codex CLI"));
    }

    #[test]
    fn app_config_resolved_llm_for_claude_code_cli() {
        let config = AppConfig {
            ai_provider: AiProvider::ClaudeCodeCli,
            ..AppConfig::default()
        };

        let resolved = config.resolved_llm();
        assert_eq!(resolved.provider, AiProvider::ClaudeCodeCli);
        assert_eq!(resolved.model_name, "claude-code-print");
        assert_eq!(resolved.model_label, "CLI default");
        assert!(resolved.api_key.is_empty());
        assert!(
            AiProvider::ClaudeCodeCli
                .setup_help()
                .contains("Claude Code CLI")
        );
    }

    #[test]
    fn app_config_default_document_repository_settings_are_initialized() {
        let config = AppConfig::default();
        assert_eq!(config.document_repository, DocumentRepositoryKind::None);
        assert!(config.document_repository_target.is_empty());
        assert_eq!(config.learnchain_site_url, LEARNCHAIN_DEFAULT_SITE_URL);
        assert!(config.learnchain_email.is_empty());
        assert!(config.learnchain_access_token.is_empty());
        assert!(config.learnchain_refresh_token.is_empty());
        assert!(config.learnchain_password.is_empty());
    }

    #[test]
    fn document_repository_target_validation_accepts_empty_for_none_and_text_for_notion() {
        assert!(validate_document_repository_target(DocumentRepositoryKind::None, "").is_ok());
        assert!(
            validate_document_repository_target(DocumentRepositoryKind::LearnChain, "").is_ok()
        );
        assert!(
            validate_document_repository_target(DocumentRepositoryKind::Notion, "database/abc")
                .is_ok()
        );
    }

    #[test]
    fn document_repository_target_validation_rejects_empty_values_for_selected_repository() {
        assert!(validate_document_repository_target(DocumentRepositoryKind::Notion, "").is_err());
        assert!(
            validate_document_repository_target(DocumentRepositoryKind::Notion, "   ").is_err()
        );
    }

    #[test]
    fn config_form_round_trips_document_repository_target() {
        let config = AppConfig {
            document_repository: DocumentRepositoryKind::Notion,
            document_repository_target: "database/abc".to_string(),
            notion_api_token: "secret_notion".to_string(),
            ..AppConfig::default()
        };
        let mut form = ConfigForm::from_config(config.clone());
        assert_eq!(form.document_repository, DocumentRepositoryKind::Notion);
        assert_eq!(form.document_repository_target, "database/abc");
        assert_eq!(form.notion_api_token, "secret_notion");

        form.apply_saved(config);
        assert_eq!(form.document_repository, DocumentRepositoryKind::Notion);
        assert_eq!(form.document_repository_target, "database/abc");
        assert_eq!(form.notion_api_token, "secret_notion");
        assert!(!form.is_editing_document_repository_target());
    }

    #[test]
    fn config_form_visible_fields_hide_repository_target_when_no_repository_is_selected() {
        let form = ConfigForm::from_config(AppConfig::default());
        assert_eq!(
            form.visible_fields(),
            vec![
                ConfigField::MaxEvents,
                ConfigField::MinQuiz,
                ConfigField::SamplingPercentage,
                ConfigField::SessionSource,
                ConfigField::DeepDiveSessionMetadata,
                ConfigField::DeepDiveGoal,
                ConfigField::DeepDiveAccomplishments,
                ConfigField::DeepDiveInterestingLearnings,
                ConfigField::DeepDiveTeachingNarrative,
                ConfigField::DeepDiveReviewedExternalSources,
                ConfigField::DeepDiveReferencedUrls,
                ConfigField::OutputArtifacts,
                ConfigField::DocumentRepository,
                ConfigField::AiProvider,
                ConfigField::OpenAiModel,
                ConfigField::OpenAiKey,
            ]
        );
    }

    #[test]
    fn config_form_visible_fields_include_document_repository_target_for_selected_repository() {
        let form = ConfigForm::from_config(AppConfig {
            document_repository: DocumentRepositoryKind::Notion,
            document_repository_target: "database/abc".to_string(),
            ..AppConfig::default()
        });
        assert_eq!(
            form.visible_fields(),
            vec![
                ConfigField::MaxEvents,
                ConfigField::MinQuiz,
                ConfigField::SamplingPercentage,
                ConfigField::SessionSource,
                ConfigField::DeepDiveSessionMetadata,
                ConfigField::DeepDiveGoal,
                ConfigField::DeepDiveAccomplishments,
                ConfigField::DeepDiveInterestingLearnings,
                ConfigField::DeepDiveTeachingNarrative,
                ConfigField::DeepDiveReviewedExternalSources,
                ConfigField::DeepDiveReferencedUrls,
                ConfigField::OutputArtifacts,
                ConfigField::DocumentRepository,
                ConfigField::DocumentRepositoryTarget,
                ConfigField::NotionApiToken,
                ConfigField::AiProvider,
                ConfigField::OpenAiModel,
                ConfigField::OpenAiKey,
            ]
        );
    }

    #[test]
    fn config_form_visible_fields_stop_at_provider_for_codex_cli() {
        let form = ConfigForm::from_config(AppConfig {
            ai_provider: AiProvider::CodexCli,
            ..AppConfig::default()
        });
        assert_eq!(
            form.visible_fields(),
            vec![
                ConfigField::MaxEvents,
                ConfigField::MinQuiz,
                ConfigField::SamplingPercentage,
                ConfigField::SessionSource,
                ConfigField::DeepDiveSessionMetadata,
                ConfigField::DeepDiveGoal,
                ConfigField::DeepDiveAccomplishments,
                ConfigField::DeepDiveInterestingLearnings,
                ConfigField::DeepDiveTeachingNarrative,
                ConfigField::DeepDiveReviewedExternalSources,
                ConfigField::DeepDiveReferencedUrls,
                ConfigField::OutputArtifacts,
                ConfigField::DocumentRepository,
                ConfigField::AiProvider,
            ]
        );
    }

    #[test]
    fn config_form_visible_fields_stop_at_provider_for_claude_code_cli() {
        let form = ConfigForm::from_config(AppConfig {
            ai_provider: AiProvider::ClaudeCodeCli,
            ..AppConfig::default()
        });
        assert_eq!(
            form.visible_fields(),
            vec![
                ConfigField::MaxEvents,
                ConfigField::MinQuiz,
                ConfigField::SamplingPercentage,
                ConfigField::SessionSource,
                ConfigField::DeepDiveSessionMetadata,
                ConfigField::DeepDiveGoal,
                ConfigField::DeepDiveAccomplishments,
                ConfigField::DeepDiveInterestingLearnings,
                ConfigField::DeepDiveTeachingNarrative,
                ConfigField::DeepDiveReviewedExternalSources,
                ConfigField::DeepDiveReferencedUrls,
                ConfigField::OutputArtifacts,
                ConfigField::DocumentRepository,
                ConfigField::AiProvider,
            ]
        );
    }

    #[test]
    fn config_form_visible_fields_include_learnchain_fields_for_selected_repository() {
        let form = ConfigForm::from_config(AppConfig {
            document_repository: DocumentRepositoryKind::LearnChain,
            ..AppConfig::default()
        });
        assert_eq!(
            form.visible_fields(),
            vec![
                ConfigField::MaxEvents,
                ConfigField::MinQuiz,
                ConfigField::SamplingPercentage,
                ConfigField::SessionSource,
                ConfigField::DeepDiveSessionMetadata,
                ConfigField::DeepDiveGoal,
                ConfigField::DeepDiveAccomplishments,
                ConfigField::DeepDiveInterestingLearnings,
                ConfigField::DeepDiveTeachingNarrative,
                ConfigField::DeepDiveReviewedExternalSources,
                ConfigField::DeepDiveReferencedUrls,
                ConfigField::OutputArtifacts,
                ConfigField::DocumentRepository,
                ConfigField::LearnChainSiteUrl,
                ConfigField::LearnChainEmail,
                ConfigField::LearnChainAuthCode,
                ConfigField::AiProvider,
                ConfigField::OpenAiModel,
                ConfigField::OpenAiKey,
            ]
        );
    }

    #[test]
    fn config_form_groups_fields_into_sections() {
        let form = ConfigForm::from_config(AppConfig {
            document_repository: DocumentRepositoryKind::Notion,
            ai_provider: AiProvider::OpenRouter,
            openrouter_model: "openrouter/model".to_string(),
            ..AppConfig::default()
        });

        assert_eq!(
            form.visible_fields_in_section(ConfigSection::Session),
            vec![
                ConfigField::MaxEvents,
                ConfigField::MinQuiz,
                ConfigField::SamplingPercentage,
                ConfigField::SessionSource,
            ]
        );
        assert_eq!(
            form.visible_fields_in_section(ConfigSection::DeepDive),
            vec![
                ConfigField::DeepDiveSessionMetadata,
                ConfigField::DeepDiveGoal,
                ConfigField::DeepDiveAccomplishments,
                ConfigField::DeepDiveInterestingLearnings,
                ConfigField::DeepDiveTeachingNarrative,
                ConfigField::DeepDiveReviewedExternalSources,
                ConfigField::DeepDiveReferencedUrls,
            ]
        );
        assert_eq!(
            form.visible_fields_in_section(ConfigSection::Export),
            vec![
                ConfigField::OutputArtifacts,
                ConfigField::DocumentRepository,
                ConfigField::DocumentRepositoryTarget,
                ConfigField::NotionApiToken,
            ]
        );
        assert_eq!(
            form.visible_fields_in_section(ConfigSection::Ai),
            vec![
                ConfigField::AiProvider,
                ConfigField::OpenRouterModel,
                ConfigField::OpenRouterKey,
            ]
        );
    }

    #[test]
    fn config_form_section_navigation_updates_the_current_step() {
        let mut form = ConfigForm::from_config(AppConfig::default());

        assert_eq!(form.selected_section(), ConfigSection::Session);
        assert_eq!(form.current_field(), ConfigField::MaxEvents);
        assert_eq!(form.selected_index_in_section(), 0);

        form.select_next();
        assert_eq!(form.selected_section(), ConfigSection::DeepDive);
        assert_eq!(form.current_field(), ConfigField::DeepDiveSessionMetadata);
        assert_eq!(form.selected_index_in_section(), 0);

        form.focus_step_navigation();
        form.select_next();
        form.select_next();
        assert_eq!(form.current_field(), ConfigField::DeepDiveAccomplishments);
        assert_eq!(form.selected_index_in_section(), 2);

        form.focus_section_navigation();
        form.select_previous();
        form.select_previous();
        assert_eq!(form.selected_section(), ConfigSection::Ai);
        assert_eq!(form.current_field(), ConfigField::AiProvider);
        assert_eq!(form.selected_index_in_section(), 0);
    }

    #[test]
    fn deep_dive_section_toggles_mark_form_dirty() {
        let mut form = ConfigForm::from_config(AppConfig::default());
        form.selected_section = ConfigSection::DeepDive;
        form.field = ConfigField::DeepDiveReviewedExternalSources;
        assert!(form.deep_dive_sections.reviewed_external_sources);

        form.adjust_current(1);

        assert!(!form.deep_dive_sections.reviewed_external_sources);
        assert!(form.dirty);
    }

    #[test]
    fn document_repository_target_edit_marks_form_dirty_only_on_change() {
        let mut form = ConfigForm::from_config(AppConfig::default());
        form.document_repository = DocumentRepositoryKind::Notion;
        form.start_editing_document_repository_target();
        for ch in "database/abc".chars() {
            form.push_document_repository_target_char(ch);
        }
        form.apply_document_repository_target_edit();
        assert!(form.dirty);
        assert_eq!(form.document_repository_target, "database/abc");

        form.dirty = false;
        form.start_editing_document_repository_target();
        form.apply_document_repository_target_edit();
        assert!(!form.dirty);
    }

    #[test]
    fn learnchain_auth_feedback_defaults_to_neutral_and_resets_after_save() {
        let mut form = ConfigForm::from_config(AppConfig::default());
        assert_eq!(
            form.learnchain_auth_feedback(),
            LearnChainAuthFeedback::Neutral
        );

        form.mark_learnchain_auth_success();
        assert_eq!(
            form.learnchain_auth_feedback(),
            LearnChainAuthFeedback::Success
        );

        form.apply_saved(AppConfig::default());
        assert_eq!(
            form.learnchain_auth_feedback(),
            LearnChainAuthFeedback::Neutral
        );
    }

    #[test]
    fn learnchain_auth_feedback_clears_when_login_code_changes() {
        let mut form = ConfigForm::from_config(AppConfig::default());
        form.learnchain_auth_code = "OLDCODE".to_string();
        form.mark_learnchain_auth_failure();

        form.start_editing_learnchain_auth_code();
        form.learnchain_auth_code_buffer = "newcode".to_string();
        form.apply_learnchain_auth_code_edit();

        assert_eq!(form.learnchain_auth_code, "NEWCODE");
        assert_eq!(
            form.learnchain_auth_feedback(),
            LearnChainAuthFeedback::Neutral
        );
    }

    #[test]
    fn learnchain_auth_feedback_clears_when_site_url_changes_or_session_is_cleared() {
        let mut form = ConfigForm::from_config(AppConfig::default());
        form.learnchain_site_url = "http://localhost:3000".to_string();
        form.mark_learnchain_auth_success();

        form.start_editing_learnchain_site_url();
        form.learnchain_site_url_buffer = "https://learnchain.ai".to_string();
        form.apply_learnchain_site_url_edit();
        assert_eq!(
            form.learnchain_auth_feedback(),
            LearnChainAuthFeedback::Neutral
        );

        form.mark_learnchain_auth_failure();
        form.learnchain_email = "user@example.com".to_string();
        form.learnchain_access_token = "token".to_string();
        form.clear_learnchain_session();
        assert_eq!(
            form.learnchain_auth_feedback(),
            LearnChainAuthFeedback::Neutral
        );
    }

    #[test]
    fn learnchain_site_url_normalizes_bare_localhost_hosts() {
        assert_eq!(
            normalize_learnchain_site_url("localhost:3000"),
            "http://localhost:3000"
        );
        assert_eq!(
            normalize_learnchain_site_url("127.0.0.1:3000/"),
            "http://127.0.0.1:3000"
        );
    }

    #[test]
    fn config_form_document_repository_selector_cycles_supported_repositories() {
        let mut form = ConfigForm::from_config(AppConfig::default());
        assert_eq!(form.document_repository, DocumentRepositoryKind::None);

        form.selected_section = ConfigSection::Export;
        form.field = ConfigField::DocumentRepository;
        form.adjust_current(1);
        assert_eq!(form.document_repository, DocumentRepositoryKind::Notion);

        form.adjust_current(1);
        assert_eq!(form.document_repository, DocumentRepositoryKind::LearnChain);

        form.adjust_current(1);
        assert_eq!(form.document_repository, DocumentRepositoryKind::None);
    }

    #[test]
    fn ai_provider_selector_cycles_through_cli_providers() {
        let mut form = ConfigForm::from_config(AppConfig::default());
        form.selected_section = ConfigSection::Ai;
        form.field = ConfigField::AiProvider;

        form.adjust_current(1);
        assert_eq!(form.ai_provider, AiProvider::Anthropic);

        form.adjust_current(1);
        assert_eq!(form.ai_provider, AiProvider::OpenRouter);

        form.adjust_current(1);
        assert_eq!(form.ai_provider, AiProvider::CodexCli);
        assert_eq!(
            form.status.as_deref(),
            Some(codex_cli_config_help_message())
        );

        form.adjust_current(1);
        assert_eq!(form.ai_provider, AiProvider::ClaudeCodeCli);
        assert_eq!(
            form.status.as_deref(),
            Some(claude_code_cli_config_help_message())
        );

        form.adjust_current(1);
        assert_eq!(form.ai_provider, AiProvider::OpenAI);

        form.adjust_current(-1);
        assert_eq!(form.ai_provider, AiProvider::ClaudeCodeCli);

        form.adjust_current(-1);
        assert_eq!(form.ai_provider, AiProvider::CodexCli);
    }

    #[test]
    fn learnchain_signup_url_uses_normalized_site_url() {
        assert_eq!(
            learnchain_signup_url("http://localhost:3000/"),
            "http://localhost:3000/login"
        );
    }

    #[test]
    fn learnchain_signup_url_canonicalizes_public_host_to_https() {
        assert_eq!(
            learnchain_signup_url("https://learnchain.co/"),
            "https://learnchain.co/login"
        );
        assert_eq!(
            learnchain_dashboard_url("http://learnchain.co"),
            "https://learnchain.co/dashboard"
        );
    }

    #[test]
    fn empty_learnchain_site_url_edit_resets_to_default() {
        let mut form = ConfigForm::from_config(AppConfig::default());
        form.learnchain_site_url = "http://localhost:3000".to_string();

        form.start_editing_learnchain_site_url();
        form.learnchain_site_url_buffer.clear();
        form.apply_learnchain_site_url_edit();

        assert_eq!(form.learnchain_site_url, LEARNCHAIN_DEFAULT_SITE_URL);
        assert_eq!(
            form.status.as_deref(),
            Some(
                "Reset LearnChain site URL to the default (https://learnchain.co). Dashboard: https://learnchain.co/dashboard"
            )
        );
    }

    #[test]
    fn app_config_normalize_migrates_legacy_prefixed_notion_target() {
        let mut config: AppConfig = toml::from_str(
            r#"
default_max_events = 15
min_quiz_questions = 5
session_source = "codex"
write_output_artifacts = false
document_repository_target = "notion:database/abc"
ai_provider = "open_a_i"
openai_model = "gpt5-mini"
openai_api_key = ""
anthropic_model = "claude-sonnet4"
anthropic_api_key = ""
openrouter_model = ""
openrouter_api_key = ""
sampling_percentage = 10
"#,
        )
        .unwrap();

        config.normalize();
        assert_eq!(config.document_repository, DocumentRepositoryKind::Notion);
        assert_eq!(config.document_repository_target, "database/abc");
    }

    #[test]
    fn deep_dive_sections_default_to_all_enabled() {
        let config = AppConfig::default();
        assert_eq!(
            config.deep_dive_sections.enabled_count(),
            DeepDiveSectionsConfig::total_count()
        );
    }

    #[test]
    fn notion_api_token_edit_marks_form_dirty_only_on_change() {
        let mut form = ConfigForm::from_config(AppConfig::default());
        form.start_editing_notion_api_token();
        for ch in "secret_notion".chars() {
            form.push_notion_api_token_char(ch);
        }
        form.apply_notion_api_token_edit();
        assert!(form.dirty);
        assert_eq!(form.notion_api_token, "secret_notion");

        form.dirty = false;
        form.start_editing_notion_api_token();
        form.apply_notion_api_token_edit();
        assert!(!form.dirty);
    }

    #[test]
    fn app_config_deserializes_without_document_repository_target() {
        let config: AppConfig = toml::from_str(
            r#"
default_max_events = 15
min_quiz_questions = 5
session_source = "codex"
write_output_artifacts = false
document_repository = "none"
ai_provider = "open_a_i"
openai_model = "gpt5-mini"
openai_api_key = ""
anthropic_model = "claude-sonnet4"
anthropic_api_key = ""
openrouter_model = ""
openrouter_api_key = ""
sampling_percentage = 10
"#,
        )
        .unwrap();

        assert!(config.document_repository_target.is_empty());
        assert!(config.notion_api_token.is_empty());
        assert_eq!(config.learnchain_site_url, LEARNCHAIN_DEFAULT_SITE_URL);
        assert!(config.learnchain_email.is_empty());
        assert!(config.learnchain_access_token.is_empty());
        assert!(config.learnchain_refresh_token.is_empty());
        assert!(config.learnchain_password.is_empty());
        assert_eq!(
            config.deep_dive_sections.enabled_count(),
            DeepDiveSectionsConfig::total_count()
        );
    }

    #[test]
    fn app_config_deserializes_codex_cli_provider() {
        let config: AppConfig = toml::from_str(
            r#"
default_max_events = 15
min_quiz_questions = 5
session_source = "codex"
write_output_artifacts = false
document_repository = "none"
ai_provider = "codex_cli"
openai_model = "gpt5-mini"
openai_api_key = ""
anthropic_model = "claude-sonnet4"
anthropic_api_key = ""
openrouter_model = ""
openrouter_api_key = ""
sampling_percentage = 10
"#,
        )
        .unwrap();

        assert_eq!(config.ai_provider, AiProvider::CodexCli);
    }

    #[test]
    fn app_config_deserializes_claude_code_cli_provider() {
        let config: AppConfig = toml::from_str(
            r#"
default_max_events = 15
min_quiz_questions = 5
session_source = "codex"
write_output_artifacts = false
document_repository = "none"
ai_provider = "claude_code_cli"
openai_model = "gpt5-mini"
openai_api_key = ""
anthropic_model = "claude-sonnet4"
anthropic_api_key = ""
openrouter_model = ""
openrouter_api_key = ""
sampling_percentage = 10
"#,
        )
        .unwrap();

        assert_eq!(config.ai_provider, AiProvider::ClaudeCodeCli);
    }

    #[test]
    fn resolve_global_config_dir_returns_none_without_supported_env_vars() {
        assert_eq!(resolve_global_config_dir(None, None, None), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_global_config_dir_uses_application_support_on_macos() {
        let dir = resolve_global_config_dir(None, None, Some(PathBuf::from("/Users/tester")));
        assert_eq!(
            dir,
            Some(PathBuf::from(
                "/Users/tester/Library/Application Support/learnchain"
            ))
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn resolve_global_config_dir_uses_appdata_on_windows() {
        let dir = resolve_global_config_dir(
            None,
            Some(PathBuf::from(r"C:\Users\tester\AppData\Roaming")),
            Some(PathBuf::from(r"C:\Users\tester")),
        );
        assert_eq!(
            dir,
            Some(PathBuf::from(r"C:\Users\tester\AppData\Roaming\LearnChain"))
        );
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn resolve_global_config_dir_uses_xdg_or_home_on_unix() {
        let xdg_dir = resolve_global_config_dir(
            Some(PathBuf::from("/tmp/config-home")),
            None,
            Some(PathBuf::from("/home/tester")),
        );
        assert_eq!(xdg_dir, Some(PathBuf::from("/tmp/config-home/learnchain")));

        let home_dir = resolve_global_config_dir(None, None, Some(PathBuf::from("/home/tester")));
        assert_eq!(
            home_dir,
            Some(PathBuf::from("/home/tester/.config/learnchain"))
        );
    }

    #[test]
    fn load_config_from_paths_falls_back_to_legacy_file_and_migrates_it() {
        let temp = tempdir().unwrap();
        let primary_path = temp.path().join("global").join("app_config.toml");
        let legacy_path = temp.path().join("config").join("app_config.toml");

        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(
            &legacy_path,
            r#"
default_max_events = 21
min_quiz_questions = 5
session_source = "codex"
write_output_artifacts = false
document_repository = "none"
ai_provider = "codex_cli"
openai_model = "gpt5-mini"
openai_api_key = ""
anthropic_model = "claude-sonnet4"
anthropic_api_key = ""
openrouter_model = ""
openrouter_api_key = ""
sampling_percentage = 10
"#,
        )
        .unwrap();

        let config = load_config_from_paths(&primary_path, &legacy_path).unwrap();
        assert_eq!(config.default_max_events, 21);
        assert_eq!(config.ai_provider, AiProvider::CodexCli);

        let migrated = load_config_from_path(&primary_path).unwrap().unwrap();
        assert_eq!(migrated.default_max_events, 21);
        assert_eq!(migrated.ai_provider, AiProvider::CodexCli);
    }
}
