use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::{Duration, NaiveDate, Utc};
use color_eyre::eyre::{Context, Result, eyre};
use rusqlite::{Connection, params};
use std::collections::{BTreeSet, HashMap};

use crate::{ai_manager::StructuredLearningResponse, output_manager::OutputManager};

const DATABASE_FILENAME: &str = "learning_history.sqlite";

#[derive(Debug, Clone, Default)]
pub struct DailyAnalytics {
    pub date: NaiveDate,
    pub total_questions: u32,
    pub first_try_correct: u32,
    pub total_attempts: u32,
    pub cumulative_groups: u32,
}

#[derive(Debug, Clone, Default)]
pub struct KnowledgeAnalytics {
    pub daily: Vec<DailyAnalytics>,
    pub total_questions: u32,
    pub total_first_try_correct: u32,
    pub total_attempts: u32,
    pub knowledge_groups: Vec<String>,
}

/// Persist AI knowledge responses in a lightweight SQLite database for later analysis.
pub fn record_learning_response(
    session_date: &str,
    response: &StructuredLearningResponse,
) -> Result<()> {
    let db_path = database_path()?;
    record_learning_response_at_path(&db_path, session_date, response)
}

pub(crate) fn record_learning_response_at_path(
    db_path: &Path,
    session_date: &str,
    response: &StructuredLearningResponse,
) -> Result<()> {
    if response.response.is_empty() {
        return Ok(());
    }

    let mut connection = connection_for_path(db_path)?;
    initialize_schema(&mut connection)?;
    persist_learning_entries(&mut connection, session_date, response)
}

/// Record the user's first attempt for a quiz question. Subsequent attempts are ignored.
pub fn record_quiz_first_attempt(
    session_date: &str,
    knowledge_type_group: &str,
    knowledge_type_language: Option<&str>,
    question: &str,
    first_try_correct: bool,
) -> Result<()> {
    let db_path = database_path()?;
    record_quiz_first_attempt_at_path(
        &db_path,
        session_date,
        knowledge_type_group,
        knowledge_type_language,
        question,
        first_try_correct,
    )
}

pub(crate) fn record_quiz_first_attempt_at_path(
    db_path: &Path,
    session_date: &str,
    knowledge_type_group: &str,
    knowledge_type_language: Option<&str>,
    question: &str,
    first_try_correct: bool,
) -> Result<()> {
    let mut connection = connection_for_path(db_path)?;
    initialize_schema(&mut connection)?;
    insert_quiz_attempt(
        &mut connection,
        session_date,
        knowledge_type_group,
        knowledge_type_language,
        question,
        first_try_correct,
    )
}

pub fn load_analytics_snapshot() -> Result<KnowledgeAnalytics> {
    let db_path = database_path()?;
    load_analytics_snapshot_from_path(&db_path, 30)
}

pub(crate) fn load_analytics_snapshot_from_path(
    db_path: &Path,
    days: usize,
) -> Result<KnowledgeAnalytics> {
    let mut connection = connection_for_path(db_path)?;
    initialize_schema(&mut connection)?;

    let today = Utc::now().date_naive();
    let start = today - Duration::days(days.saturating_sub(1) as i64);

    let mut daily_map: HashMap<NaiveDate, DailyAnalytics> = HashMap::new();
    let mut daily_groups: HashMap<NaiveDate, BTreeSet<String>> = HashMap::new();

    let mut total_questions: u32 = 0;

    {
        let mut stmt = connection.prepare(
            "SELECT session_date, SUM(quiz_question_count) FROM knowledge_responses
            WHERE session_date >= ?1 GROUP BY session_date",
        )?;
        let rows = stmt.query_map([start.format("%Y-%m-%d").to_string()], |row| {
            let date_str: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((date_str, count))
        })?;

        for result in rows {
            let (date_str, count) = result?;
            if let Ok(date) = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
                let entry = daily_map.entry(date).or_default();
                entry.date = date;
                entry.total_questions = entry.total_questions.saturating_add(count as u32);
                total_questions = total_questions.saturating_add(count as u32);
            }
        }
    }

    let mut total_first_try_correct: u32 = 0;
    let mut total_attempts: u32 = 0;

    {
        let mut stmt = connection.prepare(
            "SELECT session_date, SUM(first_try_correct), COUNT(*) FROM quiz_attempts
            WHERE session_date >= ?1 GROUP BY session_date",
        )?;

        let rows = stmt.query_map([start.format("%Y-%m-%d").to_string()], |row| {
            let date_str: String = row.get(0)?;
            let correct: i64 = row.get(1)?;
            let attempts: i64 = row.get(2)?;
            Ok((date_str, correct, attempts))
        })?;

        for result in rows {
            let (date_str, correct, attempts) = result?;
            if let Ok(date) = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
                let entry = daily_map.entry(date).or_default();
                entry.date = date;
                entry.first_try_correct = entry.first_try_correct.saturating_add(correct as u32);
                entry.total_attempts = entry.total_attempts.saturating_add(attempts as u32);
                total_first_try_correct = total_first_try_correct.saturating_add(correct as u32);
                total_attempts = total_attempts.saturating_add(attempts as u32);
            }
        }
    }

    {
        let mut stmt = connection.prepare(
            "SELECT session_date, knowledge_type_group FROM knowledge_responses
            WHERE session_date >= ?1",
        )?;
        let rows = stmt.query_map([start.format("%Y-%m-%d").to_string()], |row| {
            let date_str: String = row.get(0)?;
            let group: String = row.get(1)?;
            Ok((date_str, group))
        })?;

        for result in rows {
            let (date_str, group) = result?;
            if let Ok(date) = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
                daily_groups.entry(date).or_default().insert(group);
            }
        }
    }

    let mut groups: BTreeSet<String> = BTreeSet::new();
    {
        let mut stmt =
            connection.prepare("SELECT DISTINCT knowledge_type_group FROM knowledge_responses")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for result in rows {
            if let Ok(group) = result {
                groups.insert(group);
            }
        }
    }

    let mut daily: Vec<DailyAnalytics> = Vec::with_capacity(days);
    let mut cumulative_groups: BTreeSet<String> = BTreeSet::new();
    for offset in 0..days {
        let date = start + Duration::days(offset as i64);
        let mut summary = daily_map.remove(&date).unwrap_or_else(|| DailyAnalytics {
            date,
            total_questions: 0,
            first_try_correct: 0,
            total_attempts: 0,
            cumulative_groups: 0,
        });
        summary.date = date;
        if let Some(groups_for_day) = daily_groups.remove(&date) {
            for group in groups_for_day {
                cumulative_groups.insert(group);
            }
        }
        summary.cumulative_groups = cumulative_groups.len() as u32;
        daily.push(summary);
    }

    Ok(KnowledgeAnalytics {
        daily,
        total_questions,
        total_first_try_correct,
        total_attempts,
        knowledge_groups: groups.into_iter().collect(),
    })
}

fn persist_learning_entries(
    connection: &mut Connection,
    session_date: &str,
    response: &StructuredLearningResponse,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let transaction = connection
        .transaction()
        .wrap_err("failed to start transaction for knowledge store")?;

    for entry in &response.response {
        let quiz_json = serde_json::to_string(&entry.quiz)
            .wrap_err("failed to serialise quiz payload for knowledge store")?;
        let question_count = entry.quiz.len() as i64;
        transaction
            .execute(
                "INSERT INTO knowledge_responses (
                    session_date,
                    recorded_at,
                    knowledge_type_group,
                    summary,
                    knowledge_type_language,
                    quiz_json,
                    quiz_question_count
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    session_date,
                    &now,
                    &entry.knowledge_type_group,
                    &entry.summary,
                    &entry.knowledge_type_language,
                    &quiz_json,
                    question_count,
                ],
            )
            .wrap_err("failed to insert knowledge response into store")?;
    }

    transaction
        .commit()
        .wrap_err("failed to commit knowledge store transaction")?;

    Ok(())
}

fn insert_quiz_attempt(
    connection: &mut Connection,
    session_date: &str,
    knowledge_type_group: &str,
    knowledge_type_language: Option<&str>,
    question: &str,
    first_try_correct: bool,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    connection
        .execute(
            "INSERT OR IGNORE INTO quiz_attempts (
                session_date,
                recorded_at,
                knowledge_type_group,
                knowledge_type_language,
                question,
                first_try_correct
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_date,
                &now,
                knowledge_type_group,
                knowledge_type_language,
                question,
                if first_try_correct { 1 } else { 0 },
            ],
        )
        .wrap_err("failed to insert quiz attempt into store")?;

    Ok(())
}

fn initialize_schema(connection: &mut Connection) -> Result<()> {
    connection
        .execute(
            "CREATE TABLE IF NOT EXISTS knowledge_responses (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_date TEXT NOT NULL,
                recorded_at TEXT NOT NULL,
                knowledge_type_group TEXT NOT NULL,
                summary TEXT NOT NULL,
                knowledge_type_language TEXT NOT NULL,
                quiz_json TEXT NOT NULL,
                quiz_question_count INTEGER NOT NULL
            )",
            [],
        )
        .wrap_err("failed to create knowledge_responses table")?;

    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_knowledge_responses_session_date
            ON knowledge_responses(session_date)",
            [],
        )
        .wrap_err("failed to create knowledge_responses indexes")?;

    connection
        .execute(
            "CREATE TABLE IF NOT EXISTS quiz_attempts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_date TEXT NOT NULL,
                recorded_at TEXT NOT NULL,
                knowledge_type_group TEXT NOT NULL,
                knowledge_type_language TEXT,
                question TEXT NOT NULL,
                first_try_correct INTEGER NOT NULL,
                UNIQUE(session_date, knowledge_type_group, question)
            )",
            [],
        )
        .wrap_err("failed to create quiz_attempts table")?;

    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_quiz_attempts_session_date
            ON quiz_attempts(session_date)",
            [],
        )
        .wrap_err("failed to create quiz_attempts indexes")?;

    Ok(())
}

fn database_path() -> Result<PathBuf> {
    let manager = OutputManager::new();
    let mut output_dir = manager.output_directory().map_err(|err| eyre!(err))?;
    fs::create_dir_all(&output_dir).wrap_err_with(|| {
        format!(
            "failed to create output directory at {}",
            output_dir.display()
        )
    })?;
    output_dir.push(DATABASE_FILENAME);
    Ok(output_dir)
}

fn connection_for_path(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).wrap_err_with(|| {
            format!(
                "failed to create directory for knowledge store at {}",
                parent.display()
            )
        })?;
    }

    Connection::open(db_path)
        .wrap_err_with(|| format!("failed to open knowledge store at {}", db_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_manager::{KnowledgeResponse, QuizItem, QuizOption, StructuredLearningResponse};
    use chrono::{Duration, Utc};
    use std::{fs, time::SystemTime};

    fn sample_response() -> StructuredLearningResponse {
        StructuredLearningResponse {
            response: vec![KnowledgeResponse {
                knowledge_type_group: "Rust Fundamentals".to_string(),
                summary: "Borrow checker overview".to_string(),
                quiz: vec![QuizItem {
                    question: "What guarantees memory safety?".to_string(),
                    options: vec![QuizOption {
                        selection: "The borrow checker".to_string(),
                        is_correct_answer: true,
                    }],
                    resources: vec!["https://doc.rust-lang.org/".to_string()],
                }],
                knowledge_type_language: "Rust".to_string(),
            }],
        }
    }

    #[test]
    fn record_learning_response_at_custom_path_persists_rows() {
        let mut temp_dir = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        temp_dir.push(format!("learnchain-knowledge-store-{unique}"));
        fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test.sqlite");

        let response = sample_response();
        record_learning_response_at_path(&db_path, "2024-05-01", &response).unwrap();

        let connection = Connection::open(&db_path).unwrap();
        let row_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM knowledge_responses", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(row_count, response.response.len() as i64);

        let question_count: i64 = connection
            .query_row(
                "SELECT quiz_question_count FROM knowledge_responses LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(question_count, 1);

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn record_learning_response_ignores_empty_payloads() {
        let mut temp_dir = std::env::temp_dir();
        temp_dir.push(format!(
            "learnchain-knowledge-store-empty-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test.sqlite");

        let empty = StructuredLearningResponse {
            response: Vec::new(),
        };
        record_learning_response_at_path(&db_path, "2024-05-01", &empty).unwrap();

        // The database file should not exist because there was nothing to persist.
        assert!(!db_path.exists());

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn record_quiz_first_attempt_at_path_persists_only_first_result() {
        let mut temp_dir = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        temp_dir.push(format!("learnchain-quiz-attempts-{unique}"));
        fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test.sqlite");

        record_quiz_first_attempt_at_path(
            &db_path,
            "2024-05-01",
            "Rust Fundamentals",
            Some("Rust"),
            "What guarantees memory safety?",
            false,
        )
        .unwrap();

        // A second attempt should be ignored.
        record_quiz_first_attempt_at_path(
            &db_path,
            "2024-05-01",
            "Rust Fundamentals",
            Some("Rust"),
            "What guarantees memory safety?",
            true,
        )
        .unwrap();

        let connection = Connection::open(&db_path).unwrap();
        let row_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM quiz_attempts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(row_count, 1);

        let first_try_correct: i64 = connection
            .query_row(
                "SELECT first_try_correct FROM quiz_attempts LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(first_try_correct, 0);

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn record_quiz_first_attempt_handles_missing_language() {
        let mut temp_dir = std::env::temp_dir();
        temp_dir.push(format!(
            "learnchain-quiz-attempts-lang-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test.sqlite");

        record_quiz_first_attempt_at_path(
            &db_path,
            "2024-05-01",
            "Rust Fundamentals",
            None,
            "What guarantees memory safety?",
            true,
        )
        .unwrap();

        let connection = Connection::open(&db_path).unwrap();
        let language: Option<String> = connection
            .query_row(
                "SELECT knowledge_type_language FROM quiz_attempts LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(language.is_none());

        fs::remove_dir_all(&temp_dir).unwrap();
    }

    #[test]
    fn load_analytics_snapshot_from_path_summarises_recent_activity() {
        let mut temp_dir = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        temp_dir.push(format!("learnchain-analytics-{unique}"));
        fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test.sqlite");

        let response = StructuredLearningResponse {
            response: vec![
                KnowledgeResponse {
                    knowledge_type_group: "Rust Ownership".to_string(),
                    summary: String::new(),
                    quiz: vec![
                        QuizItem {
                            question: "Question 1".to_string(),
                            options: vec![QuizOption {
                                selection: "Answer".to_string(),
                                is_correct_answer: true,
                            }],
                            resources: Vec::new(),
                        },
                        QuizItem {
                            question: "Question 2".to_string(),
                            options: vec![QuizOption {
                                selection: "Answer".to_string(),
                                is_correct_answer: true,
                            }],
                            resources: Vec::new(),
                        },
                    ],
                    knowledge_type_language: "Rust".to_string(),
                },
                KnowledgeResponse {
                    knowledge_type_group: "Traits".to_string(),
                    summary: String::new(),
                    quiz: vec![QuizItem {
                        question: "Question 3".to_string(),
                        options: vec![QuizOption {
                            selection: "Answer".to_string(),
                            is_correct_answer: true,
                        }],
                        resources: Vec::new(),
                    }],
                    knowledge_type_language: "Rust".to_string(),
                },
            ],
        };

        let today = Utc::now().date_naive();
        let day1 = today - Duration::days(2);
        let day2 = today - Duration::days(1);
        let day1_str = day1.format("%Y-%m-%d").to_string();
        let day2_str = day2.format("%Y-%m-%d").to_string();

        record_learning_response_at_path(&db_path, &day1_str, &response).unwrap();
        record_learning_response_at_path(&db_path, &day2_str, &response).unwrap();

        record_quiz_first_attempt_at_path(
            &db_path,
            &day1_str,
            "Rust Ownership",
            Some("Rust"),
            "Question 1",
            true,
        )
        .unwrap();
        record_quiz_first_attempt_at_path(
            &db_path,
            &day2_str,
            "Traits",
            Some("Rust"),
            "Question 3",
            false,
        )
        .unwrap();

        let analytics = load_analytics_snapshot_from_path(&db_path, 3).unwrap();
        assert_eq!(analytics.daily.len(), 3);
        assert_eq!(analytics.total_questions, 6);
        assert_eq!(analytics.total_attempts, 2);
        assert_eq!(analytics.total_first_try_correct, 1);
        assert!(
            analytics
                .knowledge_groups
                .iter()
                .any(|name| name == "Rust Ownership")
        );
        assert!(
            analytics
                .knowledge_groups
                .iter()
                .any(|name| name == "Traits")
        );

        let first_day = analytics
            .daily
            .iter()
            .find(|entry| entry.date == day1)
            .unwrap();
        assert_eq!(first_day.total_questions, 3);
        assert_eq!(first_day.first_try_correct, 1);
        assert_eq!(first_day.cumulative_groups, 2);

        let second_day = analytics
            .daily
            .iter()
            .find(|entry| entry.date == day2)
            .unwrap();
        assert_eq!(second_day.total_questions, 3);
        assert_eq!(second_day.first_try_correct, 0);
        assert_eq!(second_day.cumulative_groups, 2);

        fs::remove_dir_all(&temp_dir).unwrap();
    }
}
