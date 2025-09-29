mod ai_manager;
mod config;
mod learning_manager;
mod log_util;
mod markdown_rules;
mod output_manager;
mod session_manager;
mod ui_renderer;

use ai_manager::{AiManager, StructuredLearningResponse};
use color_eyre::{
    Result,
    eyre::{WrapErr, eyre},
};
use config::ConfigForm;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use dotenvy::dotenv;
use learning_manager::LearningManager;
use log_util::log_debug;
use output_manager::OutputManager;
use ratatui::{DefaultTerminal, Frame};
use serde_json::to_string_pretty;
use session_manager::{SessionEvent, SessionLoad, SessionManager};
use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Duration,
};
use tokio::runtime::Runtime;
use ui_renderer::UiRenderer;

pub(crate) const MENU_OPTIONS: [&str; 3] = [
    "1. View session events",
    "2. Generate learning response",
    "3. Configure defaults",
];
const DEFAULT_OPENAI_MODEL: &str = "gpt-5-mini";
pub(crate) const AI_LOADING_FRAMES: [&str; 4] = ["-", "\\", "|", "/"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppView {
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
    ai_result_receiver: Option<Receiver<AiTaskMessage>>,
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
            self.poll_ai_messages();
            terminal.draw(|frame| self.render(frame))?;
            self.handle_crossterm_events(tick_rate)?;
        }
        Ok(())
    }

    /// Dispatch rendering based on the active view.
    fn render(&mut self, frame: &mut Frame) {
        UiRenderer::new(self).render(frame);
    }
    /// Renders the learning response view.
    fn on_config_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => {
                self.config_form.select_next();
            }
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => {
                self.config_form.select_previous();
            }
            (KeyModifiers::NONE, KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-')) => {
                self.config_form.adjust_current(-1);
            }
            (
                KeyModifiers::NONE,
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('='),
            ) => {
                self.config_form.adjust_current(1);
            }
            (KeyModifiers::NONE, KeyCode::Char('s')) | (KeyModifiers::NONE, KeyCode::Enter) => {
                self.save_config_changes();
            }
            (KeyModifiers::NONE, KeyCode::Char('r')) => self.reset_config_form(),
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.return_to_menu(),
            _ => {}
        }
    }

    fn save_config_changes(&mut self) {
        if !self.config_form.dirty {
            self.config_form.set_status("No pending changes to save.");
            return;
        }

        let target_max = self.config_form.max_events;
        let target_min = self.config_form.min_quiz_questions;

        match config::update(|config| {
            config.default_max_events = target_max;
            config.min_quiz_questions = target_min;
        }) {
            Ok(updated) => {
                self.config_form.apply_saved(updated);
                self.config_form.set_status(format!(
                    "Saved configuration to {}",
                    config::config_file_path().display()
                ));
                log_debug("App: configuration saved");
            }
            Err(err) => {
                Self::push_error(
                    &mut self.error,
                    format!("Failed to save configuration: {}", err),
                );
                self.config_form
                    .set_status("Failed to save configuration. Check error panel.");
                log_debug(&format!("App: failed to save configuration: {}", err));
            }
        }
    }

    fn reset_config_form(&mut self) {
        let current = config::current();
        self.config_form = ConfigForm::from_config(current);
        self.config_form
            .set_status("Reverted to saved configuration values.");
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
            self.poll_ai_messages();
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
        self.poll_ai_messages();
    }

    fn poll_ai_messages(&mut self) {
        let mut clear_receiver = false;
        if let Some(receiver) = self.ai_result_receiver.as_ref() {
            match receiver.try_recv() {
                Ok(message) => {
                    self.ai_loading = false;
                    clear_receiver = true;
                    match message {
                        AiTaskMessage::Success(response) => self.handle_ai_success(response),
                        AiTaskMessage::Error(message) => self.handle_ai_error(message),
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.ai_loading = false;
                    clear_receiver = true;
                    self.handle_ai_error("Background AI worker disconnected".to_string());
                }
            }
        }

        if clear_receiver {
            self.ai_result_receiver = None;
        }
    }

    fn update_loading_status(&mut self) {
        if self.ai_loading {
            let frame = AI_LOADING_FRAMES[self.ai_loading_frame % AI_LOADING_FRAMES.len()];
            self.ai_status = Some(format!("{} Generating learning response…", frame));
        }
    }

    fn handle_ai_success(&mut self, mut structured: StructuredLearningResponse) {
        LearningManager::shuffle_quiz_options(&mut structured);
        let group_count = structured.response.len();
        let total_questions: usize = structured
            .response
            .iter()
            .map(|group| group.quiz.len())
            .sum();

        let save_result = self.write_ai_response(&structured);
        let mut status_parts = Vec::new();
        match save_result {
            Ok(saved_path) => {
                status_parts.push(format!("Saved to {}", saved_path.display()));
                log_debug(&format!(
                    "App: learning response saved to {}",
                    saved_path.display()
                ));
            }
            Err(err) => {
                Self::push_error(
                    &mut self.error,
                    format!("Failed to save learning response: {}", err),
                );
                status_parts.push("Failed to save learning response".to_string());
                log_debug(&format!("App: failed to write learning response: {}", err));
            }
        }

        status_parts.push(format!("Knowledge groups: {}", group_count));
        status_parts.push(format!("Total quiz questions: {}", total_questions));
        self.ai_status = Some(status_parts.join(" • "));

        self.learning_group_index = 0;
        self.learning_quiz_index = 0;
        self.learning_option_index = 0;
        reset_learning_feedback(
            &mut self.learning_feedback,
            &mut self.learning_summary_revealed,
            &mut self.learning_waiting_for_next,
        );
        self.learning_response = Some(structured);
        log_debug(&format!(
            "App: loaded learning response with {} group(s)",
            group_count
        ));

        self.view = AppView::Learning;
        log_debug("App: switched to learning view");
    }

    fn handle_ai_error(&mut self, message: String) {
        let trimmed = message.trim().to_string();
        if trimmed.starts_with("Failed to build Tokio runtime") {
            Self::push_error(&mut self.error, trimmed.clone());
            log_debug(&format!("App: {}", trimmed));
            self.ai_status = Some("Unable to start AI runtime".to_string());
        } else {
            Self::push_error(
                &mut self.error,
                format!("AI generation failed: {}", trimmed),
            );
            log_debug(&format!("App: AI generation error: {}", trimmed));
            self.ai_status = Some("AI generation failed".to_string());
        }

        if !matches!(self.view, AppView::Learning) {
            self.view = AppView::Menu;
        }
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc | KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => self.quit(),
            _ => match self.view {
                AppView::Menu => self.on_menu_key(key),
                AppView::Events => self.on_events_key(key),
                AppView::Learning => self.on_learning_key(key),
                AppView::Config => self.on_config_key(key),
            },
        }
    }

    fn on_menu_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => self.menu_next(),
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => self.menu_previous(),
            (KeyModifiers::NONE, KeyCode::Enter) => self.activate_menu_option(),
            (KeyModifiers::NONE, KeyCode::Char('1')) => {
                self.menu_index = 0;
                self.activate_menu_option();
            }
            (KeyModifiers::NONE, KeyCode::Char('2')) => {
                self.menu_index = 1;
                self.activate_menu_option();
            }
            (KeyModifiers::NONE, KeyCode::Char('3')) => {
                self.menu_index = 2;
                self.activate_menu_option();
            }
            (KeyModifiers::NONE, KeyCode::Char('c') | KeyCode::Char('C')) => self.show_config(),
            (KeyModifiers::NONE, KeyCode::Char('l')) => self.show_learning(),
            _ => {}
        }
    }

    fn on_events_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => self.select_next(),
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => self.select_previous(),
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.return_to_menu(),
            (KeyModifiers::NONE, KeyCode::Char('l')) => self.show_learning(),
            _ => {}
        }
    }

    fn menu_next(&mut self) {
        self.menu_index = (self.menu_index + 1) % MENU_OPTIONS.len();
    }

    fn menu_previous(&mut self) {
        if self.menu_index == 0 {
            self.menu_index = MENU_OPTIONS.len() - 1;
        } else {
            self.menu_index -= 1;
        }
    }

    fn activate_menu_option(&mut self) {
        match self.menu_index {
            0 => self.show_events(),
            1 => self.generate_ai_learning_response(),
            2 => self.show_config(),
            _ => {}
        }
    }

    fn show_events(&mut self) {
        self.view = AppView::Events;
        if self.events.is_empty() {
            self.selected_event = None;
        } else if self.selected_event.is_none() {
            self.selected_event = Some(0);
        }
    }

    fn show_learning(&mut self) {
        if self.learning_response.is_some() {
            self.view = AppView::Learning;
            self.ensure_learning_indices();
            log_debug("App: opened learning view");
        } else {
            Self::push_error(
                &mut self.error,
                "No learning response available. Generate one from the menu.".to_string(),
            );
        }
    }

    fn show_config(&mut self) {
        self.config_form = ConfigForm::from_config(config::current());
        self.config_form
            .set_status("Use ←/→ to adjust values, s to save changes.");
        self.view = AppView::Config;
    }

    fn on_learning_key(&mut self, key: KeyEvent) {
        if self.learning_waiting_for_next {
            self.learning_waiting_for_next = false;
            LearningManager::new(self).next_question();
            return;
        }
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Left | KeyCode::Char('h')) => {
                LearningManager::new(self).previous_group()
            }
            (KeyModifiers::NONE, KeyCode::Right | KeyCode::Char('l')) => {
                LearningManager::new(self).next_group()
            }
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => {
                LearningManager::new(self).next_option()
            }
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => {
                LearningManager::new(self).previous_option()
            }
            (KeyModifiers::NONE, KeyCode::Char('n'))
            | (KeyModifiers::NONE, KeyCode::Char('N'))
            | (KeyModifiers::NONE, KeyCode::Char(']'))
            | (KeyModifiers::NONE, KeyCode::Char('}'))
            | (KeyModifiers::NONE, KeyCode::PageDown)
            | (KeyModifiers::NONE, KeyCode::Tab) => LearningManager::new(self).next_question(),
            (KeyModifiers::NONE, KeyCode::Char('p'))
            | (KeyModifiers::NONE, KeyCode::Char('P'))
            | (KeyModifiers::NONE, KeyCode::Char('['))
            | (KeyModifiers::NONE, KeyCode::Char('{'))
            | (KeyModifiers::NONE, KeyCode::PageUp)
            | (KeyModifiers::NONE, KeyCode::BackTab) => {
                LearningManager::new(self).previous_question()
            }
            (KeyModifiers::NONE, KeyCode::Enter)
            | (KeyModifiers::NONE, KeyCode::Char(' '))
            | (KeyModifiers::NONE, KeyCode::Char('s')) => {
                LearningManager::new(self).select_option()
            }
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.return_to_menu(),
            (KeyModifiers::NONE, KeyCode::Char('e')) => self.show_events(),
            _ => {}
        }
    }

    fn return_to_menu(&mut self) {
        if matches!(self.view, AppView::Config) {
            self.config_form = ConfigForm::from_config(config::current());
        }
        self.view = AppView::Menu;
    }

    fn generate_ai_learning_response(&mut self) {
        log_debug("App: menu option 'Generate learning response' selected");
        if self.ai_loading {
            log_debug("App: AI generation already in progress; ignoring duplicate request");
            return;
        }

        let manager = match self.ai_manager.clone() {
            Some(manager) => manager,
            None => {
                Self::push_error(
                    &mut self.error,
                    "AI manager unavailable. Configure OPENAI_API_KEY.".to_string(),
                );
                log_debug("App: AI manager unavailable; aborting generation");
                return;
            }
        };

        let (sender, receiver) = mpsc::channel();
        self.ai_result_receiver = Some(receiver);
        self.ai_loading = true;
        self.ai_loading_frame = 0;
        self.update_loading_status();
        self.view = AppView::Learning;
        log_debug("App: displaying learning loading spinner");
        log_debug("App: starting OpenAI generation task");

        thread::spawn(move || {
            log_debug("App: background OpenAI generation task started");
            let runtime = match Runtime::new() {
                Ok(runtime) => runtime,
                Err(err) => {
                    let _ = sender.send(AiTaskMessage::Error(format!(
                        "Failed to build Tokio runtime: {}",
                        err
                    )));
                    return;
                }
            };

            let result = runtime.block_on(manager.generate_learning_response());
            drop(runtime);

            match result {
                Ok(structured) => {
                    let _ = sender.send(AiTaskMessage::Success(structured));
                }
                Err(err) => {
                    let _ = sender.send(AiTaskMessage::Error(err.to_string()));
                }
            }
        });
    }

    pub(crate) fn ensure_learning_indices(&mut self) {
        if let Some(response) = &self.learning_response {
            if response.response.is_empty() {
                self.learning_group_index = 0;
                self.learning_quiz_index = 0;
                self.learning_option_index = 0;
                reset_learning_feedback(
                    &mut self.learning_feedback,
                    &mut self.learning_summary_revealed,
                    &mut self.learning_waiting_for_next,
                );
                return;
            }

            if self.learning_group_index >= response.response.len() {
                self.learning_group_index = 0;
                reset_learning_feedback(
                    &mut self.learning_feedback,
                    &mut self.learning_summary_revealed,
                    &mut self.learning_waiting_for_next,
                );
            }

            if let Some(group) = response.response.get(self.learning_group_index) {
                if group.quiz.is_empty() {
                    self.learning_quiz_index = 0;
                    self.learning_option_index = 0;
                    reset_learning_feedback(
                        &mut self.learning_feedback,
                        &mut self.learning_summary_revealed,
                        &mut self.learning_waiting_for_next,
                    );
                } else if self.learning_quiz_index >= group.quiz.len() {
                    self.learning_quiz_index = 0;
                    reset_learning_feedback(
                        &mut self.learning_feedback,
                        &mut self.learning_summary_revealed,
                        &mut self.learning_waiting_for_next,
                    );
                }

                if let Some(question) = group.quiz.get(self.learning_quiz_index) {
                    if question.options.is_empty() {
                        self.learning_option_index = 0;
                        reset_learning_feedback(
                            &mut self.learning_feedback,
                            &mut self.learning_summary_revealed,
                            &mut self.learning_waiting_for_next,
                        );
                    } else if self.learning_option_index >= question.options.len() {
                        self.learning_option_index = 0;
                        reset_learning_feedback(
                            &mut self.learning_feedback,
                            &mut self.learning_summary_revealed,
                            &mut self.learning_waiting_for_next,
                        );
                    }
                } else {
                    self.learning_option_index = 0;
                    reset_learning_feedback(
                        &mut self.learning_feedback,
                        &mut self.learning_summary_revealed,
                        &mut self.learning_waiting_for_next,
                    );
                }
            } else {
                self.learning_quiz_index = 0;
                self.learning_option_index = 0;
                reset_learning_feedback(
                    &mut self.learning_feedback,
                    &mut self.learning_summary_revealed,
                    &mut self.learning_waiting_for_next,
                );
            }
        } else {
            self.learning_group_index = 0;
            self.learning_quiz_index = 0;
            self.learning_option_index = 0;
            reset_learning_feedback(
                &mut self.learning_feedback,
                &mut self.learning_summary_revealed,
                &mut self.learning_waiting_for_next,
            );
        }
    }

    fn write_ai_response(&self, response: &StructuredLearningResponse) -> Result<PathBuf> {
        let manager = OutputManager::new();
        let output_dir = manager.output_directory().map_err(|err| eyre!(err))?;
        fs::create_dir_all(&output_dir).wrap_err_with(|| {
            format!(
                "failed to create output directory at {}",
                output_dir.display()
            )
        })?;

        let mut path = output_dir.join(format!("learning-response-{}.json", self.session_date));
        let mut counter = 2;
        while path.exists() {
            path = output_dir.join(format!(
                "learning-response-{}-{}.json",
                self.session_date, counter
            ));
            counter += 1;
        }

        let serialized =
            to_string_pretty(response).wrap_err("failed to serialise learning response to JSON")?;
        fs::write(&path, serialized)
            .wrap_err_with(|| format!("failed to write learning response to {}", path.display()))?;
        Ok(path)
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }

    /// Move selection to the next event, wrapping to the start.
    fn select_next(&mut self) {
        if self.events.is_empty() {
            self.selected_event = None;
            return;
        }
        let next = match self.selected_event {
            Some(index) if index + 1 < self.events.len() => index + 1,
            _ => 0,
        };
        self.selected_event = Some(next);
    }

    /// Move selection to the previous event, wrapping to the end.
    fn select_previous(&mut self) {
        if self.events.is_empty() {
            self.selected_event = None;
            return;
        }
        let previous = match self.selected_event {
            Some(index) if index > 0 => index - 1,
            _ => self.events.len() - 1,
        };
        self.selected_event = Some(previous);
    }

    /// Append a message to an optional error slot.
    fn push_error(slot: &mut Option<String>, message: String) {
        if let Some(existing) = slot {
            existing.push_str(" | ");
            existing.push_str(&message);
        } else {
            *slot = Some(message);
        }
    }
}
