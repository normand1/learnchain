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
use view_managers::{ConfigManager, LearningManager, MenuManager, menu_manager::MENU_OPTIONS};

const DEFAULT_OPENAI_MODEL: &str = "gpt-5-mini";
pub(crate) const AI_LOADING_FRAMES: [&str; 4] = ["-", "\\", "|", "/"];

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
    /// Most recent session file for today, if any.
    pub(crate) latest_file: Option<PathBuf>,
    /// Absolute path to the aggregated markdown summary, if generated.
    pub(crate) summary_file: Option<PathBuf>,
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

        let manager = SessionManager::new();
        let SessionLoad {
            session_date,
            session_dir,
            latest_file,
            events,
            error: session_error,
            ..
        } = manager.load_today_events();
        let output_manager = OutputManager::new();
        let (summary_file, summary_error) =
            output_manager.write_markdown_summary(&events, &session_date, latest_file.as_deref());
        let selected_event = if events.is_empty() { None } else { Some(0) };
        if let Some(error) = session_error {
            Self::push_error(&mut aggregated_error, error);
        }
        if let Some(summary_error) = summary_error {
            Self::push_error(&mut aggregated_error, summary_error);
        }
        let ai_manager = match AiManager::from_env("output", DEFAULT_OPENAI_MODEL) {
            Ok(manager) => Some(manager),
            Err(err) => {
                Self::push_error(&mut aggregated_error, format!("AI unavailable: {}", err));
                None
            }
        };
        Self {
            running: false,
            view: AppView::Menu,
            menu_index: 0,
            events,
            selected_event,
            session_dir,
            session_date,
            latest_file,
            summary_file,
            error: aggregated_error,
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
            config_form: ConfigForm::from_config(config::current()),
        }
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
