use std::{sync::mpsc, thread};

use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    App, DocumentExportMessage,
    config::{self, AppConfig, DocumentRepositoryKind},
    llm::{DeepDiveDocument, StructuredLearningResponse},
    log_util::log_debug,
    output_manager::{LibraryArtifactEntry, OutputManager},
};

const NOTION_API_BASE: &str = "https://api.notion.com/v1";
const NOTION_VERSION: &str = "2025-09-03";
const NOTION_BLOCK_BATCH_SIZE: usize = 100;
const NOTION_TEXT_LIMIT: usize = 1800;
const LEARNCHAIN_CLI_EXCHANGE_PATH: &str = "/api/auth/cli/exchange";
const LEARNCHAIN_CLI_LOGIN_PATH: &str = "/api/auth/cli/login";
const LEARNCHAIN_CLI_REFRESH_PATH: &str = "/api/auth/cli/refresh";
const LEARNCHAIN_DOCUMENTS_PATH: &str = "/api/documents";

#[derive(Debug, Clone)]
pub(crate) struct RepositoryExportResult {
    pub(crate) repository_label: String,
    pub(crate) document_title: String,
    pub(crate) remote_url: Option<String>,
}

impl RepositoryExportResult {
    fn status_message(&self) -> String {
        match self.remote_url.as_deref() {
            Some(url) => format!(
                "Exported \"{}\" to {}: {}",
                self.document_title, self.repository_label, url
            ),
            None => format!(
                "Exported \"{}\" to {}.",
                self.document_title, self.repository_label
            ),
        }
    }
}

#[derive(Debug, Clone)]
struct ExportableDocument {
    title: String,
    markdown: String,
}

#[derive(Debug, Clone)]
enum NotionParent {
    Page { id: String },
    DataSource { id: String, title_property: String },
}

#[derive(Debug, Clone)]
struct NotionCreatedPage {
    id: String,
    url: Option<String>,
}

#[derive(Debug, Clone)]
struct NotionDataSourceTarget {
    id: String,
    title_property: String,
}

#[derive(Debug)]
struct LearnChainClient {
    client: Client,
    site_url: String,
    access_token: String,
    refresh_token: String,
    email: String,
    password: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LearnChainStoredSession {
    pub(crate) account_label: String,
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct LearnChainUploadEnvelope {
    document: LearnChainUploadedDocument,
}

#[derive(Debug, Deserialize)]
struct LearnChainUploadedDocument {
    id: String,
}

#[derive(Debug, Deserialize)]
struct LearnChainApiErrorEnvelope {
    error: LearnChainApiError,
}

#[derive(Debug, Deserialize)]
struct LearnChainApiError {
    code: String,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LearnChainAuthEnvelope {
    session: LearnChainAuthSession,
    user: LearnChainAuthUser,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LearnChainAuthSession {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    #[serde(default)]
    expires_at: Option<u64>,
    token_type: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LearnChainAuthUser {
    id: String,
    email: Option<String>,
    username: Option<String>,
}

enum LearnChainUploadError {
    Unauthorized,
    Message(String),
}

#[derive(Debug)]
struct NotionClient {
    client: Client,
    token: String,
}

impl NotionClient {
    fn new(token: String) -> Result<Self, String> {
        let client = Client::builder()
            .user_agent(format!("learnchain/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| format!("failed to build Notion client: {}", err))?;
        Ok(Self { client, token })
    }

    async fn export_document(
        &self,
        config: &AppConfig,
        document: ExportableDocument,
    ) -> Result<RepositoryExportResult, String> {
        let parent = self
            .resolve_parent(&config.document_repository_target)
            .await?;
        let blocks = markdown_to_notion_blocks(&document.markdown);
        let page = self.create_page(&parent, &document.title).await?;
        if !blocks.is_empty() {
            self.append_blocks(&page.id, &blocks).await?;
        }

        Ok(RepositoryExportResult {
            repository_label: config.document_repository.label().to_string(),
            document_title: document.title,
            remote_url: page.url,
        })
    }

    async fn resolve_parent(&self, target: &str) -> Result<NotionParent, String> {
        let normalized = extract_notion_id(target)
            .or_else(|| normalize_notion_id(target.trim()))
            .ok_or_else(|| {
                "Could not parse a Notion page or database identifier from the configured target."
                    .to_string()
            })?;

        if self.page_exists(&normalized).await? {
            return Ok(NotionParent::Page { id: normalized });
        }

        if let Some(target) = self.resolve_database_target(&normalized).await? {
            return Ok(NotionParent::DataSource {
                id: target.id,
                title_property: target.title_property,
            });
        }

        if let Some(target) = self.resolve_data_source_target(&normalized).await? {
            return Ok(NotionParent::DataSource {
                id: target.id,
                title_property: target.title_property,
            });
        }

        Err(
            "The configured Notion destination was not found as a page, database, or data source."
                .to_string(),
        )
    }

    async fn page_exists(&self, id: &str) -> Result<bool, String> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("{}/pages/{}", NOTION_API_BASE, id),
            )
            .send()
            .await
            .map_err(|err| format!("failed to reach Notion: {}", err))?;
        if response.status().is_success() {
            return Ok(true);
        }
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        Err(parse_notion_error(response, "page lookup").await)
    }

    async fn resolve_database_target(
        &self,
        id: &str,
    ) -> Result<Option<NotionDataSourceTarget>, String> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("{}/databases/{}", NOTION_API_BASE, id),
            )
            .send()
            .await
            .map_err(|err| format!("failed to reach Notion: {}", err))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(parse_notion_error(response, "database lookup").await);
        }

        let body = parse_json_response(response, "Notion database response").await?;
        let Some(data_source_id) = body
            .get("data_sources")
            .and_then(Value::as_array)
            .and_then(|sources| sources.first())
            .and_then(|source| source.get("id"))
            .and_then(Value::as_str)
        else {
            return Err(
                "The configured Notion database does not expose a data source that LearnChain can write to."
                    .to_string(),
            );
        };

        self.resolve_data_source_target(data_source_id).await
    }

    async fn resolve_data_source_target(
        &self,
        id: &str,
    ) -> Result<Option<NotionDataSourceTarget>, String> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("{}/data_sources/{}", NOTION_API_BASE, id),
            )
            .send()
            .await
            .map_err(|err| format!("failed to reach Notion: {}", err))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(parse_notion_error(response, "data source lookup").await);
        }

        let body = parse_json_response(response, "Notion data source response").await?;
        let Some(properties) = body.get("properties").and_then(Value::as_object) else {
            return Err(
                "The configured Notion data source does not expose any properties.".to_string(),
            );
        };
        let Some(title_property) = properties.iter().find_map(|(name, property)| {
            property
                .get("type")
                .and_then(Value::as_str)
                .filter(|property_type| *property_type == "title")
                .map(|_| name.clone())
        }) else {
            return Err(
                "The configured Notion data source does not have a title property.".to_string(),
            );
        };

        Ok(Some(NotionDataSourceTarget {
            id: id.to_string(),
            title_property,
        }))
    }

    async fn create_page(
        &self,
        parent: &NotionParent,
        title: &str,
    ) -> Result<NotionCreatedPage, String> {
        let safe_title = if title.trim().is_empty() {
            "LearnChain Document"
        } else {
            title.trim()
        };
        let properties = match parent {
            NotionParent::Page { .. } => {
                let mut map = Map::new();
                map.insert(
                    "title".to_string(),
                    json!({ "title": [text_object(safe_title)] }),
                );
                map
            }
            NotionParent::DataSource { title_property, .. } => {
                let mut map = Map::new();
                map.insert(
                    title_property.clone(),
                    json!({ "title": [text_object(safe_title)] }),
                );
                map
            }
        };
        let parent_json = match parent {
            NotionParent::Page { id } => json!({ "type": "page_id", "page_id": id }),
            NotionParent::DataSource { id, .. } => {
                json!({ "type": "data_source_id", "data_source_id": id })
            }
        };
        let response = self
            .request(reqwest::Method::POST, &format!("{}/pages", NOTION_API_BASE))
            .body(serialize_json(&json!({
                "parent": parent_json,
                "properties": Value::Object(properties),
            }))?)
            .send()
            .await
            .map_err(|err| format!("failed to reach Notion: {}", err))?;

        if !response.status().is_success() {
            return Err(parse_notion_error(response, "page creation").await);
        }

        let body = parse_json_response(response, "Notion page response").await?;
        let id = body
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Notion page creation response did not include a page id.".to_string())?
            .to_string();
        let url = body.get("url").and_then(Value::as_str).map(str::to_string);
        Ok(NotionCreatedPage { id, url })
    }

    async fn append_blocks(&self, page_id: &str, blocks: &[Value]) -> Result<(), String> {
        for chunk in blocks.chunks(NOTION_BLOCK_BATCH_SIZE) {
            let response = self
                .request(
                    reqwest::Method::PATCH,
                    &format!("{}/blocks/{}/children", NOTION_API_BASE, page_id),
                )
                .body(serialize_json(&json!({ "children": chunk }))?)
                .send()
                .await
                .map_err(|err| format!("failed to reach Notion: {}", err))?;

            if !response.status().is_success() {
                return Err(parse_notion_error(response, "block append").await);
            }
        }

        Ok(())
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .bearer_auth(&self.token)
            .header("Notion-Version", NOTION_VERSION)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
    }
}

impl LearnChainClient {
    fn new(config: &AppConfig) -> Result<Self, String> {
        let site_url = config.learnchain_site_url.trim().to_string();
        let access_token = config.learnchain_access_token.clone();
        let refresh_token = config.learnchain_refresh_token.clone();
        let email = config.learnchain_email.trim().to_string();
        let password = config.learnchain_password.clone();

        let client = Client::builder()
            .user_agent(format!("learnchain/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| format!("failed to build LearnChain client: {}", err))?;

        Ok(Self {
            client,
            site_url,
            access_token,
            refresh_token,
            email,
            password,
        })
    }

    async fn export_document(
        &self,
        document: ExportableDocument,
    ) -> Result<RepositoryExportResult, String> {
        if !self.access_token.trim().is_empty() {
            match self
                .upload_document(&document, self.access_token.trim())
                .await
            {
                Ok(uploaded) => {
                    return Ok(RepositoryExportResult {
                        repository_label: "LearnChain".to_string(),
                        document_title: document.title,
                        remote_url: Some(format!(
                            "{}{}/{}",
                            self.site_url, LEARNCHAIN_DOCUMENTS_PATH, uploaded.document.id
                        )),
                    });
                }
                Err(LearnChainUploadError::Unauthorized) => {}
                Err(LearnChainUploadError::Message(message)) => return Err(message),
            }
        }

        let session = self.authorize().await?;
        let uploaded = match self
            .upload_document(&document, &session.session.access_token)
            .await
        {
            Ok(uploaded) => uploaded,
            Err(LearnChainUploadError::Unauthorized) => {
                let refreshed = self.refresh_session(&session.session.refresh_token).await?;
                self.validate_session(&refreshed, "refresh")?;
                persist_learnchain_session(&refreshed)?;
                self.upload_document(&document, &refreshed.session.access_token)
                    .await
                    .map_err(|err| match err {
                        LearnChainUploadError::Unauthorized => {
                            "LearnChain upload failed after refreshing the session. Sign in again and retry.".to_string()
                        }
                        LearnChainUploadError::Message(message) => message,
                    })?
            }
            Err(LearnChainUploadError::Message(message)) => return Err(message),
        };

        Ok(RepositoryExportResult {
            repository_label: "LearnChain".to_string(),
            document_title: document.title,
            remote_url: Some(format!(
                "{}{}/{}",
                self.site_url, LEARNCHAIN_DOCUMENTS_PATH, uploaded.document.id
            )),
        })
    }

    async fn authorize(&self) -> Result<LearnChainAuthEnvelope, String> {
        if !self.refresh_token.trim().is_empty() {
            match self.refresh_session(self.refresh_token.trim()).await {
                Ok(session) => {
                    self.validate_session(&session, "refresh")?;
                    persist_learnchain_session(&session)?;
                    return Ok(session);
                }
                Err(refresh_error)
                    if !self.email.trim().is_empty() && !self.password.trim().is_empty() =>
                {
                    log_debug(&format!(
                        "LearnChain refresh failed, falling back to password login: {}",
                        refresh_error
                    ));
                }
                Err(refresh_error) => {
                    return Err(format!(
                        "{} {}",
                        refresh_error,
                        config::learnchain_authorization_help_message(&self.site_url)
                    ));
                }
            }
        }

        if self.email.trim().is_empty() || self.password.trim().is_empty() {
            return Err(config::learnchain_authorization_help_message(
                &self.site_url,
            ));
        }

        let session = self.login().await?;
        self.validate_session(&session, "login")?;
        persist_learnchain_session(&session)?;
        Ok(session)
    }

    async fn login(&self) -> Result<LearnChainAuthEnvelope, String> {
        let response = self
            .client
            .post(format!("{}{}", self.site_url, LEARNCHAIN_CLI_LOGIN_PATH))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json;charset=UTF-8",
            )
            .body(serialize_json(&json!({
                "email": self.email,
                "password": self.password,
            }))?)
            .send()
            .await
            .map_err(|err| format!("failed to reach LearnChain auth API: {}", err))?;

        if !response.status().is_success() {
            return Err(parse_learnchain_auth_error(response, "login", &self.site_url).await);
        }

        let body = parse_json_response(response, "LearnChain login response").await?;
        serde_json::from_value(body)
            .map_err(|err| format!("failed to parse LearnChain login response: {}", err))
    }

    async fn refresh_session(&self, refresh_token: &str) -> Result<LearnChainAuthEnvelope, String> {
        let response = self
            .client
            .post(format!("{}{}", self.site_url, LEARNCHAIN_CLI_REFRESH_PATH))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json;charset=UTF-8",
            )
            .body(serialize_json(&json!({
                "refreshToken": refresh_token,
            }))?)
            .send()
            .await
            .map_err(|err| format!("failed to reach LearnChain refresh API: {}", err))?;

        if !response.status().is_success() {
            return Err(
                parse_learnchain_auth_error(response, "session refresh", &self.site_url).await,
            );
        }

        let body = parse_json_response(response, "LearnChain refresh response").await?;
        serde_json::from_value(body)
            .map_err(|err| format!("failed to parse LearnChain refresh response: {}", err))
    }

    fn validate_session(&self, auth: &LearnChainAuthEnvelope, context: &str) -> Result<(), String> {
        if !auth.session.token_type.eq_ignore_ascii_case("bearer") {
            return Err(format!(
                "LearnChain {} returned an unsupported token type: {}",
                context, auth.session.token_type
            ));
        }

        log_debug(&format!(
            "LearnChain {} succeeded for user {} ({}) expires_at={:?} expires_in={}",
            context,
            auth.user.id,
            auth.user
                .email
                .as_deref()
                .or(auth.user.username.as_deref())
                .unwrap_or("unknown"),
            auth.session.expires_at,
            auth.session.expires_in
        ));

        Ok(())
    }

    async fn upload_document(
        &self,
        document: &ExportableDocument,
        access_token: &str,
    ) -> Result<LearnChainUploadEnvelope, LearnChainUploadError> {
        let file_name = learnchain_filename(&document.title);
        let file_part = reqwest::multipart::Part::bytes(document.markdown.clone().into_bytes())
            .file_name(file_name)
            .mime_str("text/markdown")
            .map_err(|err| {
                LearnChainUploadError::Message(format!(
                    "failed to prepare LearnChain upload: {}",
                    err
                ))
            })?;
        let form = reqwest::multipart::Form::new().part("file", file_part);

        let response = self
            .client
            .post(format!("{}{}", self.site_url, LEARNCHAIN_DOCUMENTS_PATH))
            .bearer_auth(access_token)
            .multipart(form)
            .send()
            .await
            .map_err(|err| {
                LearnChainUploadError::Message(format!("failed to reach LearnChain: {}", err))
            })?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(LearnChainUploadError::Unauthorized);
        }

        if !response.status().is_success() {
            return Err(LearnChainUploadError::Message(
                parse_learnchain_api_error(response, "document upload").await,
            ));
        }

        let body = parse_json_response(response, "LearnChain upload response")
            .await
            .map_err(LearnChainUploadError::Message)?;
        serde_json::from_value(body).map_err(|err| {
            LearnChainUploadError::Message(format!(
                "failed to parse LearnChain upload response: {}",
                err
            ))
        })
    }
}

fn build_learnchain_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(format!("learnchain/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| format!("failed to build LearnChain client: {}", err))
}

fn normalize_learnchain_site_url(site_url: &str) -> String {
    site_url.trim().trim_end_matches('/').to_string()
}

fn learnchain_account_label(user: &LearnChainAuthUser) -> String {
    user.email
        .as_deref()
        .or(user.username.as_deref())
        .unwrap_or(&user.id)
        .to_string()
}

fn persist_learnchain_session(auth: &LearnChainAuthEnvelope) -> Result<(), String> {
    let account_label = learnchain_account_label(&auth.user);
    config::update(|cfg| {
        cfg.learnchain_email = account_label.clone();
        cfg.learnchain_access_token = auth.session.access_token.clone();
        cfg.learnchain_refresh_token = auth.session.refresh_token.clone();
    })
    .map(|_| ())
    .map_err(|err| format!("failed to persist LearnChain session: {}", err))
}

pub(crate) fn exchange_learnchain_login_code(
    site_url: &str,
    code: &str,
) -> Result<LearnChainStoredSession, String> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|err| format!("failed to build Tokio runtime: {}", err))?;
    runtime.block_on(exchange_learnchain_login_code_async(site_url, code))
}

async fn exchange_learnchain_login_code_async(
    site_url: &str,
    code: &str,
) -> Result<LearnChainStoredSession, String> {
    let client = build_learnchain_client()?;
    let normalized_site_url = normalize_learnchain_site_url(site_url);
    let response = client
        .post(format!(
            "{}{}",
            normalized_site_url, LEARNCHAIN_CLI_EXCHANGE_PATH
        ))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/json;charset=UTF-8",
        )
        .body(serialize_json(&json!({
            "code": code.trim(),
        }))?)
        .send()
        .await
        .map_err(|err| format!("failed to reach LearnChain auth API: {}", err))?;

    if !response.status().is_success() {
        return Err(
            parse_learnchain_auth_error(response, "code exchange", &normalized_site_url).await,
        );
    }

    let body = parse_json_response(response, "LearnChain code exchange response").await?;
    let auth: LearnChainAuthEnvelope = serde_json::from_value(body)
        .map_err(|err| format!("failed to parse LearnChain code exchange response: {}", err))?;

    if !auth.session.token_type.eq_ignore_ascii_case("bearer") {
        return Err(format!(
            "LearnChain code exchange returned an unsupported token type: {}",
            auth.session.token_type
        ));
    }

    Ok(LearnChainStoredSession {
        account_label: learnchain_account_label(&auth.user),
        access_token: auth.session.access_token,
        refresh_token: auth.session.refresh_token,
    })
}

pub(crate) fn trigger_library_export(app: &mut App, entry: LibraryArtifactEntry) {
    if app.document_export_loading {
        app.ai_status = Some("A document export is already in progress.".to_string());
        return;
    }

    let config_snapshot = config::current();
    if config_snapshot.document_repository == DocumentRepositoryKind::None {
        let message =
            "No document repository is configured. Open Config and choose a repository first."
                .to_string();
        App::push_error(&mut app.error, message.clone());
        app.ai_status = Some(message);
        return;
    }

    if let Err(err) = config::validate_document_repository_target(
        config_snapshot.document_repository,
        &config_snapshot.document_repository_target,
    ) {
        App::push_error(
            &mut app.error,
            format!("Invalid document repository target: {}", err),
        );
        app.ai_status = Some(format!("Invalid document repository target. {}", err));
        return;
    }

    if config_snapshot.document_repository == DocumentRepositoryKind::Notion
        && config_snapshot.notion_api_token.trim().is_empty()
    {
        let help = config::notion_token_help_message().to_string();
        App::push_error(&mut app.error, help.clone());
        app.ai_status = Some(help);
        return;
    }

    if config_snapshot.document_repository == DocumentRepositoryKind::LearnChain {
        if let Err(err) = config::validate_learnchain_site_url(&config_snapshot.learnchain_site_url)
        {
            App::push_error(&mut app.error, err.clone());
            app.ai_status = Some(err);
            return;
        }

        if config_snapshot.learnchain_access_token.trim().is_empty()
            && config_snapshot.learnchain_refresh_token.trim().is_empty()
            && (config_snapshot.learnchain_email.trim().is_empty()
                || config_snapshot.learnchain_password.is_empty())
        {
            let help =
                config::learnchain_authorization_help_message(&config_snapshot.learnchain_site_url);
            App::push_error(&mut app.error, help.clone());
            app.ai_status = Some(help);
            return;
        }
    }

    let label = library_entry_label(&entry);
    let (sender, receiver) = mpsc::channel();
    app.document_export_receiver = Some(receiver);
    app.document_export_loading = true;
    app.ai_status = Some(format!(
        "Sending {} to {}...",
        label,
        config_snapshot.document_repository.label()
    ));

    thread::spawn(move || {
        log_debug(&format!(
            "App: background export started for {} to {}",
            label,
            config_snapshot.document_repository.label()
        ));
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = sender.send(DocumentExportMessage::Error(format!(
                    "Failed to build Tokio runtime: {}",
                    err
                )));
                return;
            }
        };

        let result = runtime.block_on(export_library_artifact(&config_snapshot, entry));
        drop(runtime);

        let _ = match result {
            Ok(result) => sender.send(DocumentExportMessage::Success(result)),
            Err(err) => sender.send(DocumentExportMessage::Error(err)),
        };
    });
}

pub(crate) fn poll_export_messages(app: &mut App) {
    let mut clear_receiver = false;
    if let Some(receiver) = app.document_export_receiver.as_ref() {
        match receiver.try_recv() {
            Ok(DocumentExportMessage::Success(result)) => {
                app.document_export_loading = false;
                app.ai_status = Some(result.status_message());
                clear_receiver = true;
            }
            Ok(DocumentExportMessage::Error(message)) => {
                app.document_export_loading = false;
                App::push_error(
                    &mut app.error,
                    format!("Document export failed: {}", message),
                );
                app.ai_status = Some("Document export failed".to_string());
                clear_receiver = true;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.document_export_loading = false;
                App::push_error(
                    &mut app.error,
                    "Document export worker disconnected.".to_string(),
                );
                app.ai_status = Some("Document export failed".to_string());
                clear_receiver = true;
            }
        }
    }

    if clear_receiver {
        app.document_export_receiver = None;
    }
}

async fn export_library_artifact(
    config: &AppConfig,
    entry: LibraryArtifactEntry,
) -> Result<RepositoryExportResult, String> {
    let document = load_exportable_document(&entry)?;
    export_document_to_repository(config, document).await
}

pub(crate) async fn export_deep_dive_document(
    config: &AppConfig,
    document: &DeepDiveDocument,
) -> Result<RepositoryExportResult, String> {
    export_document_to_repository(config, exportable_deep_dive_document(document)).await
}

async fn export_document_to_repository(
    config: &AppConfig,
    document: ExportableDocument,
) -> Result<RepositoryExportResult, String> {
    match config.document_repository {
        DocumentRepositoryKind::None => Err("No document repository is configured.".to_string()),
        DocumentRepositoryKind::Notion => {
            let client = NotionClient::new(config.notion_api_token.trim().to_string())?;
            client.export_document(config, document).await
        }
        DocumentRepositoryKind::LearnChain => {
            let client = LearnChainClient::new(config)?;
            client.export_document(document).await
        }
    }
}

fn load_exportable_document(entry: &LibraryArtifactEntry) -> Result<ExportableDocument, String> {
    let output_manager = OutputManager::new();
    match entry {
        LibraryArtifactEntry::DeepDive(entry) => {
            let document = output_manager.read_deep_dive_markdown(&entry.path)?;
            Ok(exportable_deep_dive_document(&document))
        }
        LibraryArtifactEntry::Quiz(entry) => {
            let response = output_manager.read_learning_response(&entry.path)?;
            Ok(ExportableDocument {
                title: quiz_title(entry),
                markdown: render_learning_markdown(&response, &entry.session_date),
            })
        }
    }
}

fn exportable_deep_dive_document(document: &DeepDiveDocument) -> ExportableDocument {
    let title = if document.metadata.title.trim().is_empty() {
        document
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("LearnChain Deep Dive")
            .to_string()
    } else {
        document.metadata.title.clone()
    };

    ExportableDocument {
        title,
        markdown: document.markdown.clone(),
    }
}

fn quiz_title(entry: &crate::output_manager::LearningArtifactHistoryEntry) -> String {
    if entry.session_date.trim().is_empty() {
        "LearnChain Quiz".to_string()
    } else {
        format!("LearnChain Quiz - {}", entry.session_date)
    }
}

fn render_learning_markdown(response: &StructuredLearningResponse, session_date: &str) -> String {
    let mut markdown = String::new();
    let title = if session_date.trim().is_empty() {
        "LearnChain Quiz".to_string()
    } else {
        format!("LearnChain Quiz - {}", session_date)
    };
    markdown.push_str("# ");
    markdown.push_str(&title);
    markdown.push_str("\n\n");

    for (group_index, group) in response.response.iter().enumerate() {
        let heading = if group.knowledge_type_group.trim().is_empty() {
            format!("Knowledge Group {}", group_index + 1)
        } else {
            group.knowledge_type_group.clone()
        };
        markdown.push_str("## ");
        markdown.push_str(&heading);
        markdown.push('\n');

        if !group.knowledge_type_language.trim().is_empty() {
            markdown.push_str("- Language: ");
            markdown.push_str(group.knowledge_type_language.trim());
            markdown.push('\n');
        }
        if !group.summary.trim().is_empty() {
            markdown.push('\n');
            markdown.push_str(group.summary.trim());
            markdown.push_str("\n\n");
        } else {
            markdown.push('\n');
        }

        for (quiz_index, quiz) in group.quiz.iter().enumerate() {
            markdown.push_str("### Question ");
            markdown.push_str(&(quiz_index + 1).to_string());
            markdown.push('\n');
            markdown.push_str(quiz.question.trim());
            markdown.push_str("\n\n");

            for option in &quiz.options {
                markdown.push_str("- ");
                markdown.push_str(option.selection.trim());
                if option.is_correct_answer {
                    markdown.push_str(" (correct)");
                }
                markdown.push('\n');
            }

            if !quiz.resources.is_empty() {
                markdown.push_str("\nResources:\n");
                for resource in &quiz.resources {
                    markdown.push_str("- ");
                    markdown.push_str(resource.trim());
                    markdown.push('\n');
                }
            }

            markdown.push('\n');
        }
    }

    markdown.trim_end().to_string()
}

fn markdown_to_notion_blocks(markdown: &str) -> Vec<Value> {
    let mut blocks = Vec::new();
    let mut paragraph_lines = Vec::new();
    let mut lines = markdown.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        let plain = trimmed.trim();

        if plain.starts_with("```") {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            let language = plain.trim_start_matches('`').trim();
            let mut code_lines = Vec::new();
            for code_line in lines.by_ref() {
                if code_line.trim() == "```" {
                    break;
                }
                code_lines.push(code_line);
            }
            let code = code_lines.join("\n");
            push_code_blocks(&mut blocks, &code, notion_code_language(language));
            continue;
        }

        if plain.is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            continue;
        }

        if let Some(text) = plain.strip_prefix("# ") {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            push_rich_text_blocks(&mut blocks, "heading_1", text.trim());
            continue;
        }
        if let Some(text) = plain.strip_prefix("## ") {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            push_rich_text_blocks(&mut blocks, "heading_2", text.trim());
            continue;
        }
        if let Some(text) = plain.strip_prefix("### ") {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            push_rich_text_blocks(&mut blocks, "heading_3", text.trim());
            continue;
        }
        if let Some(text) = strip_bullet_marker(plain) {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            push_rich_text_blocks(&mut blocks, "bulleted_list_item", text.trim());
            continue;
        }
        if let Some(text) = strip_numbered_marker(plain) {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            push_rich_text_blocks(&mut blocks, "numbered_list_item", text.trim());
            continue;
        }

        paragraph_lines.push(plain.to_string());
    }

    flush_paragraph(&mut blocks, &mut paragraph_lines);
    blocks
}

fn flush_paragraph(blocks: &mut Vec<Value>, paragraph_lines: &mut Vec<String>) {
    if paragraph_lines.is_empty() {
        return;
    }

    let paragraph = paragraph_lines.join("\n");
    push_rich_text_blocks(blocks, "paragraph", paragraph.trim());
    paragraph_lines.clear();
}

fn push_rich_text_blocks(blocks: &mut Vec<Value>, block_type: &str, text: &str) {
    let chunks = split_text_chunks(text);
    if chunks.is_empty() {
        return;
    }

    for chunk in chunks {
        blocks.push(json!({
            "object": "block",
            "type": block_type,
            (block_type): {
                "rich_text": [text_object(&chunk)],
            }
        }));
    }
}

fn push_code_blocks(blocks: &mut Vec<Value>, text: &str, language: &str) {
    let chunks = split_text_chunks(text);
    if chunks.is_empty() {
        return;
    }

    for chunk in chunks {
        blocks.push(json!({
            "object": "block",
            "type": "code",
            "code": {
                "rich_text": [text_object(&chunk)],
                "language": language,
            }
        }));
    }
}

fn split_text_chunks(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in trimmed.chars() {
        current.push(ch);
        if current.chars().count() >= NOTION_TEXT_LIMIT {
            chunks.push(current);
            current = String::new();
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn text_object(content: &str) -> Value {
    json!({
        "type": "text",
        "text": {
            "content": content
        }
    })
}

fn notion_code_language(language: &str) -> &'static str {
    match language.trim().to_ascii_lowercase().as_str() {
        "rust" => "rust",
        "bash" | "sh" | "shell" | "zsh" => "shell",
        "json" => "json",
        "toml" => "toml",
        "markdown" | "md" => "markdown",
        "typescript" | "ts" => "typescript",
        "javascript" | "js" => "javascript",
        "python" | "py" => "python",
        "sql" => "sql",
        "yaml" | "yml" => "yaml",
        _ => "plain text",
    }
}

fn strip_bullet_marker(value: &str) -> Option<&str> {
    value
        .strip_prefix("- ")
        .or_else(|| value.strip_prefix("* "))
        .or_else(|| value.strip_prefix("+ "))
}

fn strip_numbered_marker(value: &str) -> Option<&str> {
    let dot_index = value.find(". ")?;
    if value[..dot_index].chars().all(|ch| ch.is_ascii_digit()) {
        Some(&value[(dot_index + 2)..])
    } else {
        None
    }
}

fn library_entry_label(entry: &LibraryArtifactEntry) -> &'static str {
    match entry {
        LibraryArtifactEntry::DeepDive(_) => "deep dive",
        LibraryArtifactEntry::Quiz(_) => "quiz",
    }
}

fn extract_notion_id(value: &str) -> Option<String> {
    static RAW_ID_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static DASHED_ID_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

    let dashed = DASHED_ID_RE
        .get_or_init(|| {
            Regex::new(r"(?i)([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})")
                .expect("valid dashed Notion id regex")
        })
        .captures_iter(value)
        .last()
        .and_then(|captures| captures.get(1).map(|capture| capture.as_str().to_string()));
    if let Some(id) = dashed {
        return normalize_notion_id(&id);
    }

    let raw = RAW_ID_RE
        .get_or_init(|| Regex::new(r"(?i)([0-9a-f]{32})").expect("valid Notion id regex"))
        .captures_iter(value)
        .last()
        .and_then(|captures| captures.get(1).map(|capture| capture.as_str().to_string()));
    raw.as_deref().and_then(normalize_notion_id)
}

fn normalize_notion_id(value: &str) -> Option<String> {
    let compact: String = value
        .chars()
        .filter(|ch| *ch != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    if compact.len() != 32 || !compact.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }

    Some(format!(
        "{}-{}-{}-{}-{}",
        &compact[0..8],
        &compact[8..12],
        &compact[12..16],
        &compact[16..20],
        &compact[20..32]
    ))
}

async fn parse_notion_error(response: reqwest::Response, context: &str) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if let Ok(json) = serde_json::from_str::<Value>(&body)
        && let Some(message) = json.get("message").and_then(Value::as_str)
    {
        return format!("Notion {} failed ({}): {}", context, status, message);
    }
    if body.trim().is_empty() {
        format!("Notion {} failed ({})", context, status)
    } else {
        format!("Notion {} failed ({}): {}", context, status, body.trim())
    }
}

async fn parse_learnchain_api_error(response: reqwest::Response, context: &str) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if let Ok(api_error) = serde_json::from_str::<LearnChainApiErrorEnvelope>(&body) {
        return format!(
            "LearnChain {} failed ({} {}): {}",
            context, status, api_error.error.code, api_error.error.message
        );
    }

    if body.trim().is_empty() {
        format!("LearnChain {} failed ({})", context, status)
    } else {
        format!(
            "LearnChain {} failed ({}): {}",
            context,
            status,
            body.trim()
        )
    }
}

async fn parse_learnchain_auth_error(
    response: reqwest::Response,
    context: &str,
    site_url: &str,
) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let signup_url = config::learnchain_signup_url(site_url);

    if let Ok(api_error) = serde_json::from_str::<LearnChainApiErrorEnvelope>(&body) {
        return format!(
            "LearnChain {} failed ({} {}): {} If you need an account, sign up at {}.",
            context, status, api_error.error.code, api_error.error.message, signup_url
        );
    }

    format!(
        "LearnChain {} failed ({}). If you need an account, sign up at {}.",
        context, status, signup_url
    )
}

async fn parse_json_response(response: reqwest::Response, label: &str) -> Result<Value, String> {
    let body = response
        .text()
        .await
        .map_err(|err| format!("failed to read {}: {}", label, err))?;
    serde_json::from_str(&body).map_err(|err| format!("failed to parse {}: {}", label, err))
}

fn serialize_json(value: &Value) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| format!("failed to serialize JSON request: {}", err))
}

fn learnchain_filename(title: &str) -> String {
    let mut name = title
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else if ch.is_whitespace() || matches!(ch, '-' | '_') {
                '-'
            } else {
                '-'
            }
        })
        .collect::<String>();
    while name.contains("--") {
        name = name.replace("--", "-");
    }
    let name = name.trim_matches('-');
    if name.is_empty() {
        "learnchain-document.md".to_string()
    } else if name.ends_with(".md") || name.ends_with(".markdown") {
        name.to_string()
    } else {
        format!("{}.md", name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::{KnowledgeResponse, QuizItem, QuizOption};

    #[test]
    fn extract_notion_id_supports_urls_and_raw_ids() {
        assert_eq!(
            extract_notion_id("https://www.notion.so/Welcome-31b0f905b7ec80a68a2cea54112a0d4a"),
            Some("31b0f905-b7ec-80a6-8a2c-ea54112a0d4a".to_string())
        );
        assert_eq!(
            extract_notion_id("31b0f905-b7ec-80a6-8a2c-ea54112a0d4a"),
            Some("31b0f905-b7ec-80a6-8a2c-ea54112a0d4a".to_string())
        );
        assert_eq!(extract_notion_id("not-a-notion-id"), None);
    }

    #[test]
    fn render_learning_markdown_includes_groups_questions_and_resources() {
        let response = StructuredLearningResponse {
            response: vec![KnowledgeResponse {
                knowledge_type_group: "Rust".to_string(),
                summary: "Borrowing overview".to_string(),
                quiz: vec![QuizItem {
                    question: "What does borrowing prevent?".to_string(),
                    options: vec![
                        QuizOption {
                            selection: "Data races".to_string(),
                            is_correct_answer: true,
                        },
                        QuizOption {
                            selection: "Compilation".to_string(),
                            is_correct_answer: false,
                        },
                    ],
                    resources: vec!["https://doc.rust-lang.org/".to_string()],
                }],
                knowledge_type_language: "Rust".to_string(),
            }],
        };

        let markdown = render_learning_markdown(&response, "2026-03-06");

        assert!(markdown.contains("# LearnChain Quiz - 2026-03-06"));
        assert!(markdown.contains("## Rust"));
        assert!(markdown.contains("### Question 1"));
        assert!(markdown.contains("Data races (correct)"));
        assert!(markdown.contains("Resources:"));
    }

    #[test]
    fn markdown_to_notion_blocks_supports_headings_lists_and_code() {
        let blocks = markdown_to_notion_blocks(
            "# Title\n\nParagraph text\n- Bullet\n1. Numbered\n\n```rust\nfn main() {}\n```",
        );

        assert_eq!(blocks.len(), 5);
        assert_eq!(blocks[0]["type"], "heading_1");
        assert_eq!(blocks[1]["type"], "paragraph");
        assert_eq!(blocks[2]["type"], "bulleted_list_item");
        assert_eq!(blocks[3]["type"], "numbered_list_item");
        assert_eq!(blocks[4]["type"], "code");
    }

    #[test]
    fn learnchain_filename_normalizes_titles() {
        assert_eq!(
            learnchain_filename(" LearnChain Quiz: Rust / Borrowing "),
            "learnchain-quiz-rust-borrowing.md"
        );
        assert_eq!(learnchain_filename(""), "learnchain-document.md");
    }

    #[test]
    fn learnchain_auth_payload_deserializes() {
        let auth: LearnChainAuthEnvelope = serde_json::from_value(json!({
            "session": {
                "accessToken": "access-token",
                "refreshToken": "refresh-token",
                "expiresAt": 1772900000u64,
                "expiresIn": 3600,
                "tokenType": "bearer"
            },
            "user": {
                "id": "user-123",
                "email": "person@example.com",
                "username": "person"
            }
        }))
        .expect("auth payload");

        assert_eq!(auth.session.access_token, "access-token");
        assert_eq!(auth.session.refresh_token, "refresh-token");
        assert_eq!(auth.session.expires_at, Some(1772900000));
        assert_eq!(auth.session.expires_in, 3600);
        assert_eq!(auth.session.token_type, "bearer");
        assert_eq!(auth.user.id, "user-123");
        assert_eq!(auth.user.email.as_deref(), Some("person@example.com"));
        assert_eq!(auth.user.username.as_deref(), Some("person"));
    }
}
