mod backend;
pub(crate) mod types;

pub(crate) use backend::LearningGenerator;
pub(crate) use types::{LearningGenerationResult, StructuredLearningResponse};

use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Sender, TryRecvError},
    thread,
    time::Instant,
};

use color_eyre::eyre::{Result, WrapErr, eyre};
use serde_json::to_string_pretty;

use crate::{
    AI_LOADING_FRAMES, AiTaskMessage, App, AppView, knowledge_store, log_util::log_debug,
    output_manager::OutputManager, reset_learning_feedback, view_managers::LearningManager,
};

pub(crate) fn handle_ai_success(app: &mut App, mut result: LearningGenerationResult) {
    LearningManager::shuffle_quiz_options(&mut result.response);
    let usage = result.usage.clone();
    let structured = result.response;
    let group_count = structured.response.len();
    let total_questions: usize = structured
        .response
        .iter()
        .map(|group| group.quiz.len())
        .sum();

    let save_result = write_ai_response(app, &structured);
    let store_result = Some(knowledge_store::record_learning_response(
        &app.session_date,
        &structured,
    ));
    let mut status_parts = Vec::new();
    match save_result {
        Ok(saved_path) => {
            status_parts.push(format!("Saved to {}", saved_path.display()));
            log_debug(&format!(
                "App: learning response saved to {}",
                saved_path.display()
            ));
        }
        Err(err) => {
            App::push_error(
                &mut app.error,
                format!("Failed to save learning response: {}", err),
            );
            status_parts.push("Failed to save learning response".to_string());
            log_debug(&format!("App: failed to write learning response: {}", err));
        }
    }

    if let Some(store_result) = store_result {
        match store_result {
            Ok(_) => {
                status_parts.push("Knowledge history updated".to_string());
                log_debug("App: recorded learning response in knowledge store");
            }
            Err(err) => {
                App::push_error(
                    &mut app.error,
                    format!("Failed to record knowledge history: {}", err),
                );
                log_debug(&format!(
                    "App: failed to persist learning response to knowledge store: {}",
                    err
                ));
            }
        }
    }

    status_parts.push(format!("Knowledge groups: {}", group_count));
    status_parts.push(format!("Total quiz questions: {}", total_questions));
    if let Some(usage) = usage.as_ref() {
        status_parts.push(format!("Tokens: {}", usage.total_tokens));
    }
    app.ai_status = Some(status_parts.join(" • "));

    app.learning_group_index = 0;
    app.learning_quiz_index = 0;
    app.learning_option_index = 0;
    reset_learning_feedback(
        &mut app.learning_feedback,
        &mut app.learning_summary_revealed,
        &mut app.learning_waiting_for_next,
    );
    app.quiz_first_attempts.clear();
    app.analytics_snapshot = None;
    app.analytics_refreshed_at = None;
    app.learning_response = Some(structured);
    app.last_quiz_event_timestamp = app.last_event_timestamp.clone();
    log_debug(&format!(
        "App: loaded learning response with {} group(s)",
        group_count
    ));

    app.view = AppView::Learning;
    log_debug("App: switched to learning view");
}

pub(crate) fn handle_ai_error(app: &mut App, message: String) {
    let trimmed = message.trim().to_string();
    if trimmed.starts_with("Failed to build Tokio runtime") {
        App::push_error(&mut app.error, trimmed.clone());
        log_debug(&format!("App: {}", trimmed));
        app.ai_status = Some("Unable to start AI runtime".to_string());
    } else {
        App::push_error(&mut app.error, format!("AI generation failed: {}", trimmed));
        log_debug(&format!("App: AI generation error: {}", trimmed));
        app.ai_status = Some("AI generation failed".to_string());
    }

    if !matches!(app.view, AppView::Learning) {
        app.view = AppView::Menu;
    }
}

pub(crate) fn trigger_learning_response(app: &mut App) {
    log_debug("App: menu option 'Generate learning response' selected");
    if app.ai_loading {
        log_debug("App: AI generation already in progress; ignoring duplicate request");
        return;
    }

    let generator = match app.learning_generator.clone() {
        Some(generator) => generator,
        None => {
            let help = app.ai_provider.missing_key_help().to_string();
            App::push_error(&mut app.error, help.clone());
            app.ai_status = Some(help);
            log_debug("App: learning generator unavailable; aborting generation");
            return;
        }
    };

    if let Err(message) = app.sync_new_session_events() {
        App::push_error(&mut app.error, message.clone());
        app.ai_status = Some(message.clone());
        log_debug(&format!(
            "App: generation aborted because no new events were available: {}",
            message
        ));
        return;
    }

    let (sender, receiver) = mpsc::channel();
    app.ai_result_receiver = Some(receiver);
    app.ai_loading = true;
    app.ai_loading_frame = 0;
    app.ai_loading_start = Some(Instant::now());
    app.ai_progress_percent = 0;
    app.ai_progress_message = "Initializing...".to_string();
    app.update_loading_status();
    app.view = AppView::Learning;
    log_debug("App: displaying learning loading spinner");
    log_debug(&format!(
        "App: starting {} generation task",
        app.ai_provider.label()
    ));

    let summary_override = app.summary_content.clone();
    let provider_label = app.ai_provider.label().to_string();

    thread::spawn(move || {
        log_debug(&format!(
            "App: background {} generation task started",
            provider_label
        ));

        send_progress(&sender, "Loading session summary...", 10);

        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = sender.send(AiTaskMessage::Error(format!(
                    "Failed to build Tokio runtime: {}",
                    err
                )));
                return;
            }
        };

        send_progress(&sender, "Calling LLM via Rig...", 30);

        let result = runtime.block_on(
            generator.generate_learning_response_with_progress(summary_override, &sender),
        );
        drop(runtime);

        match result {
            Ok(structured) => {
                send_progress(&sender, "Finalizing quiz...", 95);
                let _ = sender.send(AiTaskMessage::Success(structured));
            }
            Err(err) => {
                let _ = sender.send(AiTaskMessage::Error(err.to_string()));
            }
        }
    });
}

pub(crate) fn trigger_learning_response_skip_sync(app: &mut App) {
    log_debug("App: generating learning response from selected session");
    if app.ai_loading {
        log_debug("App: AI generation already in progress; ignoring duplicate request");
        return;
    }

    let generator = match app.learning_generator.clone() {
        Some(generator) => generator,
        None => {
            let help = app.ai_provider.missing_key_help().to_string();
            App::push_error(&mut app.error, help.clone());
            app.ai_status = Some(help);
            log_debug("App: learning generator unavailable; aborting generation");
            return;
        }
    };

    if app.summary_content.is_none() {
        App::push_error(
            &mut app.error,
            "No session content available for generation.".to_string(),
        );
        return;
    }

    let (sender, receiver) = mpsc::channel();
    app.ai_result_receiver = Some(receiver);
    app.ai_loading = true;
    app.ai_loading_frame = 0;
    app.ai_loading_start = Some(Instant::now());
    app.ai_progress_percent = 0;
    app.ai_progress_message = "Initializing...".to_string();
    app.update_loading_status();
    app.view = AppView::Learning;
    log_debug("App: displaying learning loading spinner");
    log_debug("App: starting learning generation task from selected session");

    let summary_override = app.summary_content.clone();
    let provider_label = app.ai_provider.label().to_string();

    thread::spawn(move || {
        log_debug(&format!(
            "App: background {} generation task started (from selected session)",
            provider_label
        ));

        send_progress(&sender, "Loading session summary...", 10);

        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = sender.send(AiTaskMessage::Error(format!(
                    "Failed to build Tokio runtime: {}",
                    err
                )));
                return;
            }
        };

        send_progress(&sender, "Calling LLM via Rig...", 30);

        let result = runtime.block_on(
            generator.generate_learning_response_with_progress(summary_override, &sender),
        );
        drop(runtime);

        match result {
            Ok(structured) => {
                send_progress(&sender, "Finalizing quiz...", 95);
                let _ = sender.send(AiTaskMessage::Success(structured));
            }
            Err(err) => {
                let _ = sender.send(AiTaskMessage::Error(err.to_string()));
            }
        }
    });
}

fn send_progress(sender: &Sender<AiTaskMessage>, message: &str, percent: u8) {
    let _ = sender.send(AiTaskMessage::Progress(message.to_string(), percent));
}

impl App {
    pub(crate) fn update_loading_status(&mut self) {
        if self.ai_loading {
            let frame = AI_LOADING_FRAMES[self.ai_loading_frame % AI_LOADING_FRAMES.len()];
            self.ai_status = Some(format!("{} Generating learning response…", frame));
        }
    }
}

fn write_ai_response(app: &App, response: &StructuredLearningResponse) -> Result<PathBuf> {
    if !app.write_output_artifacts {
        let serialized =
            to_string_pretty(response).wrap_err("failed to serialise learning response to JSON")?;
        return Ok(PathBuf::from(format!(
            "<in-memory: {} bytes>",
            serialized.len()
        )));
    }

    let manager = OutputManager::new();
    let output_dir = manager.output_directory().map_err(|err| eyre!(err))?;
    fs::create_dir_all(&output_dir).wrap_err_with(|| {
        format!(
            "failed to create output directory at {}",
            output_dir.display()
        )
    })?;

    let mut path = output_dir.join(format!("learning-response-{}.json", app.session_date));
    let mut counter = 2;
    while path.exists() {
        path = output_dir.join(format!(
            "learning-response-{}-{}.json",
            app.session_date, counter
        ));
        counter += 1;
    }

    let serialized =
        to_string_pretty(response).wrap_err("failed to serialise learning response to JSON")?;
    fs::write(&path, serialized)
        .wrap_err_with(|| format!("failed to write learning response to {}", path.display()))?;
    Ok(path)
}

pub(crate) fn poll_ai_messages(app: &mut App) {
    let mut clear_receiver = false;
    if let Some(receiver) = app.ai_result_receiver.as_ref() {
        loop {
            match receiver.try_recv() {
                Ok(message) => match message {
                    AiTaskMessage::Success(response) => {
                        app.ai_loading = false;
                        app.ai_loading_start = None;
                        clear_receiver = true;
                        handle_ai_success(app, response);
                        break;
                    }
                    AiTaskMessage::Error(message) => {
                        app.ai_loading = false;
                        app.ai_loading_start = None;
                        clear_receiver = true;
                        handle_ai_error(app, message);
                        break;
                    }
                    AiTaskMessage::Progress(message, percent) => {
                        app.ai_progress_message = message;
                        app.ai_progress_percent = percent;
                    }
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    app.ai_loading = false;
                    app.ai_loading_start = None;
                    clear_receiver = true;
                    handle_ai_error(app, "Background AI worker disconnected".to_string());
                    break;
                }
            }
        }
    }

    if clear_receiver {
        app.ai_result_receiver = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AiProvider, AnthropicModelKind, AppConfig, ConfigForm, OpenAiModelKind};
    use crate::llm::types::{KnowledgeResponse, LlmUsage, QuizItem, QuizOption};
    use std::{collections::HashSet, sync::mpsc};

    fn test_app() -> App {
        App {
            running: false,
            view: AppView::Menu,
            menu_index: 0,
            events: Vec::new(),
            selected_event: None,
            sessions: Vec::new(),
            selected_session: None,
            viewing_sessions_list: true,
            session_dir: PathBuf::new(),
            session_date: "2024-05-01".to_string(),
            session_source: String::new(),
            latest_file: None,
            summary_file: None,
            summary_content: None,
            error: None,
            ai_provider: AiProvider::OpenAI,
            learning_generator: None,
            ai_status: None,
            ai_loading: false,
            ai_loading_frame: 0,
            ai_loading_start: None,
            ai_progress_percent: 0,
            ai_progress_message: String::new(),
            ai_result_receiver: None,
            learning_response: None,
            learning_group_index: 0,
            learning_quiz_index: 0,
            learning_option_index: 0,
            learning_feedback: None,
            learning_summary_revealed: false,
            learning_waiting_for_next: false,
            learning_selecting_session: false,
            learning_selected_session: None,
            projects: Vec::new(),
            learning_selected_project: None,
            learning_viewing_projects: true,
            config_form: ConfigForm::from_config(AppConfig::default()),
            write_output_artifacts: false,
            openai_model: OpenAiModelKind::Gpt5Mini,
            anthropic_model: AnthropicModelKind::ClaudeSonnet4,
            openrouter_model: String::new(),
            quiz_first_attempts: HashSet::new(),
            quiz_first_attempt_results: std::collections::HashMap::new(),
            analytics_snapshot: None,
            analytics_error: None,
            analytics_refreshed_at: None,
            last_event_timestamp: None,
            last_quiz_event_timestamp: None,
            learning_showing_summary: false,
            quiz_summary_results: Vec::new(),
        }
    }

    fn sample_result() -> LearningGenerationResult {
        LearningGenerationResult {
            response: StructuredLearningResponse {
                response: vec![KnowledgeResponse {
                    knowledge_type_group: "Rust Basics".to_string(),
                    summary: "Borrowing overview".to_string(),
                    quiz: vec![QuizItem {
                        question: "What does borrow checking ensure?".to_string(),
                        options: vec![
                            QuizOption {
                                selection: "Memory safety".to_string(),
                                is_correct_answer: true,
                            },
                            QuizOption {
                                selection: "Runtime polymorphism".to_string(),
                                is_correct_answer: false,
                            },
                        ],
                        resources: vec!["https://doc.rust-lang.org/".to_string()],
                    }],
                    knowledge_type_language: "Rust".to_string(),
                }],
            },
            usage: Some(LlmUsage {
                input_tokens: 120,
                output_tokens: 40,
                total_tokens: 160,
            }),
        }
    }

    #[test]
    fn write_ai_response_returns_in_memory_path_when_not_persisting() {
        let app = test_app();
        let path = write_ai_response(&app, &sample_result().response).unwrap();
        let display = path.display().to_string();
        assert!(display.starts_with("<in-memory:"));
        assert!(display.contains("bytes>"));
    }

    #[test]
    fn handle_ai_success_sets_learning_state_and_status() {
        let mut app = test_app();
        handle_ai_success(&mut app, sample_result());

        assert_eq!(app.view, AppView::Learning);
        assert_eq!(app.learning_group_index, 0);
        assert_eq!(app.learning_quiz_index, 0);
        assert_eq!(app.learning_option_index, 0);
        assert!(app.learning_response.is_some());
        assert!(app.learning_feedback.is_none());
        assert!(!app.learning_summary_revealed);
        assert!(!app.learning_waiting_for_next);
        let status = app.ai_status.as_ref().unwrap();
        assert!(status.contains("Knowledge groups: 1"));
        assert!(status.contains("Total quiz questions: 1"));
        assert!(status.contains("Tokens: 160"));
    }

    #[test]
    fn handle_ai_error_distinguishes_runtime_failure() {
        let mut app = test_app();
        app.view = AppView::Learning;
        handle_ai_error(
            &mut app,
            "Failed to build Tokio runtime: missing permissions".to_string(),
        );

        let error = app.error.as_ref().unwrap();
        assert!(error.contains("Failed to build Tokio runtime"));
        assert_eq!(app.ai_status.as_deref(), Some("Unable to start AI runtime"));
        assert_eq!(app.view, AppView::Learning);

        let mut app = test_app();
        handle_ai_error(&mut app, "network issue".to_string());
        let error = app.error.as_ref().unwrap();
        assert!(error.contains("AI generation failed: network issue"));
        assert_eq!(app.ai_status.as_deref(), Some("AI generation failed"));
        assert_eq!(app.view, AppView::Menu);
    }

    #[test]
    fn trigger_learning_response_without_generator_surfaces_provider_help() {
        let mut app = test_app();
        app.ai_provider = AiProvider::Anthropic;
        trigger_learning_response(&mut app);
        let error = app.error.as_ref().unwrap();
        assert!(error.contains(AiProvider::Anthropic.missing_key_help()));
        assert_eq!(
            app.ai_status.as_deref(),
            Some(AiProvider::Anthropic.missing_key_help())
        );
        assert!(!app.ai_loading);
        assert_eq!(app.view, AppView::Menu);
    }

    #[test]
    fn poll_ai_messages_processes_success_and_clears_receiver() {
        let mut app = test_app();
        app.ai_loading = true;
        let (sender, receiver) = mpsc::channel();
        app.ai_result_receiver = Some(receiver);
        sender
            .send(AiTaskMessage::Success(sample_result()))
            .unwrap();

        poll_ai_messages(&mut app);

        assert!(!app.ai_loading);
        assert!(app.ai_result_receiver.is_none());
        assert!(app.learning_response.is_some());
        assert_eq!(app.view, AppView::Learning);
    }

    #[test]
    fn poll_ai_messages_processes_error_and_clears_receiver() {
        let mut app = test_app();
        app.ai_loading = true;
        let (sender, receiver) = mpsc::channel();
        app.ai_result_receiver = Some(receiver);
        sender
            .send(AiTaskMessage::Error("failure".to_string()))
            .unwrap();

        poll_ai_messages(&mut app);

        assert!(!app.ai_loading);
        assert!(app.ai_result_receiver.is_none());
        let error = app.error.as_ref().unwrap();
        assert!(error.contains("AI generation failed: failure"));
    }
}
