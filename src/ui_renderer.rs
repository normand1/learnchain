use crate::view_managers::menu_manager::MENU_OPTIONS;
use crate::{
    AI_LOADING_FRAMES, App, AppView, config, reset_learning_feedback,
    view_managers::LearningManager,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style, Stylize},
    text::Line,
    widgets::{Block, List, ListItem, ListState, Paragraph, Wrap},
};

pub(crate) struct UiRenderer<'a> {
    app: &'a mut App,
}

impl<'a> UiRenderer<'a> {
    pub(crate) fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(crate) fn render(&mut self, frame: &mut Frame) {
        match self.app.view {
            AppView::Menu => self.render_menu(frame),
            AppView::Events => self.render_events(frame),
            AppView::Learning => self.render_learning(frame),
            AppView::Config => self.render_config(frame),
        }
    }

    fn render_menu(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;
        let session_title = if app.session_source == "Claude Code" {
            "Claude Sessions"
        } else {
            "Codex Sessions"
        };
        let header_title = Line::from(format!("{} • {}", session_title, app.session_date))
            .bold()
            .blue()
            .centered();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(4),
            ])
            .split(frame.area());

        frame.render_widget(
            Paragraph::new(Self::header_text(app))
                .block(Block::bordered().title(header_title))
                .centered(),
            layout[0],
        );

        let menu_sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3)])
            .split(layout[1]);

        let actions_items = vec![ListItem::new(MENU_OPTIONS[0])];
        let mut actions_state = ListState::default();
        if app.menu_index == 0 {
            actions_state.select(Some(0));
        }

        frame.render_stateful_widget(
            List::new(actions_items)
                .block(Block::bordered().title(Line::from("Actions")))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            menu_sections[0],
            &mut actions_state,
        );

        let config_items: Vec<ListItem> = MENU_OPTIONS[1..]
            .iter()
            .map(|label| ListItem::new(*label))
            .collect();
        let mut config_state = ListState::default();
        if app.menu_index > 0 {
            config_state.select(Some(app.menu_index - 1));
        }

        frame.render_stateful_widget(
            List::new(config_items)
                .block(Block::bordered().title(Line::from("Config")))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            menu_sections[1],
            &mut config_state,
        );

        let mut status_lines = Vec::new();
        if let Some(error) = &app.error {
            status_lines.push(format!("Error: {}", error));
        }
        if let Some(status) = &app.ai_status {
            status_lines.push(format!("AI: {}", status));
        }
        status_lines.push("Use ↑/↓ or j/k to choose. Press Enter to select.".to_string());
        status_lines
            .push("Press 1, 2, or 3 for quick selection. Esc, Ctrl-C, or q to quit.".to_string());
        if app.learning_response.is_some() {
            status_lines.push("Press l to revisit the latest learning response.".to_string());
        }
        status_lines.push("Press c to configure details.".to_string());

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn render_events(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;
        let session_title = if app.session_source == "Claude Code" {
            "Claude Sessions"
        } else {
            "Codex Sessions"
        };
        let header_title = Line::from(format!("{} • {}", session_title, app.session_date))
            .bold()
            .blue()
            .centered();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(4),
            ])
            .split(frame.area());

        frame.render_widget(
            Paragraph::new(Self::header_text(app))
                .block(Block::bordered().title(header_title))
                .centered(),
            layout[0],
        );

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(layout[1]);

        let list_items: Vec<ListItem> = if app.events.is_empty() {
            vec![ListItem::new(
                "No matching events found in the latest session file.",
            )]
        } else {
            app.events
                .iter()
                .map(|event| {
                    ListItem::new(format!(
                        "{:<19} | {:<24} | {}",
                        event.payload_type,
                        event.call_id.as_deref().unwrap_or("-"),
                        event.timestamp
                    ))
                })
                .collect()
        };

        let mut list_state = ListState::default();
        list_state.select(app.selected_event);

        frame.render_stateful_widget(
            List::new(list_items)
                .block(Block::bordered().title(Line::from("Events")))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            body[0],
            &mut list_state,
        );

        let detail_text = match app.selected_event.and_then(|index| app.events.get(index)) {
            Some(event) => {
                let header = format!(
                    "type: {}\ncall_id: {}\ntimestamp: {}\n",
                    event.payload_type,
                    event.call_id.as_deref().unwrap_or("-"),
                    event.timestamp
                );

                let mut sections = Vec::new();
                if !event.content_texts.is_empty() {
                    sections.push(event.content_texts.join("\n\n"));
                }
                if let Some(arguments) = event
                    .arguments
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                {
                    sections.push(format!("arguments:\n{}", arguments));
                }
                if let Some(output) = event
                    .output
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                {
                    sections.push(format!("output:\n{}", output));
                }

                if sections.is_empty() {
                    format!(
                        "{}\n{}",
                        header, "No payload details available for this event."
                    )
                } else {
                    format!("{}\n{}", header, sections.join("\n\n"))
                }
            }
            None => "Select an event to view its payload details.".to_string(),
        };

        frame.render_widget(
            Paragraph::new(detail_text)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(Line::from("Output"))),
            body[1],
        );

        let mut status_lines = Vec::new();
        if let Some(error) = &app.error {
            status_lines.push(format!("Error: {}", error));
        }
        if let Some(status) = &app.ai_status {
            status_lines.push(format!("AI: {}", status));
        }
        status_lines.push(format!("Matching events: {}", app.events.len()));
        status_lines.push(
            "Use ↑/↓ or j/k to navigate. Press m for menu. Esc, Ctrl-C, or q to quit.".to_string(),
        );
        if app.learning_response.is_some() {
            status_lines.push("Press l to view generated learning prompts.".to_string());
        }

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn render_learning(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;
        LearningManager::ensure_indices_for(app);

        let session_title = if app.session_source == "Claude Code" {
            "Claude Sessions"
        } else {
            "Codex Sessions"
        };
        let header_title = Line::from(format!("{} • {}", session_title, app.session_date))
            .bold()
            .blue()
            .centered();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(4),
            ])
            .split(frame.area());

        frame.render_widget(
            Paragraph::new(Self::header_text(app))
                .block(Block::bordered().title(header_title))
                .centered(),
            layout[0],
        );

        let main_sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(6)])
            .split(layout[1]);

        let mut question_text =
            String::from("No learning response available. Generate one from the main menu.");
        let mut resources_text = String::from("No resources to display.");
        let mut status_lines: Vec<String> = Vec::new();

        if app.ai_loading {
            let frame_symbol = AI_LOADING_FRAMES[app.ai_loading_frame % AI_LOADING_FRAMES.len()];
            question_text = format!(
                "{} Generating learning response…\n\nWe'll show the quiz once the AI reply is ready.",
                frame_symbol
            );
            resources_text = String::from("Resources will appear after generation completes.");
        } else if let Some(response) = &app.learning_response {
            if response.response.is_empty() {
                question_text =
                    String::from("The generated response did not include any knowledge groups.");
                resources_text = String::from("No additional resources provided.");
            } else {
                let group_count = response.response.len();
                let group_index = app.learning_group_index.min(group_count.saturating_sub(1));
                let group = &response.response[group_index];
                let quiz_count = group.quiz.len();
                let language_line = match group.knowledge_type_language.trim() {
                    "" => String::new(),
                    lang => format!("\nLanguage: {}", lang),
                };

                if quiz_count == 0 {
                    question_text = format!(
                        "Knowledge group {}/{}\nName: {}{}\nSummary: {}\n\nNo quiz questions were provided for this topic.",
                        group_index + 1,
                        group_count,
                        group.knowledge_type_group,
                        language_line,
                        group.summary
                    );
                    app.learning_option_index = 0;
                    reset_learning_feedback(
                        &mut app.learning_feedback,
                        &mut app.learning_summary_revealed,
                        &mut app.learning_waiting_for_next,
                    );
                    resources_text = String::from("No additional resources provided.");
                } else {
                    let quiz_index = app.learning_quiz_index.min(quiz_count - 1);
                    let question = group.quiz.get(quiz_index).cloned().unwrap_or_default();

                    let option_count = question.options.len();
                    let mut option_lines = Vec::new();
                    if option_count == 0 {
                        option_lines.push(String::from("- No answer options provided"));
                        app.learning_option_index = 0;
                        reset_learning_feedback(
                            &mut app.learning_feedback,
                            &mut app.learning_summary_revealed,
                            &mut app.learning_waiting_for_next,
                        );
                    } else {
                        let selected_option = app.learning_option_index.min(option_count - 1);
                        let answered = app.learning_feedback.is_some();
                        for (index, option) in question.options.iter().enumerate() {
                            let label = ((b'A' + (index % 26) as u8) as char).to_string();
                            let marker = if answered && option.is_correct_answer {
                                "[✓]"
                            } else {
                                "[ ]"
                            };
                            let prefix = if index == selected_option { "▶" } else { " " };
                            option_lines.push(format!(
                                "{} {} {} {}",
                                prefix, marker, label, option.selection
                            ));
                        }
                        app.learning_option_index = selected_option;
                    }
                    let options_text = option_lines.join("\n");
                    let summary_line = if app.learning_summary_revealed {
                        format!("\n\nSummary: {}", group.summary)
                    } else {
                        String::new()
                    };
                    let feedback_line = if let Some(feedback) = app.learning_feedback.as_deref() {
                        format!("\n\nFeedback: {}", feedback)
                    } else {
                        String::new()
                    };

                    if app.learning_waiting_for_next {
                        let mut segments = vec![format!(
                            "Knowledge group {}/{}\nName: {}{}",
                            group_index + 1,
                            group_count,
                            group.knowledge_type_group,
                            language_line,
                        )];
                        segments.push(format!("Summary: {}", group.summary));
                        if let Some(feedback) = app.learning_feedback.as_deref() {
                            segments.push(format!("Result: {}", feedback));
                        }
                        segments.push(String::from("Press any key to continue."));
                        question_text = segments.join("\n\n");
                    } else {
                        question_text = format!(
                            "Knowledge group {}/{}\nName: {}{}\n\nQuestion {}/{}:\n{}\n\nOptions:\n{}{}{}",
                            group_index + 1,
                            group_count,
                            group.knowledge_type_group,
                            language_line,
                            quiz_index + 1,
                            quiz_count,
                            question.question,
                            options_text,
                            feedback_line,
                            summary_line
                        );
                    }

                    app.learning_option_index = app
                        .learning_option_index
                        .min(option_count.saturating_sub(1));

                    resources_text = if question.resources.is_empty() {
                        String::from("No additional resources provided.")
                    } else {
                        question
                            .resources
                            .into_iter()
                            .enumerate()
                            .map(|(index, resource)| format!("{}. {}", index + 1, resource))
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                }
            }
        }

        if let Some(error) = &app.error {
            status_lines.push(format!("Error: {}", error));
        }
        if let Some(status) = &app.ai_status {
            status_lines.push(format!("AI: {}", status));
        }
        status_lines.push("Press r to regenerate quiz from the latest session events.".to_string());
        status_lines.push("Press m to return to the main menu.".to_string());

        frame.render_widget(
            Paragraph::new(question_text)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(Line::from("Learning Question"))),
            main_sections[0],
        );

        frame.render_widget(
            Paragraph::new(resources_text)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(Line::from("Resources"))),
            main_sections[1],
        );

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn render_config(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;
        let session_title = if app.session_source == "Claude Code" {
            "Claude Sessions"
        } else {
            "Codex Sessions"
        };
        let header_title = Line::from(format!("{} • {}", session_title, app.session_date))
            .bold()
            .blue()
            .centered();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(4),
            ])
            .split(frame.area());

        let config_path = config::config_file_path();
        let header_text = format!(
            "Config file: {}\nAdjust default limits used for summaries and AI prompts.",
            config_path.display()
        );

        frame.render_widget(
            Paragraph::new(header_text)
                .block(Block::bordered().title(header_title))
                .centered(),
            layout[0],
        );

        let items = vec![
            ListItem::new(format!(
                "Default max events (markdown summaries): {}",
                app.config_form.max_events
            )),
            ListItem::new(format!(
                "Minimum quiz questions (AI prompt): {}",
                app.config_form.min_quiz_questions
            )),
            ListItem::new(format!(
                "Session source: {}",
                app.config_form.session_source.label()
            )),
            ListItem::new(format!(
                "Write artifacts to output: {}",
                if app.config_form.write_output_artifacts {
                    "Enabled"
                } else {
                    "Disabled"
                }
            )),
            ListItem::new(format!(
                "OpenAI model: {}",
                app.config_form.openai_model.label()
            )),
            ListItem::new(if app.config_form.is_editing_openai_key() {
                format!(
                    "OpenAI API key (editing): {}",
                    app.config_form.masked_openai_key_buffer()
                )
            } else {
                format!("OpenAI API key: {}", app.config_form.masked_openai_key())
            }),
        ];

        let mut list_state = ListState::default();
        list_state.select(Some(app.config_form.selected_index()));

        frame.render_stateful_widget(
            List::new(items)
                .block(Block::bordered().title(Line::from("Defaults")))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            layout[1],
            &mut list_state,
        );

        let mut status_lines = Vec::new();
        if let Some(error) = &app.error {
            status_lines.push(format!("Error: {}", error));
        }
        if let Some(ai_status) = &app.ai_status {
            status_lines.push(format!("AI: {}", ai_status));
        }
        status_lines.push(
            "↑/↓ or j/k choose field. ←/→ or h/l adjust value or cycle source/model toggles."
                .to_string(),
        );
        status_lines.push(
            "Select \"OpenAI API key\" and press Enter to edit. Type to update, Enter to save, Esc to cancel.".to_string(),
        );
        status_lines
            .push("Press s to save, r to reset, m to save and return to the menu.".to_string());

        if app.config_form.dirty {
            status_lines.push("Unsaved changes".to_string());
        }
        if let Some(config_status) = &app.config_form.status {
            status_lines.push(config_status.clone());
        }

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn header_text(app: &App) -> String {
        let latest_line = match &app.latest_file {
            Some(path) => format!("Latest file: {}", path.display()),
            None => "Latest file: <none>".to_string(),
        };
        let summary_line = if !app.write_output_artifacts {
            if app.summary_content.is_some() {
                "Summary: <in-memory>".to_string()
            } else {
                "Summary: <none>".to_string()
            }
        } else {
            match &app.summary_file {
                Some(path) => format!("Summary: {}", path.display()),
                None => "Summary: <none>".to_string(),
            }
        };
        let source_line = format!("Source: {}", app.session_source);
        format!(
            "Directory: {}\n{}\n{}\n{}",
            app.session_dir.display(),
            latest_line,
            summary_line,
            source_line
        )
    }
}
