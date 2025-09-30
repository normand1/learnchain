use super::{
    config_manager::ConfigManager, events_manager::EventsManager, learning_manager::LearningManager,
};
use crate::{App, ai_manager};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) const MENU_OPTIONS: [&str; 3] = [
    "1. Generate Learning lesson",
    "2. View session events",
    "3. Configure details",
];

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
            (KeyModifiers::NONE, KeyCode::Char('1')) => {
                self.app.menu_index = 0;
                self.activate_menu_option();
            }
            (KeyModifiers::NONE, KeyCode::Char('2')) => {
                self.app.menu_index = 1;
                self.activate_menu_option();
            }
            (KeyModifiers::NONE, KeyCode::Char('3')) => {
                self.app.menu_index = 2;
                self.activate_menu_option();
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
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.app.return_to_menu(),
            (KeyModifiers::NONE, KeyCode::Char('l')) => LearningManager::show_learning(self.app),
            _ => {}
        }
    }

    fn menu_next(&mut self) {
        self.app.menu_index = (self.app.menu_index + 1) % MENU_OPTIONS.len();
    }

    fn menu_previous(&mut self) {
        if self.app.menu_index == 0 {
            self.app.menu_index = MENU_OPTIONS.len() - 1;
        } else {
            self.app.menu_index -= 1;
        }
    }

    fn activate_menu_option(&mut self) {
        match self.app.menu_index {
            0 => ai_manager::trigger_learning_response(self.app),
            1 => EventsManager::show_events(self.app),
            2 => ConfigManager::new(self.app).show_config(),
            _ => {}
        }
    }
}
