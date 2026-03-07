use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    time::{Duration, SystemTime},
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
pub struct LlmBackend {
    client: RigProviderClient,
    provider: AiProvider,
    model_name: String,
    output_root: PathBuf,
}

impl LlmBackend {
    pub fn from_config(
        resolved: ResolvedLlmConfig,
        output_root: impl Into<PathBuf>,
    ) -> Result<Self> {
        if resolved.api_key.trim().is_empty() {
            return Err(eyre!(
                "{} API key is not configured",
                resolved.provider.label()
            ));
        }

        let client = match resolved.provider {
            AiProvider::OpenAI => RigProviderClient::OpenAi(
                openai::CompletionsClient::builder()
                    .api_key(&resolved.api_key)
                    .build()
                    .wrap_err("failed to build OpenAI client")?,
            ),
            AiProvider::Anthropic => RigProviderClient::Anthropic(
                anthropic::Client::builder()
                    .api_key(&resolved.api_key)
                    .build()
                    .wrap_err("failed to build Anthropic client")?,
            ),
            AiProvider::OpenRouter => RigProviderClient::OpenRouter(
                openrouter::Client::builder()
                    .api_key(&resolved.api_key)
                    .build()
                    .wrap_err("failed to build OpenRouter client")?,
            ),
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
            RigProviderClient::OpenAi(client) => {
                extract_with_client(client, &self.model_name, preamble, prompt, error_context).await
            }
            RigProviderClient::Anthropic(client) => {
                extract_with_client(client, &self.model_name, preamble, prompt, error_context).await
            }
            RigProviderClient::OpenRouter(client) => {
                extract_with_openrouter(client, &self.model_name, preamble, prompt, error_context)
                    .await
            }
        }
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

fn build_learning_prompt(summary: &str) -> String {
    format!(
        "Generate a structured learning response from the following session summary.\n\nSession summary:\n```markdown\n{}\n```",
        summary
    )
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
    use schemars::schema_for;

    #[test]
    fn resolved_llm_factory_requires_api_key() {
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
    fn build_prompt_includes_summary_without_raw_schema() {
        let prompt = build_learning_prompt("## Session\nUpdated foo.rs");
        assert!(prompt.contains("Updated foo.rs"));
        assert!(!prompt.contains("\"additionalProperties\""));
        assert!(!prompt.contains("provided schema"));
    }

    #[test]
    fn structured_learning_response_schema_generation_succeeds() {
        let schema = schema_for!(StructuredLearningResponse);
        let schema_json = serde_json::to_value(schema).unwrap();
        assert!(schema_json.is_object());
    }

    #[test]
    fn is_markdown_detects_md_extension() {
        assert!(is_markdown(Path::new("note.md")));
        assert!(is_markdown(Path::new("note.MD")));
        assert!(!is_markdown(Path::new("note.txt")));
    }
}
