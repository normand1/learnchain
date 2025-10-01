mod ai_manager;
mod config;
mod log_util;
mod markdown_rules;
mod output_manager;
mod session_manager;
mod ui_renderer;
mod view_managers;

use ai_manager::{AiManager, StructuredLearningResponse, poll_ai_messages};
use color_eyre::Result;
use config::ConfigForm;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use dotenvy::dotenv;
use output_manager::OutputManager;
use ratatui::{DefaultTerminal, Frame};
use session_manager::{SessionEvent, SessionLoad, SessionManager};
use std::{path::PathBuf, sync::mpsc::Receiver, time::Duration};
use ui_renderer::UiRenderer;
use view_managers::{ConfigManager, LearningManager, MenuManager};

pub(crate) const AI_LOADING_FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
pub(crate) const OPENAI_KEY_HELP: &str = "OpenAI API key not configured. Open the Config view (select \"OpenAI API key\" and press Enter) or run `learnchain --set-openai-key <your-key>` to add it.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppView {
    Menu,
    Events,
    Learning,
    Config,
}

#[derive(Debug)]
enum AiTaskMessage {
    Success(StructuredLearningResponse),
    Error(String),
}

pub(crate) fn reset_learning_feedback(
    feedback: &mut Option<String>,
    summary_revealed: &mut bool,
    waiting_for_next: &mut bool,
) {
    *feedback = None;
    *summary_revealed = false;
    *waiting_for_next = false;
}

fn main() -> color_eyre::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--set-openai-key" => {
                if let Some(key) = args.get(2) {
                    config::update(|cfg| cfg.openai_api_key = key.trim().to_string())?;
                    println!("Stored OpenAI API key in config/app_config.toml.");
                    return Ok(());
                } else {
                    eprintln!("Usage: learnchain --set-openai-key <key>");
                    std::process::exit(1);
                }
            }
            "--clear-openai-key" => {
                config::update(|cfg| cfg.openai_api_key.clear())?;
                println!("Cleared OpenAI API key from config/app_config.toml.");
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "learnchain options:\n  --set-openai-key <key>    store your OpenAI API key in the app config\n  --clear-openai-key       remove the stored OpenAI API key\n  --help                   show this message\n  --version                show version"
                );
                return Ok(());
            }
            "--version" | "-V" => {
                println!("learnchain {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {
                eprintln!(
                    "Unrecognized option '{}'. Run `learnchain --help` for usage.",
                    args[1]
                );
                std::process::exit(1);
            }
        }
    }

    dotenv().ok();
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = App::new().run(terminal);
    ratatui::restore();
    result
}

/// The main application which holds the state and logic of the application.
#[derive(Debug)]
pub struct App {
    /// Is the application running?
    pub(crate) running: bool,
    /// Current view being displayed.
    pub(crate) view: AppView,
    /// Currently selected index in the main menu.
    pub(crate) menu_index: usize,
    /// Parsed session events filtered for function calls.
    pub(crate) events: Vec<SessionEvent>,
    /// Currently selected event index.
    pub(crate) selected_event: Option<usize>,
    /// Absolute path to today's session directory.
    pub(crate) session_dir: PathBuf,
    /// Human-readable label for today's date.
    pub(crate) session_date: String,
    /// Label describing the active session source.
    pub(crate) session_source: String,
    /// Most recent session file for today, if any.
    pub(crate) latest_file: Option<PathBuf>,
    /// Absolute path to the aggregated markdown summary, if generated.
    pub(crate) summary_file: Option<PathBuf>,
    /// Markdown summary content cached in memory.
    pub(crate) summary_content: Option<String>,
    /// Any error encountered while loading files or parsing events.
    pub(crate) error: Option<String>,
    /// Lazily configured OpenAI integration.
    pub(crate) ai_manager: Option<AiManager>,
    /// Latest status message related to AI generation requests.
    pub(crate) ai_status: Option<String>,
    /// Indicates whether an AI request is currently running.
    pub(crate) ai_loading: bool,
    /// Spinner frame index for the active loading indicator.
    pub(crate) ai_loading_frame: usize,
    /// Receives background AI task updates.
    pub(crate) ai_result_receiver: Option<Receiver<AiTaskMessage>>,
    /// Cached learning response from the most recent AI generation.
    pub(crate) learning_response: Option<StructuredLearningResponse>,
    /// Index of the currently selected knowledge group within the learning response.
    pub(crate) learning_group_index: usize,
    /// Index of the currently selected quiz item within the active knowledge group.
    pub(crate) learning_quiz_index: usize,
    /// Index of the currently selected answer option within the active quiz item.
    pub(crate) learning_option_index: usize,
    /// Feedback for the most recent answer selection.
    pub(crate) learning_feedback: Option<String>,
    /// Whether the current quiz summary should be revealed.
    pub(crate) learning_summary_revealed: bool,
    /// Indicates that the correct answer was chosen and we are waiting to advance.
    pub(crate) learning_waiting_for_next: bool,
    /// Holds the editable configuration state when rendering the config view.
    pub(crate) config_form: ConfigForm,
    /// Whether artifacts should be written to disk.
    pub(crate) write_output_artifacts: bool,
    /// Currently selected OpenAI model.
    pub(crate) openai_model: config::OpenAiModelKind,
}

impl App {
    /// Construct a new instance of [`App`].
    pub fn new() -> Self {
        let mut aggregated_error: Option<String> = None;

        if let Err(err) = config::initialize() {
            Self::push_error(
                &mut aggregated_error,
                format!("Configuration load failed: {}", err),
            );
        }

        let config_snapshot = config::current();
        let write_output_artifacts = config_snapshot.write_output_artifacts;
        let openai_model = config_snapshot.openai_model;
        let session_manager = SessionManager::from_source(config_snapshot.session_source);
        let session_load = session_manager.load_today_events();

        let openai_key = config_snapshot.openai_api_key.clone();
        let ai_manager = if openai_key.trim().is_empty() {
            None
        } else {
            match AiManager::from_config("output", openai_model.as_model_name(), openai_key.clone())
            {
                Ok(manager) => Some(manager),
                Err(err) => {
                    Self::push_error(&mut aggregated_error, format!("AI unavailable: {}", err));
                    None
                }
            }
        };

        let mut app = Self {
            running: false,
            view: AppView::Menu,
            menu_index: 0,
            events: Vec::new(),
            selected_event: None,
            session_dir: PathBuf::new(),
            session_date: String::new(),
            session_source: String::new(),
            latest_file: None,
            summary_file: None,
            summary_content: None,
            error: None,
            ai_manager,
            ai_status: None,
            ai_loading: false,
            ai_loading_frame: 0,
            ai_result_receiver: None,
            learning_response: None,
            learning_group_index: 0,
            learning_quiz_index: 0,
            learning_option_index: 0,
            learning_feedback: None,
            learning_summary_revealed: false,
            learning_waiting_for_next: false,
            config_form: ConfigForm::from_config(config_snapshot.clone()),
            write_output_artifacts,
            openai_model,
        };

        app.apply_session_load(session_load);

        if app.ai_manager.is_none() && openai_key.trim().is_empty() {
            app.ai_status = Some(OPENAI_KEY_HELP.to_string());
        } else {
            app.ai_status = None;
        }

        if let Some(error) = aggregated_error {
            Self::push_error(&mut app.error, error);
        }

        app
    }

    /// Run the application's main loop.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;
        let tick_rate = Duration::from_millis(120);
        while self.running {
            poll_ai_messages(&mut self);
            terminal.draw(|frame| self.render(frame))?;
            self.handle_crossterm_events(tick_rate)?;
        }
        Ok(())
    }

    /// Dispatch rendering based on the active view.
    fn render(&mut self, frame: &mut Frame) {
        UiRenderer::new(self).render(frame);
    }

    fn apply_session_load(&mut self, load: SessionLoad) {
        self.session_source = load.source;
        self.session_date = load.session_date;
        self.session_dir = load.session_dir;
        self.latest_file = load.latest_file;
        self.events = load.events;
        self.selected_event = if self.events.is_empty() {
            None
        } else {
            Some(0)
        };
        self.error = load.error;

        let output_manager = OutputManager::new();
        let artifact = output_manager.write_markdown_summary(
            &self.events,
            &self.session_date,
            self.latest_file.as_deref(),
            self.write_output_artifacts,
        );
        self.summary_file = artifact.path;
        self.summary_content = Some(artifact.content);
        if let Some(summary_error) = artifact.error {
            Self::push_error(&mut self.error, summary_error);
        }
    }

    pub(crate) fn reload_session_from_config(&mut self) {
        let config_snapshot = config::current();
        self.write_output_artifacts = config_snapshot.write_output_artifacts;
        self.openai_model = config_snapshot.openai_model;
        if config_snapshot.openai_api_key.trim().is_empty() {
            self.ai_manager = None;
            App::push_error(&mut self.error, OPENAI_KEY_HELP.to_string());
            self.ai_status = Some(OPENAI_KEY_HELP.to_string());
        } else {
            let key = config_snapshot.openai_api_key.clone();
            self.ai_manager =
                AiManager::from_config("output", self.openai_model.as_model_name(), key).ok();
            self.ai_status = None;
        }
        let manager = SessionManager::from_source(config_snapshot.session_source);
        let load = manager.load_today_events();
        self.apply_session_load(load);
    }

    /// Reads the crossterm events and updates the state of [`App`].
    fn handle_crossterm_events(&mut self, tick_rate: Duration) -> Result<()> {
        if event::poll(tick_rate)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key),
                Event::Mouse(_) => {}
                Event::Resize(_, _) => {}
                _ => {}
            }
            poll_ai_messages(self);
        } else {
            self.on_tick();
        }
        Ok(())
    }

    fn on_tick(&mut self) {
        if self.ai_loading {
            self.ai_loading_frame = (self.ai_loading_frame + 1) % AI_LOADING_FRAMES.len();
            self.update_loading_status();
        }
        poll_ai_messages(self);
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc | KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => self.quit(),
            _ => match self.view {
                AppView::Menu => MenuManager::new(self).handle_menu_key(key),
                AppView::Events => MenuManager::new(self).handle_events_key(key),
                AppView::Learning => LearningManager::new(self).handle_key(key),
                AppView::Config => ConfigManager::new(self).handle_key(key),
            },
        }
    }

    pub(crate) fn return_to_menu(&mut self) {
        if matches!(self.view, AppView::Config) {
            self.config_form = ConfigForm::from_config(config::current());
        }
        self.view = AppView::Menu;
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }

    /// Append a message to an optional error slot.
    pub(crate) fn push_error(slot: &mut Option<String>, message: String) {
        if let Some(existing) = slot {
            existing.push_str(" | ");
            existing.push_str(&message);
        } else {
            *slot = Some(message);
        }
    }
}
