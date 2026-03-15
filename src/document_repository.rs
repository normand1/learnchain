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
    output_manager::{
        LibraryArtifactEntry, OutputManager, render_deep_dive_contents, render_learning_markdown,
    },
};

const NOTION_API_BASE: &str = "https://api.notion.com/v1";
const NOTION_VERSION: &str = "2025-09-03";
const NOTION_BLOCK_BATCH_SIZE: usize = 100;
const NOTION_TEXT_LIMIT: usize = 1800;
const LEARNCHAIN_CLI_EXCHANGE_PATH: &str = "/api/auth/cli/exchange";
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct LearnChainPublicAuthConfig {
    supabase_url: String,
    publishable_key: String,
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
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
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

#[derive(Debug, Deserialize)]
struct LearnChainPasswordAuthEnvelope {
    access_token: String,
    refresh_token: String,
    token_type: String,
    user: LearnChainAuthUser,
}

#[derive(Debug, Deserialize)]
struct LearnChainPasswordAuthErrorEnvelope {
    error: Option<String>,
    error_description: Option<String>,
    message: Option<String>,
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
                        remote_url: Some(self.document_url(&uploaded.document.id)),
                    });
                }
                Err(LearnChainUploadError::Unauthorized) => {
                    log_debug(
                        "LearnChain upload rejected the stored access token; attempting reauthorization.",
                    );
                }
                Err(LearnChainUploadError::Message(message)) => return Err(message),
            }
        }

        let session = self.authorize().await?;
        let uploaded = match self.upload_document(&document, &session.access_token).await {
            Ok(uploaded) => uploaded,
            Err(LearnChainUploadError::Unauthorized) => {
                if session.refresh_token.trim().is_empty() {
                    return Err(
                        "LearnChain upload failed after reauthorizing. Sign in again and retry."
                            .to_string(),
                    );
                }

                let refreshed = self.refresh_session(session.refresh_token.trim()).await?;
                persist_learnchain_session(&refreshed)?;
                self.upload_document(&document, &refreshed.access_token)
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
            remote_url: Some(self.document_url(&uploaded.document.id)),
        })
    }

    async fn authorize(&self) -> Result<LearnChainStoredSession, String> {
        if !self.refresh_token.trim().is_empty() {
            match self.refresh_session(self.refresh_token.trim()).await {
                Ok(session) => {
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

        let session = self.sign_in_with_password().await?;
        persist_learnchain_session(&session)?;
        Ok(session)
    }

    async fn sign_in_with_password(&self) -> Result<LearnChainStoredSession, String> {
        let auth_config =
            discover_learnchain_public_auth_config(&self.client, &self.site_url).await?;
        let response = self
            .client
            .post(format!(
                "{}/auth/v1/token?grant_type=password",
                auth_config.supabase_url.trim_end_matches('/')
            ))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json;charset=UTF-8",
            )
            .header("apikey", &auth_config.publishable_key)
            .header(
                "x-client-info",
                format!("learnchain/{}", env!("CARGO_PKG_VERSION")),
            )
            .body(serialize_json(&json!({
                "email": self.email.trim(),
                "password": self.password,
            }))?)
            .send()
            .await
            .map_err(|err| format!("failed to reach LearnChain password auth API: {}", err))?;

        if !response.status().is_success() {
            return Err(parse_learnchain_supabase_auth_error(
                response,
                &self.site_url,
                "password sign-in",
            )
            .await);
        }

        let body = parse_json_response(response, "LearnChain password sign-in response").await?;
        let auth: LearnChainPasswordAuthEnvelope = serde_json::from_value(body).map_err(|err| {
            format!(
                "failed to parse LearnChain password sign-in response: {}",
                err
            )
        })?;
        stored_session_from_supabase_auth(auth, "password sign-in")
    }

    async fn refresh_session(
        &self,
        refresh_token: &str,
    ) -> Result<LearnChainStoredSession, String> {
        let auth_config =
            discover_learnchain_public_auth_config(&self.client, &self.site_url).await?;
        let response = self
            .client
            .post(format!(
                "{}/auth/v1/token?grant_type=refresh_token",
                auth_config.supabase_url.trim_end_matches('/')
            ))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json;charset=UTF-8",
            )
            .header("apikey", &auth_config.publishable_key)
            .header(
                "x-client-info",
                format!("learnchain/{}", env!("CARGO_PKG_VERSION")),
            )
            .body(serialize_json(&json!({
                "refresh_token": refresh_token.trim(),
            }))?)
            .send()
            .await
            .map_err(|err| format!("failed to reach LearnChain refresh API: {}", err))?;

        if !response.status().is_success() {
            return Err(parse_learnchain_supabase_auth_error(
                response,
                &self.site_url,
                "session refresh",
            )
            .await);
        }

        let body = parse_json_response(response, "LearnChain refresh response").await?;
        let auth: LearnChainPasswordAuthEnvelope = serde_json::from_value(body)
            .map_err(|err| format!("failed to parse LearnChain refresh response: {}", err))?;
        stored_session_from_supabase_auth(auth, "session refresh")
    }

    fn document_url(&self, document_id: &str) -> String {
        format!(
            "{}/documents/{}",
            config::learnchain_dashboard_url(&self.site_url),
            document_id
        )
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
    let trimmed = site_url.trim();
    if let Ok(mut url) = reqwest::Url::parse(trimmed) {
        let host = url.host_str().unwrap_or_default();
        if matches!(host, "learnchain.co" | "www.learnchain.co") {
            let _ = url.set_scheme("https");
            let _ = url.set_host(Some("www.learnchain.co"));
            return url.to_string().trim_end_matches('/').to_string();
        }
    }

    trimmed.trim_end_matches('/').to_string()
}

fn learnchain_account_label(user: &LearnChainAuthUser) -> String {
    user.email
        .as_deref()
        .or(user.username.as_deref())
        .unwrap_or(&user.id)
        .to_string()
}

pub(crate) fn apply_learnchain_session(config: &mut AppConfig, session: &LearnChainStoredSession) {
    config.learnchain_email = session.account_label.clone();
    config.learnchain_access_token = session.access_token.clone();
    config.learnchain_refresh_token = session.refresh_token.clone();
    config.learnchain_password.clear();
}

fn persist_learnchain_session(session: &LearnChainStoredSession) -> Result<(), String> {
    config::update(|config| {
        apply_learnchain_session(config, session);
    })
    .map(|_| ())
    .map_err(|err| format!("failed to persist LearnChain session: {}", err))
}

fn validate_learnchain_session(auth: &LearnChainAuthEnvelope, context: &str) -> Result<(), String> {
    if !auth.session.token_type.eq_ignore_ascii_case("bearer") {
        return Err(format!(
            "LearnChain {} returned an unsupported token type: {}",
            context, auth.session.token_type
        ));
    }

    log_debug(&format!(
        "LearnChain {} succeeded for user {} ({}) expires_at={:?} expires_in={:?}",
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

    validate_learnchain_session(&auth, "code exchange")?;

    Ok(LearnChainStoredSession {
        account_label: learnchain_account_label(&auth.user),
        access_token: auth.session.access_token,
        refresh_token: auth.session.refresh_token.unwrap_or_default(),
    })
}

pub(crate) fn sign_in_learnchain_with_password(
    site_url: &str,
    email: &str,
    password: &str,
) -> Result<LearnChainStoredSession, String> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|err| format!("failed to build Tokio runtime: {}", err))?;
    runtime.block_on(sign_in_learnchain_with_password_async(
        site_url, email, password,
    ))
}

async fn sign_in_learnchain_with_password_async(
    site_url: &str,
    email: &str,
    password: &str,
) -> Result<LearnChainStoredSession, String> {
    let client = build_learnchain_client()?;
    let normalized_site_url = normalize_learnchain_site_url(site_url);
    let auth_config = discover_learnchain_public_auth_config(&client, &normalized_site_url).await?;
    let response = client
        .post(format!(
            "{}/auth/v1/token?grant_type=password",
            auth_config.supabase_url.trim_end_matches('/')
        ))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/json;charset=UTF-8",
        )
        .header("apikey", &auth_config.publishable_key)
        .header(
            "x-client-info",
            format!("learnchain/{}", env!("CARGO_PKG_VERSION")),
        )
        .body(serialize_json(&json!({
            "email": email.trim(),
            "password": password,
        }))?)
        .send()
        .await
        .map_err(|err| format!("failed to reach LearnChain password auth API: {}", err))?;

    if !response.status().is_success() {
        return Err(parse_learnchain_supabase_auth_error(
            response,
            &normalized_site_url,
            "password sign-in",
        )
        .await);
    }

    let body = parse_json_response(response, "LearnChain password sign-in response").await?;
    let auth: LearnChainPasswordAuthEnvelope = serde_json::from_value(body).map_err(|err| {
        format!(
            "failed to parse LearnChain password sign-in response: {}",
            err
        )
    })?;
    stored_session_from_supabase_auth(auth, "password sign-in")
}

async fn discover_learnchain_public_auth_config(
    client: &Client,
    site_url: &str,
) -> Result<LearnChainPublicAuthConfig, String> {
    let login_url = config::learnchain_signup_url(site_url);
    let response = client
        .get(&login_url)
        .send()
        .await
        .map_err(|err| format!("failed to reach LearnChain login page: {}", err))?;
    if !response.status().is_success() {
        return Err(format!(
            "LearnChain login page request failed ({}). Open {} in your browser to confirm the site is available.",
            response.status(),
            login_url
        ));
    }

    let html = response
        .text()
        .await
        .map_err(|err| format!("failed to read LearnChain login page: {}", err))?;

    if let Some(config) = extract_learnchain_public_auth_config_from_html(&html) {
        return Ok(config);
    }

    let chunk_paths = extract_next_chunk_paths(&html);
    for chunk_path in chunk_paths {
        let chunk_url = format!("{}{}", site_url.trim_end_matches('/'), chunk_path);
        let chunk_response = client
            .get(&chunk_url)
            .send()
            .await
            .map_err(|err| format!("failed to reach LearnChain login script: {}", err))?;
        if !chunk_response.status().is_success() {
            continue;
        }
        let script = chunk_response
            .text()
            .await
            .map_err(|err| format!("failed to read LearnChain login script: {}", err))?;
        if let Some(config) = extract_learnchain_public_auth_config_from_script(&script) {
            return Ok(config);
        }
    }

    Err(format!(
        "Could not resolve LearnChain password sign-in configuration from {}.",
        login_url
    ))
}

pub(crate) fn trigger_library_export(app: &mut App, entry: LibraryArtifactEntry) {
    let config_snapshot = config::current();
    let label = library_entry_label(&entry);
    if !prepare_export(app, &config_snapshot, label) {
        return;
    }

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

pub(crate) fn trigger_learning_export(
    app: &mut App,
    response: StructuredLearningResponse,
    session_date: String,
) {
    let config_snapshot = config::current();
    let label = "quiz";
    if !prepare_export(app, &config_snapshot, label) {
        return;
    }

    let document = exportable_learning_document(&response, &session_date);
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

        let result = runtime.block_on(export_document_to_repository(&config_snapshot, document));
        drop(runtime);

        let _ = match result {
            Ok(result) => sender.send(DocumentExportMessage::Success(result)),
            Err(err) => sender.send(DocumentExportMessage::Error(err)),
        };
    });
}

pub(crate) fn trigger_deep_dive_export(app: &mut App, document: DeepDiveDocument) {
    let config_snapshot = config::current();
    let label = "deep dive";
    if !prepare_export(app, &config_snapshot, label) {
        return;
    }

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

        let result = runtime.block_on(export_deep_dive_document(&config_snapshot, &document));
        drop(runtime);

        let _ = match result {
            Ok(result) => sender.send(DocumentExportMessage::Success(result)),
            Err(err) => sender.send(DocumentExportMessage::Error(err)),
        };
    });
}

fn prepare_export(app: &mut App, config_snapshot: &AppConfig, label: &str) -> bool {
    if app.document_export_loading {
        app.ai_status = Some("A document export is already in progress.".to_string());
        return false;
    }

    if config_snapshot.document_repository == DocumentRepositoryKind::None {
        let message =
            "No document repository is configured. Open Config and choose a repository first."
                .to_string();
        App::push_error(&mut app.error, message.clone());
        app.ai_status = Some(message);
        return false;
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
        return false;
    }

    if config_snapshot.document_repository == DocumentRepositoryKind::Notion
        && config_snapshot.notion_api_token.trim().is_empty()
    {
        let help = config::notion_token_help_message().to_string();
        App::push_error(&mut app.error, help.clone());
        app.ai_status = Some(help);
        return false;
    }

    if config_snapshot.document_repository == DocumentRepositoryKind::LearnChain {
        if let Err(err) = config::validate_learnchain_site_url(&config_snapshot.learnchain_site_url)
        {
            App::push_error(&mut app.error, err.clone());
            app.ai_status = Some(err);
            return false;
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
            return false;
        }
    }

    log_debug(&format!(
        "App: validated {} export to {}",
        label,
        config_snapshot.document_repository.label()
    ));
    true
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
            client
                .export_document(prepare_document_for_learnchain(document))
                .await
        }
    }
}

fn prepare_document_for_learnchain(document: ExportableDocument) -> ExportableDocument {
    ExportableDocument {
        title: document.title,
        markdown: strip_quiz_answer_reveal_markers(&document.markdown),
    }
}

fn strip_quiz_answer_reveal_markers(markdown: &str) -> String {
    markdown
        .lines()
        .map(|line| {
            let trimmed = line.trim_end();
            if let Some(without_marker) = trimmed.strip_suffix(" (correct)")
                && without_marker.trim_start().starts_with("- ")
            {
                without_marker.to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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
            Ok(exportable_learning_document(&response, &entry.session_date))
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
    let markdown = render_deep_dive_contents(&document.metadata, &document.markdown)
        .unwrap_or_else(|_| document.markdown.clone());

    ExportableDocument { title, markdown }
}

fn exportable_learning_document(
    response: &StructuredLearningResponse,
    session_date: &str,
) -> ExportableDocument {
    ExportableDocument {
        title: quiz_title_for_session_date(session_date),
        markdown: render_learning_markdown(response, session_date),
    }
}

fn quiz_title_for_session_date(session_date: &str) -> String {
    if session_date.trim().is_empty() {
        "LearnChain Quiz".to_string()
    } else {
        format!("LearnChain Quiz - {}", session_date)
    }
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

async fn parse_learnchain_supabase_auth_error(
    response: reqwest::Response,
    site_url: &str,
    context: &str,
) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let signup_url = config::learnchain_signup_url(site_url);

    if let Ok(error) = serde_json::from_str::<LearnChainPasswordAuthErrorEnvelope>(&body) {
        let detail = error
            .error_description
            .or(error.message)
            .or(error.error)
            .unwrap_or_else(|| format!("LearnChain {} failed.", context));
        return format!(
            "LearnChain {} failed ({}): {} If you need an account, sign in at {} first.",
            context, status, detail, signup_url
        );
    }

    if body.trim().is_empty() {
        format!(
            "LearnChain {} failed ({}). If you need an account, sign in at {} first.",
            context, status, signup_url
        )
    } else {
        format!(
            "LearnChain {} failed ({}): {} If you need an account, sign in at {} first.",
            context,
            status,
            body.trim(),
            signup_url
        )
    }
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

fn stored_session_from_supabase_auth(
    auth: LearnChainPasswordAuthEnvelope,
    context: &str,
) -> Result<LearnChainStoredSession, String> {
    if !auth.token_type.eq_ignore_ascii_case("bearer") {
        return Err(format!(
            "LearnChain {} returned an unsupported token type: {}",
            context, auth.token_type
        ));
    }

    log_debug(&format!(
        "LearnChain {} succeeded for user {} ({})",
        context,
        auth.user.id,
        auth.user
            .email
            .as_deref()
            .or(auth.user.username.as_deref())
            .unwrap_or("unknown")
    ));

    Ok(LearnChainStoredSession {
        account_label: learnchain_account_label(&auth.user),
        access_token: auth.access_token,
        refresh_token: auth.refresh_token,
    })
}

fn extract_learnchain_public_auth_config_from_html(
    html: &str,
) -> Option<LearnChainPublicAuthConfig> {
    extract_learnchain_public_auth_config_from_script(html)
}

fn extract_learnchain_public_auth_config_from_script(
    script: &str,
) -> Option<LearnChainPublicAuthConfig> {
    static SUPABASE_URL_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static SUPABASE_KEY_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

    let supabase_url = SUPABASE_URL_RE
        .get_or_init(|| {
            Regex::new(r#"https://[a-z0-9-]+\.supabase\.co"#)
                .expect("valid LearnChain Supabase URL regex")
        })
        .captures(script)
        .and_then(|captures| captures.get(0))
        .map(|capture| capture.as_str().to_string())?;
    let publishable_key = SUPABASE_KEY_RE
        .get_or_init(|| {
            Regex::new(r#"sb_publishable_[A-Za-z0-9_-]+"#)
                .expect("valid LearnChain publishable key regex")
        })
        .captures(script)
        .and_then(|captures| captures.get(0))
        .map(|capture| capture.as_str().trim_end_matches('"').to_string())?;

    Some(LearnChainPublicAuthConfig {
        supabase_url,
        publishable_key,
    })
}

fn extract_next_chunk_paths(html: &str) -> Vec<String> {
    static CHUNK_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

    let mut chunk_paths = Vec::new();
    for capture in CHUNK_RE
        .get_or_init(|| {
            Regex::new(r#"/_next/static/chunks/[^"]+\.js"#)
                .expect("valid LearnChain Next.js chunk regex")
        })
        .captures_iter(html)
    {
        let Some(path) = capture.get(0).map(|value| value.as_str().to_string()) else {
            continue;
        };
        if !chunk_paths.contains(&path) {
            chunk_paths.push(path);
        }
    }

    chunk_paths
}

fn learnchain_filename(title: &str) -> String {
    let mut name = title
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
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
    use crate::llm::DeepDiveArtifactMetadata;
    use crate::llm::types::{KnowledgeResponse, QuizItem, QuizOption};
    use crate::session_analytics::{
        AdjustmentKind, AdjustmentMarker, ExternalResourceKind, ExternalResourceRef,
        SessionAnalytics,
    };

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
    fn normalize_learnchain_site_url_uses_www_canonical_host() {
        assert_eq!(
            normalize_learnchain_site_url("https://learnchain.co"),
            "https://www.learnchain.co"
        );
        assert_eq!(
            normalize_learnchain_site_url("https://www.learnchain.co/"),
            "https://www.learnchain.co"
        );
    }

    #[test]
    fn learnchain_document_url_uses_dashboard_path() {
        let client = LearnChainClient {
            client: build_learnchain_client().expect("client"),
            site_url: "https://learnchain.co".to_string(),
            access_token: String::new(),
            refresh_token: String::new(),
            email: String::new(),
            password: String::new(),
        };

        assert_eq!(
            client.document_url("025f92ed-f08e-464d-ab13-68940f14df73"),
            "https://www.learnchain.co/dashboard/documents/025f92ed-f08e-464d-ab13-68940f14df73"
        );
    }

    #[test]
    fn learnchain_document_url_preserves_local_site_origin() {
        let client = LearnChainClient {
            client: build_learnchain_client().expect("client"),
            site_url: "http://localhost:3000/".to_string(),
            access_token: String::new(),
            refresh_token: String::new(),
            email: String::new(),
            password: String::new(),
        };

        assert_eq!(
            client.document_url("doc-123"),
            "http://localhost:3000/dashboard/documents/doc-123"
        );
    }

    #[test]
    fn learnchain_auth_payload_deserializes() {
        let auth: LearnChainAuthEnvelope = serde_json::from_value(json!({
            "session": {
                "accessToken": "access-token",
                "refreshToken": null,
                "expiresAt": null,
                "expiresIn": null,
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
        assert_eq!(auth.session.refresh_token, None);
        assert_eq!(auth.session.expires_at, None);
        assert_eq!(auth.session.expires_in, None);
        assert_eq!(auth.session.token_type, "bearer");
        assert_eq!(auth.user.id, "user-123");
        assert_eq!(auth.user.email.as_deref(), Some("person@example.com"));
        assert_eq!(auth.user.username.as_deref(), Some("person"));
    }

    #[test]
    fn extract_next_chunk_paths_deduplicates_results() {
        let html = r#"
            <script src="/_next/static/chunks/aaa.js"></script>
            <script src="/_next/static/chunks/bbb.js"></script>
            <script src="/_next/static/chunks/aaa.js"></script>
        "#;

        let paths = extract_next_chunk_paths(html);

        assert_eq!(
            paths,
            vec![
                "/_next/static/chunks/aaa.js".to_string(),
                "/_next/static/chunks/bbb.js".to_string()
            ]
        );
    }

    #[test]
    fn extract_learnchain_public_auth_config_from_script_finds_supabase_values() {
        let script = r#"
            let config = {
                url: "https://dkwlzdoklmamyfscnctn.supabase.co",
                publishableKey: "sb_publishable_demo_key_123"
            };
        "#;

        let auth_config = extract_learnchain_public_auth_config_from_script(script)
            .expect("expected LearnChain auth config");

        assert_eq!(
            auth_config,
            LearnChainPublicAuthConfig {
                supabase_url: "https://dkwlzdoklmamyfscnctn.supabase.co".to_string(),
                publishable_key: "sb_publishable_demo_key_123".to_string(),
            }
        );
    }

    #[test]
    fn learnchain_password_auth_payload_maps_to_stored_session() {
        let auth: LearnChainPasswordAuthEnvelope = serde_json::from_value(json!({
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "token_type": "bearer",
            "user": {
                "id": "user-123",
                "email": "person@example.com",
                "username": "person"
            }
        }))
        .expect("password auth payload");

        let session = LearnChainStoredSession {
            account_label: learnchain_account_label(&auth.user),
            access_token: auth.access_token,
            refresh_token: auth.refresh_token,
        };

        assert_eq!(session.account_label, "person@example.com");
        assert_eq!(session.access_token, "access-token");
        assert_eq!(session.refresh_token, "refresh-token");
    }

    #[test]
    fn exportable_deep_dive_document_preserves_front_matter() {
        let document = DeepDiveDocument {
            metadata: DeepDiveArtifactMetadata {
                artifact_type: "session_deep_dive".to_string(),
                title: "Deep Dive".to_string(),
                generated_at: "2026-03-08T12:00:00Z".to_string(),
                session_source: "Codex CLI".to_string(),
                session_id: "session-123".to_string(),
                session_timestamp: "2026-03-08T12:00:00Z".to_string(),
                session_date: "2026-03-08".to_string(),
                project_name: "learnchain".to_string(),
                project_cwd: "/workspace/learnchain".to_string(),
                source_file: "/tmp/session.jsonl".to_string(),
                referenced_url_count: 2,
                reviewed_url_count: 1,
                session_analytics: SessionAnalytics {
                    total_tool_calls: 4,
                    successful_tool_calls: 2,
                    failed_tool_calls: 1,
                    unknown_outcome_tool_calls: 1,
                    mcp_tool_calls: 1,
                    external_lookup_calls: 2,
                    adjust_course_count: 1,
                    external_resources: vec![ExternalResourceRef {
                        kind: ExternalResourceKind::Web,
                        tool_name: "web.search_query".to_string(),
                        label: "rust iterators".to_string(),
                        count: 2,
                    }],
                    adjustments: vec![AdjustmentMarker {
                        kind: AdjustmentKind::PostFailurePivot,
                        from_tool_name: "shell".to_string(),
                        to_tool_name: "web.search_query".to_string(),
                        from_argument_summary: Some("cmd=cat missing.txt".to_string()),
                        to_argument_summary: Some("rust iterators".to_string()),
                    }],
                },
            },
            markdown: "# Deep Dive\n\nBody".to_string(),
            path: "output/deep-dives/deep-dive.md".into(),
        };

        let exportable = exportable_deep_dive_document(&document);

        assert!(exportable.markdown.starts_with("+++\n"));
        assert!(exportable.markdown.contains("[session_analytics]"));
        assert!(exportable.markdown.contains("total_tool_calls = 4"));
        assert!(exportable.markdown.contains("# Deep Dive\n\nBody"));
    }

    #[test]
    fn prepare_document_for_learnchain_strips_quiz_answer_reveal_markers() {
        let document = ExportableDocument {
            title: "Deep Dive".to_string(),
            markdown: [
                "+++",
                "title = \"Deep Dive\"",
                "+++",
                "",
                "## Quiz",
                "",
                "- Correct option (correct)",
                "- Distractor",
            ]
            .join("\n"),
        };

        let exportable = prepare_document_for_learnchain(document);

        assert!(!exportable.markdown.contains("- Correct option (correct)"));
        assert!(exportable.markdown.contains("- Correct option"));
        assert!(exportable.markdown.contains("- Distractor"));
        assert!(exportable.markdown.starts_with("+++\n"));
    }

    #[test]
    fn prepare_document_for_learnchain_preserves_non_quiz_correct_text() {
        let document = ExportableDocument {
            title: "Deep Dive".to_string(),
            markdown: [
                "# Notes",
                "",
                "The correct fix was to add a timeout (correct)",
                "  - Indented bullet (correct)",
            ]
            .join("\n"),
        };

        let exportable = prepare_document_for_learnchain(document);

        assert!(
            exportable
                .markdown
                .contains("The correct fix was to add a timeout (correct)")
        );
        assert!(exportable.markdown.contains("  - Indented bullet"));
        assert!(
            !exportable
                .markdown
                .contains("  - Indented bullet (correct)")
        );
    }
}
