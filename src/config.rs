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
        }
    }
}

const DEFAULT_MAX_EVENTS: usize = 15;
const DEFAULT_MIN_QUIZ_QUESTIONS: usize = 5;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigField {
    MaxEvents,
    MinQuiz,
}

#[derive(Debug, Clone)]
pub struct ConfigForm {
    pub(crate) max_events: usize,
    pub(crate) min_quiz_questions: usize,
    field: ConfigField,
    pub(crate) dirty: bool,
    pub(crate) status: Option<String>,
}

impl ConfigForm {
    pub(crate) fn from_config(config: AppConfig) -> Self {
        Self {
            max_events: config.default_max_events,
            min_quiz_questions: config.min_quiz_questions,
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

        let (value, minimum) = match self.field {
            ConfigField::MaxEvents => (&mut self.max_events, 1),
            ConfigField::MinQuiz => (&mut self.min_quiz_questions, 1),
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
        self.dirty = false;
        self.status = None;
    }

    pub(crate) fn set_status<S: Into<String>>(&mut self, status: S) {
        self.status = Some(status.into());
    }
}

impl ConfigField {
    fn index(self) -> usize {
        match self {
            Self::MaxEvents => 0,
            Self::MinQuiz => 1,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::MaxEvents => Self::MinQuiz,
            Self::MinQuiz => Self::MaxEvents,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::MaxEvents => Self::MinQuiz,
            Self::MinQuiz => Self::MaxEvents,
        }
    }
}
