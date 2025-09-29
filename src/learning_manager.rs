use crate::{
    App, ai_manager::StructuredLearningResponse, log_util::log_debug, reset_learning_feedback,
};
use rand::{seq::SliceRandom, thread_rng};

pub(crate) struct LearningManager<'a> {
    app: &'a mut App,
}

impl<'a> LearningManager<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn shuffle_quiz_options(response: &mut StructuredLearningResponse) {
        let mut rng = thread_rng();
        for group in &mut response.response {
            for quiz in &mut group.quiz {
                quiz.options.shuffle(&mut rng);
            }
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
        self.app.ensure_learning_indices();
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
        self.app.ensure_learning_indices();
    }

    pub(crate) fn next_question(&mut self) {
        let Some(quiz_len) = self.active_group_quiz_len() else {
            return;
        };
        self.app.learning_quiz_index = (self.app.learning_quiz_index + 1) % quiz_len;
        self.app.learning_option_index = 0;
        self.reset_feedback();
        log_debug(&format!(
            "App: moved to question {} of {} in group {}",
            self.app.learning_quiz_index + 1,
            quiz_len,
            self.app.learning_group_index + 1
        ));
        self.app.ensure_learning_indices();
    }

    pub(crate) fn previous_question(&mut self) {
        let Some(quiz_len) = self.active_group_quiz_len() else {
            return;
        };
        if self.app.learning_quiz_index == 0 {
            self.app.learning_quiz_index = quiz_len - 1;
        } else {
            self.app.learning_quiz_index -= 1;
        }
        self.app.learning_option_index = 0;
        self.reset_feedback();
        log_debug(&format!(
            "App: moved to question {} of {} in group {}",
            self.app.learning_quiz_index + 1,
            quiz_len,
            self.app.learning_group_index + 1
        ));
        self.app.ensure_learning_indices();
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
        self.app.ensure_learning_indices();
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
        self.app.ensure_learning_indices();
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

    fn active_group_quiz_len(&self) -> Option<usize> {
        let response = self.app.learning_response.as_ref()?;
        let group = response.response.get(self.app.learning_group_index)?;
        let quiz_len = group.quiz.len();
        if quiz_len == 0 { None } else { Some(quiz_len) }
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

    fn reset_question_state(&mut self) {
        self.app.learning_quiz_index = 0;
        self.app.learning_option_index = 0;
        self.reset_feedback();
    }

    fn reset_feedback(&mut self) {
        reset_learning_feedback(
            &mut self.app.learning_feedback,
            &mut self.app.learning_summary_revealed,
            &mut self.app.learning_waiting_for_next,
        );
    }
}
