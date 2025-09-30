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
        if app.events.is_empty() {
            app.selected_event = None;
        } else if app.selected_event.is_none() {
            app.selected_event = Some(0);
        }
    }

    pub(crate) fn select_next(&mut self) {
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

    pub(crate) fn select_previous(&mut self) {
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
}
