use chrono::Local;
use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout},
    style::Stylize,
    text::Line,
    widgets::{Block, List, ListItem, Paragraph},
};
use std::{env, fs, path::PathBuf};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = App::new().run(terminal);
    ratatui::restore();
    result
}

/// The main application which holds the state and logic of the application.
#[derive(Debug)]
pub struct App {
    /// Is the application running?
    running: bool,
    /// Files detected in today's Codex session directory.
    files: Vec<String>,
    /// Absolute path to today's session directory.
    session_dir: PathBuf,
    /// Human-readable label for today's date.
    session_date: String,
    /// Any error encountered while loading files.
    error: Option<String>,
}

impl App {
    /// Construct a new instance of [`App`].
    pub fn new() -> Self {
        let (session_date, session_dir, files, error) = Self::load_session_files();
        Self {
            running: false,
            files,
            session_dir,
            session_date,
            error,
        }
    }

    /// Run the application's main loop.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;
        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_crossterm_events()?;
        }
        Ok(())
    }

    /// Renders the user interface.
    ///
    /// This is where you add new widgets. See the following resources for more information:
    ///
    /// - <https://docs.rs/ratatui/latest/ratatui/widgets/index.html>
    /// - <https://github.com/ratatui/ratatui/tree/main/ratatui-widgets/examples>
    fn render(&mut self, frame: &mut Frame) {
        let header_title = Line::from(format!("Codex Sessions • {}", self.session_date))
            .bold()
            .blue()
            .centered();

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(frame.area());

        frame.render_widget(
            Paragraph::new(format!("Directory: {}", self.session_dir.display()))
                .block(Block::bordered().title(header_title))
                .centered(),
            layout[0],
        );

        let list_items: Vec<ListItem> = if self.files.is_empty() {
            vec![ListItem::new("No files found for today.")]
        } else {
            self.files
                .iter()
                .map(|file| ListItem::new(file.clone()))
                .collect()
        };

        frame.render_widget(
            List::new(list_items).block(Block::bordered().title(Line::from("Files"))),
            layout[1],
        );

        let mut status_lines = vec!["Press `Esc`, `Ctrl-C` or `q` to stop running.".to_string()];
        if let Some(error) = &self.error {
            status_lines.push(format!("Error: {}", error));
        }
        frame.render_widget(
            Paragraph::new(status_lines.join("\n"))
                .block(Block::bordered().title(Line::from("Status"))),
            layout[2],
        );
    }

    /// Reads the crossterm events and updates the state of [`App`].
    ///
    /// If your application needs to perform work in between handling events, you can use the
    /// [`event::poll`] function to check if there are any events available with a timeout.
    fn handle_crossterm_events(&mut self) -> Result<()> {
        match event::read()? {
            // it's important to check KeyEventKind::Press to avoid handling key release events
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key),
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {}
            _ => {}
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc | KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => self.quit(),
            // Add other key handlers here.
            _ => {}
        }
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }

    /// Load file names from today's Codex session directory.
    fn load_session_files() -> (String, PathBuf, Vec<String>, Option<String>) {
        let now = Local::now();
        let session_date = now.format("%Y-%m-%d").to_string();

        let mut session_dir = env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("~"));
        session_dir.push(".codex");
        session_dir.push("sessions");
        session_dir.push(now.format("%Y").to_string());
        session_dir.push(now.format("%m").to_string());
        session_dir.push(now.format("%d").to_string());

        match fs::read_dir(&session_dir) {
            Ok(entries) => {
                let mut files = Vec::new();
                let mut entry_error = None;
                for entry in entries {
                    match entry {
                        Ok(entry) => files.push(entry.file_name().to_string_lossy().into_owned()),
                        Err(err) => {
                            entry_error = Some(format!("{}: {}", session_dir.display(), err));
                        }
                    }
                }
                files.sort();
                (session_date, session_dir, files, entry_error)
            }
            Err(err) => {
                let message = format!("{}: {}", session_dir.display(), err);
                (session_date, session_dir, Vec::new(), Some(message))
            }
        }
    }
}
