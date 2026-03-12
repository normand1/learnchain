use crate::{
    App, AppView, LearnChainSetupAuthMethod, LearnChainSetupField, LearnChainSetupState,
    LearnChainSetupStep, config, document_repository,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) struct LearnChainSetupManager<'a> {
    app: &'a mut App,
}

impl<'a> LearnChainSetupManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn show(app: &'a mut App) {
        let config_snapshot = config::current();
        let setup = LearnChainSetupState {
            email_input: config_snapshot.learnchain_email,
            status: Some(format!(
                "Create or sign into your LearnChain account at {}.",
                config::learnchain_signup_url(&config_snapshot.learnchain_site_url)
            )),
            ..LearnChainSetupState::default()
        };
        app.learnchain_setup = setup;
        app.view = AppView::LearnChainSetup;
        app.ai_status = Some("LearnChain setup is ready.".to_string());
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.app.learnchain_setup.step == LearnChainSetupStep::Success {
            self.app.return_to_menu();
            return;
        }

        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.app.return_to_menu(),
            _ => match self.app.learnchain_setup.step {
                LearnChainSetupStep::Account => self.handle_account_step(key),
                LearnChainSetupStep::Authenticate => self.handle_auth_step(key),
                LearnChainSetupStep::Success => self.app.return_to_menu(),
            },
        }
    }

    pub(crate) fn handle_paste(&mut self, text: &str) {
        if self.app.learnchain_setup.step != LearnChainSetupStep::Authenticate {
            return;
        }

        let sanitized: String = text
            .chars()
            .filter(|ch| *ch != '\n' && *ch != '\r')
            .collect();
        if sanitized.is_empty() {
            return;
        }

        match self.app.learnchain_setup.field {
            LearnChainSetupField::Email => {
                self.app.learnchain_setup.email_input.push_str(&sanitized)
            }
            LearnChainSetupField::Password => {
                self.app
                    .learnchain_setup
                    .password_input
                    .push_str(&sanitized);
            }
            LearnChainSetupField::AuthCode => {
                self.app
                    .learnchain_setup
                    .auth_code_input
                    .push_str(&sanitized.to_ascii_uppercase());
            }
            _ => {}
        }
    }

    fn handle_account_step(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')) => {
                self.app.learnchain_setup.step = LearnChainSetupStep::Authenticate;
                self.app.learnchain_setup.status = Some(
                    "Choose an authentication method, fill the active field, then submit."
                        .to_string(),
                );
                self.app.learnchain_setup.error = None;
            }
            (KeyModifiers::NONE, KeyCode::Backspace) => self.app.return_to_menu(),
            _ => {}
        }
    }

    fn handle_auth_step(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab) => {
                self.select_next_field()
            }
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab) => {
                self.select_previous_field()
            }
            (KeyModifiers::NONE, KeyCode::Left | KeyCode::Char('h')) => self.adjust_or_move(-1),
            (KeyModifiers::NONE, KeyCode::Right | KeyCode::Char('l')) => self.adjust_or_move(1),
            (KeyModifiers::NONE, KeyCode::Enter) => {
                if self.app.learnchain_setup.field == LearnChainSetupField::Submit {
                    self.submit_authentication();
                } else if self.app.learnchain_setup.field == LearnChainSetupField::AuthMethod {
                    self.toggle_auth_method();
                } else {
                    self.select_next_field();
                }
            }
            (KeyModifiers::NONE, KeyCode::Backspace) => self.handle_backspace(),
            (KeyModifiers::NONE, KeyCode::Char(ch)) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.push_input_char(ch);
                }
            }
            _ => {}
        }
    }

    fn visible_fields(&self) -> Vec<LearnChainSetupField> {
        let mut fields = vec![LearnChainSetupField::AuthMethod];
        match self.app.learnchain_setup.auth_method {
            LearnChainSetupAuthMethod::EmailPassword => {
                fields.push(LearnChainSetupField::Email);
                fields.push(LearnChainSetupField::Password);
            }
            LearnChainSetupAuthMethod::CliAuthCode => {
                fields.push(LearnChainSetupField::AuthCode);
            }
        }
        fields.push(LearnChainSetupField::Submit);
        fields
    }

    fn select_next_field(&mut self) {
        let fields = self.visible_fields();
        let current_index = fields
            .iter()
            .position(|field| *field == self.app.learnchain_setup.field)
            .unwrap_or(0);
        let next_index = (current_index + 1) % fields.len();
        self.app.learnchain_setup.field = fields[next_index];
    }

    fn select_previous_field(&mut self) {
        let fields = self.visible_fields();
        let current_index = fields
            .iter()
            .position(|field| *field == self.app.learnchain_setup.field)
            .unwrap_or(0);
        let previous_index = if current_index == 0 {
            fields.len() - 1
        } else {
            current_index - 1
        };
        self.app.learnchain_setup.field = fields[previous_index];
    }

    fn adjust_or_move(&mut self, direction: isize) {
        if self.app.learnchain_setup.field == LearnChainSetupField::AuthMethod {
            self.toggle_auth_method();
        } else if direction > 0 {
            self.select_next_field();
        } else {
            self.select_previous_field();
        }
    }

    fn toggle_auth_method(&mut self) {
        self.app.learnchain_setup.auth_method = match self.app.learnchain_setup.auth_method {
            LearnChainSetupAuthMethod::EmailPassword => LearnChainSetupAuthMethod::CliAuthCode,
            LearnChainSetupAuthMethod::CliAuthCode => LearnChainSetupAuthMethod::EmailPassword,
        };
        self.app.learnchain_setup.field = LearnChainSetupField::AuthMethod;
        self.app.learnchain_setup.error = None;
        self.app.learnchain_setup.status = Some(format!(
            "Using {} authentication.",
            self.app.learnchain_setup.auth_method.label()
        ));
    }

    fn handle_backspace(&mut self) {
        match self.app.learnchain_setup.field {
            LearnChainSetupField::Email => {
                if self.app.learnchain_setup.email_input.is_empty() {
                    self.select_previous_field();
                } else {
                    self.app.learnchain_setup.email_input.pop();
                }
            }
            LearnChainSetupField::Password => {
                if self.app.learnchain_setup.password_input.is_empty() {
                    self.select_previous_field();
                } else {
                    self.app.learnchain_setup.password_input.pop();
                }
            }
            LearnChainSetupField::AuthCode => {
                if self.app.learnchain_setup.auth_code_input.is_empty() {
                    self.select_previous_field();
                } else {
                    self.app.learnchain_setup.auth_code_input.pop();
                }
            }
            LearnChainSetupField::AuthMethod => self.app.return_to_menu(),
            LearnChainSetupField::Submit => self.select_previous_field(),
        }
    }

    fn push_input_char(&mut self, ch: char) {
        match self.app.learnchain_setup.field {
            LearnChainSetupField::Email => self.app.learnchain_setup.email_input.push(ch),
            LearnChainSetupField::Password => self.app.learnchain_setup.password_input.push(ch),
            LearnChainSetupField::AuthCode => self
                .app
                .learnchain_setup
                .auth_code_input
                .push(ch.to_ascii_uppercase()),
            LearnChainSetupField::AuthMethod | LearnChainSetupField::Submit => {}
        }
    }

    fn submit_authentication(&mut self) {
        self.app.learnchain_setup.error = None;
        self.app.learnchain_setup.status = Some("Authenticating LearnChain account...".to_string());

        let site_url = config::current().learnchain_site_url;
        let result = match self.app.learnchain_setup.auth_method {
            LearnChainSetupAuthMethod::EmailPassword => {
                let email = self.app.learnchain_setup.email_input.trim().to_string();
                let password = self.app.learnchain_setup.password_input.clone();
                if email.is_empty() || password.is_empty() {
                    self.set_auth_error(
                        "Enter both your LearnChain email and password before continuing."
                            .to_string(),
                    );
                    return;
                }
                document_repository::sign_in_learnchain_with_password(&site_url, &email, &password)
            }
            LearnChainSetupAuthMethod::CliAuthCode => {
                let auth_code = self.app.learnchain_setup.auth_code_input.trim().to_string();
                if auth_code.is_empty() {
                    self.set_auth_error(
                        "Paste your LearnChain CLI auth code before continuing.".to_string(),
                    );
                    return;
                }
                document_repository::exchange_learnchain_login_code(&site_url, &auth_code)
            }
        };

        match result {
            Ok(session) => match self.app.persist_learnchain_session(session) {
                Ok(account_label) => {
                    self.app.learnchain_setup.step = LearnChainSetupStep::Success;
                    self.app.learnchain_setup.success_account_label = account_label.clone();
                    self.app.learnchain_setup.password_input.clear();
                    self.app.learnchain_setup.auth_code_input.clear();
                    self.app.learnchain_setup.confetti_frame = 0;
                    self.app.learnchain_setup.status = Some(format!(
                        "LearnChain is linked as {}. Press any key to return to the menu.",
                        account_label
                    ));
                    self.app.learnchain_setup.error = None;
                    self.app.ai_status = Some(format!("LearnChain linked as {}.", account_label));
                }
                Err(err) => self.set_auth_error(format!(
                    "LearnChain authentication succeeded but saving the session failed: {}",
                    err
                )),
            },
            Err(err) => self.set_auth_error(err),
        }
    }

    fn set_auth_error(&mut self, message: String) {
        self.app.learnchain_setup.error = Some(message.clone());
        self.app.learnchain_setup.status = Some("Authentication failed.".to_string());
        self.app.error = Some(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AiTaskMessage, DeepDiveDocument, EmbeddedSkillTarget, QuizSummaryResult,
        SessionSelectionTarget, config,
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
            session_selection_target: None::<SessionSelectionTarget>,
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
    fn show_opens_setup_on_account_step() {
        let mut app = test_app();

        LearnChainSetupManager::show(&mut app);

        assert_eq!(app.view, AppView::LearnChainSetup);
        assert_eq!(app.learnchain_setup.step, LearnChainSetupStep::Account);
    }

    #[test]
    fn account_step_advances_to_authenticate() {
        let mut app = test_app();
        LearnChainSetupManager::show(&mut app);

        LearnChainSetupManager::new(&mut app).handle_key(KeyEvent::from(KeyCode::Enter));

        assert_eq!(app.learnchain_setup.step, LearnChainSetupStep::Authenticate);
    }

    #[test]
    fn switching_auth_methods_updates_visible_input_mode() {
        let mut app = test_app();
        LearnChainSetupManager::show(&mut app);
        app.learnchain_setup.step = LearnChainSetupStep::Authenticate;

        LearnChainSetupManager::new(&mut app).handle_key(KeyEvent::from(KeyCode::Right));

        assert_eq!(
            app.learnchain_setup.auth_method,
            LearnChainSetupAuthMethod::CliAuthCode
        );
        assert_eq!(app.learnchain_setup.field, LearnChainSetupField::AuthMethod);
    }

    #[test]
    fn empty_submit_stays_on_authenticate_and_sets_error() {
        let mut app = test_app();
        LearnChainSetupManager::show(&mut app);
        app.learnchain_setup.step = LearnChainSetupStep::Authenticate;
        app.learnchain_setup.field = LearnChainSetupField::Submit;

        LearnChainSetupManager::new(&mut app).handle_key(KeyEvent::from(KeyCode::Enter));

        assert_eq!(app.learnchain_setup.step, LearnChainSetupStep::Authenticate);
        assert!(
            app.learnchain_setup.error.as_deref().is_some_and(
                |error| error.contains("Enter both your LearnChain email and password")
            )
        );
    }

    #[test]
    fn success_state_returns_to_menu_on_any_key() {
        let mut app = test_app();
        LearnChainSetupManager::show(&mut app);
        app.learnchain_setup.step = LearnChainSetupStep::Success;

        LearnChainSetupManager::new(&mut app).handle_key(KeyEvent::from(KeyCode::Char('q')));

        assert_eq!(app.view, AppView::Menu);
    }
}
