mod config;
mod document_repository;
mod knowledge_store;
mod llm;
mod log_util;
mod markdown_rules;
mod output_manager;
mod session_analytics;
mod session_manager;
mod session_sources;
mod ui_renderer;
mod view_managers;

use color_eyre::Result;
use config::ConfigForm;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use document_repository::{RepositoryExportResult, poll_export_messages};
use dotenvy::dotenv;
use knowledge_store::KnowledgeAnalytics;
use llm::{
    DeepDiveDocument, DeepDiveGenerationResult, DeepDiveHistoryEntry, LearningGenerationResult,
    LlmBackend, StructuredLearningResponse, poll_ai_messages,
};
use output_manager::{LibraryArtifactEntry, OutputManager};
use ratatui::{DefaultTerminal, Frame};
use session_manager::SessionManager;
use session_sources::{CodexCliSource, Session, SessionEvent, SessionLoad};
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender},
    time::{Duration, Instant},
};
use ui_renderer::UiRenderer;
use view_managers::{
    AnalyticsManager, ConfigManager, DeepDiveManager, LearningManager, LibraryManager, MenuManager,
    SessionPickerManager,
};

pub(crate) const AI_LOADING_FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
const CODEX_SKILL_NAME: &str = "learnchain-deep-dive";
const EMBEDDED_CODEX_SKILL: &str =
    include_str!("../assets/codex-skills/learnchain-deep-dive/SKILL.md");
const EMBEDDED_CODEX_SKILL_OPENAI_YAML: &str =
    include_str!("../assets/codex-skills/learnchain-deep-dive/agents/openai.yaml");
const HELP_TEXT: &str = "learnchain options:\n  --debug, -d               write runtime debug logs to output/learnchain-debug.log\n  --set-openai-key <key>    store your OpenAI API key\n  --clear-openai-key        remove the stored OpenAI API key\n  --set-anthropic-key <key> store your Anthropic API key\n  --clear-anthropic-key     remove the stored Anthropic API key\n  --set-openrouter-key <key> store your OpenRouter API key\n  --clear-openrouter-key    remove the stored OpenRouter API key\n  --set-document-repository <none|notion|learnchain>\n                           store the selected document repository\n  --clear-document-repository\n                           clear the selected document repository and target\n  --set-document-repository-target <target>\n                           store the document repository target\n  --clear-document-repository-target\n                           remove the stored document repository target\n  --set-notion-api-token <token>\n                           store the Notion API token for Notion exports\n  --clear-notion-api-token\n                           remove the stored Notion API token\n  --set-learnchain-site-url <url>\n                           store the LearnChain site URL (for example http://localhost:3000)\n  --clear-learnchain-site-url\n                           reset the LearnChain site URL to its default\n  --set-learnchain-email <email>\n                           store the LearnChain email used for document upload\n  --clear-learnchain-email\n                           remove the stored LearnChain email\n  --set-learnchain-password <password>\n                           store the LearnChain password used for document upload\n  --clear-learnchain-password\n                           remove the stored LearnChain password\n  --generate-codex-deep-dive\n                           generate a deep dive for the current or specified Codex session\n  --codex-thread-id <id>\n                           target a specific Codex session id with --generate-codex-deep-dive\n  --export-to-document-repository\n                           export the generated Codex deep dive to the configured document repository\n  --print-codex-deep-dive-action\n                           print a copy/paste Codex custom command template\n  --install-codex-deep-dive-skill\n                           install the bundled LearnChain Codex skill into your Codex skills folder\n  --help                    show this message\n  --version                 show version";

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    SetOpenAiKey(String),
    ClearOpenAiKey,
    SetAnthropicKey(String),
    ClearAnthropicKey,
    SetOpenRouterKey(String),
    ClearOpenRouterKey,
    SetDocumentRepository(config::DocumentRepositoryKind),
    ClearDocumentRepository,
    SetDocumentRepositoryTarget(String),
    ClearDocumentRepositoryTarget,
    SetNotionApiToken(String),
    ClearNotionApiToken,
    SetLearnChainSiteUrl(String),
    ClearLearnChainSiteUrl,
    SetLearnChainEmail(String),
    ClearLearnChainEmail,
    SetLearnChainPassword(String),
    ClearLearnChainPassword,
    GenerateCodexDeepDive,
    PrintCodexDeepDiveAction,
    InstallCodexDeepDiveSkill,
    Help,
    Version,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CliOptions {
    debug_logging: bool,
    codex_thread_id: Option<String>,
    export_document_repository: bool,
    command: Option<CliCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppView {
    Menu,
    Events,
    SessionPicker,
    Learning,
    DeepDive,
    Library,
    Config,
    Analytics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionSelectionTarget {
    Quiz,
    DeepDive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AiTaskKind {
    LearningLesson,
    SessionDeepDive,
}

#[derive(Debug)]
pub(crate) enum AiTaskMessage {
    LearningSuccess(LearningGenerationResult),
    DeepDiveSuccess(DeepDiveGenerationResult),
    Error(AiTaskKind, String),
    Progress(AiTaskKind, String, u8), // (kind, message, percentage)
}

#[derive(Debug)]
pub(crate) enum DocumentExportMessage {
    Success(document_repository::RepositoryExportResult),
    Error(String),
}

pub(crate) fn reset_learning_feedback(
    feedback: &mut Option<String>,
    summary_revealed: &mut bool,
    waiting_for_next: &mut bool,
) {
    *feedback = None;
    *summary_revealed = false;
    *waiting_for_next = false;
}

fn main() -> color_eyre::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cli = match parse_cli_options(&args[1..]) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{}", message);
            std::process::exit(1);
        }
    };

    if cli.debug_logging {
        let log_path = log_util::enable_runtime_debug_logging()?;
        eprintln!("Debug logging enabled: {}", log_path.display());
        log_util::log_debug("App: runtime debug logging enabled via CLI flag");
    }

    let codex_thread_id = cli.codex_thread_id.clone();
    let export_document_repository = cli.export_document_repository;
    if let Some(command) = cli.command {
        match command {
            CliCommand::SetOpenAiKey(key) => {
                config::update(|cfg| cfg.openai_api_key = key.trim().to_string())?;
                println!("Stored OpenAI API key in config/app_config.toml.");
                return Ok(());
            }
            CliCommand::ClearOpenAiKey => {
                config::update(|cfg| cfg.openai_api_key.clear())?;
                println!("Cleared OpenAI API key from config/app_config.toml.");
                return Ok(());
            }
            CliCommand::SetAnthropicKey(key) => {
                config::update(|cfg| cfg.anthropic_api_key = key.trim().to_string())?;
                println!("Stored Anthropic API key in config/app_config.toml.");
                return Ok(());
            }
            CliCommand::ClearAnthropicKey => {
                config::update(|cfg| cfg.anthropic_api_key.clear())?;
                println!("Cleared Anthropic API key from config/app_config.toml.");
                return Ok(());
            }
            CliCommand::SetOpenRouterKey(key) => {
                config::update(|cfg| cfg.openrouter_api_key = key.trim().to_string())?;
                println!("Stored OpenRouter API key in config/app_config.toml.");
                return Ok(());
            }
            CliCommand::ClearOpenRouterKey => {
                config::update(|cfg| cfg.openrouter_api_key.clear())?;
                println!("Cleared OpenRouter API key from config/app_config.toml.");
                return Ok(());
            }
            CliCommand::SetDocumentRepository(repository) => {
                config::update(|cfg| cfg.document_repository = repository)?;
                println!("Stored document repository in config/app_config.toml.");
                return Ok(());
            }
            CliCommand::ClearDocumentRepository => {
                config::update(|cfg| {
                    cfg.document_repository = config::DocumentRepositoryKind::None;
                    cfg.document_repository_target.clear();
                })?;
                println!("Cleared document repository from config/app_config.toml.");
                return Ok(());
            }
            CliCommand::SetDocumentRepositoryTarget(target) => {
                let trimmed = target.trim().to_string();
                let current = config::current();
                config::validate_document_repository_target(current.document_repository, &trimmed)
                    .map_err(|err| color_eyre::eyre::eyre!(err))?;
                config::update(|cfg| cfg.document_repository_target = trimmed.clone())?;
                println!("Stored document repository target in config/app_config.toml.");
                return Ok(());
            }
            CliCommand::ClearDocumentRepositoryTarget => {
                config::update(|cfg| cfg.document_repository_target.clear())?;
                println!("Cleared document repository target from config/app_config.toml.");
                return Ok(());
            }
            CliCommand::SetNotionApiToken(token) => {
                config::update(|cfg| cfg.notion_api_token = token.trim().to_string())?;
                println!("Stored Notion API token in config/app_config.toml.");
                return Ok(());
            }
            CliCommand::ClearNotionApiToken => {
                config::update(|cfg| cfg.notion_api_token.clear())?;
                println!("Cleared Notion API token from config/app_config.toml.");
                return Ok(());
            }
            CliCommand::SetLearnChainSiteUrl(url) => {
                config::validate_learnchain_site_url(&url)
                    .map_err(|err| color_eyre::eyre::eyre!(err))?;
                config::update(|cfg| cfg.learnchain_site_url = url.trim().to_string())?;
                println!("Stored LearnChain site URL in config/app_config.toml.");
                return Ok(());
            }
            CliCommand::ClearLearnChainSiteUrl => {
                config::update(|cfg| cfg.learnchain_site_url.clear())?;
                println!("Reset LearnChain site URL in config/app_config.toml.");
                return Ok(());
            }
            CliCommand::SetLearnChainEmail(email) => {
                config::update(|cfg| cfg.learnchain_email = email.trim().to_string())?;
                println!("Stored LearnChain email in config/app_config.toml.");
                return Ok(());
            }
            CliCommand::ClearLearnChainEmail => {
                config::update(|cfg| cfg.learnchain_email.clear())?;
                println!("Cleared LearnChain email from config/app_config.toml.");
                return Ok(());
            }
            CliCommand::SetLearnChainPassword(password) => {
                config::update(|cfg| cfg.learnchain_password = password.clone())?;
                println!("Stored LearnChain password in config/app_config.toml.");
                return Ok(());
            }
            CliCommand::ClearLearnChainPassword => {
                config::update(|cfg| cfg.learnchain_password.clear())?;
                println!("Cleared LearnChain password from config/app_config.toml.");
                return Ok(());
            }
            CliCommand::GenerateCodexDeepDive => {
                if let Err(message) = run_codex_deep_dive_command(
                    codex_thread_id.as_deref(),
                    export_document_repository,
                ) {
                    eprintln!("{}", message);
                    std::process::exit(1);
                }
                return Ok(());
            }
            CliCommand::PrintCodexDeepDiveAction => {
                println!("{}", codex_deep_dive_action_template());
                return Ok(());
            }
            CliCommand::InstallCodexDeepDiveSkill => {
                let installed_path = install_embedded_codex_skill_default()?;
                println!(
                    "Installed LearnChain Codex skill to {}\nRestart Codex to pick up new skills.",
                    installed_path.display()
                );
                return Ok(());
            }
            CliCommand::Help => {
                println!("{}", HELP_TEXT);
                return Ok(());
            }
            CliCommand::Version => {
                println!("learnchain {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
        }
    }

    dotenv().ok();
    color_eyre::install()?;
    log_util::log_debug("App: starting TUI application");
    crossterm::execute!(std::io::stdout(), event::EnableBracketedPaste)?;
    let terminal = ratatui::init();
    let result = App::new().run(terminal);
    let _ = crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste);
    ratatui::restore();
    result
}

#[derive(Debug, Clone)]
struct CodexSessionResolution {
    session: Session,
    fallback_note: Option<String>,
}

fn run_codex_deep_dive_command(
    explicit_thread_id: Option<&str>,
    export_document_repository: bool,
) -> std::result::Result<(), String> {
    config::initialize().map_err(|err| format!("Failed to load configuration: {}", err))?;

    let app_config = config::current();
    let resolved_llm = app_config.resolved_llm();
    let provider = resolved_llm.provider;
    let backend = LlmBackend::from_config(resolved_llm.clone(), "output").map_err(|err| {
        if resolved_llm.api_key.trim().is_empty()
            || (provider == config::AiProvider::OpenRouter
                && resolved_llm.model_name.trim().is_empty())
        {
            provider.setup_help().to_string()
        } else {
            err.to_string()
        }
    })?;

    let source = CodexCliSource::default();
    let resolution = resolve_codex_session(&source, explicit_thread_id)?;
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|err| format!("Failed to build Tokio runtime: {}", err))?;
    let result = runtime
        .block_on(llm::deep_dive::generate_deep_dive_with_progress(
            &backend,
            "Codex CLI",
            resolution.session,
            app_config.deep_dive_sections.clone(),
            None,
        ))
        .map_err(|err| err.to_string())?;

    let export_result = if export_document_repository {
        Some(
            runtime
                .block_on(document_repository::export_deep_dive_document(
                    &app_config,
                    &result.document,
                ))
                .map_err(|err| {
                    format!(
                        "LearnChain deep dive created at {} but export failed: {}",
                        result.document.path.display(),
                        err
                    )
                })?,
        )
    } else {
        None
    };

    println!(
        "{}",
        format_codex_deep_dive_success(
            &result,
            resolution.fallback_note.as_deref(),
            export_result.as_ref(),
        )
    );
    Ok(())
}

fn resolve_codex_session(
    source: &CodexCliSource,
    explicit_thread_id: Option<&str>,
) -> std::result::Result<CodexSessionResolution, String> {
    if let Some(thread_id) = explicit_thread_id.filter(|value| !value.trim().is_empty()) {
        let session = source.load_session_by_id(thread_id)?;
        return Ok(CodexSessionResolution {
            session,
            fallback_note: None,
        });
    }

    if let Ok(thread_id) = std::env::var("CODEX_THREAD_ID") {
        let trimmed = thread_id.trim();
        if !trimmed.is_empty() {
            match source.load_session_by_id(trimmed) {
                Ok(session) => {
                    return Ok(CodexSessionResolution {
                        session,
                        fallback_note: None,
                    });
                }
                Err(err) if err.contains("No Codex session file matched session id") => {
                    let session = source.load_latest_session()?;
                    return Ok(CodexSessionResolution {
                        session,
                        fallback_note: Some(format!(
                            "Note: CODEX_THREAD_ID '{}' was not found on disk, so LearnChain used the most recent Codex session instead.",
                            trimmed
                        )),
                    });
                }
                Err(err) => return Err(err),
            }
        }
    }

    let session = source.load_latest_session()?;
    Ok(CodexSessionResolution {
        session,
        fallback_note: None,
    })
}

fn format_codex_deep_dive_success(
    result: &DeepDiveGenerationResult,
    fallback_note: Option<&str>,
    export_result: Option<&RepositoryExportResult>,
) -> String {
    let mut lines = vec!["LearnChain deep dive created".to_string()];
    lines.push(format!("Path: {}", result.document.path.display()));
    lines.push(format!("Title: {}", result.response.title));
    lines.push(format!("Goal: {}", result.response.goal));

    for accomplishment in result.response.accomplishments.iter().take(3) {
        lines.push(format!("- {}", accomplishment));
    }

    lines.push(format!(
        "Reviewed URLs: {}",
        result.document.metadata.reviewed_url_count
    ));
    lines.push(format!(
        "Fetch failures: {}",
        result.reviewed_source_failures.len()
    ));

    if let Some(export_result) = export_result {
        lines.push(format!("Exported to: {}", export_result.repository_label));
        if let Some(url) = export_result.remote_url.as_deref() {
            lines.push(format!("Export URL: {}", url));
        }
    }

    if let Some(note) = fallback_note {
        lines.push(note.to_string());
    }

    lines.join("\n")
}

fn codex_deep_dive_action_template() -> &'static str {
    r#"Codex custom command template

Name: /learnchain-deep-dive
Description: Generate a LearnChain deep dive for the current Codex session.

Prompt:
Run `learnchain --generate-codex-deep-dive --codex-thread-id "$CODEX_THREAD_ID"` from the workspace root.

If the command succeeds, respond with:
- the saved path
- the deep-dive title
- a short summary of what the deep dive covers

If the command fails, surface the LearnChain error exactly as written, including any setup or configuration guidance.
"#
}

fn install_embedded_codex_skill_default() -> color_eyre::Result<PathBuf> {
    let root = codex_skills_root()?;
    install_embedded_codex_skill_at(&root)
}

fn install_embedded_codex_skill_at(root: &Path) -> color_eyre::Result<PathBuf> {
    let skill_dir = root.join(CODEX_SKILL_NAME);
    let agents_dir = skill_dir.join("agents");

    fs::create_dir_all(&agents_dir)?;
    fs::write(skill_dir.join("SKILL.md"), EMBEDDED_CODEX_SKILL)?;
    fs::write(
        agents_dir.join("openai.yaml"),
        EMBEDDED_CODEX_SKILL_OPENAI_YAML,
    )?;

    Ok(skill_dir)
}

fn codex_skills_root() -> color_eyre::Result<PathBuf> {
    if let Some(codex_home) = env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(codex_home).join("skills"));
    }

    if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home).join(".codex").join("skills"));
    }

    if let Some(user_profile) = env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(user_profile).join(".codex").join("skills"));
    }

    Err(color_eyre::eyre::eyre!(
        "Could not determine Codex home directory. Set CODEX_HOME and retry."
    ))
}

fn parse_cli_options(args: &[String]) -> std::result::Result<CliOptions, String> {
    let mut options = CliOptions::default();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--debug" | "-d" => {
                options.debug_logging = true;
                index += 1;
            }
            "--set-openai-key" => {
                let key = args
                    .get(index + 1)
                    .ok_or_else(|| "Usage: learnchain --set-openai-key <key>".to_string())?;
                set_command(&mut options.command, CliCommand::SetOpenAiKey(key.clone()))?;
                index += 2;
            }
            "--clear-openai-key" => {
                set_command(&mut options.command, CliCommand::ClearOpenAiKey)?;
                index += 1;
            }
            "--set-anthropic-key" => {
                let key = args
                    .get(index + 1)
                    .ok_or_else(|| "Usage: learnchain --set-anthropic-key <key>".to_string())?;
                set_command(
                    &mut options.command,
                    CliCommand::SetAnthropicKey(key.clone()),
                )?;
                index += 2;
            }
            "--clear-anthropic-key" => {
                set_command(&mut options.command, CliCommand::ClearAnthropicKey)?;
                index += 1;
            }
            "--set-openrouter-key" => {
                let key = args
                    .get(index + 1)
                    .ok_or_else(|| "Usage: learnchain --set-openrouter-key <key>".to_string())?;
                set_command(
                    &mut options.command,
                    CliCommand::SetOpenRouterKey(key.clone()),
                )?;
                index += 2;
            }
            "--clear-openrouter-key" => {
                set_command(&mut options.command, CliCommand::ClearOpenRouterKey)?;
                index += 1;
            }
            "--set-document-repository" => {
                let repository = args.get(index + 1).ok_or_else(|| {
                    "Usage: learnchain --set-document-repository <none|notion|learnchain>"
                        .to_string()
                })?;
                let repository =
                    config::DocumentRepositoryKind::parse(repository).ok_or_else(|| {
                        "Document repository must be one of: none, notion, learnchain.".to_string()
                    })?;
                set_command(
                    &mut options.command,
                    CliCommand::SetDocumentRepository(repository),
                )?;
                index += 2;
            }
            "--clear-document-repository" => {
                set_command(&mut options.command, CliCommand::ClearDocumentRepository)?;
                index += 1;
            }
            "--set-document-repository-target" => {
                let target = args.get(index + 1).ok_or_else(|| {
                    "Usage: learnchain --set-document-repository-target <target>".to_string()
                })?;
                set_command(
                    &mut options.command,
                    CliCommand::SetDocumentRepositoryTarget(target.clone()),
                )?;
                index += 2;
            }
            "--clear-document-repository-target" => {
                set_command(
                    &mut options.command,
                    CliCommand::ClearDocumentRepositoryTarget,
                )?;
                index += 1;
            }
            "--set-notion-api-token" => {
                let token = args.get(index + 1).ok_or_else(|| {
                    "Usage: learnchain --set-notion-api-token <token>".to_string()
                })?;
                set_command(
                    &mut options.command,
                    CliCommand::SetNotionApiToken(token.clone()),
                )?;
                index += 2;
            }
            "--clear-notion-api-token" => {
                set_command(&mut options.command, CliCommand::ClearNotionApiToken)?;
                index += 1;
            }
            "--set-learnchain-site-url" => {
                let url = args.get(index + 1).ok_or_else(|| {
                    "Usage: learnchain --set-learnchain-site-url <url>".to_string()
                })?;
                set_command(
                    &mut options.command,
                    CliCommand::SetLearnChainSiteUrl(url.clone()),
                )?;
                index += 2;
            }
            "--clear-learnchain-site-url" => {
                set_command(&mut options.command, CliCommand::ClearLearnChainSiteUrl)?;
                index += 1;
            }
            "--set-learnchain-email" => {
                let email = args.get(index + 1).ok_or_else(|| {
                    "Usage: learnchain --set-learnchain-email <email>".to_string()
                })?;
                set_command(
                    &mut options.command,
                    CliCommand::SetLearnChainEmail(email.clone()),
                )?;
                index += 2;
            }
            "--clear-learnchain-email" => {
                set_command(&mut options.command, CliCommand::ClearLearnChainEmail)?;
                index += 1;
            }
            "--set-learnchain-password" => {
                let password = args.get(index + 1).ok_or_else(|| {
                    "Usage: learnchain --set-learnchain-password <password>".to_string()
                })?;
                set_command(
                    &mut options.command,
                    CliCommand::SetLearnChainPassword(password.clone()),
                )?;
                index += 2;
            }
            "--clear-learnchain-password" => {
                set_command(&mut options.command, CliCommand::ClearLearnChainPassword)?;
                index += 1;
            }
            "--generate-codex-deep-dive" => {
                set_command(&mut options.command, CliCommand::GenerateCodexDeepDive)?;
                index += 1;
            }
            "--codex-thread-id" => {
                let thread_id = args
                    .get(index + 1)
                    .ok_or_else(|| "Usage: learnchain --codex-thread-id <id>".to_string())?;
                options.codex_thread_id = Some(thread_id.clone());
                index += 2;
            }
            "--export-to-document-repository" => {
                options.export_document_repository = true;
                index += 1;
            }
            "--print-codex-deep-dive-action" => {
                set_command(&mut options.command, CliCommand::PrintCodexDeepDiveAction)?;
                index += 1;
            }
            "--install-codex-deep-dive-skill" => {
                set_command(&mut options.command, CliCommand::InstallCodexDeepDiveSkill)?;
                index += 1;
            }
            "--help" | "-h" => {
                set_command(&mut options.command, CliCommand::Help)?;
                index += 1;
            }
            "--version" | "-V" => {
                set_command(&mut options.command, CliCommand::Version)?;
                index += 1;
            }
            value => {
                return Err(format!(
                    "Unrecognized option '{}'. Run `learnchain --help` for usage.",
                    value
                ));
            }
        }
    }

    if options.codex_thread_id.is_some()
        && options.command != Some(CliCommand::GenerateCodexDeepDive)
    {
        return Err(
            "`--codex-thread-id` can only be used with `--generate-codex-deep-dive`.".to_string(),
        );
    }

    if options.export_document_repository
        && options.command != Some(CliCommand::GenerateCodexDeepDive)
    {
        return Err(
            "`--export-to-document-repository` can only be used with `--generate-codex-deep-dive`."
                .to_string(),
        );
    }

    Ok(options)
}

fn set_command(
    slot: &mut Option<CliCommand>,
    command: CliCommand,
) -> std::result::Result<(), String> {
    if slot.is_some() {
        return Err("Multiple commands are not supported in a single invocation.".to_string());
    }
    *slot = Some(command);
    Ok(())
}

/// The main application which holds the state and logic of the application.
#[derive(Debug)]
pub struct App {
    /// Is the application running?
    pub(crate) running: bool,
    /// Current view being displayed.
    pub(crate) view: AppView,
    /// Currently selected index in the main menu.
    pub(crate) menu_index: usize,
    /// Parsed session events filtered for function calls.
    pub(crate) events: Vec<SessionEvent>,
    /// Currently selected event index.
    pub(crate) selected_event: Option<usize>,
    /// All loaded sessions for hierarchical viewing.
    pub(crate) sessions: Vec<Session>,
    /// Currently selected session index in sessions list view.
    pub(crate) selected_session: Option<usize>,
    /// Whether viewing sessions list (true) or events within a session (false).
    pub(crate) viewing_sessions_list: bool,
    /// Absolute path to today's session directory.
    pub(crate) session_dir: PathBuf,
    /// Human-readable label for today's date.
    pub(crate) session_date: String,
    /// Label describing the active session source.
    pub(crate) session_source: String,
    /// Most recent session file for today, if any.
    pub(crate) latest_file: Option<PathBuf>,
    /// Absolute path to the aggregated markdown summary, if generated.
    pub(crate) summary_file: Option<PathBuf>,
    /// Markdown summary content cached in memory.
    pub(crate) summary_content: Option<String>,
    /// Any error encountered while loading files or parsing events.
    pub(crate) error: Option<String>,
    /// Current AI provider selection.
    pub(crate) ai_provider: config::AiProvider,
    /// Lazily configured LLM integration.
    pub(crate) llm_backend: Option<LlmBackend>,
    /// Latest status message related to AI generation requests.
    pub(crate) ai_status: Option<String>,
    /// Indicates whether an AI request is currently running.
    pub(crate) ai_loading: bool,
    /// The active AI task kind, when loading is in progress.
    pub(crate) ai_task_kind: Option<AiTaskKind>,
    /// Spinner frame index for the active loading indicator.
    pub(crate) ai_loading_frame: usize,
    /// When the AI loading started, for elapsed time display.
    pub(crate) ai_loading_start: Option<Instant>,
    /// Current progress percentage (0-100).
    pub(crate) ai_progress_percent: u8,
    /// Current progress stage message.
    pub(crate) ai_progress_message: String,
    /// Receives background AI task updates.
    pub(crate) ai_result_receiver: Option<Receiver<AiTaskMessage>>,
    /// Sends background AI task updates from spawned workers.
    pub(crate) ai_sender: Option<Sender<AiTaskMessage>>,
    /// Receives background document export updates.
    pub(crate) document_export_receiver: Option<Receiver<DocumentExportMessage>>,
    /// Indicates whether a document export is currently running.
    pub(crate) document_export_loading: bool,
    /// Cached learning response from the most recent AI generation.
    pub(crate) learning_response: Option<StructuredLearningResponse>,
    /// Index of the currently selected knowledge group within the learning response.
    pub(crate) learning_group_index: usize,
    /// Index of the currently selected quiz item within the active knowledge group.
    pub(crate) learning_quiz_index: usize,
    /// Index of the currently selected answer option within the active quiz item.
    pub(crate) learning_option_index: usize,
    /// Feedback for the most recent answer selection.
    pub(crate) learning_feedback: Option<String>,
    /// Whether the current quiz summary should be revealed.
    pub(crate) learning_summary_revealed: bool,
    /// Indicates that the correct answer was chosen and we are waiting to advance.
    pub(crate) learning_waiting_for_next: bool,
    /// The active target for the shared session picker.
    pub(crate) session_selection_target: Option<SessionSelectionTarget>,
    /// Index of selected session in shared session picker view.
    pub(crate) session_picker_selected_session: Option<usize>,
    /// Projects grouped by cwd for session selection.
    pub(crate) projects: Vec<Project>,
    /// Index of selected project in shared session picker view.
    pub(crate) session_picker_selected_project: Option<usize>,
    /// Whether viewing projects list (true) or sessions within a project (false).
    pub(crate) session_picker_viewing_projects: bool,
    /// Holds the editable configuration state when rendering the config view.
    pub(crate) config_form: ConfigForm,
    /// Whether artifacts should be written to disk.
    pub(crate) write_output_artifacts: bool,
    /// Currently selected OpenAI model.
    pub(crate) openai_model: config::OpenAiModelKind,
    /// Currently selected Anthropic model.
    pub(crate) anthropic_model: config::AnthropicModelKind,
    /// Currently selected OpenRouter model (free-text).
    pub(crate) openrouter_model: String,
    /// Tracks which quiz questions have already had their first attempt persisted.
    pub(crate) quiz_first_attempts: HashSet<(usize, usize)>,
    /// Tracks first-try correctness for each question (group_index, question_index) -> was_correct.
    pub(crate) quiz_first_attempt_results: std::collections::HashMap<(usize, usize), bool>,
    /// Cached analytics snapshot for the dashboard view.
    pub(crate) analytics_snapshot: Option<KnowledgeAnalytics>,
    /// Any error that occurred when loading analytics data.
    pub(crate) analytics_error: Option<String>,
    /// Timestamp of the most recent analytics refresh.
    pub(crate) analytics_refreshed_at: Option<String>,
    /// Timestamp marker of the most recent session event observed.
    pub(crate) last_event_timestamp: Option<String>,
    /// Timestamp marker at which the last quiz generation consumed events.
    pub(crate) last_quiz_event_timestamp: Option<String>,
    /// Whether the quiz summary screen is being displayed.
    pub(crate) learning_showing_summary: bool,
    /// Summary results for display after quiz completion.
    pub(crate) quiz_summary_results: Vec<QuizSummaryResult>,
    /// Most recently generated or opened deep dive document.
    pub(crate) deep_dive_document: Option<DeepDiveDocument>,
    /// History-loaded deep dive temporarily overlaid on top of the current document.
    pub(crate) deep_dive_history_document: Option<DeepDiveDocument>,
    /// Scroll offset within the active deep dive markdown view.
    pub(crate) deep_dive_scroll: u16,
    /// Previously generated deep-dive artifacts discovered on disk.
    pub(crate) deep_dive_history: Vec<DeepDiveHistoryEntry>,
    /// Selected history row within deep-dive history mode.
    pub(crate) deep_dive_history_selected: Option<usize>,
    /// Whether the deep-dive view is currently showing history list mode.
    pub(crate) deep_dive_showing_history: bool,
    /// All saved artifacts available in the library view.
    pub(crate) library_artifacts: Vec<LibraryArtifactEntry>,
    /// Selected row within the library view.
    pub(crate) library_selected: Option<usize>,
}

/// Stores the result of a single quiz question for the summary screen.
#[derive(Debug, Clone)]
pub(crate) struct QuizSummaryResult {
    pub(crate) question: String,
    pub(crate) correct_answer: String,
    pub(crate) first_try_correct: bool,
}

/// A project groups sessions by their working directory (cwd).
#[derive(Debug, Clone)]
pub(crate) struct Project {
    /// The full cwd path.
    pub(crate) cwd: String,
    /// A short display name (last component of path).
    pub(crate) name: String,
    /// Indices of sessions belonging to this project.
    pub(crate) session_indices: Vec<usize>,
}

impl Project {
    /// Extract cwd from a session's first event content_texts.
    pub(crate) fn extract_cwd(session: &session_sources::Session) -> String {
        if !session.cwd.trim().is_empty() && session.cwd != "Unknown" {
            return session.cwd.clone();
        }

        session
            .events
            .first()
            .and_then(|event| {
                event
                    .content_texts
                    .iter()
                    .find(|text| text.starts_with("cwd: "))
                    .map(|text| text.trim_start_matches("cwd: ").to_string())
            })
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Get short project name from cwd path.
    pub(crate) fn name_from_cwd(cwd: &str) -> String {
        std::path::Path::new(cwd)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(cwd)
            .to_string()
    }

    /// Group sessions into projects by cwd.
    pub(crate) fn group_sessions(sessions: &[session_sources::Session]) -> Vec<Project> {
        use std::collections::HashMap;

        let mut cwd_to_indices: HashMap<String, Vec<usize>> = HashMap::new();

        for (idx, session) in sessions.iter().enumerate() {
            let cwd = Self::extract_cwd(session);
            cwd_to_indices.entry(cwd).or_default().push(idx);
        }

        let mut projects: Vec<Project> = cwd_to_indices
            .into_iter()
            .map(|(cwd, session_indices)| {
                let name = Self::name_from_cwd(&cwd);
                Project {
                    cwd,
                    name,
                    session_indices,
                }
            })
            .collect();

        // Sort projects by most recent session timestamp (descending - most recent first)
        projects.sort_by(|a, b| {
            let a_timestamp = a
                .session_indices
                .first()
                .and_then(|&idx| sessions.get(idx))
                .map(|s| s.timestamp.as_str())
                .unwrap_or("");
            let b_timestamp = b
                .session_indices
                .first()
                .and_then(|&idx| sessions.get(idx))
                .map(|s| s.timestamp.as_str())
                .unwrap_or("");
            // Reverse order for most recent first
            b_timestamp.cmp(a_timestamp)
        });

        projects
    }
}

fn is_llm_setup_error(message: &str) -> bool {
    message.contains("not configured")
}

impl App {
    /// Construct a new instance of [`App`].
    pub fn new() -> Self {
        let mut aggregated_error: Option<String> = None;

        if let Err(err) = config::initialize() {
            Self::push_error(
                &mut aggregated_error,
                format!("Configuration load failed: {}", err),
            );
        }

        let config_snapshot = config::current();
        let write_output_artifacts = config_snapshot.write_output_artifacts;
        let ai_provider = config_snapshot.ai_provider;
        let openai_model = config_snapshot.openai_model;
        let anthropic_model = config_snapshot.anthropic_model;
        let openrouter_model = config_snapshot.openrouter_model.clone();
        let resolved_llm = config_snapshot.resolved_llm();
        let session_manager = SessionManager::from_source(config_snapshot.session_source);
        let session_load = session_manager.load_today_events();

        let llm_backend = match LlmBackend::from_config(resolved_llm.clone(), "output") {
            Ok(generator) => Some(generator),
            Err(err) => {
                let message = err.to_string();
                if !is_llm_setup_error(&message) {
                    Self::push_error(
                        &mut aggregated_error,
                        format!("AI unavailable: {}", message),
                    );
                }
                None
            }
        };

        let mut app = Self {
            running: false,
            view: AppView::Menu,
            menu_index: 0,
            events: Vec::new(),
            selected_event: None,
            sessions: Vec::new(),
            selected_session: None,
            viewing_sessions_list: true,
            session_dir: PathBuf::new(),
            session_date: String::new(),
            session_source: String::new(),
            latest_file: None,
            summary_file: None,
            summary_content: None,
            error: None,
            ai_provider,
            llm_backend,
            ai_status: None,
            ai_loading: false,
            ai_task_kind: None,
            ai_loading_frame: 0,
            ai_loading_start: None,
            ai_progress_percent: 0,
            ai_progress_message: String::new(),
            ai_result_receiver: None,
            ai_sender: None,
            document_export_receiver: None,
            document_export_loading: false,
            learning_response: None,
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
            config_form: ConfigForm::from_config(config_snapshot.clone()),
            write_output_artifacts,
            openai_model,
            anthropic_model,
            openrouter_model,
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
        };

        app.apply_session_load(session_load);

        if app.llm_backend.is_none() {
            app.ai_status = Some(ai_provider.setup_help().to_string());
        } else {
            app.ai_status = None;
        }

        if let Some(error) = aggregated_error {
            Self::push_error(&mut app.error, error);
        }

        app
    }

    /// Run the application's main loop.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;
        let tick_rate = Duration::from_millis(120);
        while self.running {
            poll_ai_messages(&mut self);
            poll_export_messages(&mut self);
            terminal.draw(|frame| self.render(frame))?;
            self.handle_crossterm_events(tick_rate)?;
        }
        Ok(())
    }

    /// Dispatch rendering based on the active view.
    fn render(&mut self, frame: &mut Frame) {
        UiRenderer::new(self).render(frame);
    }

    fn apply_session_load(&mut self, load: SessionLoad) {
        self.session_source = load.source;
        self.session_date = load.session_date;
        self.session_dir = load.session_dir;
        self.latest_file = load.latest_file;
        self.events = load.events;
        self.selected_event = if self.events.is_empty() {
            None
        } else {
            Some(0)
        };
        self.error = load.error;

        let output_manager = OutputManager::new();
        let artifact = output_manager.write_markdown_summary(
            &self.events,
            &self.session_date,
            self.latest_file.as_deref(),
            self.write_output_artifacts,
        );
        self.summary_file = artifact.path;
        self.summary_content = Some(artifact.content);
        if let Some(summary_error) = artifact.error {
            Self::push_error(&mut self.error, summary_error);
        }

        self.last_event_timestamp = self.events.last().map(|event| event.timestamp.clone());
    }

    pub(crate) fn reload_session_from_config(&mut self) {
        let config_snapshot = config::current();
        self.write_output_artifacts = config_snapshot.write_output_artifacts;
        self.ai_provider = config_snapshot.ai_provider;
        self.openai_model = config_snapshot.openai_model;
        self.anthropic_model = config_snapshot.anthropic_model;
        self.openrouter_model = config_snapshot.openrouter_model.clone();
        let resolved_llm = config_snapshot.resolved_llm();

        match LlmBackend::from_config(resolved_llm, "output") {
            Ok(generator) => {
                self.llm_backend = Some(generator);
                self.ai_status = None;
            }
            Err(err) => {
                let message = err.to_string();
                self.llm_backend = None;
                if !is_llm_setup_error(&message) {
                    App::push_error(&mut self.error, format!("AI unavailable: {}", message));
                }
                self.ai_status = Some(self.ai_provider.setup_help().to_string());
            }
        }

        let manager = SessionManager::from_source(config_snapshot.session_source);
        let load = manager.load_today_events();
        self.apply_session_load(load);

        // Clear cached sessions so they reload with new source next time
        self.sessions.clear();
        self.selected_session = None;
    }

    /// Reads the crossterm events and updates the state of [`App`].
    fn handle_crossterm_events(&mut self, tick_rate: Duration) -> Result<()> {
        if event::poll(tick_rate)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key),
                Event::Paste(text) => self.on_paste_event(text),
                Event::Mouse(_) => {}
                Event::Resize(_, _) => {}
                _ => {}
            }
            poll_ai_messages(self);
            poll_export_messages(self);
        } else {
            self.on_tick();
        }
        Ok(())
    }

    fn on_tick(&mut self) {
        if self.ai_loading {
            self.ai_loading_frame = (self.ai_loading_frame + 1) % AI_LOADING_FRAMES.len();
            self.update_loading_status();
        }
        poll_ai_messages(self);
        poll_export_messages(self);
    }

    /// Handles the key events and updates the state of [`App`].
    fn on_key_event(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc | KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => self.quit(),
            _ => match self.view {
                AppView::Menu => MenuManager::new(self).handle_menu_key(key),
                AppView::Events => MenuManager::new(self).handle_events_key(key),
                AppView::SessionPicker => SessionPickerManager::new(self).handle_key(key),
                AppView::Learning => LearningManager::new(self).handle_key(key),
                AppView::DeepDive => DeepDiveManager::new(self).handle_key(key),
                AppView::Library => LibraryManager::new(self).handle_key(key),
                AppView::Config => ConfigManager::new(self).handle_key(key),
                AppView::Analytics => AnalyticsManager::new(self).handle_key(key),
            },
        }
    }

    fn on_paste_event(&mut self, text: String) {
        if matches!(self.view, AppView::Config) {
            ConfigManager::new(self).handle_paste(&text);
        }
    }

    pub(crate) fn return_to_menu(&mut self) {
        if matches!(self.view, AppView::Config) {
            self.config_form = ConfigForm::from_config(config::current());
        }
        self.learning_showing_summary = false;
        self.session_selection_target = None;
        self.session_picker_selected_session = None;
        self.session_picker_viewing_projects = true;
        self.deep_dive_showing_history = false;
        self.deep_dive_history_document = None;
        self.deep_dive_scroll = 0;
        self.library_selected = None;
        self.quiz_summary_results.clear();
        self.view = AppView::Menu;
    }

    /// Set running to false to quit the application.
    fn quit(&mut self) {
        self.running = false;
    }

    /// Append a message to an optional error slot.
    pub(crate) fn push_error(slot: &mut Option<String>, message: String) {
        if let Some(existing) = slot {
            existing.push_str(" | ");
            existing.push_str(&message);
        } else {
            *slot = Some(message);
        }
    }

    pub(crate) fn active_deep_dive_document(&self) -> Option<&DeepDiveDocument> {
        self.deep_dive_history_document
            .as_ref()
            .or(self.deep_dive_document.as_ref())
    }

    pub(crate) fn show_deep_dive_document(&mut self, document: DeepDiveDocument) {
        self.deep_dive_document = Some(document);
        self.deep_dive_history_document = None;
        self.deep_dive_showing_history = false;
        self.deep_dive_scroll = 0;
        self.view = AppView::DeepDive;
    }

    pub(crate) fn show_history_deep_dive_document(&mut self, document: DeepDiveDocument) {
        self.deep_dive_history_document = Some(document);
        self.deep_dive_showing_history = false;
        self.deep_dive_scroll = 0;
        self.view = AppView::DeepDive;
    }

    pub(crate) fn record_quiz_first_attempt(
        &mut self,
        group_index: usize,
        question_index: usize,
        correct: bool,
    ) {
        if !self
            .quiz_first_attempts
            .insert((group_index, question_index))
        {
            return;
        }
        // Track the first-try result for summary screen
        self.quiz_first_attempt_results
            .insert((group_index, question_index), correct);

        let Some(response) = self.learning_response.as_ref() else {
            crate::log_util::log_debug(
                "App: cannot record quiz attempt because no learning response is loaded",
            );
            return;
        };
        let Some(group) = response.response.get(group_index) else {
            crate::log_util::log_debug(&format!(
                "App: quiz attempt group index {} out of bounds",
                group_index
            ));
            return;
        };
        let Some(question) = group.quiz.get(question_index) else {
            crate::log_util::log_debug(&format!(
                "App: quiz attempt question index {} out of bounds for group {}",
                question_index, group_index
            ));
            return;
        };

        let language = if group.knowledge_type_language.trim().is_empty() {
            None
        } else {
            Some(group.knowledge_type_language.as_str())
        };

        match crate::knowledge_store::record_quiz_first_attempt(
            &self.session_date,
            &group.knowledge_type_group,
            language,
            &question.question,
            correct,
        ) {
            Ok(_) => {
                crate::log_util::log_debug(&format!(
                    "App: recorded first attempt for '{}' (correct: {})",
                    question.question, correct
                ));
                self.analytics_snapshot = None;
                self.analytics_refreshed_at = None;
            }
            Err(err) => {
                Self::push_error(
                    &mut self.error,
                    format!("Failed to record quiz attempt: {}", err),
                );
                crate::log_util::log_debug(&format!(
                    "App: failed to persist quiz attempt for '{}': {}",
                    question.question, err
                ));
            }
        }
    }

    pub(crate) fn sync_new_session_events(&mut self) -> Result<(), String> {
        if self.last_quiz_event_timestamp.is_none() {
            return Ok(());
        }

        let baseline = self
            .last_quiz_event_timestamp
            .as_deref()
            .expect("last_quiz_event_timestamp checked to be Some");
        let config_snapshot = config::current();
        let manager = SessionManager::from_source(config_snapshot.session_source);
        let load = manager.load_new_events(self.latest_file.as_deref(), Some(baseline));

        if load.events.is_empty() {
            let message = load.error.unwrap_or_else(|| {
                "No new session events available yet. Run another coding session before generating a new quiz.".to_string()
            });
            return Err(message);
        }

        let mut combined_events = self.events.clone();
        combined_events.extend(load.events.iter().cloned());

        let mut merged_load = load;
        merged_load.events = combined_events;
        if merged_load.latest_file.is_none() {
            merged_load.latest_file = self.latest_file.clone();
        }

        self.apply_session_load(merged_load);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parse_cli_options_supports_debug_flag_without_command() {
        let options = parse_cli_options(&args(&["--debug"])).unwrap();
        assert!(options.debug_logging);
        assert!(options.command.is_none());
    }

    #[test]
    fn parse_cli_options_supports_debug_flag_with_command() {
        let options =
            parse_cli_options(&args(&["--debug", "--set-openai-key", "secret-key"])).unwrap();
        assert!(options.debug_logging);
        assert_eq!(
            options.command,
            Some(CliCommand::SetOpenAiKey("secret-key".to_string()))
        );
    }

    #[test]
    fn parse_cli_options_rejects_multiple_commands() {
        let error = parse_cli_options(&args(&["--help", "--version"])).unwrap_err();
        assert!(error.contains("Multiple commands"));
    }

    #[test]
    fn parse_cli_options_supports_setting_document_repository_target() {
        let options =
            parse_cli_options(&args(&["--set-document-repository-target", "database/abc"]))
                .unwrap();

        assert_eq!(
            options.command,
            Some(CliCommand::SetDocumentRepositoryTarget(
                "database/abc".to_string()
            ))
        );
    }

    #[test]
    fn parse_cli_options_supports_clearing_document_repository_target() {
        let options = parse_cli_options(&args(&["--clear-document-repository-target"])).unwrap();
        assert_eq!(
            options.command,
            Some(CliCommand::ClearDocumentRepositoryTarget)
        );
    }

    #[test]
    fn parse_cli_options_supports_setting_notion_api_token() {
        let options = parse_cli_options(&args(&["--set-notion-api-token", "secret_test"])).unwrap();
        assert_eq!(
            options.command,
            Some(CliCommand::SetNotionApiToken("secret_test".to_string()))
        );
    }

    #[test]
    fn parse_cli_options_supports_clearing_notion_api_token() {
        let options = parse_cli_options(&args(&["--clear-notion-api-token"])).unwrap();
        assert_eq!(options.command, Some(CliCommand::ClearNotionApiToken));
    }

    #[test]
    fn parse_cli_options_supports_setting_document_repository() {
        let options = parse_cli_options(&args(&["--set-document-repository", "notion"])).unwrap();
        assert_eq!(
            options.command,
            Some(CliCommand::SetDocumentRepository(
                config::DocumentRepositoryKind::Notion
            ))
        );
    }

    #[test]
    fn parse_cli_options_supports_setting_learnchain_repository() {
        let options =
            parse_cli_options(&args(&["--set-document-repository", "learnchain"])).unwrap();
        assert_eq!(
            options.command,
            Some(CliCommand::SetDocumentRepository(
                config::DocumentRepositoryKind::LearnChain
            ))
        );
    }

    #[test]
    fn parse_cli_options_supports_setting_learnchain_site_url() {
        let options = parse_cli_options(&args(&[
            "--set-learnchain-site-url",
            "http://localhost:3000",
        ]))
        .unwrap();
        assert_eq!(
            options.command,
            Some(CliCommand::SetLearnChainSiteUrl(
                "http://localhost:3000".to_string()
            ))
        );
    }

    #[test]
    fn parse_cli_options_supports_setting_learnchain_email() {
        let options =
            parse_cli_options(&args(&["--set-learnchain-email", "learner@example.com"])).unwrap();
        assert_eq!(
            options.command,
            Some(CliCommand::SetLearnChainEmail(
                "learner@example.com".to_string()
            ))
        );
    }

    #[test]
    fn parse_cli_options_supports_setting_learnchain_password() {
        let options =
            parse_cli_options(&args(&["--set-learnchain-password", "secret-pass"])).unwrap();
        assert_eq!(
            options.command,
            Some(CliCommand::SetLearnChainPassword("secret-pass".to_string()))
        );
    }

    #[test]
    fn parse_cli_options_supports_clearing_document_repository() {
        let options = parse_cli_options(&args(&["--clear-document-repository"])).unwrap();
        assert_eq!(options.command, Some(CliCommand::ClearDocumentRepository));
    }

    #[test]
    fn parse_cli_options_supports_generating_codex_deep_dive() {
        let options = parse_cli_options(&args(&["--generate-codex-deep-dive"])).unwrap();
        assert_eq!(options.command, Some(CliCommand::GenerateCodexDeepDive));
        assert!(options.codex_thread_id.is_none());
    }

    #[test]
    fn parse_cli_options_supports_codex_thread_id_with_deep_dive_command() {
        let options = parse_cli_options(&args(&[
            "--generate-codex-deep-dive",
            "--codex-thread-id",
            "thread-123",
        ]))
        .unwrap();
        assert_eq!(options.command, Some(CliCommand::GenerateCodexDeepDive));
        assert_eq!(options.codex_thread_id.as_deref(), Some("thread-123"));
    }

    #[test]
    fn parse_cli_options_supports_exporting_document_repository_with_deep_dive_command() {
        let options = parse_cli_options(&args(&[
            "--generate-codex-deep-dive",
            "--export-to-document-repository",
        ]))
        .unwrap();
        assert_eq!(options.command, Some(CliCommand::GenerateCodexDeepDive));
        assert!(options.export_document_repository);
    }

    #[test]
    fn parse_cli_options_rejects_codex_thread_id_without_deep_dive_command() {
        let error = parse_cli_options(&args(&["--codex-thread-id", "thread-123"])).unwrap_err();
        assert!(error.contains("--generate-codex-deep-dive"));
    }

    #[test]
    fn parse_cli_options_rejects_export_without_deep_dive_command() {
        let error = parse_cli_options(&args(&["--export-to-document-repository"])).unwrap_err();
        assert!(error.contains("--generate-codex-deep-dive"));
    }

    #[test]
    fn parse_cli_options_supports_printing_codex_action_template() {
        let options = parse_cli_options(&args(&["--print-codex-deep-dive-action"])).unwrap();
        assert_eq!(options.command, Some(CliCommand::PrintCodexDeepDiveAction));
    }

    #[test]
    fn parse_cli_options_supports_installing_codex_skill() {
        let options = parse_cli_options(&args(&["--install-codex-deep-dive-skill"])).unwrap();
        assert_eq!(options.command, Some(CliCommand::InstallCodexDeepDiveSkill));
    }

    #[test]
    fn codex_action_template_contains_expected_command() {
        let template = codex_deep_dive_action_template();
        assert!(template.contains("/learnchain-deep-dive"));
        assert!(template.contains(
            "learnchain --generate-codex-deep-dive --codex-thread-id \"$CODEX_THREAD_ID\""
        ));
    }

    #[test]
    fn codex_deep_dive_success_formatter_includes_key_fields() {
        let result = DeepDiveGenerationResult {
            document: DeepDiveDocument {
                path: PathBuf::from("/tmp/deep-dive.md"),
                ..DeepDiveDocument::default()
            },
            response: llm::StructuredDeepDiveResponse {
                title: "Session Deep Dive".to_string(),
                goal: "Ship the feature".to_string(),
                accomplishments: vec![
                    "Added CLI support".to_string(),
                    "Added session lookup".to_string(),
                ],
                ..llm::StructuredDeepDiveResponse::default()
            },
            reviewed_source_failures: vec!["https://example.com".to_string()],
            ..DeepDiveGenerationResult::default()
        };

        let export_result = RepositoryExportResult {
            repository_label: "Notion".to_string(),
            document_title: "Session Deep Dive".to_string(),
            remote_url: Some("https://notion.example/doc".to_string()),
        };
        let formatted = format_codex_deep_dive_success(
            &result,
            Some("Note: fallback used."),
            Some(&export_result),
        );
        assert!(formatted.contains("LearnChain deep dive created"));
        assert!(formatted.contains("Path: /tmp/deep-dive.md"));
        assert!(formatted.contains("Title: Session Deep Dive"));
        assert!(formatted.contains("Goal: Ship the feature"));
        assert!(formatted.contains("- Added CLI support"));
        assert!(formatted.contains("Fetch failures: 1"));
        assert!(formatted.contains("Exported to: Notion"));
        assert!(formatted.contains("Export URL: https://notion.example/doc"));
        assert!(formatted.contains("Note: fallback used."));
    }

    #[test]
    fn install_embedded_codex_skill_writes_skill_files() {
        let temp_dir = tempdir().unwrap();
        let skill_dir = install_embedded_codex_skill_at(temp_dir.path()).unwrap();

        assert_eq!(skill_dir, temp_dir.path().join(CODEX_SKILL_NAME));
        let skill_markdown = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        let openai_yaml = std::fs::read_to_string(skill_dir.join("agents/openai.yaml")).unwrap();

        assert!(skill_markdown.contains("name: learnchain-deep-dive"));
        assert!(skill_markdown.contains("learnchain --generate-codex-deep-dive"));
        assert!(openai_yaml.contains("display_name: \"LearnChain Deep Dive\""));
        assert!(openai_yaml.contains("default_prompt:"));
    }
}
