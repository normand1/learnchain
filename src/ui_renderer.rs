use crate::view_managers::menu_manager::MENU_OPTIONS;
use crate::{
    AI_LOADING_FRAMES, App, AppView, SessionSelectionTarget, config,
    knowledge_store::{DailyAnalytics, KnowledgeAnalytics},
    markdown_rules::MarkdownRules,
    reset_learning_feedback,
    session_sources::{Session, SessionEvent},
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
            AppView::SessionPicker => self.render_session_picker(frame),
            AppView::Learning => self.render_learning(frame),
            AppView::DeepDive => self.render_deep_dive(frame),
            AppView::Library => self.render_library(frame),
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
                Constraint::Length(7),
                Constraint::Length(3),
                Constraint::Length(10),
                Constraint::Length(4),
            ])
            .split(frame.area());

        let ascii_art = vec![
            Line::from(vec![
                Span::styled(
                    "    ______ ______       ",
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    " ___                          ",
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    "_____ _           _       ",
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "   _/      Y      \\_     ",
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    "| |    ___  __ _ _ __ _ __   ",
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    "/ ___| |__   __ _(_)_ __  ",
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "  // ~~ ~~ | ~~ ~  \\\\    ",
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    "| |   / _ \\/ _` | '__| '_ \\ ",
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    "| |   | '_ \\ / _` | | '_ \\ ",
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    " // ~~ ~ ~ | ~~~ ~~ \\\\   ",
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    "| |__|  __/ (_| | |  | | | |",
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    "| |___| | | | (_| | | | | |",
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "//________.|.________\\\\  ",
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    "|_____\\___|\\__,_|_|  |_| |_|",
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    " \\____|_| |_|\\__,_|_|_| |_|",
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![Span::styled(
                "`---------`-'---------'                                                     ",
                Style::default().fg(Color::Yellow),
            )]),
        ];

        frame.render_widget(Paragraph::new(ascii_art), layout[0]);

        frame.render_widget(
            Paragraph::new(Self::header_text(app))
                .block(Block::bordered().title(header_title))
                .centered(),
            layout[1],
        );

        let menu_sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(6), Constraint::Length(4)])
            .split(layout[2]);

        let actions_items: Vec<ListItem> = MENU_OPTIONS[..4]
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

        let config_items: Vec<ListItem> = MENU_OPTIONS[4..]
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
        status_lines.push("Press 1-6 for quick selection. Esc, Ctrl-C, or q to quit.".to_string());
        if app.learning_response.is_some() {
            status_lines.push("Press l to revisit the latest learning response.".to_string());
        }
        status_lines.push("Press c to configure details.".to_string());

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[3],
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

        if app.viewing_sessions_list {
            self.render_sessions_list(frame);
        } else {
            self.render_session_events(frame);
        }
    }

    fn render_sessions_list(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;
        let header_title = Line::from("All Sessions").bold().blue().centered();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(4),
            ])
            .split(frame.area());

        frame.render_widget(
            Paragraph::new(format!("{} sessions loaded", app.sessions.len()))
                .block(Block::bordered().title(header_title))
                .centered(),
            layout[0],
        );

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);

        let list_items: Vec<ListItem> = if app.sessions.is_empty() {
            vec![ListItem::new("No sessions found.")]
        } else {
            app.sessions
                .iter()
                .map(|session| {
                    let truncated_summary = truncate_string(&session.summary, 40);
                    ListItem::new(format!(
                        "{} | {} | {} events",
                        session.date,
                        truncated_summary,
                        session.events.len()
                    ))
                })
                .collect()
        };

        let mut list_state = ListState::default();
        list_state.select(app.selected_session);

        frame.render_stateful_widget(
            List::new(list_items)
                .block(Block::bordered().title(Line::from("Sessions")))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            body[0],
            &mut list_state,
        );

        let detail_text = match app.selected_session.and_then(|idx| app.sessions.get(idx)) {
            Some(session) => {
                let mut details = vec![
                    format!("Session ID: {}", session.id),
                    format!("Date: {}", session.date),
                    format!("Events: {}", session.events.len()),
                ];

                // Extract branch and cwd from first event if available
                if let Some(first_event) = session.events.first() {
                    for text in &first_event.content_texts {
                        if text.starts_with("branch: ") {
                            details.push(text.clone());
                        }
                        if text.starts_with("cwd: ") {
                            details.push(text.clone());
                        }
                    }
                }

                details.push(String::new());
                details.push(format!("Source: {}", session.source_file.display()));

                // Show full user prompt if available
                if let Some(ref prompt) = session.first_user_prompt {
                    details.push(String::new());
                    details.push("─── First User Prompt ───".to_string());
                    details.push(prompt.clone());
                }

                details.join("\n")
            }
            None => "Select a session to view its details.".to_string(),
        };

        frame.render_widget(
            Paragraph::new(detail_text)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(Line::from("Session Details"))),
            body[1],
        );

        let mut status_lines = Vec::new();
        if let Some(error) = &app.error {
            status_lines.push(format!("Error: {}", error));
        }
        status_lines.push(format!("Total sessions: {}", app.sessions.len()));
        status_lines
            .push("Use ↑/↓ or j/k to navigate. Press Enter to view session events.".to_string());
        status_lines.push("Press Backspace or m for menu. Esc, Ctrl-C, or q to quit.".to_string());

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn render_session_picker(&mut self, frame: &mut Frame) {
        if self.app.session_picker_viewing_projects {
            self.render_session_picker_projects(frame);
        } else {
            self.render_session_picker_sessions(frame);
        }
    }

    fn render_session_picker_projects(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;
        let title = match app.session_selection_target {
            Some(SessionSelectionTarget::DeepDive) => "Select Project for Deep Dive",
            _ => "Select Project for Quiz",
        };
        let header_title = Line::from(title).bold().blue().centered();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(4),
            ])
            .split(frame.area());

        frame.render_widget(
            Paragraph::new(format!(
                "{} projects • {} sessions",
                app.projects.len(),
                app.sessions.len()
            ))
            .block(Block::bordered().title(header_title))
            .centered(),
            layout[0],
        );

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);

        let list_items: Vec<ListItem> = if app.projects.is_empty() {
            vec![ListItem::new("No projects found.")]
        } else {
            app.projects
                .iter()
                .map(|project| {
                    ListItem::new(format!(
                        "{} ({} sessions)",
                        project.name,
                        project.session_indices.len()
                    ))
                })
                .collect()
        };

        let mut list_state = ListState::default();
        list_state.select(app.session_picker_selected_project);

        frame.render_stateful_widget(
            List::new(list_items)
                .block(Block::bordered().title(Line::from("Projects")))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            body[0],
            &mut list_state,
        );

        let detail_text = match app
            .session_picker_selected_project
            .and_then(|idx| app.projects.get(idx))
        {
            Some(project) => {
                // Calculate total tokens for all sessions in project
                let total_tokens: usize = project
                    .session_indices
                    .iter()
                    .filter_map(|&idx| app.sessions.get(idx))
                    .map(estimate_session_tokens)
                    .sum();

                let mut details = vec![
                    format!("Project: {}", project.name),
                    format!("Path: {}", project.cwd),
                    format!("Sessions: {}", project.session_indices.len()),
                    format!("Total tokens: {}", format_tokens(total_tokens)),
                ];

                // Show recent session dates with token counts
                if !project.session_indices.is_empty() {
                    details.push(String::new());
                    details.push("─── Recent Sessions ───".to_string());
                    for &idx in project.session_indices.iter().take(5) {
                        if let Some(session) = app.sessions.get(idx) {
                            let summary = truncate_string(&session.summary, 30);
                            let tokens = estimate_session_tokens(session);
                            details.push(format!(
                                "  {} • {} ({})",
                                session.date,
                                summary,
                                format_tokens(tokens)
                            ));
                        }
                    }
                    if project.session_indices.len() > 5 {
                        details.push(format!(
                            "  ... and {} more",
                            project.session_indices.len() - 5
                        ));
                    }
                }

                details.join("\n")
            }
            None => "Select a project to view its sessions.".to_string(),
        };

        frame.render_widget(
            Paragraph::new(detail_text)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(Line::from("Project Details"))),
            body[1],
        );

        let mut status_lines = Vec::new();
        if let Some(error) = &app.error {
            status_lines.push(format!("Error: {}", error));
        }
        status_lines.push(format!(
            "{} projects • {} total sessions",
            app.projects.len(),
            app.sessions.len()
        ));
        status_lines.push(match app.session_selection_target {
            Some(SessionSelectionTarget::DeepDive) => {
                "Use ↑/↓ or j/k to navigate. Press Enter to view sessions. Press h for history."
                    .to_string()
            }
            _ => "Use ↑/↓ or j/k to navigate. Press Enter to view sessions.".to_string(),
        });
        status_lines.push("Press Backspace or m for menu. Esc, Ctrl-C, or q to quit.".to_string());

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn render_session_picker_sessions(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;

        let project_name = app
            .session_picker_selected_project
            .and_then(|idx| app.projects.get(idx))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let header_title = Line::from(format!("Sessions in {}", project_name))
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

        let session_count = app
            .session_picker_selected_project
            .and_then(|idx| app.projects.get(idx))
            .map(|p| p.session_indices.len())
            .unwrap_or(0);

        frame.render_widget(
            Paragraph::new(format!("{} sessions in project", session_count))
                .block(Block::bordered().title(header_title))
                .centered(),
            layout[0],
        );

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);

        // Get sessions for the current project
        let project_sessions: Vec<&crate::session_sources::Session> = app
            .session_picker_selected_project
            .and_then(|idx| app.projects.get(idx))
            .map(|project| {
                project
                    .session_indices
                    .iter()
                    .filter_map(|&idx| app.sessions.get(idx))
                    .collect()
            })
            .unwrap_or_default();

        let list_items: Vec<ListItem> = if project_sessions.is_empty() {
            vec![ListItem::new("No sessions found.")]
        } else {
            project_sessions
                .iter()
                .map(|session| {
                    let truncated_summary = truncate_string(&session.summary, 40);
                    ListItem::new(format!(
                        "{} | {} | {} events",
                        session.date,
                        truncated_summary,
                        session.events.len()
                    ))
                })
                .collect()
        };

        let mut list_state = ListState::default();
        list_state.select(app.session_picker_selected_session);

        frame.render_stateful_widget(
            List::new(list_items)
                .block(Block::bordered().title(Line::from("Sessions")))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            body[0],
            &mut list_state,
        );

        let detail_text = match app
            .session_picker_selected_session
            .and_then(|idx| project_sessions.get(idx))
        {
            Some(session) => {
                let mut details = vec![
                    format!("Session ID: {}", session.id),
                    format!("Date: {}", session.date),
                    format!("Events: {}", session.events.len()),
                ];

                // Token estimation
                let total_tokens = estimate_session_tokens(session);
                let (sampled_tokens, is_sampled) = estimate_sampled_tokens(session);
                details.push(format!("Tokens: {}", format_tokens(total_tokens)));
                if is_sampled {
                    details.push(format!("After sampling: {}", format_tokens(sampled_tokens)));
                }

                // Extract branch and cwd from first event if available
                if let Some(first_event) = session.events.first() {
                    for text in &first_event.content_texts {
                        if text.starts_with("branch: ") {
                            details.push(text.clone());
                        }
                        if text.starts_with("cwd: ") {
                            details.push(text.clone());
                        }
                    }
                }

                details.push(String::new());
                details.push(format!("Source: {}", session.source_file.display()));

                // Show full user prompt if available
                if let Some(ref prompt) = session.first_user_prompt {
                    details.push(String::new());
                    details.push("─── First User Prompt ───".to_string());
                    details.push(prompt.clone());
                }

                details.join("\n")
            }
            None => match app.session_selection_target {
                Some(SessionSelectionTarget::DeepDive) => {
                    "Select a session to generate a deep dive from.".to_string()
                }
                _ => "Select a session to generate a quiz from.".to_string(),
            },
        };

        frame.render_widget(
            Paragraph::new(detail_text)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(Line::from("Session Details"))),
            body[1],
        );

        let mut status_lines = Vec::new();
        if let Some(error) = &app.error {
            status_lines.push(format!("Error: {}", error));
        }
        status_lines.push(format!("{} sessions in this project", session_count));
        status_lines.push(match app.session_selection_target {
            Some(SessionSelectionTarget::DeepDive) => {
                "Use ↑/↓ or j/k to navigate. Press Enter to generate deep dive. Press h for history."
                    .to_string()
            }
            _ => "Use ↑/↓ or j/k to navigate. Press Enter to generate quiz.".to_string(),
        });
        status_lines.push(
            "Press Backspace for projects, m for menu. Esc, Ctrl-C, or q to quit.".to_string(),
        );

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn render_session_events(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;

        let session_info = app
            .selected_session
            .and_then(|idx| app.sessions.get(idx))
            .map(|s| format!("Session: {} • {}", s.date, s.id))
            .unwrap_or_else(|| "Session Events".to_string());

        let header_title = Line::from(session_info).bold().blue().centered();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(4),
            ])
            .split(frame.area());

        frame.render_widget(
            Paragraph::new(format!("{} events in this session", app.events.len()))
                .block(Block::bordered().title(header_title))
                .centered(),
            layout[0],
        );

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(layout[1]);

        let list_items: Vec<ListItem> = if app.events.is_empty() {
            vec![ListItem::new("No events in this session.")]
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
        status_lines.push(format!("Events in session: {}", app.events.len()));
        status_lines.push(
            "Use ↑/↓ or j/k to navigate. Press Backspace to return to sessions list.".to_string(),
        );
        status_lines.push("Press m for menu. Esc, Ctrl-C, or q to quit.".to_string());

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn render_learning(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;

        // Show summary screen if quiz is complete
        if app.learning_showing_summary {
            self.render_quiz_summary(frame);
            return;
        }

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
            let progress_bar = Self::render_progress_bar(app.ai_progress_percent, 30);
            let elapsed = app
                .ai_loading_start
                .map(|start| {
                    let secs = start.elapsed().as_secs();
                    format!(" ({}s)", secs)
                })
                .unwrap_or_default();
            let stage_message = if app.ai_progress_message.is_empty() {
                "Initializing..."
            } else {
                &app.ai_progress_message
            };
            question_text = format!(
                "{} Generating learning response…\n\n{} {}%\n{}{}\n\nWe'll show the quiz once the AI reply is ready.",
                frame_symbol, progress_bar, app.ai_progress_percent, stage_message, elapsed
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
                        "Knowledge group {}/{}\nName: {}{}\n\nNo quiz questions were provided for this topic.",
                        group_index + 1,
                        group_count,
                        group.knowledge_type_group,
                        language_line
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
                        if let Some(feedback) = app.learning_feedback.as_deref() {
                            segments.push(format!("Result: {}", feedback));
                        }
                        segments.push(String::from("Press any key to continue."));
                        question_text = segments.join("\n\n");
                    } else {
                        question_text = format!(
                            "Knowledge group {}/{}\nName: {}{}\n\nQuestion {}/{}:\n{}\n\nOptions:\n{}{}",
                            group_index + 1,
                            group_count,
                            group.knowledge_type_group,
                            language_line,
                            quiz_index + 1,
                            quiz_count,
                            question.question,
                            options_text,
                            feedback_line
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

    fn render_deep_dive(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;

        if app.ai_loading {
            let frame_symbol = AI_LOADING_FRAMES[app.ai_loading_frame % AI_LOADING_FRAMES.len()];
            let progress_bar = Self::render_progress_bar(app.ai_progress_percent, 30);
            let elapsed = app
                .ai_loading_start
                .map(|start| format!(" ({}s)", start.elapsed().as_secs()))
                .unwrap_or_default();
            let stage_message = if app.ai_progress_message.is_empty() {
                "Initializing..."
            } else {
                &app.ai_progress_message
            };
            let text = format!(
                "{} Generating session deep dive…\n\n{} {}%\n{}{}\n\nWe'll show the markdown once the deep dive is ready.",
                frame_symbol, progress_bar, app.ai_progress_percent, stage_message, elapsed
            );
            frame.render_widget(
                Paragraph::new(text)
                    .wrap(Wrap { trim: false })
                    .block(Block::bordered().title(Line::from("Session Deep Dive"))),
                frame.area(),
            );
            return;
        }

        if app.deep_dive_showing_history {
            self.render_deep_dive_history(frame);
            return;
        }

        let document = app.active_deep_dive_document();
        let title = document
            .map(|doc| doc.metadata.title.clone())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "Session Deep Dive".to_string());
        let header_title = Line::from(title).bold().blue().centered();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(10),
                Constraint::Length(5),
            ])
            .split(frame.area());

        let header_text = match document {
            Some(doc) => format!(
                "Saved file: {}\nSession: {} • {}",
                doc.path.display(),
                doc.metadata.session_date,
                doc.metadata.session_id
            ),
            None => "No deep dive loaded. Generate one from the menu or press h to open history."
                .to_string(),
        };

        frame.render_widget(
            Paragraph::new(header_text)
                .block(Block::bordered().title(header_title))
                .wrap(Wrap { trim: false }),
            layout[0],
        );

        let markdown = document
            .map(|doc| strip_leading_toml_front_matter(&doc.markdown).to_string())
            .unwrap_or_else(|| "No deep dive loaded.".to_string());
        frame.render_widget(
            Paragraph::new(markdown)
                .wrap(Wrap { trim: false })
                .scroll((app.deep_dive_scroll, 0))
                .block(Block::bordered().title(Line::from("Markdown"))),
            layout[1],
        );

        let mut status_lines = Vec::new();
        if let Some(error) = &app.error {
            status_lines.push(format!("Error: {}", error));
        }
        if let Some(status) = &app.ai_status {
            status_lines.push(format!("AI: {}", status));
        }
        status_lines.push(
            "Use j/k or PgUp/PgDn to scroll. Press h for history. Backspace returns from a history document."
                .to_string(),
        );
        status_lines.push("Press m to return to the menu.".to_string());

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn render_deep_dive_history(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(5),
            ])
            .split(frame.area());

        frame.render_widget(
            Paragraph::new(format!("{} saved deep dives", app.deep_dive_history.len()))
                .block(Block::bordered().title(Line::from("Deep Dive History").bold().blue()))
                .centered(),
            layout[0],
        );

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);

        let items: Vec<ListItem> = if app.deep_dive_history.is_empty() {
            vec![ListItem::new("No deep dives have been saved yet.")]
        } else {
            app.deep_dive_history
                .iter()
                .map(|entry| {
                    let title = if entry.metadata.title.trim().is_empty() {
                        "Untitled Deep Dive"
                    } else {
                        &entry.metadata.title
                    };
                    let project = if entry.metadata.project_name.trim().is_empty() {
                        "Unknown project"
                    } else {
                        &entry.metadata.project_name
                    };
                    ListItem::new(format!(
                        "{} | {} | {}",
                        entry.metadata.generated_at,
                        project,
                        truncate_string(title, 36)
                    ))
                })
                .collect()
        };

        let mut list_state = ListState::default();
        list_state.select(app.deep_dive_history_selected);
        frame.render_stateful_widget(
            List::new(items)
                .block(Block::bordered().title(Line::from("Artifacts")))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            body[0],
            &mut list_state,
        );

        let detail_text = match app
            .deep_dive_history_selected
            .and_then(|index| app.deep_dive_history.get(index))
        {
            Some(entry) => format!(
                "Title: {}\nGenerated: {}\nProject: {}\nPath: {}\nReferenced URLs: {}\nReviewed URLs: {}",
                entry.metadata.title,
                entry.metadata.generated_at,
                entry.metadata.project_name,
                entry.path.display(),
                entry.metadata.referenced_url_count,
                entry.metadata.reviewed_url_count
            ),
            None => "Select a saved deep dive to view it.".to_string(),
        };

        frame.render_widget(
            Paragraph::new(detail_text)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(Line::from("Details"))),
            body[1],
        );

        let mut status_lines = Vec::new();
        if let Some(error) = &app.error {
            status_lines.push(format!("Error: {}", error));
        }
        status_lines.push(
            "Use ↑/↓ or j/k to navigate. Press Enter to open a deep dive. Press r to refresh history."
                .to_string(),
        );
        status_lines.push(
            "Press Backspace to return. Press m for menu. Esc, Ctrl-C, or q to quit.".to_string(),
        );

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn render_library(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(5),
            ])
            .split(frame.area());

        frame.render_widget(
            Paragraph::new(format!("{} saved artifacts", app.library_artifacts.len()))
                .block(Block::bordered().title(Line::from("Library").bold().blue()))
                .centered(),
            layout[0],
        );

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[1]);

        let items: Vec<ListItem> = if app.library_artifacts.is_empty() {
            vec![ListItem::new("No saved deep dives or quizzes found.")]
        } else {
            app.library_artifacts
                .iter()
                .map(|entry| match entry {
                    crate::output_manager::LibraryArtifactEntry::DeepDive(entry) => {
                        let title = if entry.metadata.title.trim().is_empty() {
                            "Untitled Deep Dive"
                        } else {
                            &entry.metadata.title
                        };
                        ListItem::new(format!(
                            "{} | {} | {}",
                            crate::output_manager::LibraryArtifactKind::DeepDive.label(),
                            entry
                                .metadata
                                .generated_at
                                .split('T')
                                .next()
                                .unwrap_or(&entry.metadata.generated_at),
                            truncate_string(title, 40)
                        ))
                    }
                    crate::output_manager::LibraryArtifactEntry::Quiz(entry) => {
                        let session_date = if entry.session_date.trim().is_empty() {
                            "<unknown date>"
                        } else {
                            &entry.session_date
                        };
                        ListItem::new(format!(
                            "{} | {} | {} questions",
                            crate::output_manager::LibraryArtifactKind::Quiz.label(),
                            session_date,
                            entry.question_count
                        ))
                    }
                })
                .collect()
        };

        let mut list_state = ListState::default();
        list_state.select(app.library_selected);
        frame.render_stateful_widget(
            List::new(items)
                .block(Block::bordered().title(Line::from("Artifacts")))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            body[0],
            &mut list_state,
        );

        let detail_text = match app
            .library_selected
            .and_then(|index| app.library_artifacts.get(index))
        {
            Some(crate::output_manager::LibraryArtifactEntry::DeepDive(entry)) => format!(
                "Type: {}\nTitle: {}\nGenerated: {}\nProject: {}\nPath: {}",
                crate::output_manager::LibraryArtifactKind::DeepDive.label(),
                entry.metadata.title,
                entry.metadata.generated_at,
                if entry.metadata.project_name.trim().is_empty() {
                    "Unknown project"
                } else {
                    &entry.metadata.project_name
                },
                entry.path.display()
            ),
            Some(crate::output_manager::LibraryArtifactEntry::Quiz(entry)) => format!(
                "Type: {}\nSession date: {}\nKnowledge groups: {}\nQuestions: {}\nPath: {}",
                crate::output_manager::LibraryArtifactKind::Quiz.label(),
                if entry.session_date.trim().is_empty() {
                    "<unknown>".to_string()
                } else {
                    entry.session_date.clone()
                },
                entry.knowledge_group_count,
                entry.question_count,
                entry.path.display()
            ),
            None => "Select a saved deep dive or quiz to open it.".to_string(),
        };

        frame.render_widget(
            Paragraph::new(detail_text)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(Line::from("Details"))),
            body[1],
        );

        let mut status_lines = Vec::new();
        if let Some(error) = &app.error {
            status_lines.push(format!("Error: {}", error));
        }
        if let Some(status) = &app.ai_status {
            status_lines.push(format!("Status: {}", status));
        }
        let repository = config::current().document_repository;
        status_lines.push(
            "Use ↑/↓ or j/k to navigate. Press Enter to open. Press r to refresh.".to_string(),
        );
        if repository != config::DocumentRepositoryKind::None {
            status_lines.push(format!(
                "Press e to send the selected document to {}.",
                repository.label()
            ));
        } else {
            status_lines.push(
                "Configure a document repository in Config to enable export from the library."
                    .to_string(),
            );
        }
        status_lines.push(
            "Press Backspace or m to return to the menu. Esc, Ctrl-C, or q to quit.".to_string(),
        );

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    fn render_quiz_summary(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;

        let header_title = Line::from("Quiz Complete!").bold().green().centered();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Min(10),
                Constraint::Length(3),
            ])
            .split(frame.area());

        // Calculate summary statistics
        let total_questions = app.quiz_summary_results.len();
        let correct_first_try = app
            .quiz_summary_results
            .iter()
            .filter(|r| r.first_try_correct)
            .count();
        let accuracy = if total_questions > 0 {
            (correct_first_try as f64 / total_questions as f64) * 100.0
        } else {
            0.0
        };

        let summary_text = format!(
            "You completed all {} questions!\n\nFirst-try accuracy: {} of {} ({:.1}%)",
            total_questions, correct_first_try, total_questions, accuracy
        );

        frame.render_widget(
            Paragraph::new(summary_text)
                .style(Style::default().fg(Color::Rgb(189, 255, 154)))
                .block(
                    Block::bordered()
                        .title(header_title)
                        .border_style(Style::default().fg(Color::Rgb(120, 140, 220))),
                )
                .centered(),
            layout[0],
        );

        // Build the question results list
        let mut lines: Vec<Line> = Vec::new();
        for (index, result) in app.quiz_summary_results.iter().enumerate() {
            let status_symbol = if result.first_try_correct {
                Span::styled(
                    "✓",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    "✗",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )
            };

            // Question line with status
            lines.push(Line::from(vec![
                Span::styled(
                    format!("Q{}: ", index + 1),
                    Style::default()
                        .fg(Color::Rgb(180, 205, 255))
                        .add_modifier(Modifier::BOLD),
                ),
                status_symbol,
                Span::raw(" "),
                Span::styled(
                    truncate_string(&result.question, 60),
                    Style::default().fg(Color::White),
                ),
            ]));

            // Correct answer line
            lines.push(Line::from(vec![
                Span::styled(
                    "   Answer: ",
                    Style::default().fg(Color::Rgb(140, 160, 220)),
                ),
                Span::styled(
                    truncate_string(&result.correct_answer, 55),
                    Style::default().fg(Color::Rgb(189, 255, 154)),
                ),
            ]));

            // Add spacing between questions
            if index < app.quiz_summary_results.len() - 1 {
                lines.push(Line::from(""));
            }
        }

        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .block(
                    Block::bordered()
                        .title(Line::from("Question Results"))
                        .border_style(Style::default().fg(Color::Rgb(120, 140, 220))),
                )
                .wrap(Wrap { trim: false }),
            layout[1],
        );

        frame.render_widget(
            Paragraph::new("Press any key to return to the main menu.")
                .style(Style::default().fg(Color::Rgb(180, 205, 255)))
                .block(
                    Block::bordered()
                        .title(Line::from("Navigation"))
                        .border_style(Style::default().fg(Color::Rgb(120, 140, 220))),
                )
                .centered(),
            layout[2],
        );
    }

    fn render_config(&mut self, frame: &mut Frame) {
        let app = &mut *self.app;
        let active_section = app.config_form.selected_section();
        let current_field = app.config_form.current_field();
        let fields_in_section = app.config_form.visible_fields_in_section(active_section);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(12),
                Constraint::Length(6),
            ])
            .split(frame.area());

        let header_status = if app.config_form.is_editing_text_field() {
            format!(
                "Editing now: {}",
                Self::config_field_title(app, current_field)
            )
        } else {
            format!(
                "Focused setting: {}",
                Self::config_field_title(app, current_field)
            )
        };
        frame.render_widget(
            Paragraph::new(format!(
                "{}\n{}",
                header_status,
                active_section.description()
            ))
            .style(Style::default().fg(Color::Rgb(210, 225, 255)))
            .block(
                Block::bordered().title(Line::from(format!("Config • {}", active_section.title()))),
            ),
            layout[0],
        );

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(28),
                Constraint::Min(36),
                Constraint::Length(42),
            ])
            .split(layout[1]);

        let sections = [
            config::ConfigSection::Session,
            config::ConfigSection::DeepDive,
            config::ConfigSection::Export,
            config::ConfigSection::Ai,
        ];
        let section_items = sections
            .iter()
            .map(|section| {
                ListItem::new(Text::from(vec![
                    Line::from(section.title().to_string()),
                    Line::from(Self::config_section_summary(app, *section))
                        .style(Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect::<Vec<_>>();
        let mut section_state = ListState::default();
        section_state.select(Some(Self::config_section_index(active_section)));
        frame.render_stateful_widget(
            List::new(section_items)
                .block(Block::bordered().title(Line::from("Sections")))
                .highlight_symbol("▸ ")
                .highlight_style(
                    Style::default()
                        .fg(Color::Rgb(120, 200, 255))
                        .add_modifier(Modifier::BOLD),
                ),
            columns[0],
            &mut section_state,
        );

        let field_items = fields_in_section
            .iter()
            .map(|field| {
                ListItem::new(Text::from(vec![
                    Line::from(format!(
                        "{}: {}",
                        Self::config_field_title(app, *field),
                        Self::config_field_value(app, *field)
                    )),
                    Line::from(Self::config_field_summary(app, *field))
                        .style(Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect::<Vec<_>>();
        let mut field_state = ListState::default();
        field_state.select(Some(app.config_form.selected_index_in_section()));
        frame.render_stateful_widget(
            List::new(field_items)
                .block(Block::bordered().title(Line::from(active_section.title())))
                .highlight_symbol("▶ ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            columns[1],
            &mut field_state,
        );

        let mut detail_lines = vec![
            Line::from(Self::config_field_summary(app, current_field)),
            Line::from(""),
            Line::from(format!(
                "Current value: {}",
                Self::config_field_value(app, current_field)
            )),
            Line::from(""),
            Line::from("How it changes the app:"),
            Line::from(Self::config_field_effect(app, current_field)),
            Line::from(""),
            Line::from("How to change it:"),
            Line::from(Self::config_field_controls(app, current_field)),
        ];
        if let Some(extra_help) = Self::config_field_extra_help(app, current_field) {
            detail_lines.push(Line::from(""));
            detail_lines.push(Line::from("Watch for:"));
            detail_lines.push(Line::from(extra_help));
        }
        frame.render_widget(
            Paragraph::new(Text::from(detail_lines))
                .block(Block::bordered().title(Line::from(format!(
                    "Focus • {}",
                    Self::config_field_title(app, current_field)
                ))))
                .wrap(Wrap { trim: false }),
            columns[2],
        );

        let status_lines = if let Some(error) = &app.error {
            vec![
                format!("Error: {}", error),
                format!(
                    "Section: {} • {} item(s) in view",
                    active_section.title(),
                    fields_in_section.len()
                ),
                "Use the focused field help on the right to resolve the issue.".to_string(),
            ]
        } else if let Some(config_status) = &app.config_form.status {
            vec![
                config_status.clone(),
                format!(
                    "Section: {} • {} item(s) in view",
                    active_section.title(),
                    fields_in_section.len()
                ),
                if app.config_form.is_editing_text_field() {
                    "Typing affects only the active field. Enter saves. Esc cancels.".to_string()
                } else {
                    "↑↓ choose a field. ←→ changes selectors and numbers. Enter edits text fields."
                        .to_string()
                },
            ]
        } else if app.config_form.dirty {
            vec![
                "Unsaved changes in the current config form.".to_string(),
                format!(
                    "Section: {} • {} item(s) in view",
                    active_section.title(),
                    fields_in_section.len()
                ),
                "s saves • r reverts to disk • m saves and returns to the menu".to_string(),
            ]
        } else {
            vec![
                format!(
                    "Ready. Section: {} • {} item(s) in view",
                    active_section.title(),
                    fields_in_section.len()
                ),
                "↑↓ select within this section • ←→ adjust values • Enter edits text fields"
                    .to_string(),
                "s save • r reset • m save+menu • Esc/q quit".to_string(),
            ]
        };

        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Navigation & Status"))),
            layout[2],
        );
    }

    fn config_section_index(section: config::ConfigSection) -> usize {
        match section {
            config::ConfigSection::Session => 0,
            config::ConfigSection::DeepDive => 1,
            config::ConfigSection::Export => 2,
            config::ConfigSection::Ai => 3,
        }
    }

    fn config_section_summary(app: &App, section: config::ConfigSection) -> String {
        match section {
            config::ConfigSection::Session => format!(
                "Source {} • max {} • sample {}%",
                app.config_form.session_source.label(),
                app.config_form.max_events,
                app.config_form.sampling_percentage
            ),
            config::ConfigSection::DeepDive => format!(
                "{} / {} sections enabled",
                app.config_form.deep_dive_sections.enabled_count(),
                config::DeepDiveSectionsConfig::total_count()
            ),
            config::ConfigSection::Export => {
                let repository = app.config_form.document_repository.label();
                let artifact_mode = if app.config_form.write_output_artifacts {
                    "file output on"
                } else {
                    "file output off"
                };
                format!("Repo {} • {}", repository, artifact_mode)
            }
            config::ConfigSection::Ai => {
                let model = match app.config_form.ai_provider {
                    config::AiProvider::OpenAI => app.config_form.openai_model.label().to_string(),
                    config::AiProvider::Anthropic => {
                        app.config_form.anthropic_model.label().to_string()
                    }
                    config::AiProvider::OpenRouter => {
                        if app.config_form.openrouter_model.trim().is_empty() {
                            "<model not set>".to_string()
                        } else {
                            app.config_form.openrouter_model.clone()
                        }
                    }
                    config::AiProvider::CodexCli => "CLI default".to_string(),
                };
                format!("{} • {}", app.config_form.ai_provider.label(), model)
            }
        }
    }

    fn config_field_title(app: &App, field: config::ConfigField) -> String {
        match field {
            config::ConfigField::MaxEvents => "Max events per summary".to_string(),
            config::ConfigField::MinQuiz => "Minimum quiz questions".to_string(),
            config::ConfigField::SamplingPercentage => "Event sampling percent".to_string(),
            config::ConfigField::SessionSource => "Session source".to_string(),
            config::ConfigField::DeepDiveSessionMetadata => "Include session metadata".to_string(),
            config::ConfigField::DeepDiveGoal => "Include goal".to_string(),
            config::ConfigField::DeepDiveAccomplishments => "Include accomplishments".to_string(),
            config::ConfigField::DeepDiveInterestingLearnings => {
                "Include interesting learnings".to_string()
            }
            config::ConfigField::DeepDiveTeachingNarrative => {
                "Include teaching narrative".to_string()
            }
            config::ConfigField::DeepDiveReviewedExternalSources => {
                "Include reviewed external sources".to_string()
            }
            config::ConfigField::DeepDiveReferencedUrls => "Include referenced URLs".to_string(),
            config::ConfigField::OutputArtifacts => "Write quiz artifacts to output".to_string(),
            config::ConfigField::DocumentRepository => "Document repository".to_string(),
            config::ConfigField::DocumentRepositoryTarget => {
                match app.config_form.document_repository {
                    config::DocumentRepositoryKind::Notion => "Notion destination".to_string(),
                    _ => "Repository target".to_string(),
                }
            }
            config::ConfigField::NotionApiToken => "Notion API token".to_string(),
            config::ConfigField::LearnChainSiteUrl => "LearnChain site URL".to_string(),
            config::ConfigField::LearnChainEmail => "Linked LearnChain account".to_string(),
            config::ConfigField::LearnChainAuthCode => "LearnChain login code".to_string(),
            config::ConfigField::AiProvider => "AI provider".to_string(),
            config::ConfigField::OpenAiModel => "OpenAI model".to_string(),
            config::ConfigField::OpenAiKey => "OpenAI API key".to_string(),
            config::ConfigField::AnthropicModel => "Anthropic model".to_string(),
            config::ConfigField::AnthropicKey => "Anthropic API key".to_string(),
            config::ConfigField::OpenRouterModel => "OpenRouter model".to_string(),
            config::ConfigField::OpenRouterKey => "OpenRouter API key".to_string(),
        }
    }

    fn config_field_value(app: &App, field: config::ConfigField) -> String {
        match field {
            config::ConfigField::MaxEvents => app.config_form.max_events.to_string(),
            config::ConfigField::MinQuiz => app.config_form.min_quiz_questions.to_string(),
            config::ConfigField::SamplingPercentage => {
                format!("{}%", app.config_form.sampling_percentage)
            }
            config::ConfigField::SessionSource => {
                app.config_form.session_source.label().to_string()
            }
            config::ConfigField::DeepDiveSessionMetadata => {
                Self::checkbox_value(app.config_form.deep_dive_sections.session_metadata)
            }
            config::ConfigField::DeepDiveGoal => {
                Self::checkbox_value(app.config_form.deep_dive_sections.goal)
            }
            config::ConfigField::DeepDiveAccomplishments => {
                Self::checkbox_value(app.config_form.deep_dive_sections.accomplishments)
            }
            config::ConfigField::DeepDiveInterestingLearnings => {
                Self::checkbox_value(app.config_form.deep_dive_sections.interesting_learnings)
            }
            config::ConfigField::DeepDiveTeachingNarrative => {
                Self::checkbox_value(app.config_form.deep_dive_sections.teaching_narrative)
            }
            config::ConfigField::DeepDiveReviewedExternalSources => {
                Self::checkbox_value(app.config_form.deep_dive_sections.reviewed_external_sources)
            }
            config::ConfigField::DeepDiveReferencedUrls => {
                Self::checkbox_value(app.config_form.deep_dive_sections.referenced_urls)
            }
            config::ConfigField::OutputArtifacts => {
                if app.config_form.write_output_artifacts {
                    "Enabled".to_string()
                } else {
                    "Disabled".to_string()
                }
            }
            config::ConfigField::DocumentRepository => {
                app.config_form.document_repository.label().to_string()
            }
            config::ConfigField::DocumentRepositoryTarget => {
                if app.config_form.is_editing_document_repository_target() {
                    let value = app.config_form.document_repository_target_buffer();
                    if value.is_empty() {
                        "<editing empty value>".to_string()
                    } else {
                        value.to_string()
                    }
                } else if app.config_form.document_repository_target.is_empty() {
                    "<not set>".to_string()
                } else {
                    app.config_form.document_repository_target.clone()
                }
            }
            config::ConfigField::NotionApiToken => {
                if app.config_form.is_editing_notion_api_token() {
                    app.config_form.masked_notion_api_token_buffer()
                } else {
                    app.config_form.masked_notion_api_token()
                }
            }
            config::ConfigField::LearnChainSiteUrl => {
                if app.config_form.is_editing_learnchain_site_url() {
                    app.config_form.learnchain_site_url_buffer().to_string()
                } else {
                    app.config_form.learnchain_site_url.clone()
                }
            }
            config::ConfigField::LearnChainEmail => {
                if app.config_form.learnchain_email.trim().is_empty() {
                    "<not linked>".to_string()
                } else {
                    app.config_form.learnchain_email.clone()
                }
            }
            config::ConfigField::LearnChainAuthCode => {
                if app.config_form.is_editing_learnchain_auth_code() {
                    let value = app.config_form.learnchain_auth_code_buffer();
                    if value.is_empty() {
                        "<editing empty value>".to_string()
                    } else {
                        value.to_string()
                    }
                } else if app.config_form.learnchain_auth_code.trim().is_empty() {
                    "<paste code from dashboard>".to_string()
                } else {
                    app.config_form.learnchain_auth_code.clone()
                }
            }
            config::ConfigField::AiProvider => app.config_form.ai_provider.label().to_string(),
            config::ConfigField::OpenAiModel => app.config_form.openai_model.label().to_string(),
            config::ConfigField::OpenAiKey => {
                if app.config_form.is_editing_openai_key() {
                    app.config_form.masked_openai_key_buffer()
                } else {
                    app.config_form.masked_openai_key()
                }
            }
            config::ConfigField::AnthropicModel => {
                app.config_form.anthropic_model.label().to_string()
            }
            config::ConfigField::AnthropicKey => {
                if app.config_form.is_editing_anthropic_key() {
                    app.config_form.masked_anthropic_key_buffer()
                } else {
                    app.config_form.masked_anthropic_key()
                }
            }
            config::ConfigField::OpenRouterModel => {
                if app.config_form.is_editing_openrouter_model() {
                    let value = app.config_form.openrouter_model_buffer();
                    if value.is_empty() {
                        "<editing empty value>".to_string()
                    } else {
                        value.to_string()
                    }
                } else if app.config_form.openrouter_model.trim().is_empty() {
                    "<not set>".to_string()
                } else {
                    app.config_form.openrouter_model.clone()
                }
            }
            config::ConfigField::OpenRouterKey => {
                if app.config_form.is_editing_openrouter_key() {
                    app.config_form.masked_openrouter_key_buffer()
                } else {
                    app.config_form.masked_openrouter_key()
                }
            }
        }
    }

    fn config_field_summary(app: &App, field: config::ConfigField) -> String {
        match field {
            config::ConfigField::MaxEvents => {
                "Caps the number of events copied into generated markdown summaries.".to_string()
            }
            config::ConfigField::MinQuiz => {
                "Sets the minimum question count the model must produce.".to_string()
            }
            config::ConfigField::SamplingPercentage => {
                "Keeps long sessions smaller before quiz generation runs.".to_string()
            }
            config::ConfigField::SessionSource => {
                "Switches which coding assistant history LearnChain loads.".to_string()
            }
            config::ConfigField::DeepDiveSessionMetadata => {
                "Shows the session source, date, ids, and working directory in the document."
                    .to_string()
            }
            config::ConfigField::DeepDiveGoal => {
                "Keeps the short goal statement near the top of each deep dive.".to_string()
            }
            config::ConfigField::DeepDiveAccomplishments => {
                "Keeps the completed work summary as a bullet list.".to_string()
            }
            config::ConfigField::DeepDiveInterestingLearnings => {
                "Keeps the notable learnings and surprises section.".to_string()
            }
            config::ConfigField::DeepDiveTeachingNarrative => {
                "Keeps the longer educational walkthrough section.".to_string()
            }
            config::ConfigField::DeepDiveReviewedExternalSources => {
                "Keeps the source review section for fetched external references.".to_string()
            }
            config::ConfigField::DeepDiveReferencedUrls => {
                "Keeps the raw URL inventory found in the session transcript.".to_string()
            }
            config::ConfigField::OutputArtifacts => {
                "Controls whether quiz JSON artifacts are saved under output/.".to_string()
            }
            config::ConfigField::DocumentRepository => {
                "Turns on optional export destinations for items in the Library.".to_string()
            }
            config::ConfigField::DocumentRepositoryTarget => {
                match app.config_form.document_repository {
                    config::DocumentRepositoryKind::Notion => {
                        "Sets the Notion page or database used by Library exports.".to_string()
                    }
                    _ => "Sets the remote destination for exported documents.".to_string(),
                }
            }
            config::ConfigField::NotionApiToken => {
                "Stores the credential used for Notion document export.".to_string()
            }
            config::ConfigField::LearnChainSiteUrl => {
                "Defines which LearnChain server receives uploaded artifacts.".to_string()
            }
            config::ConfigField::LearnChainEmail => {
                "Shows which LearnChain account is currently linked for export.".to_string()
            }
            config::ConfigField::LearnChainAuthCode => {
                "Stores a one-time code generated from the LearnChain dashboard until you save."
                    .to_string()
            }
            config::ConfigField::AiProvider => {
                "Chooses the backend used for quizzes and deep-dive generation.".to_string()
            }
            config::ConfigField::OpenAiModel => {
                "Picks the OpenAI model used after choosing OpenAI.".to_string()
            }
            config::ConfigField::OpenAiKey => {
                "Adds the credential required for OpenAI-backed requests.".to_string()
            }
            config::ConfigField::AnthropicModel => {
                "Picks the Anthropic model used after choosing Anthropic.".to_string()
            }
            config::ConfigField::AnthropicKey => {
                "Adds the credential required for Anthropic-backed requests.".to_string()
            }
            config::ConfigField::OpenRouterModel => {
                "Sets the exact OpenRouter model id LearnChain should call.".to_string()
            }
            config::ConfigField::OpenRouterKey => {
                "Adds the credential required for OpenRouter-backed requests.".to_string()
            }
        }
    }

    fn config_field_effect(app: &App, field: config::ConfigField) -> String {
        match field {
            config::ConfigField::MaxEvents => {
                "Higher values preserve more session context, but they also make summaries larger and downstream prompts heavier.".to_string()
            }
            config::ConfigField::MinQuiz => {
                "This changes the floor for every generated quiz so the model must return at least this many questions.".to_string()
            }
            config::ConfigField::SamplingPercentage => {
                "This trims long sessions before quiz generation so response time and token usage stay under control.".to_string()
            }
            config::ConfigField::SessionSource => {
                "Changing this switches where the session picker and latest-session loader read history from across the app.".to_string()
            }
            config::ConfigField::DeepDiveSessionMetadata => {
                "Disable this if you want cleaner deep dives that skip session bookkeeping details."
                    .to_string()
            }
            config::ConfigField::DeepDiveGoal => {
                "This removes the explicit goal section from future deep dives while leaving the rest of the document intact.".to_string()
            }
            config::ConfigField::DeepDiveAccomplishments => {
                "This controls whether future deep dives include the concrete work summary section.".to_string()
            }
            config::ConfigField::DeepDiveInterestingLearnings => {
                "This controls whether future deep dives call out notable lessons or surprises separately.".to_string()
            }
            config::ConfigField::DeepDiveTeachingNarrative => {
                "This controls whether future deep dives include the longer explanatory teaching walkthrough.".to_string()
            }
            config::ConfigField::DeepDiveReviewedExternalSources => {
                "Turning this off hides the fetched-source writeup even when LearnChain reviewed external URLs during generation.".to_string()
            }
            config::ConfigField::DeepDiveReferencedUrls => {
                "Turning this off hides the raw reference list at the bottom of future deep dives.".to_string()
            }
            config::ConfigField::OutputArtifacts => {
                "When enabled, quiz JSON is written to disk in output/ so it can be revisited later from the Library or inspected manually.".to_string()
            }
            config::ConfigField::DocumentRepository => {
                "Selecting a repository reveals the extra credentials and destination fields required to export library artifacts.".to_string()
            }
            config::ConfigField::DocumentRepositoryTarget => match app.config_form.document_repository
            {
                config::DocumentRepositoryKind::Notion => {
                    "Library exports will create new pages under this Notion destination.".to_string()
                }
                _ => "Exports use this target when sending documents to the configured repository.".to_string(),
            },
            config::ConfigField::NotionApiToken => {
                "Without this token, Notion export stays unavailable even if a destination is configured.".to_string()
            }
            config::ConfigField::LearnChainSiteUrl => {
                "Uploads are sent to this base URL, and the signup link in help text is built from it.".to_string()
            }
            config::ConfigField::LearnChainEmail => {
                "Press Enter on this field to clear the stored LearnChain session and unlink the current account.".to_string()
            }
            config::ConfigField::LearnChainAuthCode => {
                "When you save with a code present, LearnChain exchanges it for a refreshable CLI session and clears the code from config.".to_string()
            }
            config::ConfigField::AiProvider => {
                "Changing the provider changes the visible model and credential fields and decides which backend serves every generation request.".to_string()
            }
            config::ConfigField::OpenAiModel => {
                "This model will be used for both quiz generation and deep dives whenever OpenAI is active.".to_string()
            }
            config::ConfigField::OpenAiKey => {
                "If the key is missing, generation requests cannot start while OpenAI is selected.".to_string()
            }
            config::ConfigField::AnthropicModel => {
                "This model will be used for both quiz generation and deep dives whenever Anthropic is active.".to_string()
            }
            config::ConfigField::AnthropicKey => {
                "If the key is missing, generation requests cannot start while Anthropic is selected.".to_string()
            }
            config::ConfigField::OpenRouterModel => {
                "OpenRouter requires an explicit model id, so leaving this blank blocks generation even if the API key is present.".to_string()
            }
            config::ConfigField::OpenRouterKey => {
                "If the key is missing, generation requests cannot start while OpenRouter is selected.".to_string()
            }
        }
    }

    fn config_field_controls(app: &App, field: config::ConfigField) -> String {
        if app.config_form.is_editing_text_field() && app.config_form.current_field() == field {
            "Type or paste the new value. Press Enter to save it or Esc to cancel the edit."
                .to_string()
        } else {
            match field {
                config::ConfigField::DocumentRepositoryTarget
                | config::ConfigField::NotionApiToken
                | config::ConfigField::LearnChainSiteUrl
                | config::ConfigField::LearnChainAuthCode
                | config::ConfigField::OpenAiKey
                | config::ConfigField::AnthropicKey
                | config::ConfigField::OpenRouterModel
                | config::ConfigField::OpenRouterKey => {
                    "Press Enter to edit this text field. Paste is supported while editing."
                        .to_string()
                }
                config::ConfigField::LearnChainEmail => {
                    "Press Enter to clear the stored LearnChain account link.".to_string()
                }
                config::ConfigField::DeepDiveSessionMetadata
                | config::ConfigField::DeepDiveGoal
                | config::ConfigField::DeepDiveAccomplishments
                | config::ConfigField::DeepDiveInterestingLearnings
                | config::ConfigField::DeepDiveTeachingNarrative
                | config::ConfigField::DeepDiveReviewedExternalSources
                | config::ConfigField::DeepDiveReferencedUrls => {
                    "Use Left/Right (or h/l) to toggle this checkbox, then press s to save."
                        .to_string()
                }
                _ => "Use Left/Right (or h/l) to cycle or adjust this value, then press s to save."
                    .to_string(),
            }
        }
    }

    fn config_field_extra_help(app: &App, field: config::ConfigField) -> Option<String> {
        match field {
            config::ConfigField::DocumentRepositoryTarget
                if app.config_form.document_repository == config::DocumentRepositoryKind::Notion =>
            {
                Some(
                    "Enter the destination database/page ID or paste the full Notion URL."
                        .to_string(),
                )
            }
            config::ConfigField::NotionApiToken => {
                Some(config::notion_token_help_message().to_string())
            }
            config::ConfigField::LearnChainSiteUrl
            | config::ConfigField::LearnChainEmail
            | config::ConfigField::LearnChainAuthCode
                if app.config_form.document_repository == config::DocumentRepositoryKind::LearnChain =>
            {
                Some(config::learnchain_authorization_help_message(
                    &app.config_form.learnchain_site_url,
                ))
            }
            config::ConfigField::AiProvider
                if app.config_form.ai_provider == config::AiProvider::CodexCli =>
            {
                Some(config::codex_cli_config_help_message().to_string())
            }
            config::ConfigField::DeepDiveSessionMetadata
            | config::ConfigField::DeepDiveGoal
            | config::ConfigField::DeepDiveAccomplishments
            | config::ConfigField::DeepDiveInterestingLearnings
            | config::ConfigField::DeepDiveTeachingNarrative
            | config::ConfigField::DeepDiveReviewedExternalSources
            | config::ConfigField::DeepDiveReferencedUrls => {
                Some("The document title is always kept. These checkboxes control the markdown sections underneath it.".to_string())
            }
            config::ConfigField::OpenAiKey
                if app.config_form.openai_api_key.trim().is_empty() =>
            {
                Some(config::AiProvider::OpenAI.setup_help().to_string())
            }
            config::ConfigField::AnthropicKey
                if app.config_form.anthropic_api_key.trim().is_empty() =>
            {
                Some(config::AiProvider::Anthropic.setup_help().to_string())
            }
            config::ConfigField::OpenRouterModel
                if app.config_form.openrouter_model.trim().is_empty() =>
            {
                Some("Enter an exact model id like openrouter/auto or another supported OpenRouter model slug.".to_string())
            }
            config::ConfigField::OpenRouterKey
                if app.config_form.openrouter_api_key.trim().is_empty() =>
            {
                Some(config::AiProvider::OpenRouter.setup_help().to_string())
            }
            _ => None,
        }
    }

    fn checkbox_value(enabled: bool) -> String {
        if enabled {
            "[x]".to_string()
        } else {
            "[ ]".to_string()
        }
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

    /// Render a text-based progress bar.
    /// Returns a string like "[████████████░░░░░░░░░░░░░░░░░░]"
    fn render_progress_bar(percent: u8, width: usize) -> String {
        let percent = percent.min(100);
        let filled = ((percent as usize) * width) / 100;
        let empty = width - filled;
        format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
    }
}

fn truncate_string(s: &str, max_len: usize) -> String {
    // Get first line only (for multi-line prompts)
    let first_line = s.lines().next().unwrap_or(s);
    if first_line.len() > max_len {
        format!("{}...", &first_line[..max_len.saturating_sub(3)])
    } else {
        first_line.to_string()
    }
}

/// Estimate token count for a single event (roughly 4 chars per token).
fn estimate_event_tokens(event: &SessionEvent) -> usize {
    let mut chars = 0;
    for text in &event.content_texts {
        chars += text.len();
    }
    if let Some(ref args) = event.arguments {
        chars += args.len();
    }
    if let Some(ref output) = event.output {
        chars += output.len();
    }
    // Add some overhead for formatting/structure
    chars += 50;
    // Roughly 4 characters per token
    chars / 4
}

/// Estimate total tokens for a session's events.
fn estimate_session_tokens(session: &Session) -> usize {
    session.events.iter().map(estimate_event_tokens).sum()
}

/// Estimate tokens after applying markdown rules (sampling + max_events).
fn estimate_sampled_tokens(session: &Session) -> (usize, bool) {
    let rules = MarkdownRules::default();
    let selected = rules.select_events(&session.events);
    let tokens: usize = selected.iter().map(|e| estimate_event_tokens(e)).sum();
    let is_sampled = selected.len() < session.events.len();
    (tokens, is_sampled)
}

/// Format token count with K suffix for large numbers.
fn format_tokens(tokens: usize) -> String {
    if tokens >= 1000 {
        format!("~{}K", tokens / 1000)
    } else {
        format!("~{}", tokens)
    }
}

fn strip_leading_toml_front_matter(markdown: &str) -> &str {
    let mut lines = markdown.split_inclusive('\n');
    let Some(first_line) = lines.next() else {
        return markdown;
    };

    if first_line.trim_end_matches(|ch| ch == '\r' || ch == '\n') != "+++" {
        return markdown;
    }

    let mut offset = first_line.len();
    for line in lines {
        offset += line.len();
        if line.trim_end_matches(|ch| ch == '\r' || ch == '\n') == "+++" {
            return markdown[offset..].trim_start_matches(|ch| ch == '\r' || ch == '\n');
        }
    }

    markdown
}

#[cfg(test)]
mod tests {
    use super::strip_leading_toml_front_matter;

    #[test]
    fn strip_leading_toml_front_matter_removes_delimited_header() {
        let markdown = "+++\ntitle = \"Example\"\nreviewed_url_count = 0\n+++\n\n# Body\nContent";

        let stripped = strip_leading_toml_front_matter(markdown);

        assert_eq!(stripped, "# Body\nContent");
    }

    #[test]
    fn strip_leading_toml_front_matter_leaves_plain_markdown_unchanged() {
        let markdown = "# Title\n\nBody";

        let stripped = strip_leading_toml_front_matter(markdown);

        assert_eq!(stripped, markdown);
    }
}
