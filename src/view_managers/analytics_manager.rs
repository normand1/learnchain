use crate::{App, AppView, knowledge_store, log_util::log_debug};
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(crate) struct AnalyticsManager<'a> {
    app: &'a mut App,
}

impl<'a> AnalyticsManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn show_analytics(app: &'a mut App) {
        let mut manager = Self::new(app);
        manager.refresh_snapshot();
        manager.app.view = AppView::Analytics;
        log_debug("App: opened analytics dashboard");
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Char('r')) | (KeyModifiers::NONE, KeyCode::Char('R')) => {
                self.refresh_snapshot();
            }
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.app.return_to_menu(),
            _ => {}
        }
    }

    pub(crate) fn refresh_snapshot(&mut self) {
        match knowledge_store::load_analytics_snapshot() {
            Ok(snapshot) => {
                self.app.analytics_snapshot = Some(snapshot);
                self.app.analytics_error = None;
                self.app.analytics_refreshed_at =
                    Some(Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string());
                log_debug("App: analytics snapshot refreshed");
            }
            Err(err) => {
                self.app.analytics_snapshot = None;
                self.app.analytics_refreshed_at = None;
                self.app.analytics_error = Some(err.to_string());
                log_debug(&format!("App: failed to load analytics snapshot: {}", err));
            }
        }
    }
}
