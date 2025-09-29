use std::{
    env, fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{config, log_util};
use color_eyre::eyre::{Context, ContextCompat, Result, eyre};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const JSON_SCHEMA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "response": {
      "type": "array",
      "description": "a list of responses",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "properties": {
          "knowledge_type_group": {
            "type": "string",
            "description": "a name that describes the type of knowledge for grouping purposes. These should be specific for example: data types, modules, libraries, frameworks, macros, keywords, etc"
          },
          "summary": {
            "type": "string",
            "description": "a short description of the concept to learn"
          },
          "quiz": {
            "type": "array",
            "description": "a list of questions related to the subject",
            "items": {
              "type": "object",
              "additionalProperties": false,
              "properties": {
                "question": {
                  "type": "string",
                  "description": "a question about this knowledge type that will test the user"
                },
                "options": {
                  "type": "array",
                  "description": "a multi-choice list of answer options",
                  "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                      "selection": {
                        "type": "string",
                        "description": "one of the multiple choice selection answers to the question"
                      },
                      "is_correct_answer": {
                        "type": "boolean",
                        "description": "this should be set to true if it's the correct answer to the question"
                      }
                    },
                    "required": [
                      "selection",
                      "is_correct_answer"
                    ]
                  }
                }
              },
              "required": [
                "question",
                "options"
              ]
            }
          },
          "resources": {
            "type": "array",
            "description": "an optional list of resources that can help the user learn more about the knowledge subject",
            "items": {
              "type": "string"
            }
          },
          "knowledge_type_language": {
            "type": "string",
            "description": "the language that this quiz is related to"
          }
        },
        "required": [
          "knowledge_type_group",
          "summary",
          "quiz",
          "resources",
          "knowledge_type_language"
        ]
      }
    }
  },
  "required": [
    "response"
  ]
}"#;

const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";

/// Structured representation returned from the LLM.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StructuredLearningResponse {
    #[serde(default)]
    pub response: Vec<KnowledgeResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeResponse {
    #[serde(default)]
    pub knowledge_type_group: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub quiz: Vec<QuizItem>,
    #[serde(default)]
    pub resources: Vec<String>,
    #[serde(default)]
    pub knowledge_type_language: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuizItem {
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub options: Vec<QuizOption>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuizOption {
    #[serde(default)]
    pub selection: String,
    #[serde(default)]
    pub is_correct_answer: bool,
}

/// Coordinates LLM requests informed by the most recent markdown session summary.
#[derive(Debug, Clone)]
pub struct AiManager {
    client: Client,
    api_key: String,
    api_base: String,
    output_root: PathBuf,
    model_name: String,
}

impl AiManager {
    /// Create a new [`AiManager`] with the supplied OpenAI API key, output root, and model name.
    pub fn new(
        api_key: impl Into<String>,
        output_root: impl Into<PathBuf>,
        model_name: impl Into<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            api_base: DEFAULT_API_BASE.to_string(),
            output_root: output_root.into(),
            model_name: model_name.into(),
        }
    }

    /// Construct an [`AiManager`] by reading the `OPENAI_API_KEY` environment variable.
    pub fn from_env(
        output_root: impl Into<PathBuf>,
        model_name: impl Into<String>,
    ) -> Result<Self> {
        let api_key = env::var("OPENAI_API_KEY")
            .wrap_err("OPENAI_API_KEY environment variable is not set")?;
        Ok(Self::new(api_key, output_root, model_name))
    }

    /// Override the base URL used for OpenAI API requests (defaults to `https://api.openai.com/v1`).
    #[allow(dead_code)]
    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
    }

    /// Locate the most recent markdown file under the configured output directory.
    fn latest_markdown_file(&self) -> Result<PathBuf> {
        let root = self.output_root.as_path();
        let entries = fs::read_dir(root)
            .wrap_err_with(|| format!("failed to read output directory at {}", root.display()))?;

        let mut newest: Option<(SystemTime, PathBuf)> = None;
        for entry in entries {
            let entry = entry.wrap_err("failed to read entry in output directory")?;
            let path = entry.path();
            if !is_markdown(&path) {
                continue;
            }

            let metadata = entry
                .metadata()
                .wrap_err_with(|| format!("failed to read metadata for {}", path.display()))?;
            let modified = metadata
                .modified()
                .wrap_err_with(|| format!("failed to read modified time for {}", path.display()))?;

            newest = match newest {
                Some((current_time, current_path)) => {
                    if modified > current_time {
                        Some((modified, path))
                    } else {
                        Some((current_time, current_path))
                    }
                }
                None => Some((modified, path)),
            };
        }

        newest
            .map(|(_, path)| path)
            .ok_or_else(|| eyre!("no markdown files found in {}", root.display()))
    }

    /// Execute the OpenAI request using the latest markdown summary and return a structured response.
    pub async fn generate_learning_response(&self) -> Result<StructuredLearningResponse> {
        let latest_markdown = self.latest_markdown_file()?;
        log_util::log_debug(&format!(
            "AiManager: selected markdown file {}",
            latest_markdown.display()
        ));
        let summary_content = fs::read_to_string(&latest_markdown).wrap_err_with(|| {
            format!(
                "failed to read contents of latest markdown file at {}",
                latest_markdown.display()
            )
        })?;
        log_util::log_debug(&format!(
            "AiManager: summary size = {} bytes",
            summary_content.len()
        ));

        let prompt = self.build_prompt(&summary_content);
        let schema = schema_value();
        let payload = json!({
            "model": self.model_name.as_str(),
            "messages": [
                {
                    "role": "system",
                    "content": config::system_prompt(),
                },
                {
                    "role": "user",
                    "content": prompt,
                }
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "structured_learning_response",
                    "schema": schema,
                    "strict": true,
                }
            }
        });

        let endpoint = format!("{}/chat/completions", self.api_base);
        log_util::log_debug(&format!(
            "AiManager: invoking {} with model {}",
            endpoint, self.model_name
        ));
        let response = self
            .client
            .post(&endpoint)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .wrap_err("failed to invoke OpenAI chat completions API")?;

        log_util::log_debug(&format!("AiManager: OpenAI status {}", response.status()));

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
            log_util::log_debug(&format!("AiManager: OpenAI error body: {}", body));
            return Err(eyre!(format!(
                "OpenAI returned {} with body: {}",
                status, body
            )));
        }

        let response_value: Value = response
            .json()
            .await
            .wrap_err("failed to parse OpenAI response body as JSON")?;
        log_util::log_debug("AiManager: received OpenAI response");

        let primary_text = extract_completion_text(&response_value)
            .context("OpenAI response did not include assistant content")?;
        log_util::log_debug("AiManager: extracted assistant content");

        let structured: StructuredLearningResponse = serde_json::from_str(&primary_text)
            .wrap_err("failed to deserialize OpenAI response into StructuredLearningResponse")?;
        log_util::log_debug("AiManager: deserialization completed successfully");

        Ok(structured)
    }

    fn build_prompt(&self, summary: &str) -> String {
        format!(
            "Analyse the following session summary and produce a JSON payload that adheres to the provided schema. Return only valid JSON with double-quoted keys and strings.\n\nSchema:\n```json\n{}\n```\n\nSession summary:\n```markdown\n{}\n```",
            JSON_SCHEMA, summary
        )
    }
}

fn schema_value() -> Value {
    serde_json::from_str(JSON_SCHEMA).expect("JSON_SCHEMA is valid")
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn extract_completion_text(value: &Value) -> Option<String> {
    let choices = value.get("choices")?.as_array()?;
    let first_choice = choices.first()?;
    let message = first_choice.get("message")?;
    let content = message.get("content")?;
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let mut buffer = String::new();
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    buffer.push_str(text);
                }
            }
            if buffer.is_empty() {
                None
            } else {
                Some(buffer)
            }
        }
        _ => None,
    }
}
