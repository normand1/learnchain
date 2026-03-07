use crate::{
    App, AppView, SessionSelectionTarget, llm, log_util::log_debug, output_manager::OutputManager,
    session_sources::Session,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) struct DeepDiveManager<'a> {
    app: &'a mut App,
}

impl<'a> DeepDiveManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn start_generation_from_session(app: &mut App, session: &Session) {
        app.session_selection_target = Some(SessionSelectionTarget::DeepDive);
        app.deep_dive_history_document = None;
        app.deep_dive_showing_history = false;
        app.deep_dive_scroll = 0;
        llm::trigger_deep_dive_response_from_session(app, session.clone());
    }

    pub(crate) fn show_history_from_picker(app: &mut App) {
        app.session_selection_target = Some(SessionSelectionTarget::DeepDive);
        let manager = OutputManager::new();
        match manager.list_deep_dive_artifacts() {
            Ok(history) => {
                app.deep_dive_history = history;
                app.deep_dive_history_selected = if app.deep_dive_history.is_empty() {
                    None
                } else {
                    Some(0)
                };
            }
            Err(err) => {
                app.deep_dive_history.clear();
                app.deep_dive_history_selected = None;
                App::push_error(
                    &mut app.error,
                    format!("Failed to load deep-dive history: {}", err),
                );
            }
        }
        app.deep_dive_showing_history = true;
        app.view = AppView::DeepDive;
        log_debug("App: opened deep dive history from session picker");
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.app.deep_dive_showing_history {
            self.handle_history_key(key);
            return;
        }

        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => self.scroll_down(1),
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => self.scroll_up(1),
            (KeyModifiers::NONE, KeyCode::PageDown) => self.scroll_down(12),
            (KeyModifiers::NONE, KeyCode::PageUp) => self.scroll_up(12),
            (KeyModifiers::NONE, KeyCode::Char('h')) => self.open_history(),
            (KeyModifiers::NONE, KeyCode::Backspace) => self.close_history_document(),
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.app.return_to_menu(),
            _ => {}
        }
    }

    fn handle_history_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => self.select_next_history(),
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => {
                self.select_previous_history()
            }
            (KeyModifiers::NONE, KeyCode::Char('r')) | (KeyModifiers::NONE, KeyCode::Char('R')) => {
                self.refresh_history()
            }
            (KeyModifiers::NONE, KeyCode::Enter) => self.open_selected_history_entry(),
            (KeyModifiers::NONE, KeyCode::Backspace) => self.close_history(),
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.app.return_to_menu(),
            _ => {}
        }
    }

    fn scroll_down(&mut self, amount: u16) {
        let max_scroll = self.max_scroll();
        self.app.deep_dive_scroll = self
            .app
            .deep_dive_scroll
            .saturating_add(amount)
            .min(max_scroll);
    }

    fn scroll_up(&mut self, amount: u16) {
        self.app.deep_dive_scroll = self.app.deep_dive_scroll.saturating_sub(amount);
    }

    fn max_scroll(&self) -> u16 {
        self.app
            .active_deep_dive_document()
            .map(|document| document.markdown.lines().count().saturating_sub(1) as u16)
            .unwrap_or(0)
    }

    fn open_history(&mut self) {
        self.refresh_history();
        self.app.deep_dive_showing_history = true;
    }

    fn close_history(&mut self) {
        self.app.deep_dive_showing_history = false;
        if self.app.active_deep_dive_document().is_none()
            && self.app.session_selection_target == Some(SessionSelectionTarget::DeepDive)
        {
            self.app.view = AppView::SessionPicker;
        }
    }

    fn close_history_document(&mut self) {
        if self.app.deep_dive_history_document.is_some() {
            self.app.deep_dive_history_document = None;
            self.app.deep_dive_scroll = 0;
        } else if self.app.deep_dive_document.is_none()
            && self.app.session_selection_target == Some(SessionSelectionTarget::DeepDive)
        {
            self.app.view = AppView::SessionPicker;
        }
    }

    fn refresh_history(&mut self) {
        let manager = OutputManager::new();
        match manager.list_deep_dive_artifacts() {
            Ok(history) => {
                self.app.deep_dive_history = history;
                self.app.deep_dive_history_selected = if self.app.deep_dive_history.is_empty() {
                    None
                } else {
                    Some(0)
                };
            }
            Err(err) => {
                self.app.deep_dive_history.clear();
                self.app.deep_dive_history_selected = None;
                App::push_error(
                    &mut self.app.error,
                    format!("Failed to load deep-dive history: {}", err),
                );
            }
        }
    }

    fn select_next_history(&mut self) {
        if self.app.deep_dive_history.is_empty() {
            self.app.deep_dive_history_selected = None;
            return;
        }

        let next = match self.app.deep_dive_history_selected {
            Some(index) if index + 1 < self.app.deep_dive_history.len() => index + 1,
            _ => 0,
        };
        self.app.deep_dive_history_selected = Some(next);
    }

    fn select_previous_history(&mut self) {
        if self.app.deep_dive_history.is_empty() {
            self.app.deep_dive_history_selected = None;
            return;
        }

        let previous = match self.app.deep_dive_history_selected {
            Some(index) if index > 0 => index - 1,
            _ => self.app.deep_dive_history.len() - 1,
        };
        self.app.deep_dive_history_selected = Some(previous);
    }

    fn open_selected_history_entry(&mut self) {
        let Some(index) = self.app.deep_dive_history_selected else {
            return;
        };
        let Some(entry) = self.app.deep_dive_history.get(index) else {
            return;
        };

        let manager = OutputManager::new();
        match manager.read_deep_dive_markdown(&entry.path) {
            Ok(document) => {
                if self.app.deep_dive_document.is_some() {
                    self.app.show_history_deep_dive_document(document);
                } else {
                    self.app.show_deep_dive_document(document);
                }
            }
            Err(err) => App::push_error(
                &mut self.app.error,
                format!("Failed to read deep dive: {}", err),
            ),
        }
    }
}
