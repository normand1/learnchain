use crate::{
    App, AppView,
    config::{self, ConfigForm},
    log_util::log_debug,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) struct ConfigManager<'a> {
    app: &'a mut App,
}

impl<'a> ConfigManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn show_config(&mut self) {
        self.app.config_form = ConfigForm::from_config(config::current());
        self.app
            .config_form
            .set_status("Use ←/→ to adjust values, s to save changes.");
        self.app.view = AppView::Config;
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => {
                self.app.config_form.select_next();
            }
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => {
                self.app.config_form.select_previous();
            }
            (KeyModifiers::NONE, KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-')) => {
                self.app.config_form.adjust_current(-1);
            }
            (
                KeyModifiers::NONE,
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('='),
            ) => {
                self.app.config_form.adjust_current(1);
            }
            (KeyModifiers::NONE, KeyCode::Char('s')) | (KeyModifiers::NONE, KeyCode::Enter) => {
                self.save_config_changes();
            }
            (KeyModifiers::NONE, KeyCode::Char('r')) => self.reset_config_form(),
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.app.return_to_menu(),
            _ => {}
        }
    }

    fn save_config_changes(&mut self) {
        if !self.app.config_form.dirty {
            self.app
                .config_form
                .set_status("No pending changes to save.");
            return;
        }

        let target_max = self.app.config_form.max_events;
        let target_min = self.app.config_form.min_quiz_questions;

        match config::update(|config| {
            config.default_max_events = target_max;
            config.min_quiz_questions = target_min;
        }) {
            Ok(updated) => {
                self.app.config_form.apply_saved(updated);
                self.app.config_form.set_status(format!(
                    "Saved configuration to {}",
                    config::config_file_path().display()
                ));
                log_debug("App: configuration saved");
            }
            Err(err) => {
                App::push_error(
                    &mut self.app.error,
                    format!("Failed to save configuration: {}", err),
                );
                self.app
                    .config_form
                    .set_status("Failed to save configuration. Check error panel.");
                log_debug(&format!("App: failed to save configuration: {}", err));
            }
        }
    }

    fn reset_config_form(&mut self) {
        let current = config::current();
        self.app.config_form = ConfigForm::from_config(current);
        self.app
            .config_form
            .set_status("Reverted to saved configuration values.");
    }
}
