use super::{
    analytics_manager::AnalyticsManager, config_manager::ConfigManager,
    events_manager::EventsManager, learnchain_setup_manager::LearnChainSetupManager,
    learning_manager::LearningManager, library_manager::LibraryManager,
    session_picker_manager::SessionPickerManager, skill_installer_manager::SkillInstallerManager,
};
use crate::{App, SessionSelectionTarget};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuOptionSection {
    Actions,
    Config,
    CodingToolInstallers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuOptionAction {
    LearnChainSetup,
    GenerateQuiz,
    GenerateSessionDeepDive,
    ViewLibrary,
    ViewAnalyticsDashboard,
    ViewSessionEvents,
    ConfigureDetails,
    InstallLearnChainAgentSkill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MenuOption {
    pub(crate) label: String,
    pub(crate) section: MenuOptionSection,
    pub(crate) action: MenuOptionAction,
    pub(crate) highlight_red: bool,
}

pub(crate) fn menu_options(app: &App) -> Vec<MenuOption> {
    let mut options = Vec::new();
    let mut number = 1;

    let mut push_option =
        |section: MenuOptionSection, action: MenuOptionAction, label: &str, highlight_red: bool| {
            options.push(MenuOption {
                label: format!("{}. {}", number, label),
                section,
                action,
                highlight_red,
            });
            number += 1;
        };

    if app.should_show_learnchain_setup_action() {
        push_option(
            MenuOptionSection::Actions,
            MenuOptionAction::LearnChainSetup,
            "First-time LearnChain setup",
            true,
        );
    }

    push_option(
        MenuOptionSection::Actions,
        MenuOptionAction::GenerateSessionDeepDive,
        "Generate session deep dive",
        false,
    );
    push_option(
        MenuOptionSection::Actions,
        MenuOptionAction::GenerateQuiz,
        "Generate quiz",
        false,
    );
    push_option(
        MenuOptionSection::Actions,
        MenuOptionAction::ViewLibrary,
        "View library",
        false,
    );
    push_option(
        MenuOptionSection::Actions,
        MenuOptionAction::ViewAnalyticsDashboard,
        "View analytics dashboard",
        false,
    );
    push_option(
        MenuOptionSection::Config,
        MenuOptionAction::ViewSessionEvents,
        "View session events",
        false,
    );
    push_option(
        MenuOptionSection::Config,
        MenuOptionAction::ConfigureDetails,
        "Configure details",
        false,
    );
    push_option(
        MenuOptionSection::CodingToolInstallers,
        MenuOptionAction::InstallLearnChainAgentSkill,
        "Install LearnChain agent skill",
        false,
    );

    options
}

pub(crate) struct MenuManager<'a> {
    app: &'a mut App,
}

impl<'a> MenuManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn handle_menu_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => self.menu_next(),
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => self.menu_previous(),
            (KeyModifiers::NONE, KeyCode::Enter) => self.activate_menu_option(),
            (KeyModifiers::NONE, KeyCode::Char(ch)) if ch.is_ascii_digit() => {
                let Some(index) = ch.to_digit(10).and_then(|digit| digit.checked_sub(1)) else {
                    return;
                };
                let options = menu_options(self.app);
                let index = index as usize;
                if index < options.len() {
                    self.app.menu_index = index;
                    self.activate_menu_option();
                }
            }
            (KeyModifiers::NONE, KeyCode::Char('c') | KeyCode::Char('C')) => {
                ConfigManager::new(self.app).show_config()
            }
            (KeyModifiers::NONE, KeyCode::Char('l')) => LearningManager::show_learning(self.app),
            _ => {}
        }
    }

    pub(crate) fn handle_events_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => {
                EventsManager::new(self.app).select_next()
            }
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => {
                EventsManager::new(self.app).select_previous()
            }
            (KeyModifiers::NONE, KeyCode::Enter) => EventsManager::new(self.app).drill_down(),
            (KeyModifiers::NONE, KeyCode::Backspace) => {
                let went_back = EventsManager::new(self.app).go_back();
                if !went_back {
                    self.app.return_to_menu();
                }
            }
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.app.return_to_menu(),
            (KeyModifiers::NONE, KeyCode::Char('l')) => LearningManager::show_learning(self.app),
            _ => {}
        }
    }

    fn menu_next(&mut self) {
        let options = menu_options(self.app);
        if options.is_empty() {
            self.app.menu_index = 0;
            return;
        }
        self.app.menu_index = (self.app.menu_index + 1) % options.len();
    }

    fn menu_previous(&mut self) {
        let options = menu_options(self.app);
        if options.is_empty() {
            self.app.menu_index = 0;
            return;
        }
        if self.app.menu_index == 0 || self.app.menu_index >= options.len() {
            self.app.menu_index = options.len() - 1;
        } else {
            self.app.menu_index -= 1;
        }
    }

    fn activate_menu_option(&mut self) {
        let options = menu_options(self.app);
        let Some(option) = options.get(self.app.menu_index) else {
            self.app.menu_index = 0;
            return;
        };

        match option.action {
            MenuOptionAction::LearnChainSetup => LearnChainSetupManager::show(self.app),
            MenuOptionAction::GenerateQuiz => {
                SessionPickerManager::show(self.app, SessionSelectionTarget::Quiz)
            }
            MenuOptionAction::GenerateSessionDeepDive => {
                SessionPickerManager::show(self.app, SessionSelectionTarget::DeepDive)
            }
            MenuOptionAction::ViewLibrary => LibraryManager::show_library(self.app),
            MenuOptionAction::ViewAnalyticsDashboard => AnalyticsManager::show_analytics(self.app),
            MenuOptionAction::ViewSessionEvents => EventsManager::show_events(self.app),
            MenuOptionAction::ConfigureDetails => ConfigManager::new(self.app).show_config(),
            MenuOptionAction::InstallLearnChainAgentSkill => SkillInstallerManager::show(self.app),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AiTaskMessage, App, AppView, DeepDiveDocument, EmbeddedSkillTarget, LearnChainSetupState,
        QuizSummaryResult, config,
        llm::{DeepDiveHistoryEntry, LlmBackend, StructuredLearningResponse},
        output_manager::LibraryArtifactEntry,
        session_sources::{Session, SessionEvent},
    };
    use std::{
        collections::HashSet,
        path::PathBuf,
        sync::mpsc::{Receiver, Sender},
        time::Instant,
    };

    fn test_app() -> App {
        App {
            running: false,
            view: AppView::Menu,
            menu_index: 0,
            events: Vec::<SessionEvent>::new(),
            selected_event: None,
            sessions: Vec::<Session>::new(),
            selected_session: None,
            viewing_sessions_list: true,
            session_dir: PathBuf::new(),
            session_date: String::new(),
            session_source: String::new(),
            latest_file: None,
            summary_file: None,
            summary_content: None,
            error: None,
            ai_provider: config::AiProvider::OpenAI,
            llm_backend: None::<LlmBackend>,
            ai_status: None,
            ai_loading: false,
            ai_task_kind: None,
            ai_loading_frame: 0,
            ai_loading_start: None::<Instant>,
            ai_progress_percent: 0,
            ai_progress_message: String::new(),
            ai_progress_timeline: Vec::new(),
            ai_result_receiver: None::<Receiver<AiTaskMessage>>,
            ai_sender: None::<Sender<AiTaskMessage>>,
            document_export_receiver: None,
            document_export_loading: false,
            learning_response: None::<StructuredLearningResponse>,
            active_quiz_session_date: String::new(),
            learning_group_index: 0,
            learning_quiz_index: 0,
            learning_option_index: 0,
            learning_feedback: None,
            learning_summary_revealed: false,
            learning_waiting_for_next: false,
            session_selection_target: None,
            session_picker_selected_session: None,
            projects: Vec::new(),
            session_picker_selected_project: None,
            session_picker_viewing_projects: true,
            config_form: config::ConfigForm::from_config(config::AppConfig::default()),
            write_output_artifacts: false,
            openai_model: config::OpenAiModelKind::Gpt5Mini,
            anthropic_model: config::AnthropicModelKind::ClaudeSonnet4,
            openrouter_model: String::new(),
            quiz_first_attempts: HashSet::new(),
            quiz_first_attempt_results: std::collections::HashMap::new(),
            analytics_snapshot: None,
            analytics_error: None,
            analytics_refreshed_at: None,
            last_event_timestamp: None,
            last_quiz_event_timestamp: None,
            learning_showing_summary: false,
            quiz_summary_results: Vec::<QuizSummaryResult>::new(),
            deep_dive_document: None::<DeepDiveDocument>,
            deep_dive_history_document: None::<DeepDiveDocument>,
            deep_dive_scroll: 0,
            deep_dive_history: Vec::<DeepDiveHistoryEntry>::new(),
            deep_dive_history_selected: None,
            deep_dive_showing_history: false,
            library_artifacts: Vec::<LibraryArtifactEntry>::new(),
            library_selected: None,
            skill_installer_selected_target: EmbeddedSkillTarget::Codex,
            learnchain_setup: LearnChainSetupState::default(),
        }
    }

    #[test]
    fn menu_options_include_setup_when_learnchain_has_no_session() {
        let mut app = test_app();
        app.config_form.document_repository = config::DocumentRepositoryKind::LearnChain;
        app.config_form.learnchain_access_token.clear();
        app.config_form.learnchain_refresh_token.clear();

        let options = menu_options(&app);

        assert_eq!(options[0].action, MenuOptionAction::LearnChainSetup);
        assert_eq!(options[1].action, MenuOptionAction::GenerateSessionDeepDive);
    }

    #[test]
    fn menu_options_hide_setup_when_learnchain_session_exists() {
        let mut app = test_app();
        app.config_form.document_repository = config::DocumentRepositoryKind::LearnChain;
        app.config_form.learnchain_access_token = "token".to_string();
        app.config_form.learnchain_refresh_token.clear();

        let options = menu_options(&app);

        assert_ne!(options[0].action, MenuOptionAction::LearnChainSetup);
        assert_eq!(options[0].action, MenuOptionAction::GenerateSessionDeepDive);
        assert_eq!(options[1].action, MenuOptionAction::GenerateQuiz);
    }

    #[test]
    fn quick_selection_uses_dynamic_numbering_for_setup() {
        let mut app = test_app();
        app.config_form.document_repository = config::DocumentRepositoryKind::LearnChain;
        app.config_form.learnchain_access_token.clear();
        app.config_form.learnchain_refresh_token.clear();

        MenuManager::new(&mut app).handle_menu_key(KeyEvent::from(KeyCode::Char('1')));

        assert_eq!(app.view, AppView::LearnChainSetup);
    }

    #[test]
    fn quick_selection_uses_deep_dive_number_after_setup() {
        let mut app = test_app();
        app.config_form.document_repository = config::DocumentRepositoryKind::LearnChain;
        app.config_form.learnchain_access_token.clear();
        app.config_form.learnchain_refresh_token.clear();

        MenuManager::new(&mut app).handle_menu_key(KeyEvent::from(KeyCode::Char('2')));

        assert_eq!(app.view, AppView::SessionPicker);
        assert_eq!(
            app.session_selection_target,
            Some(SessionSelectionTarget::DeepDive)
        );
    }
}
