use crate::config;
use crate::session_manager::SessionManager;
use crate::{App, AppView};

pub(crate) struct EventsManager<'a> {
    app: &'a mut App,
}

impl<'a> EventsManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn show_events(app: &mut App) {
        app.view = AppView::Events;
        app.viewing_sessions_list = true;

        // Load all sessions if not already loaded
        if app.sessions.is_empty() {
            let config_snapshot = config::current();
            let manager = SessionManager::from_source(config_snapshot.session_source);
            let load = manager.load_all_sessions();
            app.sessions = load.sessions;
            if let Some(err) = load.error {
                App::push_error(&mut app.error, err);
            }
        }

        if app.sessions.is_empty() {
            app.selected_session = None;
        } else if app.selected_session.is_none() {
            app.selected_session = Some(0);
        }
    }

    pub(crate) fn select_next(&mut self) {
        if self.app.viewing_sessions_list {
            self.select_next_session();
        } else {
            self.select_next_event();
        }
    }

    pub(crate) fn select_previous(&mut self) {
        if self.app.viewing_sessions_list {
            self.select_previous_session();
        } else {
            self.select_previous_event();
        }
    }

    fn select_next_session(&mut self) {
        if self.app.sessions.is_empty() {
            self.app.selected_session = None;
            return;
        }
        let next = match self.app.selected_session {
            Some(index) if index + 1 < self.app.sessions.len() => index + 1,
            _ => 0,
        };
        self.app.selected_session = Some(next);
    }

    fn select_previous_session(&mut self) {
        if self.app.sessions.is_empty() {
            self.app.selected_session = None;
            return;
        }
        let previous = match self.app.selected_session {
            Some(index) if index > 0 => index - 1,
            _ => self.app.sessions.len() - 1,
        };
        self.app.selected_session = Some(previous);
    }

    fn select_next_event(&mut self) {
        if self.app.events.is_empty() {
            self.app.selected_event = None;
            return;
        }
        let next = match self.app.selected_event {
            Some(index) if index + 1 < self.app.events.len() => index + 1,
            _ => 0,
        };
        self.app.selected_event = Some(next);
    }

    fn select_previous_event(&mut self) {
        if self.app.events.is_empty() {
            self.app.selected_event = None;
            return;
        }
        let previous = match self.app.selected_event {
            Some(index) if index > 0 => index - 1,
            _ => self.app.events.len() - 1,
        };
        self.app.selected_event = Some(previous);
    }

    pub(crate) fn drill_down(&mut self) {
        if !self.app.viewing_sessions_list {
            return;
        }

        if let Some(idx) = self.app.selected_session
            && let Some(session) = self.app.sessions.get(idx)
        {
            self.app.events = session.events.clone();
            self.app.selected_event = if session.events.is_empty() {
                None
            } else {
                Some(0)
            };
            self.app.viewing_sessions_list = false;
        }
    }

    pub(crate) fn go_back(&mut self) -> bool {
        if !self.app.viewing_sessions_list {
            self.app.viewing_sessions_list = true;
            true
        } else {
            false
        }
    }
}
