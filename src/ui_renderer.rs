use crate::view_managers::menu_manager::MENU_OPTIONS;
use crate::{
    AI_LOADING_FRAMES, App, AppView, config,
    knowledge_store::{DailyAnalytics, KnowledgeAnalytics},
    reset_learning_feedback,
    view_managers::LearningManager,
};
use chrono::{Datelike, Duration, Utc, Weekday};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, List, ListItem, ListState, Paragraph, Wrap},
};
use std::cmp;

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
            AppView::Analytics => self.render_analytics(frame),
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
                Constraint::Min(8),
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
            .constraints([Constraint::Length(5), Constraint::Min(3)])
            .split(layout[1]);

        let actions_items: Vec<ListItem> = MENU_OPTIONS[..2]
            .iter()
            .map(|label| ListItem::new(*label))
            .collect();
        let actions_len = actions_items.len();
        let mut actions_state = ListState::default();
        if app.menu_index < actions_len {
            actions_state.select(Some(app.menu_index));
        }

        frame.render_stateful_widget(
            List::new(actions_items)
                .block(Block::bordered().title(Line::from("Actions")))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            menu_sections[0],
            &mut actions_state,
        );

        let config_items: Vec<ListItem> = MENU_OPTIONS[2..]
            .iter()
            .map(|label| ListItem::new(*label))
            .collect();
        let mut config_state = ListState::default();
        if app.menu_index >= actions_len {
            config_state.select(Some(app.menu_index - actions_len));
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
        status_lines.push("Press 1-4 for quick selection. Esc, Ctrl-C, or q to quit.".to_string());
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

    fn render_analytics(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Min(14),
                Constraint::Length(5),
            ])
            .split(frame.area());

        let title = Line::from("Learning Analytics Dashboard")
            .bold()
            .green()
            .centered();

        let summary_text = if let Some(snapshot) = app.analytics_snapshot.as_ref() {
            let accuracy = if snapshot.total_attempts > 0 {
                (snapshot.total_first_try_correct as f64 / snapshot.total_attempts as f64) * 100.0
            } else {
                0.0
            };
            format!(
                "Tracking the last {} day(s). First-try accuracy: {:>5.1}%.",
                snapshot.daily.len(),
                accuracy
            )
        } else {
            "No analytics available yet. Complete a lesson then press r to refresh.".to_string()
        };

        frame.render_widget(
            Paragraph::new(summary_text)
                .style(Style::default().fg(Color::Rgb(180, 205, 255)))
                .block(
                    Block::bordered()
                        .title(title)
                        .border_style(Style::default().fg(Color::Rgb(120, 140, 220))),
                )
                .wrap(Wrap { trim: true })
                .centered(),
            layout[0],
        );

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .margin(1)
            .split(layout[1]);

        if let Some(snapshot) = app.analytics_snapshot.as_ref() {
            let heatmap = Paragraph::new(Self::analytics_heatmap(snapshot))
                .block(
                    Block::bordered()
                        .title(Line::from("Daily first-try performance"))
                        .border_style(Style::default().fg(Color::Rgb(120, 140, 220))),
                )
                .wrap(Wrap { trim: false });
            frame.render_widget(heatmap, body[0]);

            let summary_lines = Self::analytics_summary_lines(snapshot, app);
            frame.render_widget(
                Paragraph::new(Text::from(summary_lines))
                    .style(Style::default().fg(Color::Rgb(189, 255, 154)))
                    .block(
                        Block::bordered()
                            .title(Line::from("Highlights"))
                            .border_style(Style::default().fg(Color::Rgb(120, 140, 220))),
                    )
                    .wrap(Wrap { trim: true }),
                body[1],
            );
        } else {
            let message = if let Some(error) = app.analytics_error.as_ref() {
                format!("Unable to load analytics: {}", error)
            } else {
                "Analytics data will appear after you record quiz attempts.".to_string()
            };

            frame.render_widget(
                Paragraph::new(message.clone())
                    .block(
                        Block::bordered()
                            .title(Line::from("Daily first-try performance"))
                            .border_style(Style::default().fg(Color::Rgb(120, 140, 220))),
                    )
                    .wrap(Wrap { trim: true }),
                body[0],
            );

            frame.render_widget(
                Paragraph::new(message)
                    .block(
                        Block::bordered()
                            .title(Line::from("Highlights"))
                            .border_style(Style::default().fg(Color::Rgb(120, 140, 220))),
                    )
                    .wrap(Wrap { trim: true }),
                body[1],
            );
        }

        let mut footer_lines = Vec::new();
        footer_lines.push("Press r to refresh analytics.".to_string());
        footer_lines.push("Press m to return to the main menu.".to_string());
        frame.render_widget(
            Paragraph::new(footer_lines.join("\n"))
                .style(Style::default().fg(Color::Rgb(180, 205, 255)))
                .block(
                    Block::bordered()
                        .title(Line::from("Commands"))
                        .border_style(Style::default().fg(Color::Rgb(120, 140, 220))),
                ),
            layout[2],
        );
    }

    fn analytics_heatmap(snapshot: &KnowledgeAnalytics) -> Text<'static> {
        if snapshot.daily.is_empty() {
            return Text::from(vec![Line::from(
                "No analytics recorded yet. Generate a learning session to get started.",
            )]);
        }

        let start_date = snapshot
            .daily
            .first()
            .map(|day| day.date)
            .unwrap_or_else(|| Utc::now().date_naive() - Duration::days(29));
        let weeks = cmp::max((snapshot.daily.len() + 6) / 7, 1);
        let mut grid: Vec<Vec<Option<&DailyAnalytics>>> = vec![vec![None; weeks]; 7];

        for day in &snapshot.daily {
            let delta = (day.date - start_date).num_days();
            if delta < 0 {
                continue;
            }
            let column = cmp::min((delta / 7) as usize, weeks - 1);
            let row = day.date.weekday().num_days_from_monday() as usize;
            grid[row][column] = Some(day);
        }

        let max_correct = snapshot
            .daily
            .iter()
            .map(|day| day.first_try_correct)
            .max()
            .unwrap_or(0);

        let mut lines: Vec<Line> = Vec::new();
        let cell_width = 3usize;
        let mut header_spans = Vec::new();
        header_spans.push(Span::styled(
            "    ",
            Style::default()
                .fg(Color::Rgb(140, 160, 220))
                .add_modifier(Modifier::DIM),
        ));
        for col in 0..weeks {
            header_spans.push(Span::styled(
                format!("{:^width$}", format!("W{}", col + 1), width = cell_width),
                Style::default()
                    .fg(Color::Rgb(140, 160, 220))
                    .add_modifier(Modifier::DIM),
            ));
        }
        lines.push(Line::from(header_spans));

        let day_labels = [
            (Weekday::Mon, "Mon"),
            (Weekday::Tue, "Tue"),
            (Weekday::Wed, "Wed"),
            (Weekday::Thu, "Thu"),
            (Weekday::Fri, "Fri"),
            (Weekday::Sat, "Sat"),
            (Weekday::Sun, "Sun"),
        ];

        for (weekday, label) in day_labels {
            let row_index = weekday.num_days_from_monday() as usize;
            let mut spans = Vec::new();
            spans.push(Span::styled(
                format!("{label:>3} "),
                Style::default()
                    .fg(Color::Rgb(140, 160, 220))
                    .add_modifier(Modifier::DIM),
            ));

            for col in 0..weeks {
                if let Some(day) = grid[row_index][col] {
                    let color = Self::heatmap_color(day.first_try_correct, max_correct);
                    let style = Style::default()
                        .fg(color)
                        .bg(Color::Rgb(22, 24, 46))
                        .add_modifier(Modifier::BOLD);
                    let glyph = if day.total_questions == 0 && day.total_attempts == 0 {
                        "·"
                    } else if day.first_try_correct == 0 {
                        "∙"
                    } else {
                        "●"
                    };
                    spans.push(Span::styled(
                        format!("{:^width$}", glyph, width = cell_width),
                        style,
                    ));
                } else {
                    spans.push(Span::styled(
                        format!("{:^width$}", "∙", width = cell_width),
                        Style::default()
                            .fg(Color::Rgb(60, 70, 110))
                            .bg(Color::Rgb(18, 20, 34))
                            .add_modifier(Modifier::DIM),
                    ));
                }
            }

            lines.push(Line::from(spans));
        }

        if max_correct == 0 {
            lines.push(Line::from(vec![Span::styled(
                "No first-try correct answers recorded yet.",
                Style::default().fg(Color::Rgb(140, 160, 220)),
            )]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(
                    "Legend: ",
                    Style::default()
                        .fg(Color::Rgb(189, 255, 154))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "●",
                    Style::default()
                        .fg(Self::heatmap_color(max_correct, max_correct))
                        .bg(Color::Rgb(22, 24, 46)),
                ),
                Span::raw(" higher correctness  "),
                Span::styled(
                    "●",
                    Style::default()
                        .fg(Self::heatmap_color(1, max_correct))
                        .bg(Color::Rgb(22, 24, 46)),
                ),
                Span::raw(" lower correctness"),
            ]));
        }

        Text::from(lines)
    }

    fn heatmap_color(value: u32, max_value: u32) -> Color {
        if max_value == 0 || value == 0 {
            return Color::Rgb(90, 110, 150);
        }
        let ratio = value as f32 / max_value as f32;
        if ratio < 0.25 {
            Color::Rgb(137, 196, 125)
        } else if ratio < 0.5 {
            Color::Rgb(154, 222, 138)
        } else if ratio < 0.75 {
            Color::Rgb(184, 247, 153)
        } else {
            Color::Rgb(231, 252, 173)
        }
    }

    fn analytics_summary_lines(snapshot: &KnowledgeAnalytics, app: &App) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        lines.push(Self::metric_line(
            "Total quiz questions",
            snapshot.total_questions,
            Color::Rgb(189, 255, 154),
        ));

        let accuracy = if snapshot.total_attempts > 0 {
            (snapshot.total_first_try_correct as f64 / snapshot.total_attempts as f64) * 100.0
        } else {
            0.0
        };
        lines.push(Self::ratio_line(
            "First-try correct",
            snapshot.total_first_try_correct,
            snapshot.total_attempts,
            accuracy,
        ));

        let active_days = snapshot
            .daily
            .iter()
            .filter(|day| day.total_questions > 0 || day.total_attempts > 0)
            .count();
        lines.push(Self::metric_line(
            "Active study days",
            active_days as u32,
            Color::Rgb(180, 205, 255),
        ));

        let total_groups = snapshot
            .daily
            .last()
            .map(|day| day.cumulative_groups)
            .unwrap_or(snapshot.knowledge_groups.len() as u32);
        lines.push(Self::metric_line(
            "Total knowledge groups",
            total_groups,
            Color::Rgb(189, 255, 154),
        ));

        lines.extend(Self::group_bar_lines(snapshot));

        if let Some(refreshed) = app.analytics_refreshed_at.as_ref() {
            lines.push(Line::from(vec![
                Span::styled(
                    "Refreshed: ",
                    Style::default()
                        .fg(Color::Rgb(140, 160, 220))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    refreshed.clone(),
                    Style::default().fg(Color::Rgb(189, 255, 154)),
                ),
            ]));
        }

        if let Some(error) = app.analytics_error.as_ref() {
            lines.push(Line::from(vec![
                Span::styled(
                    "Warning: ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(error.clone(), Style::default().fg(Color::Red)),
            ]));
        }

        lines
    }

    fn metric_line(label: &str, value: u32, color: Color) -> Line<'static> {
        let bold = Style::default().fg(color).add_modifier(Modifier::BOLD);
        Line::from(vec![
            Span::styled(format!("{label}: "), bold),
            Span::styled(value.to_string(), Style::default().fg(color)),
        ])
    }

    fn ratio_line(label: &str, numerator: u32, denominator: u32, percentage: f64) -> Line<'static> {
        let bar = Self::ratio_bar(numerator, denominator, 12);
        Line::from(vec![
            Span::styled(
                format!("{label}: "),
                Style::default()
                    .fg(Color::Rgb(189, 255, 154))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} of {} ({:.1}%) ", numerator, denominator, percentage),
                Style::default().fg(Color::Rgb(180, 205, 255)),
            ),
            Span::styled(bar, Style::default().fg(Color::Rgb(189, 255, 154))),
        ])
    }

    fn ratio_bar(value: u32, max: u32, width: usize) -> String {
        if max == 0 {
            return "∙".repeat(width);
        }
        let filled = ((value as f64 / max as f64) * width as f64).round() as usize;
        let filled = filled.min(width);
        format!("{}{}", "█".repeat(filled), "·".repeat(width - filled))
    }

    fn group_bar_lines(snapshot: &KnowledgeAnalytics) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let weeks = (snapshot.daily.len() + 6) / 7;
        if weeks == 0 {
            lines.push(Line::from(vec![Span::styled(
                "No knowledge group activity yet.",
                Style::default().fg(Color::Rgb(140, 160, 220)),
            )]));
            return lines;
        }

        let mut weekly_totals: Vec<u32> = Vec::with_capacity(weeks);
        for week_index in 0..weeks {
            let end = ((week_index + 1) * 7).min(snapshot.daily.len());
            if end == 0 {
                weekly_totals.push(0);
                continue;
            }
            let value = snapshot.daily[end - 1].cumulative_groups;
            weekly_totals.push(value);
        }

        let max = weekly_totals.iter().copied().max().unwrap_or(0);
        if max == 0 {
            lines.push(Line::from(vec![Span::styled(
                "Knowledge groups have not been recorded yet.",
                Style::default().fg(Color::Rgb(140, 160, 220)),
            )]));
            return lines;
        }

        lines.push(Line::from(vec![Span::styled(
            "Knowledge groups growth:",
            Style::default()
                .fg(Color::Rgb(189, 255, 154))
                .add_modifier(Modifier::BOLD),
        )]));

        for (index, value) in weekly_totals.iter().enumerate() {
            let bar = Self::ratio_bar(*value, max, 16);
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" W{:>2}: ", index + 1),
                    Style::default().fg(Color::Rgb(140, 160, 220)),
                ),
                Span::styled(bar, Style::default().fg(Color::Rgb(189, 255, 154))),
                Span::styled(
                    format!(" {:>3}", value),
                    Style::default().fg(Color::Rgb(180, 205, 255)),
                ),
            ]));
        }

        lines
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
