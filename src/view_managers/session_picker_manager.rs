use crate::{
    App, AppView, Project, SessionSelectionTarget, config, log_util::log_debug,
    session_manager::SessionManager, view_managers::DeepDiveManager,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) struct SessionPickerManager<'a> {
    app: &'a mut App,
}

impl<'a> SessionPickerManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn show(app: &mut App, target: SessionSelectionTarget) {
        app.view = AppView::SessionPicker;
        app.session_selection_target = Some(target);
        app.session_picker_viewing_projects = true;

        if app.sessions.is_empty() {
            let config_snapshot = config::current();
            let manager = SessionManager::from_source(config_snapshot.session_source);
            let load = manager.load_all_sessions();
            app.sessions = load.sessions;
            if let Some(err) = load.error {
                App::push_error(&mut app.error, err);
            }
        }

        app.projects = Project::group_sessions(&app.sessions);
        app.session_picker_selected_project = if app.projects.is_empty() {
            None
        } else {
            Some(0)
        };
        app.session_picker_selected_session = None;
        log_debug("App: opened shared session picker");
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.app.session_picker_viewing_projects {
            self.handle_project_key(key);
        } else {
            self.handle_session_key(key);
        }
    }

    fn handle_project_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => self.select_next_project(),
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => {
                self.select_previous_project()
            }
            (KeyModifiers::NONE, KeyCode::Enter) => self.drill_into_project(),
            (KeyModifiers::NONE, KeyCode::Char('h'))
                if self.app.session_selection_target == Some(SessionSelectionTarget::DeepDive) =>
            {
                DeepDiveManager::show_history_from_picker(self.app)
            }
            (KeyModifiers::NONE, KeyCode::Backspace | KeyCode::Char('m')) => {
                self.app.return_to_menu()
            }
            _ => {}
        }
    }

    fn handle_session_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => self.select_next_session(),
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => {
                self.select_previous_session()
            }
            (KeyModifiers::NONE, KeyCode::Enter) => self.activate_selected_session(),
            (KeyModifiers::NONE, KeyCode::Backspace) => self.go_back_to_projects(),
            (KeyModifiers::NONE, KeyCode::Char('h'))
                if self.app.session_selection_target == Some(SessionSelectionTarget::DeepDive) =>
            {
                DeepDiveManager::show_history_from_picker(self.app)
            }
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.app.return_to_menu(),
            _ => {}
        }
    }

    fn select_next_project(&mut self) {
        if self.app.projects.is_empty() {
            self.app.session_picker_selected_project = None;
            return;
        }

        let next = match self.app.session_picker_selected_project {
            Some(index) if index + 1 < self.app.projects.len() => index + 1,
            _ => 0,
        };
        self.app.session_picker_selected_project = Some(next);
    }

    fn select_previous_project(&mut self) {
        if self.app.projects.is_empty() {
            self.app.session_picker_selected_project = None;
            return;
        }

        let previous = match self.app.session_picker_selected_project {
            Some(index) if index > 0 => index - 1,
            _ => self.app.projects.len() - 1,
        };
        self.app.session_picker_selected_project = Some(previous);
    }

    fn drill_into_project(&mut self) {
        let Some(project_idx) = self.app.session_picker_selected_project else {
            return;
        };
        let Some(project) = self.app.projects.get(project_idx) else {
            return;
        };

        if project.session_indices.is_empty() {
            App::push_error(
                &mut self.app.error,
                "No sessions in this project.".to_string(),
            );
            return;
        }

        self.app.session_picker_selected_session = Some(0);
        self.app.session_picker_viewing_projects = false;
        log_debug(&format!(
            "App: drilled into project '{}' with {} sessions",
            project.name,
            project.session_indices.len()
        ));
    }

    fn go_back_to_projects(&mut self) {
        self.app.session_picker_viewing_projects = true;
        self.app.session_picker_selected_session = None;
        log_debug("App: returned to session picker project list");
    }

    fn current_project_sessions(&self) -> Vec<&crate::session_sources::Session> {
        let Some(project_idx) = self.app.session_picker_selected_project else {
            return Vec::new();
        };
        let Some(project) = self.app.projects.get(project_idx) else {
            return Vec::new();
        };

        project
            .session_indices
            .iter()
            .filter_map(|&idx| self.app.sessions.get(idx))
            .collect()
    }

    fn select_next_session(&mut self) {
        let sessions = self.current_project_sessions();
        if sessions.is_empty() {
            self.app.session_picker_selected_session = None;
            return;
        }

        let next = match self.app.session_picker_selected_session {
            Some(index) if index + 1 < sessions.len() => index + 1,
            _ => 0,
        };
        self.app.session_picker_selected_session = Some(next);
    }

    fn select_previous_session(&mut self) {
        let sessions = self.current_project_sessions();
        if sessions.is_empty() {
            self.app.session_picker_selected_session = None;
            return;
        }

        let previous = match self.app.session_picker_selected_session {
            Some(index) if index > 0 => index - 1,
            _ => sessions.len() - 1,
        };
        self.app.session_picker_selected_session = Some(previous);
    }

    fn activate_selected_session(&mut self) {
        let Some(project_idx) = self.app.session_picker_selected_project else {
            App::push_error(&mut self.app.error, "No project selected.".to_string());
            return;
        };
        let Some(project) = self.app.projects.get(project_idx) else {
            App::push_error(&mut self.app.error, "Project not found.".to_string());
            return;
        };
        let Some(session_idx_in_project) = self.app.session_picker_selected_session else {
            App::push_error(&mut self.app.error, "No session selected.".to_string());
            return;
        };
        let Some(&global_session_idx) = project.session_indices.get(session_idx_in_project) else {
            App::push_error(
                &mut self.app.error,
                "Selected session not found in project.".to_string(),
            );
            return;
        };
        let Some(session) = self.app.sessions.get(global_session_idx).cloned() else {
            App::push_error(
                &mut self.app.error,
                "Selected session not found.".to_string(),
            );
            return;
        };

        if session.events.is_empty() {
            App::push_error(
                &mut self.app.error,
                "Selected session has no events. Choose a different session.".to_string(),
            );
            return;
        }

        match self.app.session_selection_target {
            Some(SessionSelectionTarget::Quiz) => {
                super::learning_manager::LearningManager::generate_from_session(self.app, &session)
            }
            Some(SessionSelectionTarget::DeepDive) => {
                DeepDiveManager::start_generation_from_session(self.app, &session)
            }
            None => App::push_error(
                &mut self.app.error,
                "No session selection target is active.".to_string(),
            ),
        }
    }
}
