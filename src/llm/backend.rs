use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::mpsc::Sender,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use color_eyre::eyre::{Result, WrapErr, eyre};
use rig::{
    client::CompletionClient,
    completion::TypedPrompt,
    providers::{anthropic, openai, openrouter},
};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::{io::AsyncWriteExt, process::Command};

use crate::{
    AiTaskKind, AiTaskMessage, config,
    config::{AiProvider, ResolvedLlmConfig},
    log_util::log_debug,
};

use super::types::{LearningGenerationResult, LlmUsage, StructuredLearningResponse};

const EXTRACTOR_RETRIES: u64 = 1;
const LLM_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone)]
enum RigProviderClient {
    OpenAi(openai::CompletionsClient),
    Anthropic(anthropic::Client),
    OpenRouter(openrouter::Client),
}

#[derive(Debug, Clone)]
enum BackendClient {
    Rig(RigProviderClient),
    CodexCli,
}

#[derive(Debug, Clone)]
pub struct LlmBackend {
    client: BackendClient,
    provider: AiProvider,
    model_name: String,
    output_root: PathBuf,
}

impl LlmBackend {
    pub fn from_config(
        resolved: ResolvedLlmConfig,
        output_root: impl Into<PathBuf>,
    ) -> Result<Self> {
        let client = match resolved.provider {
            AiProvider::OpenAI => BackendClient::Rig(RigProviderClient::OpenAi(
                openai::CompletionsClient::builder()
                    .api_key(require_api_key(&resolved)?)
                    .build()
                    .wrap_err("failed to build OpenAI client")?,
            )),
            AiProvider::Anthropic => BackendClient::Rig(RigProviderClient::Anthropic(
                anthropic::Client::builder()
                    .api_key(require_api_key(&resolved)?)
                    .build()
                    .wrap_err("failed to build Anthropic client")?,
            )),
            AiProvider::OpenRouter => {
                require_model_name(&resolved)?;
                BackendClient::Rig(RigProviderClient::OpenRouter(
                    openrouter::Client::builder()
                        .api_key(require_api_key(&resolved)?)
                        .build()
                        .wrap_err("failed to build OpenRouter client")?,
                ))
            }
            AiProvider::CodexCli => BackendClient::CodexCli,
        };

        Ok(Self {
            client,
            provider: resolved.provider,
            model_name: resolved.model_name,
            output_root: output_root.into(),
        })
    }

    pub async fn generate_learning_response_with_progress(
        &self,
        summary_override: Option<String>,
        progress_sender: impl Into<Option<&Sender<AiTaskMessage>>>,
    ) -> Result<LearningGenerationResult> {
        let sender = progress_sender.into();

        let summary_content = if let Some(summary) = summary_override {
            log_debug(&format!(
                "LearningGenerator: using in-memory summary ({} bytes)",
                summary.len()
            ));
            summary
        } else {
            let latest_markdown = self.latest_markdown_file()?;
            log_debug(&format!(
                "LearningGenerator: selected markdown file {}",
                latest_markdown.display()
            ));
            let summary = fs::read_to_string(&latest_markdown).wrap_err_with(|| {
                format!(
                    "failed to read contents of latest markdown file at {}",
                    latest_markdown.display()
                )
            })?;
            log_debug(&format!(
                "LearningGenerator: summary size = {} bytes",
                summary.len()
            ));
            summary
        };

        if let Some(sender) = sender {
            send_progress(
                sender,
                AiTaskKind::LearningLesson,
                "Preparing structured learning request...",
                40,
            );
        }

        let prompt = build_learning_prompt(&summary_content);
        if let Some(sender) = sender {
            send_progress(
                sender,
                AiTaskKind::LearningLesson,
                "Waiting for provider response...",
                55,
            );
        }

        let (response, usage) = self
            .extract_typed::<StructuredLearningResponse>(
                &config::system_prompt(),
                &prompt,
                "structured learning response",
            )
            .await?;

        if let Some(sender) = sender {
            send_progress(
                sender,
                AiTaskKind::LearningLesson,
                "Validating structured learning response...",
                85,
            );
        }

        self.log_usage("LearningGenerator", usage.as_ref());
        Ok(LearningGenerationResult { response, usage })
    }

    pub(crate) async fn extract_typed<T>(
        &self,
        preamble: &str,
        prompt: &str,
        error_context: &str,
    ) -> Result<(T, Option<LlmUsage>)>
    where
        T: DeserializeOwned + Serialize + JsonSchema + Send + Sync + 'static,
    {
        match &self.client {
            BackendClient::Rig(RigProviderClient::OpenAi(client)) => {
                extract_with_client(client, &self.model_name, preamble, prompt, error_context).await
            }
            BackendClient::Rig(RigProviderClient::Anthropic(client)) => {
                extract_with_client(client, &self.model_name, preamble, prompt, error_context).await
            }
            BackendClient::Rig(RigProviderClient::OpenRouter(client)) => {
                extract_with_openrouter(client, &self.model_name, preamble, prompt, error_context)
                    .await
            }
            BackendClient::CodexCli => {
                self.extract_with_codex_cli::<T>(preamble, prompt, error_context)
                    .await
            }
        }
    }

    async fn extract_with_codex_cli<T>(
        &self,
        preamble: &str,
        prompt: &str,
        error_context: &str,
    ) -> Result<(T, Option<LlmUsage>)>
    where
        T: DeserializeOwned + Serialize + JsonSchema + Send + Sync + 'static,
    {
        let schema_path = self.write_codex_schema_file::<T>()?;
        let result = self
            .run_codex_cli_request::<T>(&schema_path, preamble, prompt, error_context)
            .await;
        remove_schema_file(&schema_path);
        result
    }

    async fn run_codex_cli_request<T>(
        &self,
        schema_path: &Path,
        preamble: &str,
        prompt: &str,
        error_context: &str,
    ) -> Result<(T, Option<LlmUsage>)>
    where
        T: DeserializeOwned + Serialize + JsonSchema + Send + Sync + 'static,
    {
        let codex_prompt = build_codex_cli_prompt(preamble, prompt);
        let mut command = Command::new("codex");
        command
            .kill_on_drop(true)
            .arg("exec")
            .arg("--json")
            .arg("--color")
            .arg("never")
            .arg("--sandbox")
            .arg("read-only")
            .arg("--skip-git-repo-check")
            .arg("--output-schema")
            .arg(schema_path)
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .wrap_err("failed to spawn Codex CLI process")?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| eyre!("failed to open Codex CLI stdin"))?;
        stdin
            .write_all(codex_prompt.as_bytes())
            .await
            .wrap_err("failed to send prompt to Codex CLI")?;
        drop(stdin);

        let output = tokio::time::timeout(LLM_REQUEST_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                eyre!(
                    "Codex CLI request timed out after {} seconds while extracting {}",
                    LLM_REQUEST_TIMEOUT.as_secs(),
                    error_context
                )
            })?
            .wrap_err("failed to collect Codex CLI output")?;

        let stderr_text = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            return Err(eyre!(codex_error_message(
                format!(
                    "Codex CLI exited with status {} while extracting {}",
                    output
                        .status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    error_context
                ),
                &stderr_text,
            )));
        }

        let stdout_text =
            String::from_utf8(output.stdout).wrap_err("Codex CLI stdout was not valid UTF-8")?;
        parse_codex_exec_output::<T>(&stdout_text, error_context, &stderr_text)
    }

    fn write_codex_schema_file<T>(&self) -> Result<PathBuf>
    where
        T: JsonSchema,
    {
        let schema_dir = self.output_root.join("tmp");
        fs::create_dir_all(&schema_dir).wrap_err_with(|| {
            format!(
                "failed to create Codex schema directory at {}",
                schema_dir.display()
            )
        })?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let path = schema_dir.join(format!(
            "learnchain-codex-schema-{}-{}.json",
            std::process::id(),
            timestamp
        ));
        let schema_json = build_codex_output_schema_json::<T>()?;
        fs::write(&path, schema_json)
            .wrap_err_with(|| format!("failed to write Codex schema to {}", path.display()))?;
        Ok(path)
    }

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
                Some((current_time, current_path)) if modified <= current_time => {
                    Some((current_time, current_path))
                }
                _ => Some((modified, path)),
            };
        }

        newest
            .map(|(_, path)| path)
            .ok_or_else(|| eyre!("no markdown files found in {}", root.display()))
    }

    fn log_usage(&self, label: &str, usage: Option<&LlmUsage>) {
        if let Some(usage) = usage {
            log_debug(&format!(
                "{}: {} {} tokens in={} out={} total={}",
                label,
                self.provider.label(),
                self.model_name,
                usage.input_tokens,
                usage.output_tokens,
                usage.total_tokens
            ));
        }
    }
}

fn require_api_key(resolved: &ResolvedLlmConfig) -> Result<&str> {
    if resolved.api_key.trim().is_empty() {
        return Err(eyre!(
            "{} API key is not configured",
            resolved.provider.label()
        ));
    }

    Ok(resolved.api_key.as_str())
}

fn require_model_name(resolved: &ResolvedLlmConfig) -> Result<&str> {
    if resolved.model_name.trim().is_empty() {
        return Err(eyre!(
            "{} model is not configured",
            resolved.provider.label()
        ));
    }

    Ok(resolved.model_name.as_str())
}

fn build_learning_prompt(summary: &str) -> String {
    format!(
        "Generate a structured learning response from the following session summary.\n\nSession summary:\n```markdown\n{}\n```",
        summary
    )
}

fn build_codex_cli_prompt(preamble: &str, prompt: &str) -> String {
    format!(
        "Return only JSON that matches the provided output schema exactly.\nDo not wrap the JSON in markdown fences.\nDo not include any prose before or after the JSON.\n\nSystem instructions:\n{}\n\nTask:\n{}",
        preamble.trim(),
        prompt.trim()
    )
}

fn build_codex_output_schema_json<T>() -> Result<String>
where
    T: JsonSchema,
{
    let schema = schemars::schema_for!(T);
    let mut schema_value =
        serde_json::to_value(schema).wrap_err("failed to convert Codex schema to JSON value")?;
    normalize_codex_schema_value(&mut schema_value);
    serde_json::to_string_pretty(&schema_value).wrap_err("failed to serialize Codex schema")
}

fn normalize_codex_schema_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let required_keys = map
                .get("properties")
                .and_then(|properties| match properties {
                    serde_json::Value::Object(properties) => {
                        let mut keys = properties.keys().cloned().collect::<Vec<_>>();
                        keys.sort();
                        Some(keys)
                    }
                    _ => None,
                });

            if let Some(required_keys) = required_keys {
                let required = required_keys
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect::<Vec<_>>();
                map.insert("required".to_string(), serde_json::Value::Array(required));
            }

            for child in map.values_mut() {
                normalize_codex_schema_value(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_codex_schema_value(item);
            }
        }
        _ => {}
    }
}

fn parse_codex_exec_output<T>(
    stdout: &str,
    error_context: &str,
    stderr: &str,
) -> Result<(T, Option<LlmUsage>)>
where
    T: DeserializeOwned,
{
    let mut final_message: Option<String> = None;
    let mut usage: Option<LlmUsage> = None;

    for (index, line) in stdout.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let event: serde_json::Value = serde_json::from_str(trimmed).map_err(|err| {
            eyre!(codex_error_message(
                format!(
                    "failed to parse Codex JSONL line {} while extracting {}: {}",
                    index + 1,
                    error_context,
                    err
                ),
                stderr,
            ))
        })?;

        match event.get("type").and_then(|value| value.as_str()) {
            Some("item.completed") => {
                if event.pointer("/item/type").and_then(|value| value.as_str())
                    == Some("agent_message")
                {
                    let text = event
                        .pointer("/item/text")
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| {
                            eyre!(codex_error_message(
                                format!(
                                    "Codex CLI returned an agent_message without text while extracting {}",
                                    error_context
                                ),
                                stderr,
                            ))
                        })?;
                    final_message = Some(text.to_string());
                }
            }
            Some("turn.completed") => {
                if let Some(raw_usage) = event.get("usage") {
                    let input_tokens = raw_usage
                        .get("input_tokens")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let output_tokens = raw_usage
                        .get("output_tokens")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let total_tokens = raw_usage
                        .get("total_tokens")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(input_tokens + output_tokens);
                    usage = Some(LlmUsage {
                        input_tokens,
                        output_tokens,
                        total_tokens,
                    });
                }
            }
            _ => {}
        }
    }

    let payload = final_message.ok_or_else(|| {
        eyre!(codex_error_message(
            format!(
                "Codex CLI did not return a final structured response while extracting {}",
                error_context
            ),
            stderr,
        ))
    })?;
    let response = serde_json::from_str::<T>(&payload).map_err(|err| {
        eyre!(codex_error_message(
            format!(
                "failed to deserialize {} from Codex CLI output: {}",
                error_context, err
            ),
            stderr,
        ))
    })?;

    Ok((response, usage))
}

fn codex_error_message(message: String, stderr: &str) -> String {
    if stderr.trim().is_empty() {
        message
    } else {
        format!("{} | stderr: {}", message, stderr.trim())
    }
}

fn remove_schema_file(path: &Path) {
    if let Err(err) = fs::remove_file(path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        log_debug(&format!(
            "LlmBackend: failed to remove Codex schema file {}: {}",
            path.display(),
            err
        ));
    }
}

async fn extract_with_client<C, T>(
    client: &C,
    model_name: &str,
    preamble: &str,
    prompt: &str,
    error_context: &str,
) -> Result<(T, Option<LlmUsage>)>
where
    C: CompletionClient,
    T: DeserializeOwned + Serialize + JsonSchema + Send + Sync + 'static,
{
    let agent = client
        .agent(model_name.to_string())
        .preamble(preamble)
        .build();

    let response = tokio::time::timeout(LLM_REQUEST_TIMEOUT, async {
        agent.prompt_typed::<T>(prompt).extended_details().await
    })
    .await
    .map_err(|_| {
        eyre!(
            "provider request timed out after {} seconds",
            LLM_REQUEST_TIMEOUT.as_secs()
        )
    })?
    .wrap_err_with(|| format!("failed to deserialize {}", error_context))?;

    Ok((response.output, Some(response.usage.into())))
}

async fn extract_with_openrouter<T>(
    client: &openrouter::Client,
    model_name: &str,
    preamble: &str,
    prompt: &str,
    error_context: &str,
) -> Result<(T, Option<LlmUsage>)>
where
    T: DeserializeOwned + Serialize + JsonSchema + Send + Sync + 'static,
{
    let response = tokio::time::timeout(LLM_REQUEST_TIMEOUT, async {
        client
            .extractor::<T>(model_name.to_string())
            .preamble(preamble)
            .retries(EXTRACTOR_RETRIES)
            .build()
            .extract_with_usage(prompt)
            .await
    })
    .await
    .map_err(|_| {
        eyre!(
            "provider request timed out after {} seconds",
            LLM_REQUEST_TIMEOUT.as_secs()
        )
    })?
    .wrap_err_with(|| format!("failed to extract {}", error_context))?;

    Ok((response.data, Some(response.usage.into())))
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn send_progress(sender: &Sender<AiTaskMessage>, kind: AiTaskKind, message: &str, percent: u8) {
    let _ = sender.send(AiTaskMessage::Progress(kind, message.to_string(), percent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_llm_factory_requires_openai_api_key() {
        let resolved = ResolvedLlmConfig {
            provider: AiProvider::OpenAI,
            model_name: "gpt-5".to_string(),
            model_label: "gpt-5".to_string(),
            api_key: String::new(),
        };

        let result = LlmBackend::from_config(resolved, "output");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("OpenAI API key is not configured")
        );
    }

    #[test]
    fn resolved_llm_factory_requires_anthropic_api_key() {
        let resolved = ResolvedLlmConfig {
            provider: AiProvider::Anthropic,
            model_name: "claude-sonnet-4-20250514".to_string(),
            model_label: "Claude Sonnet 4".to_string(),
            api_key: String::new(),
        };

        let result = LlmBackend::from_config(resolved, "output");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Anthropic API key is not configured")
        );
    }

    #[test]
    fn resolved_llm_factory_requires_openrouter_api_key() {
        let resolved = ResolvedLlmConfig {
            provider: AiProvider::OpenRouter,
            model_name: "openrouter/model".to_string(),
            model_label: "openrouter/model".to_string(),
            api_key: String::new(),
        };

        let result = LlmBackend::from_config(resolved, "output");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("OpenRouter API key is not configured")
        );
    }

    #[test]
    fn resolved_llm_factory_accepts_codex_cli_without_api_key() {
        let resolved = ResolvedLlmConfig {
            provider: AiProvider::CodexCli,
            model_name: "codex-exec".to_string(),
            model_label: "CLI default".to_string(),
            api_key: String::new(),
        };

        let result = LlmBackend::from_config(resolved, "output");
        assert!(result.is_ok());
    }

    #[test]
    fn build_prompt_includes_summary_without_raw_schema() {
        let prompt = build_learning_prompt("## Session\nUpdated foo.rs");
        assert!(prompt.contains("Updated foo.rs"));
        assert!(!prompt.contains("\"additionalProperties\""));
        assert!(!prompt.contains("provided schema"));
    }

    #[test]
    fn build_codex_cli_prompt_wraps_preamble_and_task() {
        let prompt = build_codex_cli_prompt("Follow the schema", "Return a quiz");
        assert!(prompt.contains("Follow the schema"));
        assert!(prompt.contains("Return a quiz"));
        assert!(prompt.contains("Return only JSON"));
        assert!(prompt.contains("Do not wrap the JSON in markdown fences"));
    }

    #[test]
    fn parse_codex_exec_output_returns_structured_response_and_usage() {
        let stdout = r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"{\"response\":[{\"knowledge_type_group\":\"Rust ownership\",\"summary\":\"Borrowing basics\",\"quiz\":[],\"knowledge_type_language\":\"Rust\"}]}"}} 
{"type":"turn.completed","usage":{"input_tokens":120,"output_tokens":40}}"#;

        let (response, usage) = parse_codex_exec_output::<StructuredLearningResponse>(
            stdout,
            "structured learning response",
            "",
        )
        .unwrap();

        assert_eq!(response.response.len(), 1);
        assert_eq!(response.response[0].knowledge_type_group, "Rust ownership");
        assert_eq!(
            usage,
            Some(LlmUsage {
                input_tokens: 120,
                output_tokens: 40,
                total_tokens: 160,
            })
        );
    }

    #[test]
    fn parse_codex_exec_output_requires_agent_message() {
        let stdout = r#"{"type":"turn.completed","usage":{"input_tokens":120,"output_tokens":40}}"#;

        let result = parse_codex_exec_output::<StructuredLearningResponse>(
            stdout,
            "structured learning response",
            "",
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("did not return a final structured response")
        );
    }

    #[test]
    fn parse_codex_exec_output_rejects_malformed_jsonl() {
        let result = parse_codex_exec_output::<StructuredLearningResponse>(
            "not-json",
            "structured learning response",
            "",
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed to parse Codex JSONL line 1")
        );
    }

    #[test]
    fn parse_codex_exec_output_rejects_invalid_final_payload() {
        let stdout = r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"not-json"}}"#;

        let result = parse_codex_exec_output::<StructuredLearningResponse>(
            stdout,
            "structured learning response",
            "",
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains(
                "failed to deserialize structured learning response from Codex CLI output"
            )
        );
    }

    #[test]
    fn structured_learning_response_schema_generation_succeeds() {
        let schema_json = serde_json::from_str::<serde_json::Value>(
            &build_codex_output_schema_json::<StructuredLearningResponse>().unwrap(),
        )
        .unwrap();
        assert!(schema_json.is_object());
    }

    #[test]
    fn codex_schema_marks_all_properties_as_required_recursively() {
        let schema_json = serde_json::from_str::<serde_json::Value>(
            &build_codex_output_schema_json::<StructuredLearningResponse>().unwrap(),
        )
        .unwrap();

        let root_required = schema_json
            .get("required")
            .and_then(|value| value.as_array())
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(root_required, vec!["response"]);

        let knowledge_response_required = schema_json
            .pointer("/$defs/KnowledgeResponse/required")
            .and_then(|value| value.as_array())
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            knowledge_response_required,
            vec![
                "knowledge_type_group",
                "knowledge_type_language",
                "quiz",
                "summary",
            ]
        );

        let quiz_item_required = schema_json
            .pointer("/$defs/QuizItem/required")
            .and_then(|value| value.as_array())
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(quiz_item_required, vec!["options", "question", "resources"]);
    }

    #[test]
    fn is_markdown_detects_md_extension() {
        assert!(is_markdown(Path::new("note.md")));
        assert!(is_markdown(Path::new("note.MD")));
        assert!(!is_markdown(Path::new("note.txt")));
    }
}
