use crate::{App, AppView, EmbeddedSkillTarget, install_embedded_skill_default};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) struct SkillInstallerManager<'a> {
    app: &'a mut App,
}

impl<'a> SkillInstallerManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn show(app: &'a mut App) {
        app.view = AppView::SkillInstaller;
        app.skill_installer_selected_target = EmbeddedSkillTarget::Codex;
        app.ai_status =
            Some("Choose which coding tool should receive the LearnChain skill.".to_string());
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j'))
            | (KeyModifiers::NONE, KeyCode::Right | KeyCode::Char('l')) => self.select_next(),
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k'))
            | (KeyModifiers::NONE, KeyCode::Left | KeyCode::Char('h')) => self.select_previous(),
            (KeyModifiers::NONE, KeyCode::Enter) => self.install_selected(),
            (KeyModifiers::NONE, KeyCode::Backspace | KeyCode::Char('m')) => {
                self.app.return_to_menu()
            }
            _ => {}
        }
    }

    fn select_next(&mut self) {
        self.app.skill_installer_selected_target = match self.app.skill_installer_selected_target {
            EmbeddedSkillTarget::Codex => EmbeddedSkillTarget::ClaudeCode,
            EmbeddedSkillTarget::ClaudeCode => EmbeddedSkillTarget::Codex,
        };
    }

    fn select_previous(&mut self) {
        self.select_next();
    }

    fn install_selected(&mut self) {
        let target = self.app.skill_installer_selected_target;
        match install_embedded_skill_default(target) {
            Ok(path) => {
                self.on_install_success(target, &path);
            }
            Err(err) => {
                self.app.ai_status = Some(format!(
                    "Failed to install {}. Check the error.",
                    target.installation_label()
                ));
                App::push_error(
                    &mut self.app.error,
                    format!("Failed to install {}: {}", target.installation_label(), err),
                );
            }
        }
    }

    fn on_install_success(&mut self, target: EmbeddedSkillTarget, path: &std::path::Path) {
        self.app.error = None;
        self.app.view = AppView::SkillInstaller;
        self.app.ai_status = Some(format!(
            "Installed {} to {}. Press Backspace or m to return to the menu.",
            target.installation_label(),
            path.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AiTaskMessage, App, AppView, DeepDiveDocument, EmbeddedSkillTarget, QuizSummaryResult,
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
            learnchain_setup: crate::LearnChainSetupState::default(),
        }
    }

    #[test]
    fn show_opens_skill_installer_view() {
        let mut app = test_app();

        SkillInstallerManager::show(&mut app);

        assert_eq!(app.view, AppView::SkillInstaller);
        assert_eq!(
            app.skill_installer_selected_target,
            EmbeddedSkillTarget::Codex
        );
    }

    #[test]
    fn installer_navigation_switches_targets() {
        let mut app = test_app();
        SkillInstallerManager::show(&mut app);

        SkillInstallerManager::new(&mut app).handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(
            app.skill_installer_selected_target,
            EmbeddedSkillTarget::ClaudeCode
        );

        SkillInstallerManager::new(&mut app).handle_key(KeyEvent::from(KeyCode::Up));
        assert_eq!(
            app.skill_installer_selected_target,
            EmbeddedSkillTarget::Codex
        );
    }

    #[test]
    fn install_success_keeps_confirmation_visible_in_installer_view() {
        let mut app = test_app();
        SkillInstallerManager::show(&mut app);

        SkillInstallerManager::new(&mut app).on_install_success(
            EmbeddedSkillTarget::ClaudeCode,
            std::path::Path::new("/tmp/.claude/skills/learnchain-deep-dive"),
        );

        assert_eq!(app.view, AppView::SkillInstaller);
        assert!(
            app.ai_status
                .as_deref()
                .is_some_and(|status| status.contains("Installed LearnChain Claude Code skill"))
        );
        assert!(
            app.ai_status
                .as_deref()
                .is_some_and(|status| status.contains("Press Backspace or m to return"))
        );
    }
}
