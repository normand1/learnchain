use crate::{
    App, AppView,
    config::{self, ConfigField, ConfigForm},
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
        self.app.config_form.set_status(
            "Config is grouped by section. Use ↑/↓ to focus a setting, ←/→ to adjust selectors, and Enter to edit text fields.",
        );
        self.app.view = AppView::Config;
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        // Handle text field editing modes
        if self.app.config_form.is_editing_text_field() {
            self.handle_text_edit(key);
            return;
        }

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
            (KeyModifiers::NONE, KeyCode::Enter) => {
                // Start editing if a text field is selected, otherwise save
                match self.app.config_form.current_field() {
                    ConfigField::DocumentRepositoryTarget => {
                        self.app
                            .config_form
                            .start_editing_document_repository_target();
                    }
                    ConfigField::NotionApiToken => {
                        self.app.config_form.start_editing_notion_api_token();
                    }
                    ConfigField::LearnChainSiteUrl => {
                        self.app.config_form.start_editing_learnchain_site_url();
                    }
                    ConfigField::LearnChainEmail => {
                        self.app.config_form.start_editing_learnchain_email();
                    }
                    ConfigField::LearnChainPassword => {
                        self.app.config_form.start_editing_learnchain_password();
                    }
                    ConfigField::OpenAiKey => {
                        self.app.config_form.start_editing_openai_key();
                    }
                    ConfigField::AnthropicKey => {
                        self.app.config_form.start_editing_anthropic_key();
                    }
                    ConfigField::OpenRouterKey => {
                        self.app.config_form.start_editing_openrouter_key();
                    }
                    ConfigField::OpenRouterModel => {
                        self.app.config_form.start_editing_openrouter_model();
                    }
                    _ => {
                        self.save_config_changes();
                    }
                }
            }
            (KeyModifiers::NONE, KeyCode::Char('s')) => {
                self.save_config_changes();
            }
            (KeyModifiers::NONE, KeyCode::Char('r')) => self.reset_config_form(),
            (KeyModifiers::NONE, KeyCode::Char('m')) => {
                let was_dirty = self.app.config_form.dirty;
                self.save_config_changes();
                if !self.app.config_form.dirty || !was_dirty {
                    self.app.return_to_menu();
                }
            }
            _ => {}
        }
    }

    pub(crate) fn handle_paste(&mut self, text: &str) {
        if !self.app.config_form.is_editing_text_field() {
            return;
        }

        let sanitized: String = text
            .chars()
            .filter(|ch| *ch != '\n' && *ch != '\r')
            .collect();
        if sanitized.is_empty() {
            return;
        }

        if self.app.config_form.is_editing_document_repository_target() {
            for ch in sanitized.chars() {
                self.app
                    .config_form
                    .push_document_repository_target_char(ch);
            }
        } else if self.app.config_form.is_editing_notion_api_token() {
            for ch in sanitized.chars() {
                self.app.config_form.push_notion_api_token_char(ch);
            }
        } else if self.app.config_form.is_editing_learnchain_site_url() {
            for ch in sanitized.chars() {
                self.app.config_form.push_learnchain_site_url_char(ch);
            }
        } else if self.app.config_form.is_editing_learnchain_email() {
            for ch in sanitized.chars() {
                self.app.config_form.push_learnchain_email_char(ch);
            }
        } else if self.app.config_form.is_editing_learnchain_password() {
            for ch in sanitized.chars() {
                self.app.config_form.push_learnchain_password_char(ch);
            }
        } else if self.app.config_form.is_editing_openai_key() {
            for ch in sanitized.chars() {
                self.app.config_form.push_openai_key_char(ch);
            }
        } else if self.app.config_form.is_editing_anthropic_key() {
            for ch in sanitized.chars() {
                self.app.config_form.push_anthropic_key_char(ch);
            }
        } else if self.app.config_form.is_editing_openrouter_key() {
            for ch in sanitized.chars() {
                self.app.config_form.push_openrouter_key_char(ch);
            }
        } else if self.app.config_form.is_editing_openrouter_model() {
            for ch in sanitized.chars() {
                self.app.config_form.push_openrouter_model_char(ch);
            }
        }
    }

    fn handle_text_edit(&mut self, key: KeyEvent) {
        if self.app.config_form.is_editing_document_repository_target() {
            match key.code {
                KeyCode::Esc => self
                    .app
                    .config_form
                    .cancel_document_repository_target_edit(),
                KeyCode::Enter => self.app.config_form.apply_document_repository_target_edit(),
                KeyCode::Backspace => self.app.config_form.backspace_document_repository_target(),
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.app
                            .config_form
                            .push_document_repository_target_char(ch);
                    }
                }
                _ => {}
            }
        } else if self.app.config_form.is_editing_notion_api_token() {
            match key.code {
                KeyCode::Esc => self.app.config_form.cancel_notion_api_token_edit(),
                KeyCode::Enter => self.app.config_form.apply_notion_api_token_edit(),
                KeyCode::Backspace => self.app.config_form.backspace_notion_api_token(),
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.app.config_form.push_notion_api_token_char(ch);
                    }
                }
                _ => {}
            }
        } else if self.app.config_form.is_editing_learnchain_site_url() {
            match key.code {
                KeyCode::Esc => self.app.config_form.cancel_learnchain_site_url_edit(),
                KeyCode::Enter => self.app.config_form.apply_learnchain_site_url_edit(),
                KeyCode::Backspace => self.app.config_form.backspace_learnchain_site_url(),
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.app.config_form.push_learnchain_site_url_char(ch);
                    }
                }
                _ => {}
            }
        } else if self.app.config_form.is_editing_learnchain_email() {
            match key.code {
                KeyCode::Esc => self.app.config_form.cancel_learnchain_email_edit(),
                KeyCode::Enter => self.app.config_form.apply_learnchain_email_edit(),
                KeyCode::Backspace => self.app.config_form.backspace_learnchain_email(),
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.app.config_form.push_learnchain_email_char(ch);
                    }
                }
                _ => {}
            }
        } else if self.app.config_form.is_editing_learnchain_password() {
            match key.code {
                KeyCode::Esc => self.app.config_form.cancel_learnchain_password_edit(),
                KeyCode::Enter => self.app.config_form.apply_learnchain_password_edit(),
                KeyCode::Backspace => self.app.config_form.backspace_learnchain_password(),
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.app.config_form.push_learnchain_password_char(ch);
                    }
                }
                _ => {}
            }
        } else if self.app.config_form.is_editing_openai_key() {
            match key.code {
                KeyCode::Esc => self.app.config_form.cancel_openai_key_edit(),
                KeyCode::Enter => self.app.config_form.apply_openai_key_edit(),
                KeyCode::Backspace => self.app.config_form.backspace_openai_key(),
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.app.config_form.push_openai_key_char(ch);
                    }
                }
                _ => {}
            }
        } else if self.app.config_form.is_editing_anthropic_key() {
            match key.code {
                KeyCode::Esc => self.app.config_form.cancel_anthropic_key_edit(),
                KeyCode::Enter => self.app.config_form.apply_anthropic_key_edit(),
                KeyCode::Backspace => self.app.config_form.backspace_anthropic_key(),
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.app.config_form.push_anthropic_key_char(ch);
                    }
                }
                _ => {}
            }
        } else if self.app.config_form.is_editing_openrouter_key() {
            match key.code {
                KeyCode::Esc => self.app.config_form.cancel_openrouter_key_edit(),
                KeyCode::Enter => self.app.config_form.apply_openrouter_key_edit(),
                KeyCode::Backspace => self.app.config_form.backspace_openrouter_key(),
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.app.config_form.push_openrouter_key_char(ch);
                    }
                }
                _ => {}
            }
        } else if self.app.config_form.is_editing_openrouter_model() {
            match key.code {
                KeyCode::Esc => self.app.config_form.cancel_openrouter_model_edit(),
                KeyCode::Enter => self.app.config_form.apply_openrouter_model_edit(),
                KeyCode::Backspace => self.app.config_form.backspace_openrouter_model(),
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.app.config_form.push_openrouter_model_char(ch);
                    }
                }
                _ => {}
            }
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
        let target_sampling = self.app.config_form.sampling_percentage;
        let target_source = self.app.config_form.session_source;
        let target_write = self.app.config_form.write_output_artifacts;
        let target_document_repository_kind = self.app.config_form.document_repository;
        let target_document_repository = self.app.config_form.document_repository_target.clone();
        let target_notion_api_token = self.app.config_form.notion_api_token.clone();
        let target_learnchain_site_url = self.app.config_form.learnchain_site_url.clone();
        let target_learnchain_email = self.app.config_form.learnchain_email.clone();
        let target_learnchain_password = self.app.config_form.learnchain_password.clone();
        let target_provider = self.app.config_form.ai_provider;
        let target_openai_model = self.app.config_form.openai_model;
        let target_openai_key = self.app.config_form.openai_api_key.clone();
        let target_anthropic_model = self.app.config_form.anthropic_model;
        let target_anthropic_key = self.app.config_form.anthropic_api_key.clone();
        let target_openrouter_model = self.app.config_form.openrouter_model.clone();
        let target_openrouter_key = self.app.config_form.openrouter_api_key.clone();

        if let Err(err) = config::validate_document_repository_target(
            target_document_repository_kind,
            &target_document_repository,
        ) {
            App::push_error(
                &mut self.app.error,
                format!("Invalid document repository target: {}", err),
            );
            self.app
                .config_form
                .set_status(format!("Invalid document repository target. {}", err));
            return;
        }

        if self.app.config_form.is_notion_target_selected()
            && target_notion_api_token.trim().is_empty()
        {
            let help = config::notion_token_help_message().to_string();
            App::push_error(&mut self.app.error, help.clone());
            self.app.config_form.set_status(help);
            return;
        }

        if self.app.config_form.is_learnchain_selected()
            && let Err(err) = config::validate_learnchain_site_url(&target_learnchain_site_url)
        {
            App::push_error(&mut self.app.error, err.clone());
            self.app.config_form.set_status(err);
            return;
        }

        match config::update(|config| {
            config.default_max_events = target_max;
            config.min_quiz_questions = target_min;
            config.sampling_percentage = target_sampling;
            config.session_source = target_source;
            config.write_output_artifacts = target_write;
            config.document_repository = target_document_repository_kind;
            config.document_repository_target = target_document_repository.clone();
            config.notion_api_token = target_notion_api_token.clone();
            config.learnchain_site_url = target_learnchain_site_url.clone();
            config.learnchain_email = target_learnchain_email.clone();
            config.learnchain_password = target_learnchain_password.clone();
            config.ai_provider = target_provider;
            config.openai_model = target_openai_model;
            config.openai_api_key = target_openai_key.clone();
            config.anthropic_model = target_anthropic_model;
            config.anthropic_api_key = target_anthropic_key.clone();
            config.openrouter_model = target_openrouter_model.clone();
            config.openrouter_api_key = target_openrouter_key.clone();
        }) {
            Ok(updated) => {
                self.app.config_form.apply_saved(updated);
                self.app.reload_session_from_config();
                let resolved_llm = self.app.config_form.resolved_llm();

                let status = if self.app.config_form.is_learnchain_selected()
                    && (self.app.config_form.learnchain_email.trim().is_empty()
                        || self.app.config_form.learnchain_password.is_empty())
                {
                    config::learnchain_credentials_help_message(
                        &self.app.config_form.learnchain_site_url,
                    )
                } else if resolved_llm.provider == config::AiProvider::CodexCli {
                    "Saved • Provider: Codex CLI • Model: CLI default • Uses installed codex configuration"
                        .to_string()
                } else {
                    format!(
                        "Saved • Provider: {} • Model: {} • Key: {}",
                        resolved_llm.provider.label(),
                        resolved_llm.model_label,
                        if resolved_llm.api_key.trim().is_empty() {
                            "not set"
                        } else {
                            "set"
                        }
                    )
                };
                self.app.config_form.set_status(status);
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
