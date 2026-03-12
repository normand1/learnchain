use crate::{
    App, AppView, document_repository,
    llm::StructuredLearningResponse,
    log_util::log_debug,
    output_manager::{LibraryArtifactEntry, OutputManager},
    reset_learning_feedback,
    view_managers::LearningManager,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) struct LibraryManager<'a> {
    app: &'a mut App,
}

impl<'a> LibraryManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn show_library(app: &'a mut App) {
        let mut manager = Self { app };
        manager.refresh();
        manager.app.view = AppView::Library;
        manager.app.ai_status = Some("Browsing saved deep dives and quizzes.".to_string());
        log_debug("App: opened library view");
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => self.select_next(),
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => self.select_previous(),
            (KeyModifiers::NONE, KeyCode::Enter) => self.open_selected(),
            (KeyModifiers::NONE, KeyCode::Char('e')) | (KeyModifiers::NONE, KeyCode::Char('E')) => {
                self.export_selected()
            }
            (KeyModifiers::NONE, KeyCode::Char('r')) | (KeyModifiers::NONE, KeyCode::Char('R')) => {
                self.refresh()
            }
            (KeyModifiers::NONE, KeyCode::Backspace | KeyCode::Char('m')) => {
                self.app.return_to_menu()
            }
            _ => {}
        }
    }

    fn refresh(&mut self) {
        let manager = OutputManager::new();
        match manager.list_library_artifacts() {
            Ok(entries) => {
                self.app.library_artifacts = entries;
                self.app.library_selected = if self.app.library_artifacts.is_empty() {
                    None
                } else {
                    Some(0)
                };
                self.app.error = None;
            }
            Err(err) => {
                self.app.library_artifacts.clear();
                self.app.library_selected = None;
                App::push_error(
                    &mut self.app.error,
                    format!("Failed to load library: {}", err),
                );
            }
        }
    }

    fn select_next(&mut self) {
        if self.app.library_artifacts.is_empty() {
            self.app.library_selected = None;
            return;
        }

        let next = match self.app.library_selected {
            Some(index) if index + 1 < self.app.library_artifacts.len() => index + 1,
            _ => 0,
        };
        self.app.library_selected = Some(next);
    }

    fn select_previous(&mut self) {
        if self.app.library_artifacts.is_empty() {
            self.app.library_selected = None;
            return;
        }

        let previous = match self.app.library_selected {
            Some(index) if index > 0 => index - 1,
            _ => self.app.library_artifacts.len() - 1,
        };
        self.app.library_selected = Some(previous);
    }

    fn open_selected(&mut self) {
        let Some(index) = self.app.library_selected else {
            return;
        };
        let Some(entry) = self.app.library_artifacts.get(index).cloned() else {
            return;
        };

        let manager = OutputManager::new();
        match entry {
            LibraryArtifactEntry::DeepDive(entry) => {
                match manager.read_deep_dive_markdown(&entry.path) {
                    Ok(document) => {
                        self.app.ai_status =
                            Some(format!("Loaded deep dive from {}", document.path.display()));
                        self.app.show_deep_dive_document(document);
                    }
                    Err(err) => App::push_error(
                        &mut self.app.error,
                        format!("Failed to read deep dive: {}", err),
                    ),
                }
            }
            LibraryArtifactEntry::Quiz(entry) => {
                match manager.read_learning_response(&entry.path) {
                    Ok(response) => {
                        self.load_learning_response(response, &entry.path, &entry.session_date)
                    }
                    Err(err) => App::push_error(
                        &mut self.app.error,
                        format!("Failed to read quiz artifact: {}", err),
                    ),
                }
            }
        }
    }

    fn export_selected(&mut self) {
        let Some(index) = self.app.library_selected else {
            return;
        };
        let Some(entry) = self.app.library_artifacts.get(index).cloned() else {
            return;
        };
        document_repository::trigger_library_export(self.app, entry);
    }

    fn load_learning_response(
        &mut self,
        response: StructuredLearningResponse,
        path: &std::path::Path,
        session_date: &str,
    ) {
        self.app.learning_response = Some(response);
        self.app.active_quiz_session_date = session_date.to_string();
        self.app.learning_group_index = 0;
        self.app.learning_quiz_index = 0;
        self.app.learning_option_index = 0;
        self.app.quiz_first_attempts.clear();
        self.app.quiz_first_attempt_results.clear();
        self.app.learning_showing_summary = false;
        self.app.quiz_summary_results.clear();
        reset_learning_feedback(
            &mut self.app.learning_feedback,
            &mut self.app.learning_summary_revealed,
            &mut self.app.learning_waiting_for_next,
        );
        self.app.ai_status = Some(format!("Loaded quiz artifact from {}", path.display()));
        self.app.view = AppView::Learning;
        LearningManager::ensure_indices_for(self.app);
    }
}
