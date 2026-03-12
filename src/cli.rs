use crate::config;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliAction {
    RunTui,
    Execute(CommandAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedCli {
    pub(crate) debug_logging: bool,
    pub(crate) action: CliAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandAction {
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
    GenerateCodexDeepDive {
        thread_id: Option<String>,
        context: Option<String>,
        export_document_repository: bool,
    },
    GenerateClaudeDeepDive {
        session_id: Option<String>,
        context: Option<String>,
        export_document_repository: bool,
    },
    PrintCodexDeepDiveAction,
    InstallCodexDeepDiveSkill,
    InstallClaudeDeepDiveSkill,
}

#[derive(Debug, Parser)]
#[command(name = "learnchain", version)]
struct CliArgs {
    #[arg(
        short = 'd',
        long = "debug",
        global = true,
        help = "write runtime debug logs to output/learnchain-debug.log"
    )]
    debug_logging: bool,
    #[command(subcommand)]
    command: Option<TopLevelCommand>,
}

#[derive(Debug, Subcommand)]
enum TopLevelCommand {
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    DeepDive {
        #[command(subcommand)]
        command: DeepDiveCommand,
    },
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    Action {
        #[command(subcommand)]
        command: ActionCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Set {
        #[command(subcommand)]
        target: ConfigSetCommand,
    },
    Clear {
        #[command(subcommand)]
        target: ConfigClearCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigSetCommand {
    OpenaiKey {
        #[arg(value_name = "key")]
        key: String,
    },
    AnthropicKey {
        #[arg(value_name = "key")]
        key: String,
    },
    OpenrouterKey {
        #[arg(value_name = "key")]
        key: String,
    },
    Repository {
        #[arg(value_name = "repository")]
        repository: RepositoryKindArg,
    },
    RepositoryTarget {
        #[arg(value_name = "target")]
        target: String,
    },
    NotionToken {
        #[arg(value_name = "token")]
        token: String,
    },
    LearnchainSiteUrl {
        #[arg(value_name = "url")]
        url: String,
    },
    LearnchainEmail {
        #[arg(value_name = "email")]
        email: String,
    },
    LearnchainPassword {
        #[arg(value_name = "password")]
        password: String,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigClearCommand {
    OpenaiKey,
    AnthropicKey,
    OpenrouterKey,
    Repository,
    RepositoryTarget,
    NotionToken,
    LearnchainSiteUrl,
    LearnchainEmail,
    LearnchainPassword,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RepositoryKindArg {
    None,
    Notion,
    Learnchain,
}

impl From<RepositoryKindArg> for config::DocumentRepositoryKind {
    fn from(value: RepositoryKindArg) -> Self {
        match value {
            RepositoryKindArg::None => Self::None,
            RepositoryKindArg::Notion => Self::Notion,
            RepositoryKindArg::Learnchain => Self::LearnChain,
        }
    }
}

#[derive(Debug, Subcommand)]
enum DeepDiveCommand {
    Generate {
        #[command(subcommand)]
        target: DeepDiveGenerateCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DeepDiveGenerateCommand {
    Codex {
        #[arg(long = "thread-id", value_name = "id")]
        thread_id: Option<String>,
        #[arg(long = "context", value_name = "text")]
        context: Option<String>,
        #[arg(long = "export")]
        export_document_repository: bool,
    },
    Claude {
        #[arg(long = "session-id", value_name = "id")]
        session_id: Option<String>,
        #[arg(long = "context", value_name = "text")]
        context: Option<String>,
        #[arg(long = "export")]
        export_document_repository: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SkillCommand {
    Install {
        #[command(subcommand)]
        target: SkillInstallTarget,
    },
}

#[derive(Debug, Subcommand)]
enum SkillInstallTarget {
    Codex,
    Claude,
}

#[derive(Debug, Subcommand)]
enum ActionCommand {
    Print {
        #[command(subcommand)]
        target: ActionPrintTarget,
    },
}

#[derive(Debug, Subcommand)]
enum ActionPrintTarget {
    Codex,
}

pub(crate) fn parse() -> ParsedCli {
    CliArgs::parse().into()
}

#[cfg(test)]
pub(crate) fn try_parse_from<I, T>(iter: I) -> Result<ParsedCli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    CliArgs::try_parse_from(iter).map(Into::into)
}

impl From<CliArgs> for ParsedCli {
    fn from(value: CliArgs) -> Self {
        let action = match value.command {
            None => CliAction::RunTui,
            Some(TopLevelCommand::Config { command }) => CliAction::Execute(match command {
                ConfigCommand::Set { target } => match target {
                    ConfigSetCommand::OpenaiKey { key } => CommandAction::SetOpenAiKey(key),
                    ConfigSetCommand::AnthropicKey { key } => CommandAction::SetAnthropicKey(key),
                    ConfigSetCommand::OpenrouterKey { key } => CommandAction::SetOpenRouterKey(key),
                    ConfigSetCommand::Repository { repository } => {
                        CommandAction::SetDocumentRepository(repository.into())
                    }
                    ConfigSetCommand::RepositoryTarget { target } => {
                        CommandAction::SetDocumentRepositoryTarget(target)
                    }
                    ConfigSetCommand::NotionToken { token } => {
                        CommandAction::SetNotionApiToken(token)
                    }
                    ConfigSetCommand::LearnchainSiteUrl { url } => {
                        CommandAction::SetLearnChainSiteUrl(url)
                    }
                    ConfigSetCommand::LearnchainEmail { email } => {
                        CommandAction::SetLearnChainEmail(email)
                    }
                    ConfigSetCommand::LearnchainPassword { password } => {
                        CommandAction::SetLearnChainPassword(password)
                    }
                },
                ConfigCommand::Clear { target } => match target {
                    ConfigClearCommand::OpenaiKey => CommandAction::ClearOpenAiKey,
                    ConfigClearCommand::AnthropicKey => CommandAction::ClearAnthropicKey,
                    ConfigClearCommand::OpenrouterKey => CommandAction::ClearOpenRouterKey,
                    ConfigClearCommand::Repository => CommandAction::ClearDocumentRepository,
                    ConfigClearCommand::RepositoryTarget => {
                        CommandAction::ClearDocumentRepositoryTarget
                    }
                    ConfigClearCommand::NotionToken => CommandAction::ClearNotionApiToken,
                    ConfigClearCommand::LearnchainSiteUrl => CommandAction::ClearLearnChainSiteUrl,
                    ConfigClearCommand::LearnchainEmail => CommandAction::ClearLearnChainEmail,
                    ConfigClearCommand::LearnchainPassword => {
                        CommandAction::ClearLearnChainPassword
                    }
                },
            }),
            Some(TopLevelCommand::DeepDive { command }) => CliAction::Execute(match command {
                DeepDiveCommand::Generate { target } => match target {
                    DeepDiveGenerateCommand::Codex {
                        thread_id,
                        context,
                        export_document_repository,
                    } => CommandAction::GenerateCodexDeepDive {
                        thread_id,
                        context,
                        export_document_repository,
                    },
                    DeepDiveGenerateCommand::Claude {
                        session_id,
                        context,
                        export_document_repository,
                    } => CommandAction::GenerateClaudeDeepDive {
                        session_id,
                        context,
                        export_document_repository,
                    },
                },
            }),
            Some(TopLevelCommand::Skill { command }) => CliAction::Execute(match command {
                SkillCommand::Install { target } => match target {
                    SkillInstallTarget::Codex => CommandAction::InstallCodexDeepDiveSkill,
                    SkillInstallTarget::Claude => CommandAction::InstallClaudeDeepDiveSkill,
                },
            }),
            Some(TopLevelCommand::Action { command }) => CliAction::Execute(match command {
                ActionCommand::Print { target } => match target {
                    ActionPrintTarget::Codex => CommandAction::PrintCodexDeepDiveAction,
                },
            }),
        };

        Self {
            debug_logging: value.debug_logging,
            action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> ParsedCli {
        try_parse_from(values).unwrap()
    }

    #[test]
    fn supports_debug_flag_without_command() {
        let cli = parse(&["learnchain", "--debug"]);
        assert!(cli.debug_logging);
        assert_eq!(cli.action, CliAction::RunTui);
    }

    #[test]
    fn supports_debug_flag_with_command() {
        let cli = parse(&[
            "learnchain",
            "--debug",
            "config",
            "set",
            "openai-key",
            "secret",
        ]);
        assert!(cli.debug_logging);
        assert_eq!(
            cli.action,
            CliAction::Execute(CommandAction::SetOpenAiKey("secret".to_string()))
        );
    }

    #[test]
    fn supports_setting_each_config_value() {
        assert_eq!(
            parse(&[
                "learnchain",
                "config",
                "set",
                "repository-target",
                "database/abc"
            ])
            .action,
            CliAction::Execute(CommandAction::SetDocumentRepositoryTarget(
                "database/abc".to_string()
            ))
        );
        assert_eq!(
            parse(&["learnchain", "config", "set", "notion-token", "secret_test"]).action,
            CliAction::Execute(CommandAction::SetNotionApiToken("secret_test".to_string()))
        );
        assert_eq!(
            parse(&["learnchain", "config", "set", "repository", "notion"]).action,
            CliAction::Execute(CommandAction::SetDocumentRepository(
                config::DocumentRepositoryKind::Notion
            ))
        );
        assert_eq!(
            parse(&[
                "learnchain",
                "config",
                "set",
                "learnchain-site-url",
                "http://localhost:3000"
            ])
            .action,
            CliAction::Execute(CommandAction::SetLearnChainSiteUrl(
                "http://localhost:3000".to_string()
            ))
        );
        assert_eq!(
            parse(&[
                "learnchain",
                "config",
                "set",
                "learnchain-email",
                "learner@example.com"
            ])
            .action,
            CliAction::Execute(CommandAction::SetLearnChainEmail(
                "learner@example.com".to_string()
            ))
        );
        assert_eq!(
            parse(&[
                "learnchain",
                "config",
                "set",
                "learnchain-password",
                "secret-pass"
            ])
            .action,
            CliAction::Execute(CommandAction::SetLearnChainPassword(
                "secret-pass".to_string()
            ))
        );
    }

    #[test]
    fn supports_clearing_each_config_value() {
        assert_eq!(
            parse(&["learnchain", "config", "clear", "repository-target"]).action,
            CliAction::Execute(CommandAction::ClearDocumentRepositoryTarget)
        );
        assert_eq!(
            parse(&["learnchain", "config", "clear", "notion-token"]).action,
            CliAction::Execute(CommandAction::ClearNotionApiToken)
        );
        assert_eq!(
            parse(&["learnchain", "config", "clear", "repository"]).action,
            CliAction::Execute(CommandAction::ClearDocumentRepository)
        );
    }

    #[test]
    fn supports_codex_deep_dive_generation() {
        let cli = parse(&[
            "learnchain",
            "deep-dive",
            "generate",
            "codex",
            "--thread-id",
            "thread-123",
            "--context",
            "Focus on architecture tradeoffs",
            "--export",
        ]);
        assert_eq!(
            cli.action,
            CliAction::Execute(CommandAction::GenerateCodexDeepDive {
                thread_id: Some("thread-123".to_string()),
                context: Some("Focus on architecture tradeoffs".to_string()),
                export_document_repository: true,
            })
        );
    }

    #[test]
    fn supports_claude_deep_dive_generation() {
        let cli = parse(&[
            "learnchain",
            "deep-dive",
            "generate",
            "claude",
            "--session-id",
            "session-123",
            "--context",
            "Focus on auth flow",
            "--export",
        ]);
        assert_eq!(
            cli.action,
            CliAction::Execute(CommandAction::GenerateClaudeDeepDive {
                session_id: Some("session-123".to_string()),
                context: Some("Focus on auth flow".to_string()),
                export_document_repository: true,
            })
        );
    }

    #[test]
    fn rejects_cross_agent_flags() {
        let error = try_parse_from([
            "learnchain",
            "deep-dive",
            "generate",
            "codex",
            "--session-id",
            "session-123",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn rejects_legacy_flat_flags() {
        let error = try_parse_from(["learnchain", "--generate-codex-deep-dive"]).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn supports_action_and_skill_commands() {
        assert_eq!(
            parse(&["learnchain", "action", "print", "codex"]).action,
            CliAction::Execute(CommandAction::PrintCodexDeepDiveAction)
        );
        assert_eq!(
            parse(&["learnchain", "skill", "install", "codex"]).action,
            CliAction::Execute(CommandAction::InstallCodexDeepDiveSkill)
        );
        assert_eq!(
            parse(&["learnchain", "skill", "install", "claude"]).action,
            CliAction::Execute(CommandAction::InstallClaudeDeepDiveSkill)
        );
    }
}
