use super::events_manager::EventsManager;
use crate::{
    App, AppView,
    ai_manager::{self, StructuredLearningResponse},
    log_util::log_debug,
    reset_learning_feedback,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rand::{seq::SliceRandom, rng};

pub(crate) struct LearningManager<'a> {
    app: &'a mut App,
}

impl<'a> LearningManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn ensure_indices_for(app: &'a mut App) {
        Self::new(app).ensure_indices();
    }

    pub(crate) fn show_learning(app: &'a mut App) {
        if app.learning_response.is_some() {
            app.view = AppView::Learning;
            Self::ensure_indices_for(app);
            log_debug("App: opened learning view");
        } else {
            App::push_error(
                &mut app.error,
                "No learning response available. Generate one from the menu.".to_string(),
            );
        }
    }

    pub(crate) fn shuffle_quiz_options(response: &mut StructuredLearningResponse) {
        let mut rng = rng();
        for group in &mut response.response {
            for quiz in &mut group.quiz {
                quiz.options.shuffle(&mut rng);
            }
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        if self.app.learning_waiting_for_next {
            self.app.learning_waiting_for_next = false;
            self.next_question();
            return;
        }

        match (key.modifiers, key.code) {
            (KeyModifiers::NONE, KeyCode::Left | KeyCode::Char('h')) => self.previous_group(),
            (KeyModifiers::NONE, KeyCode::Right | KeyCode::Char('l')) => self.next_group(),
            (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) => self.next_option(),
            (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) => self.previous_option(),
            (KeyModifiers::NONE, KeyCode::Char('n'))
            | (KeyModifiers::NONE, KeyCode::Char('N'))
            | (KeyModifiers::NONE, KeyCode::Char(']'))
            | (KeyModifiers::NONE, KeyCode::Char('}'))
            | (KeyModifiers::NONE, KeyCode::PageDown)
            | (KeyModifiers::NONE, KeyCode::Tab) => self.next_question(),
            (KeyModifiers::NONE, KeyCode::Char('p'))
            | (KeyModifiers::NONE, KeyCode::Char('P'))
            | (KeyModifiers::NONE, KeyCode::Char('['))
            | (KeyModifiers::NONE, KeyCode::Char('{'))
            | (KeyModifiers::NONE, KeyCode::PageUp)
            | (KeyModifiers::NONE, KeyCode::BackTab) => self.previous_question(),
            (KeyModifiers::NONE, KeyCode::Enter)
            | (KeyModifiers::NONE, KeyCode::Char(' '))
            | (KeyModifiers::NONE, KeyCode::Char('s')) => self.select_option(),
            (KeyModifiers::NONE, KeyCode::Char('r')) | (KeyModifiers::NONE, KeyCode::Char('R')) => {
                ai_manager::trigger_learning_response(self.app)
            }
            (KeyModifiers::NONE, KeyCode::Char('m')) => self.app.return_to_menu(),
            (KeyModifiers::NONE, KeyCode::Char('e')) => EventsManager::show_events(self.app),
            _ => {}
        }
    }

    pub(crate) fn ensure_indices(&mut self) {
        let response_empty = self
            .app
            .learning_response
            .as_ref()
            .map(|resp| resp.response.is_empty())
            .unwrap_or(true);
        if response_empty {
            self.app.learning_group_index = 0;
            self.app.learning_quiz_index = 0;
            self.app.learning_option_index = 0;
            Self::reset_feedback_state(self.app);
            return;
        }

        let group_len = self
            .app
            .learning_response
            .as_ref()
            .map(|resp| resp.response.len())
            .unwrap_or(0);

        if group_len == 0 {
            self.app.learning_group_index = 0;
            self.app.learning_quiz_index = 0;
            self.app.learning_option_index = 0;
            Self::reset_feedback_state(self.app);
            return;
        }

        if self.app.learning_group_index >= group_len {
            self.app.learning_group_index = 0;
            Self::reset_feedback_state(self.app);
        }

        let quiz_len = self
            .app
            .learning_response
            .as_ref()
            .and_then(|resp| resp.response.get(self.app.learning_group_index))
            .map(|group| group.quiz.len())
            .unwrap_or(0);

        if quiz_len == 0 {
            self.app.learning_quiz_index = 0;
            self.app.learning_option_index = 0;
            Self::reset_feedback_state(self.app);
            return;
        }

        if self.app.learning_quiz_index >= quiz_len {
            self.app.learning_quiz_index = 0;
            Self::reset_feedback_state(self.app);
        }

        let option_len = self
            .app
            .learning_response
            .as_ref()
            .and_then(|resp| resp.response.get(self.app.learning_group_index))
            .and_then(|group| group.quiz.get(self.app.learning_quiz_index))
            .map(|question| question.options.len())
            .unwrap_or(0);

        if option_len == 0 {
            self.app.learning_option_index = 0;
            Self::reset_feedback_state(self.app);
        } else if self.app.learning_option_index >= option_len {
            self.app.learning_option_index = 0;
            Self::reset_feedback_state(self.app);
        }
    }

    pub(crate) fn next_group(&mut self) {
        let Some(total_groups) = self.total_groups() else {
            return;
        };
        self.app.learning_group_index = (self.app.learning_group_index + 1) % total_groups;
        self.reset_question_state();
        log_debug(&format!(
            "App: moved to learning group {} of {}",
            self.app.learning_group_index + 1,
            total_groups
        ));
        self.ensure_indices();
    }

    pub(crate) fn previous_group(&mut self) {
        let Some(total_groups) = self.total_groups() else {
            return;
        };
        if self.app.learning_group_index == 0 {
            self.app.learning_group_index = total_groups - 1;
        } else {
            self.app.learning_group_index -= 1;
        }
        self.reset_question_state();
        log_debug(&format!(
            "App: moved to learning group {} of {}",
            self.app.learning_group_index + 1,
            total_groups
        ));
        self.ensure_indices();
    }

    pub(crate) fn next_question(&mut self) {
        if let Some(quiz_len) = self.active_group_quiz_len() {
            if self.app.learning_quiz_index + 1 < quiz_len {
                self.app.learning_quiz_index += 1;
                self.app.learning_option_index = 0;
                self.reset_feedback();
                log_debug(&format!(
                    "App: moved to question {} of {} in group {}",
                    self.app.learning_quiz_index + 1,
                    quiz_len,
                    self.app.learning_group_index + 1
                ));
                self.ensure_indices();
                return;
            }
        }

        if self.move_to_next_group_with_quiz() {
            return;
        }

        self.reset_feedback();
        self.ensure_indices();
    }

    pub(crate) fn previous_question(&mut self) {
        if let Some(quiz_len) = self.active_group_quiz_len() {
            if self.app.learning_quiz_index > 0 {
                self.app.learning_quiz_index -= 1;
                self.app.learning_option_index = 0;
                self.reset_feedback();
                log_debug(&format!(
                    "App: moved to question {} of {} in group {}",
                    self.app.learning_quiz_index + 1,
                    quiz_len,
                    self.app.learning_group_index + 1
                ));
                self.ensure_indices();
                return;
            }
        }

        if self.move_to_previous_group_with_quiz() {
            return;
        }

        self.reset_feedback();
        self.ensure_indices();
    }

    pub(crate) fn next_option(&mut self) {
        let Some(option_len) = self.active_option_count() else {
            return;
        };
        self.app.learning_option_index = (self.app.learning_option_index + 1) % option_len;
        self.reset_feedback();
        log_debug(&format!(
            "App: moved to option {} of {} in question {}",
            self.app.learning_option_index + 1,
            option_len,
            self.app.learning_quiz_index + 1
        ));
        self.ensure_indices();
    }

    pub(crate) fn previous_option(&mut self) {
        let Some(option_len) = self.active_option_count() else {
            return;
        };
        if self.app.learning_option_index == 0 {
            self.app.learning_option_index = option_len - 1;
        } else {
            self.app.learning_option_index -= 1;
        }
        self.reset_feedback();
        log_debug(&format!(
            "App: moved to option {} of {} in question {}",
            self.app.learning_option_index + 1,
            option_len,
            self.app.learning_quiz_index + 1
        ));
        self.ensure_indices();
    }

    pub(crate) fn select_option(&mut self) {
        let Some(response) = self.app.learning_response.as_ref() else {
            return;
        };
        let Some(group) = response.response.get(self.app.learning_group_index) else {
            return;
        };
        let Some(question) = group.quiz.get(self.app.learning_quiz_index) else {
            return;
        };
        if question.options.is_empty() {
            self.app.learning_feedback =
                Some("No answer options available for this question.".to_string());
            self.app.learning_summary_revealed = false;
            self.app.learning_waiting_for_next = false;
            log_debug("App: selection ignored because no options exist");
            return;
        }

        let option_len = question.options.len();
        let selected_index = self.app.learning_option_index.min(option_len - 1);
        let label = ((b'A' + (selected_index % 26) as u8) as char).to_string();
        let correct = question.options[selected_index].is_correct_answer;

        if correct {
            self.app.learning_feedback =
                Some(format!("Correct! Option {} is the right answer.", label));
            self.app.learning_summary_revealed = true;
            self.app.learning_waiting_for_next = true;
        } else {
            self.app.learning_feedback = Some("Not quite. Try another option.".to_string());
            self.app.learning_summary_revealed = false;
            self.app.learning_waiting_for_next = false;
        }

        log_debug(&format!(
            "App: evaluated option {} (correct: {})",
            label, correct
        ));
    }

    fn total_groups(&self) -> Option<usize> {
        let response = self.app.learning_response.as_ref()?;
        let total = response.response.len();
        if total == 0 { None } else { Some(total) }
    }

    fn group_quiz_len(&self, group_index: usize) -> Option<usize> {
        let response = self.app.learning_response.as_ref()?;
        let group = response.response.get(group_index)?;
        let quiz_len = group.quiz.len();
        if quiz_len == 0 { None } else { Some(quiz_len) }
    }

    fn active_group_quiz_len(&self) -> Option<usize> {
        self.group_quiz_len(self.app.learning_group_index)
    }

    fn active_option_count(&self) -> Option<usize> {
        let response = self.app.learning_response.as_ref()?;
        let group = response.response.get(self.app.learning_group_index)?;
        let question = group.quiz.get(self.app.learning_quiz_index)?;
        let option_len = question.options.len();
        if option_len == 0 {
            None
        } else {
            Some(option_len)
        }
    }

    fn move_to_next_group_with_quiz(&mut self) -> bool {
        let Some(total_groups) = self.total_groups() else {
            return false;
        };

        for offset in 1..=total_groups {
            let next_index = (self.app.learning_group_index + offset) % total_groups;
            if let Some(next_quiz_len) = self.group_quiz_len(next_index) {
                self.app.learning_group_index = next_index;
                self.app.learning_quiz_index = 0;
                self.app.learning_option_index = 0;
                self.reset_feedback();
                log_debug(&format!(
                    "App: auto-advanced to learning group {} of {} with {} question(s)",
                    next_index + 1,
                    total_groups,
                    next_quiz_len
                ));
                self.ensure_indices();
                return true;
            }
        }

        false
    }

    fn move_to_previous_group_with_quiz(&mut self) -> bool {
        let Some(total_groups) = self.total_groups() else {
            return false;
        };

        for offset in 1..=total_groups {
            let prev_index = (self.app.learning_group_index + total_groups - offset) % total_groups;
            if let Some(prev_quiz_len) = self.group_quiz_len(prev_index) {
                self.app.learning_group_index = prev_index;
                self.app.learning_quiz_index = prev_quiz_len - 1;
                self.app.learning_option_index = 0;
                self.reset_feedback();
                log_debug(&format!(
                    "App: auto-rewound to learning group {} of {} with {} question(s)",
                    prev_index + 1,
                    total_groups,
                    prev_quiz_len
                ));
                self.ensure_indices();
                return true;
            }
        }

        false
    }

    fn reset_question_state(&mut self) {
        self.app.learning_quiz_index = 0;
        self.app.learning_option_index = 0;
        self.reset_feedback();
    }

    fn reset_feedback(&mut self) {
        Self::reset_feedback_state(self.app);
    }

    pub(crate) fn reset_feedback_state(app: &mut App) {
        reset_learning_feedback(
            &mut app.learning_feedback,
            &mut app.learning_summary_revealed,
            &mut app.learning_waiting_for_next,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ConfigForm, OpenAiModelKind};
    use serde_json::from_str;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn load_learning_response(filename: &str) -> StructuredLearningResponse {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(filename);
        let contents = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {}", path.display(), err));
        from_str(&contents).unwrap_or_else(|err| {
            panic!(
                "failed to parse {} as StructuredLearningResponse: {}",
                path.display(),
                err
            )
        })
    }

    fn app_with_response(response: StructuredLearningResponse) -> App {
        App {
            running: false,
            view: AppView::Menu,
            menu_index: 0,
            events: Vec::new(),
            selected_event: None,
            session_dir: PathBuf::new(),
            session_date: String::new(),
            session_source: String::new(),
            latest_file: None,
            summary_file: None,
            summary_content: None,
            error: None,
            ai_manager: None,
            ai_status: None,
            ai_loading: false,
            ai_loading_frame: 0,
            ai_result_receiver: None,
            learning_response: Some(response),
            learning_group_index: 0,
            learning_quiz_index: 0,
            learning_option_index: 0,
            learning_feedback: None,
            learning_summary_revealed: false,
            learning_waiting_for_next: false,
            config_form: ConfigForm::from_config(AppConfig::default()),
            write_output_artifacts: false,
            openai_model: OpenAiModelKind::Gpt5Mini,
        }
    }

    #[test]
    fn multiple_group_quiz_advances_and_wraps_groups() {
        let response = load_learning_response("test_fixtures/multiple_knowledge_type_groups.json");
        let mut app = app_with_response(response);

        LearningManager::show_learning(&mut app);

        assert_eq!(app.view, AppView::Learning);
        assert_eq!(app.learning_group_index, 0);
        assert_eq!(app.learning_quiz_index, 0);
        assert_eq!(app.learning_option_index, 0);
        assert!(!app.learning_summary_revealed);

        {
            let mut manager = LearningManager::new(&mut app);
            manager.next_question();
        }

        assert_eq!(
            app.learning_group_index, 1,
            "expected to advance to next knowledge group"
        );
        assert_eq!(
            app.learning_quiz_index, 0,
            "first quiz question should be active after advancing groups"
        );
        assert_eq!(app.learning_option_index, 0);

        let total_groups = app
            .learning_response
            .as_ref()
            .map(|resp| resp.response.len())
            .unwrap_or_default();
        assert!(
            total_groups > 1,
            "fixture should include multiple knowledge groups"
        );

        app.learning_group_index = total_groups - 1;
        app.learning_quiz_index = 0;

        {
            let mut manager = LearningManager::new(&mut app);
            manager.next_question();
        }

        assert_eq!(
            app.learning_group_index, 0,
            "navigation should wrap back to the first group"
        );
        assert_eq!(app.learning_quiz_index, 0);
        assert_eq!(app.learning_option_index, 0);
        assert!(!app.learning_summary_revealed);
        assert!(!app.learning_waiting_for_next);
    }

    #[test]
    fn single_group_quiz_cycles_questions_without_group_change() {
        let response = load_learning_response("test_fixtures/single_knowledge_type_group.json");
        let mut app = app_with_response(response);

        LearningManager::show_learning(&mut app);

        let total_questions = app
            .learning_response
            .as_ref()
            .and_then(|resp| resp.response.first())
            .map(|group| group.quiz.len())
            .unwrap_or_default();
        assert!(
            total_questions > 1,
            "fixture should provide multiple quiz questions"
        );

        app.learning_group_index = 0;
        app.learning_quiz_index = total_questions - 1;
        app.learning_option_index = 2;
        app.learning_summary_revealed = true;
        app.learning_waiting_for_next = true;

        {
            let mut manager = LearningManager::new(&mut app);
            manager.next_question();
        }

        assert_eq!(
            app.learning_group_index, 0,
            "single group quiz should remain on the same group"
        );
        assert_eq!(
            app.learning_quiz_index, 0,
            "question index should cycle back to the beginning"
        );
        assert_eq!(
            app.learning_option_index, 0,
            "option index should reset when cycling questions"
        );
        assert!(
            !app.learning_summary_revealed,
            "cycling should clear summary state"
        );
        assert!(
            !app.learning_waiting_for_next,
            "cycling should clear waiting state"
        );
    }
}
