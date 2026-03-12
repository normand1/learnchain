mod backend;
pub(crate) mod deep_dive;
pub(crate) mod deep_dive_types;
pub(crate) mod types;

pub(crate) use backend::LlmBackend;
pub(crate) use deep_dive_types::{
    DeepDiveArtifactMetadata, DeepDiveDocument, DeepDiveGenerationResult, DeepDiveHistoryEntry,
    DeepDiveResearchPlan, DeepDiveReviewedSource, DeepDiveTakeawayCard, StructuredDeepDiveResponse,
};
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
    AI_LOADING_FRAMES, AiTaskKind, AiTaskMessage, App, AppView, knowledge_store,
    log_util::log_debug, output_manager::OutputManager, reset_learning_feedback,
    session_sources::Session, view_managers::LearningManager,
};

pub(crate) fn handle_learning_success(app: &mut App, mut result: LearningGenerationResult) {
    LearningManager::shuffle_quiz_options(&mut result.response);
    let usage = result.usage.clone();
    let structured = result.response;
    let group_count = structured.response.len();
    let total_questions: usize = structured
        .response
        .iter()
        .map(|group| group.quiz.len())
        .sum();
    let quiz_session_date = if app.active_quiz_session_date.trim().is_empty() {
        app.session_date.clone()
    } else {
        app.active_quiz_session_date.clone()
    };

    let save_result = write_ai_response(app, &structured);
    let store_result = Some(knowledge_store::record_learning_response(
        &quiz_session_date,
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
    app.active_quiz_session_date = quiz_session_date;
    app.learning_response = Some(structured);
    app.last_quiz_event_timestamp = app.last_event_timestamp.clone();
    app.view = AppView::Learning;
    log_debug(&format!(
        "App: loaded learning response with {} group(s)",
        group_count
    ));
}

pub(crate) fn handle_deep_dive_success(app: &mut App, result: DeepDiveGenerationResult) {
    let usage = result.usage.clone();
    let reviewed_failures = result.reviewed_source_failures;
    let quiz_question_count: usize = result
        .response
        .quiz_groups
        .iter()
        .map(|group| group.quiz.len())
        .sum();
    let document = result.document;
    let mut status_parts = vec![
        format!("Saved to {}", document.path.display()),
        format!(
            "Referenced URLs: {}",
            document.metadata.referenced_url_count
        ),
        format!("Reviewed URLs: {}", document.metadata.reviewed_url_count),
        format!("Quiz questions: {}", quiz_question_count),
    ];
    if !reviewed_failures.is_empty() {
        status_parts.push(format!("Fetch failures: {}", reviewed_failures.len()));
    }
    if let Some(usage) = usage {
        status_parts.push(format!("Tokens: {}", usage.total_tokens));
    }
    app.ai_status = Some(status_parts.join(" • "));
    app.show_deep_dive_document(document);
    log_debug("App: loaded deep-dive document");
}

pub(crate) fn handle_ai_error(app: &mut App, kind: AiTaskKind, message: String) {
    let trimmed = message.trim().to_string();
    let status_label = match kind {
        AiTaskKind::LearningLesson => "AI generation failed",
        AiTaskKind::SessionDeepDive => "Deep-dive generation failed",
    };
    if trimmed.starts_with("Failed to build Tokio runtime") {
        App::push_error(&mut app.error, trimmed.clone());
        log_debug(&format!("App: {}", trimmed));
        app.ai_status = Some("Unable to start AI runtime".to_string());
    } else {
        App::push_error(&mut app.error, format!("{}: {}", status_label, trimmed));
        log_debug(&format!("App: {}: {}", status_label, trimmed));
        app.ai_status = Some(status_label.to_string());
    }

    match kind {
        AiTaskKind::LearningLesson => {
            if !matches!(app.view, AppView::Learning) {
                app.view = AppView::Menu;
            }
        }
        AiTaskKind::SessionDeepDive => {
            if !matches!(app.view, AppView::DeepDive) {
                app.view = AppView::Menu;
            }
        }
    }
}

fn learning_call_progress_message(provider: crate::config::AiProvider) -> &'static str {
    match provider {
        crate::config::AiProvider::CodexCli => "Calling Codex CLI...",
        crate::config::AiProvider::ClaudeCodeCli => "Calling Claude Code CLI...",
        _ => "Calling LLM via Rig...",
    }
}

pub(crate) fn trigger_learning_response(app: &mut App) {
    log_debug("App: menu option 'Generate quiz' selected");
    if app.ai_loading {
        log_debug("App: AI generation already in progress; ignoring duplicate request");
        return;
    }

    let backend = match app.llm_backend.clone() {
        Some(generator) => generator,
        None => {
            let help = app.ai_provider.setup_help().to_string();
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

    app.active_quiz_session_date = app.session_date.clone();
    app.view = AppView::Learning;
    start_ai_task(app, AiTaskKind::LearningLesson);
    let summary_override = app.summary_content.clone();
    let provider_label = app.ai_provider.label().to_string();
    let request_message = learning_call_progress_message(app.ai_provider);
    let sender = take_sender(app);

    thread::spawn(move || {
        log_debug(&format!(
            "App: background {} learning generation task started",
            provider_label
        ));
        send_progress(
            &sender,
            AiTaskKind::LearningLesson,
            "Loading session summary...",
            10,
        );

        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = sender.send(AiTaskMessage::Error(
                    AiTaskKind::LearningLesson,
                    format!("Failed to build Tokio runtime: {}", err),
                ));
                return;
            }
        };

        send_progress(&sender, AiTaskKind::LearningLesson, request_message, 30);

        let result = runtime
            .block_on(backend.generate_learning_response_with_progress(summary_override, &sender));
        drop(runtime);

        match result {
            Ok(structured) => {
                send_progress(
                    &sender,
                    AiTaskKind::LearningLesson,
                    "Finalizing quiz...",
                    95,
                );
                let _ = sender.send(AiTaskMessage::LearningSuccess(structured));
            }
            Err(err) => {
                let _ = sender.send(AiTaskMessage::Error(
                    AiTaskKind::LearningLesson,
                    err.to_string(),
                ));
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

    let backend = match app.llm_backend.clone() {
        Some(generator) => generator,
        None => {
            let help = app.ai_provider.setup_help().to_string();
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

    if app.active_quiz_session_date.trim().is_empty() {
        app.active_quiz_session_date = app.session_date.clone();
    }

    app.view = AppView::Learning;
    start_ai_task(app, AiTaskKind::LearningLesson);
    let summary_override = app.summary_content.clone();
    let provider_label = app.ai_provider.label().to_string();
    let request_message = learning_call_progress_message(app.ai_provider);
    let sender = take_sender(app);

    thread::spawn(move || {
        log_debug(&format!(
            "App: background {} learning generation task started (from selected session)",
            provider_label
        ));
        send_progress(
            &sender,
            AiTaskKind::LearningLesson,
            "Loading session summary...",
            10,
        );

        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = sender.send(AiTaskMessage::Error(
                    AiTaskKind::LearningLesson,
                    format!("Failed to build Tokio runtime: {}", err),
                ));
                return;
            }
        };

        send_progress(&sender, AiTaskKind::LearningLesson, request_message, 30);
        let result = runtime
            .block_on(backend.generate_learning_response_with_progress(summary_override, &sender));
        drop(runtime);

        match result {
            Ok(structured) => {
                send_progress(
                    &sender,
                    AiTaskKind::LearningLesson,
                    "Finalizing quiz...",
                    95,
                );
                let _ = sender.send(AiTaskMessage::LearningSuccess(structured));
            }
            Err(err) => {
                let _ = sender.send(AiTaskMessage::Error(
                    AiTaskKind::LearningLesson,
                    err.to_string(),
                ));
            }
        }
    });
}

pub(crate) fn trigger_deep_dive_response_from_session(app: &mut App, session: Session) {
    log_debug("App: generating deep dive from selected session");
    if app.ai_loading {
        log_debug("App: AI generation already in progress; ignoring duplicate request");
        return;
    }

    let backend = match app.llm_backend.clone() {
        Some(generator) => generator,
        None => {
            let help = app.ai_provider.setup_help().to_string();
            App::push_error(&mut app.error, help.clone());
            app.ai_status = Some(help);
            log_debug("App: deep-dive backend unavailable; aborting generation");
            return;
        }
    };

    app.view = AppView::DeepDive;
    start_ai_task(app, AiTaskKind::SessionDeepDive);
    let sender = take_sender(app);
    let session_source = app.session_source.clone();
    let provider_label = app.ai_provider.label().to_string();
    let deep_dive_sections = app.config_form.deep_dive_sections.clone();
    let min_quiz_questions = app.config_form.min_quiz_questions;

    thread::spawn(move || {
        log_debug(&format!(
            "App: background {} deep-dive generation task started",
            provider_label
        ));
        send_progress(
            &sender,
            AiTaskKind::SessionDeepDive,
            "Preparing selected session...",
            10,
        );

        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = sender.send(AiTaskMessage::Error(
                    AiTaskKind::SessionDeepDive,
                    format!("Failed to build Tokio runtime: {}", err),
                ));
                return;
            }
        };

        let result = runtime.block_on(deep_dive::generate_deep_dive_with_progress(
            &backend,
            &session_source,
            session,
            deep_dive_sections,
            min_quiz_questions,
            None,
            &sender,
        ));
        drop(runtime);

        match result {
            Ok(result) => {
                send_progress(
                    &sender,
                    AiTaskKind::SessionDeepDive,
                    "Finalizing deep dive...",
                    95,
                );
                let _ = sender.send(AiTaskMessage::DeepDiveSuccess(result));
            }
            Err(err) => {
                let _ = sender.send(AiTaskMessage::Error(
                    AiTaskKind::SessionDeepDive,
                    err.to_string(),
                ));
            }
        }
    });
}

fn start_ai_task(app: &mut App, kind: AiTaskKind) {
    let (sender, receiver) = mpsc::channel();
    app.ai_result_receiver = Some(receiver);
    app.ai_loading = true;
    app.ai_task_kind = Some(kind);
    app.ai_loading_frame = 0;
    app.ai_loading_start = Some(Instant::now());
    app.ai_progress_percent = 0;
    app.ai_progress_message = "Initializing...".to_string();
    app.ai_progress_timeline.clear();
    app.record_ai_progress(kind, "Initializing...".to_string(), 0);
    app.update_loading_status();
    app.ai_sender = Some(sender);
}

fn take_sender(app: &mut App) -> Sender<AiTaskMessage> {
    app.ai_sender
        .take()
        .expect("AI sender should be available after start_ai_task")
}

fn send_progress(sender: &Sender<AiTaskMessage>, kind: AiTaskKind, message: &str, percent: u8) {
    let _ = sender.send(AiTaskMessage::Progress(kind, message.to_string(), percent));
}

impl App {
    pub(crate) fn record_ai_progress(&mut self, kind: AiTaskKind, message: String, percent: u8) {
        let elapsed_secs = self
            .ai_loading_start
            .map(|start| start.elapsed().as_secs())
            .unwrap_or(0);
        let percent = percent.min(100);

        self.ai_task_kind = Some(kind);
        self.ai_progress_percent = percent;
        self.ai_progress_message = message.clone();

        if let Some(last_step) = self.ai_progress_timeline.last_mut() {
            if last_step.message == message {
                last_step.percent = percent;
                return;
            }

            if last_step.completed_at_secs.is_none() {
                last_step.completed_at_secs = Some(elapsed_secs);
            }
        }

        self.ai_progress_timeline.push(crate::AiProgressStep {
            message,
            percent,
            started_at_secs: elapsed_secs,
            completed_at_secs: None,
        });
    }

    pub(crate) fn finish_ai_progress_timeline(&mut self) {
        let elapsed_secs = self
            .ai_loading_start
            .map(|start| start.elapsed().as_secs())
            .unwrap_or(0);

        if let Some(last_step) = self.ai_progress_timeline.last_mut() {
            if last_step.completed_at_secs.is_none() {
                last_step.completed_at_secs = Some(elapsed_secs);
            }
        }
    }

    pub(crate) fn update_loading_status(&mut self) {
        if self.ai_loading {
            let frame = AI_LOADING_FRAMES[self.ai_loading_frame % AI_LOADING_FRAMES.len()];
            let label = match self.ai_task_kind.unwrap_or(AiTaskKind::LearningLesson) {
                AiTaskKind::LearningLesson => "Generating quiz…",
                AiTaskKind::SessionDeepDive => "Generating session deep dive…",
            };
            self.ai_status = Some(format!("{} {}", frame, label));
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

    let session_date = if app.active_quiz_session_date.trim().is_empty() {
        &app.session_date
    } else {
        &app.active_quiz_session_date
    };

    let mut path = output_dir.join(format!("learning-response-{}.json", session_date));
    let mut counter = 2;
    while path.exists() {
        path = output_dir.join(format!(
            "learning-response-{}-{}.json",
            session_date, counter
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
    loop {
        let next_message = match app.ai_result_receiver.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => break,
        };

        match next_message {
            Ok(message) => match message {
                AiTaskMessage::LearningSuccess(response) => {
                    app.finish_ai_progress_timeline();
                    app.ai_loading = false;
                    app.ai_loading_start = None;
                    app.ai_task_kind = None;
                    clear_receiver = true;
                    handle_learning_success(app, response);
                    break;
                }
                AiTaskMessage::DeepDiveSuccess(response) => {
                    app.finish_ai_progress_timeline();
                    app.ai_loading = false;
                    app.ai_loading_start = None;
                    app.ai_task_kind = None;
                    clear_receiver = true;
                    handle_deep_dive_success(app, response);
                    break;
                }
                AiTaskMessage::Error(kind, message) => {
                    app.finish_ai_progress_timeline();
                    app.ai_loading = false;
                    app.ai_loading_start = None;
                    app.ai_task_kind = None;
                    clear_receiver = true;
                    handle_ai_error(app, kind, message);
                    break;
                }
                AiTaskMessage::Progress(kind, message, percent) => {
                    app.record_ai_progress(kind, message, percent);
                }
            },
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                let kind = app.ai_task_kind.unwrap_or(AiTaskKind::LearningLesson);
                app.finish_ai_progress_timeline();
                app.ai_loading = false;
                app.ai_loading_start = None;
                app.ai_task_kind = None;
                clear_receiver = true;
                handle_ai_error(app, kind, "Background AI worker disconnected".to_string());
                break;
            }
        }
    }

    if clear_receiver {
        app.ai_result_receiver = None;
        app.ai_sender = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AiProvider, AnthropicModelKind, AppConfig, ConfigForm, OpenAiModelKind};
    use crate::llm::types::{KnowledgeResponse, LlmUsage, QuizItem, QuizOption};
    use std::{collections::HashSet, sync::mpsc, time::Duration};

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
            llm_backend: None,
            ai_status: None,
            ai_loading: false,
            ai_task_kind: None,
            ai_loading_frame: 0,
            ai_loading_start: None,
            ai_progress_percent: 0,
            ai_progress_message: String::new(),
            ai_progress_timeline: Vec::new(),
            ai_result_receiver: None,
            ai_sender: None,
            document_export_receiver: None,
            document_export_loading: false,
            learning_response: None,
            active_quiz_session_date: String::new(),
            learning_group_index: 0,
            learning_quiz_index: 0,
            learning_option_index: 0,
            learning_feedback: None,
            learning_summary_revealed: false,
            learning_waiting_for_next: false,
            session_selection_target: None,
            session_picker_selected_session: None,
            projects: Vec::new(),
            session_picker_selected_project: None,
            session_picker_viewing_projects: true,
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
            deep_dive_document: None,
            deep_dive_history_document: None,
            deep_dive_scroll: 0,
            deep_dive_history: Vec::new(),
            deep_dive_history_selected: None,
            deep_dive_showing_history: false,
            library_artifacts: Vec::new(),
            library_selected: None,
            skill_installer_selected_target: crate::EmbeddedSkillTarget::Codex,
            learnchain_setup: crate::LearnChainSetupState::default(),
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
        handle_learning_success(&mut app, sample_result());

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
            AiTaskKind::LearningLesson,
            "Failed to build Tokio runtime: missing permissions".to_string(),
        );

        let error = app.error.as_ref().unwrap();
        assert!(error.contains("Failed to build Tokio runtime"));
        assert_eq!(app.ai_status.as_deref(), Some("Unable to start AI runtime"));
        assert_eq!(app.view, AppView::Learning);

        let mut app = test_app();
        handle_ai_error(
            &mut app,
            AiTaskKind::LearningLesson,
            "network issue".to_string(),
        );
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
        assert!(error.contains(AiProvider::Anthropic.setup_help()));
        assert_eq!(
            app.ai_status.as_deref(),
            Some(AiProvider::Anthropic.setup_help())
        );
        assert!(!app.ai_loading);
        assert_eq!(app.view, AppView::Menu);
    }

    #[test]
    fn learning_call_progress_message_mentions_claude_code_cli() {
        assert_eq!(
            learning_call_progress_message(AiProvider::ClaudeCodeCli),
            "Calling Claude Code CLI..."
        );
    }

    #[test]
    fn poll_ai_messages_processes_success_and_clears_receiver() {
        let mut app = test_app();
        app.ai_loading = true;
        app.ai_loading_start = Some(
            Instant::now()
                .checked_sub(Duration::from_secs(3))
                .expect("duration should be subtractable from Instant"),
        );
        let (sender, receiver) = mpsc::channel();
        app.ai_result_receiver = Some(receiver);
        sender
            .send(AiTaskMessage::Progress(
                AiTaskKind::LearningLesson,
                "Loading session summary...".to_string(),
                10,
            ))
            .unwrap();
        sender
            .send(AiTaskMessage::LearningSuccess(sample_result()))
            .unwrap();

        poll_ai_messages(&mut app);

        assert!(!app.ai_loading);
        assert!(app.ai_result_receiver.is_none());
        assert!(app.learning_response.is_some());
        assert_eq!(app.view, AppView::Learning);
        assert_eq!(app.ai_progress_timeline.len(), 1);
        assert_eq!(
            app.ai_progress_timeline[0].message,
            "Loading session summary..."
        );
        assert!(app.ai_progress_timeline[0].completed_at_secs.is_some());
    }

    #[test]
    fn poll_ai_messages_processes_error_and_clears_receiver() {
        let mut app = test_app();
        app.ai_loading = true;
        app.ai_loading_start = Some(
            Instant::now()
                .checked_sub(Duration::from_secs(5))
                .expect("duration should be subtractable from Instant"),
        );
        let (sender, receiver) = mpsc::channel();
        app.ai_result_receiver = Some(receiver);
        sender
            .send(AiTaskMessage::Progress(
                AiTaskKind::LearningLesson,
                "Preparing structured learning request...".to_string(),
                40,
            ))
            .unwrap();
        sender
            .send(AiTaskMessage::Progress(
                AiTaskKind::LearningLesson,
                "Waiting for provider response...".to_string(),
                55,
            ))
            .unwrap();
        sender
            .send(AiTaskMessage::Error(
                AiTaskKind::LearningLesson,
                "failure".to_string(),
            ))
            .unwrap();

        poll_ai_messages(&mut app);

        assert!(!app.ai_loading);
        assert!(app.ai_result_receiver.is_none());
        let error = app.error.as_ref().unwrap();
        assert!(error.contains("AI generation failed: failure"));
        assert_eq!(app.ai_progress_timeline.len(), 2);
        assert_eq!(
            app.ai_progress_timeline[0].message,
            "Preparing structured learning request..."
        );
        assert_eq!(
            app.ai_progress_timeline[1].message,
            "Waiting for provider response..."
        );
        assert!(app.ai_progress_timeline[0].completed_at_secs.is_some());
        assert!(app.ai_progress_timeline[1].completed_at_secs.is_some());
    }
}
