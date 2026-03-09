use std::{
    collections::BTreeSet,
    sync::{OnceLock, mpsc::Sender},
    time::Duration,
};

use chrono::Utc;
use color_eyre::eyre::{Result, eyre};
use regex::Regex;
use reqwest::{Client, Url, redirect::Policy};

use super::backend::LlmRequestOptions;
use crate::{
    AiTaskKind, AiTaskMessage, Project,
    config::AiProvider,
    config::DeepDiveSectionsConfig,
    llm::{
        DeepDiveArtifactMetadata, DeepDiveGenerationResult, DeepDiveResearchPlan,
        DeepDiveReviewedSource, LlmBackend, StructuredDeepDiveResponse,
    },
    markdown_rules::MarkdownRules,
    output_manager::{OutputManager, render_quiz_groups_markdown},
    session_sources::{Session, SessionEvent},
};

const MAX_DEEP_DIVE_EVENTS: usize = 24;
const MAX_REVIEW_URLS: usize = 5;
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_FETCH_BYTES: usize = 150 * 1024;
const MAX_FIRST_PROMPT_CHARS: usize = 600;
const MAX_PLAN_EVENT_TEXT_CHARS: usize = 180;
const MAX_PLAN_ARGUMENT_CHARS: usize = 120;
const MAX_PLAN_OUTPUT_CHARS: usize = 120;
const MAX_FETCHED_SOURCE_CHARS: usize = 1800;
const MAX_FALLBACK_ACCOMPLISHMENTS: usize = 5;
const MAX_FALLBACK_LEARNINGS: usize = 3;
const MAX_FALLBACK_TEACHING_ANGLES: usize = 3;

#[derive(Debug, Clone)]
pub(crate) struct SessionResearchBundle {
    pub session: Session,
    pub session_source: String,
    pub project_name: String,
    pub project_cwd: String,
    pub selected_events: Vec<SessionEvent>,
    pub external_urls: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct FetchedSource {
    url: String,
    summary_text: String,
}

pub(crate) fn build_session_research_bundle(
    session_source: &str,
    session: &Session,
) -> SessionResearchBundle {
    let project_cwd = Project::extract_cwd(session);
    let project_name = Project::name_from_cwd(&project_cwd);
    SessionResearchBundle {
        session: session.clone(),
        session_source: session_source.to_string(),
        project_name,
        project_cwd,
        selected_events: select_balanced_events(&session.events, MAX_DEEP_DIVE_EVENTS),
        external_urls: extract_external_urls(session),
    }
}

pub(crate) async fn generate_deep_dive_with_progress(
    backend: &LlmBackend,
    session_source: &str,
    session: Session,
    sections: DeepDiveSectionsConfig,
    min_quiz_questions: usize,
    progress_sender: impl Into<Option<&Sender<AiTaskMessage>>>,
) -> Result<DeepDiveGenerationResult> {
    let sender = progress_sender.into();
    let bundle = build_session_research_bundle(session_source, &session);
    let request_options = LlmRequestOptions::session_deep_dive();

    if let Some(sender) = sender {
        send_progress(sender, "Preparing session research bundle...", 20);
    }

    let plan_prompt = build_research_plan_prompt(&bundle);
    if let Some(sender) = sender {
        send_progress(sender, "Planning deep-dive research...", 35);
    }

    let (mut plan, plan_usage) = if should_skip_llm_research_plan(backend) {
        if let Some(sender) = sender {
            send_progress(
                sender,
                "Using a compact local research plan for Codex CLI...",
                45,
            );
        }
        (build_fallback_research_plan(&bundle), None)
    } else {
        match backend
            .extract_typed_with_options::<DeepDiveResearchPlan>(
                deep_dive_plan_preamble(),
                &plan_prompt,
                "deep-dive research plan",
                request_options,
            )
            .await
        {
            Ok(result) => result,
            Err(err) if err.to_string().contains("timed out") => {
                if let Some(sender) = sender {
                    send_progress(
                        sender,
                        "Research planning timed out, using a compact local fallback...",
                        45,
                    );
                }
                (build_fallback_research_plan(&bundle), None)
            }
            Err(err) => return Err(err),
        }
    };

    plan.selected_urls = sanitize_selected_urls(&bundle.external_urls, &plan.selected_urls);

    if let Some(sender) = sender {
        send_progress(sender, "Reviewing selected external sources...", 55);
    }

    let (fetched_sources, fetch_failures) = fetch_selected_sources(&plan.selected_urls).await;

    if let Some(sender) = sender {
        send_progress(sender, "Composing deep dive...", 80);
    }

    let final_prompt = build_final_deep_dive_prompt(
        &bundle,
        &plan,
        &fetched_sources,
        &fetch_failures,
        min_quiz_questions,
    );
    let (mut response, final_usage) = backend
        .extract_typed_with_options::<StructuredDeepDiveResponse>(
            deep_dive_final_preamble(),
            &final_prompt,
            "structured deep-dive response",
            request_options,
        )
        .await?;

    response.teaching_narrative =
        normalize_teaching_narrative(response.teaching_narrative, &plan.teaching_angles);
    response.title = if response.title.trim().is_empty() {
        format!("Session Deep Dive - {}", bundle.session.date)
    } else {
        response.title.trim().to_string()
    };
    if response.goal.trim().is_empty() {
        response.goal = if plan.inferred_goal.trim().is_empty() {
            "The session goal could not be inferred with high confidence.".to_string()
        } else {
            plan.inferred_goal.trim().to_string()
        };
    }
    response.reviewed_sources =
        sanitize_reviewed_sources(&bundle.external_urls, response.reviewed_sources);

    let metadata = DeepDiveArtifactMetadata {
        artifact_type: "session_deep_dive".to_string(),
        title: response.title.clone(),
        generated_at: Utc::now().to_rfc3339(),
        session_source: bundle.session_source.clone(),
        session_id: bundle.session.id.clone(),
        session_timestamp: bundle.session.timestamp.clone(),
        session_date: bundle.session.date.clone(),
        project_name: bundle.project_name.clone(),
        project_cwd: bundle.project_cwd.clone(),
        source_file: bundle.session.source_file.display().to_string(),
        referenced_url_count: bundle.external_urls.len(),
        reviewed_url_count: response.reviewed_sources.len(),
        session_analytics: bundle.session.analytics.clone(),
    };

    let markdown =
        render_deep_dive_markdown(&metadata, &response, &bundle.external_urls, &sections);
    let document = OutputManager::new()
        .write_deep_dive_markdown(&metadata, &markdown)
        .map_err(|err| eyre!(err))?;

    let usage = merge_usage(plan_usage, final_usage);
    Ok(DeepDiveGenerationResult {
        document,
        response,
        usage,
        reviewed_source_failures: fetch_failures,
    })
}

fn should_skip_llm_research_plan(backend: &LlmBackend) -> bool {
    backend.provider() == AiProvider::CodexCli
}

pub(crate) fn select_balanced_events(
    events: &[SessionEvent],
    max_events: usize,
) -> Vec<SessionEvent> {
    let rules = MarkdownRules::with_max_events(events.len().max(1));
    let eligible: Vec<&SessionEvent> = events
        .iter()
        .filter(|event| rules.should_include_event(event))
        .collect();

    if eligible.len() <= max_events {
        return eligible.into_iter().cloned().collect();
    }

    let last_index = eligible.len() - 1;
    let mut selected_indices = BTreeSet::new();
    for slot in 0..max_events {
        let index = slot * last_index / (max_events - 1);
        selected_indices.insert(index);
    }

    selected_indices
        .into_iter()
        .filter_map(|index| eligible.get(index))
        .cloned()
        .cloned()
        .collect()
}

pub(crate) fn extract_external_urls(session: &Session) -> Vec<String> {
    let mut urls = BTreeSet::new();
    if let Some(prompt) = session.first_user_prompt.as_deref() {
        for url in extract_urls_from_text(prompt) {
            urls.insert(url);
        }
    }

    for event in &session.events {
        for text in &event.content_texts {
            for url in extract_urls_from_text(text) {
                urls.insert(url);
            }
        }
        if let Some(arguments) = event.arguments.as_deref() {
            for url in extract_urls_from_text(arguments) {
                urls.insert(url);
            }
        }
        if let Some(output) = event.output.as_deref() {
            for url in extract_urls_from_text(output) {
                urls.insert(url);
            }
        }
    }

    urls.into_iter().collect()
}

pub(crate) fn render_deep_dive_markdown(
    metadata: &DeepDiveArtifactMetadata,
    response: &StructuredDeepDiveResponse,
    external_urls: &[String],
    sections: &DeepDiveSectionsConfig,
) -> String {
    let mut markdown = Vec::new();
    markdown.push(format!("# {}", response.title));
    markdown.push(String::new());
    if sections.session_metadata {
        markdown.push("## Session Metadata".to_string());
        markdown.push(format!("- Session source: {}", metadata.session_source));
        markdown.push(format!("- Session date: {}", metadata.session_date));
        markdown.push(format!("- Session id: {}", metadata.session_id));
        markdown.push(format!("- Project: {}", metadata.project_name));
        markdown.push(format!("- Working directory: {}", metadata.project_cwd));
        markdown.push(format!("- Source file: {}", metadata.source_file));
        markdown.push(String::new());
    }
    if !metadata.session_analytics.is_empty() {
        markdown.push("## Session Analytics".to_string());
        markdown.push(format!(
            "- Tool calls: {} / {} successful",
            metadata.session_analytics.successful_tool_calls,
            metadata.session_analytics.total_tool_calls
        ));
        markdown.push(format!(
            "- Failed/problematic calls: {}",
            metadata.session_analytics.failed_tool_calls
        ));
        markdown.push(format!(
            "- Unknown outcome calls: {}",
            metadata.session_analytics.unknown_outcome_tool_calls
        ));
        markdown.push(format!(
            "- MCP calls: {}",
            metadata.session_analytics.mcp_tool_calls
        ));
        markdown.push(format!(
            "- External lookups: {}",
            metadata.session_analytics.external_lookup_calls
        ));
        markdown.push(format!(
            "- Adjustments: {}",
            metadata.session_analytics.adjust_course_count
        ));
        markdown.push(String::new());

        markdown.push("### External Resources".to_string());
        if metadata.session_analytics.external_resources.is_empty() {
            markdown.push("- None".to_string());
        } else {
            for resource in &metadata.session_analytics.external_resources {
                markdown.push(format!("- {} x{}", resource.label, resource.count));
            }
        }
        markdown.push(String::new());

        markdown.push("### Adjustments Detected".to_string());
        if metadata.session_analytics.adjustments.is_empty() {
            markdown.push("- None".to_string());
        } else {
            for adjustment in &metadata.session_analytics.adjustments {
                markdown.push(format!(
                    "- {}",
                    crate::session_analytics::describe_adjustment(adjustment)
                ));
            }
        }
        markdown.push(String::new());
    }
    if sections.goal {
        markdown.push("## Goal".to_string());
        markdown.push(response.goal.clone());
        markdown.push(String::new());
    }
    if sections.accomplishments {
        markdown.push("## What Was Accomplished".to_string());
        push_bullets(&mut markdown, &response.accomplishments);
        markdown.push(String::new());
    }
    if sections.interesting_learnings {
        markdown.push("## Interesting or Unexpected Learnings".to_string());
        push_bullets(&mut markdown, &response.interesting_learnings);
        markdown.push(String::new());
    }
    if sections.teaching_narrative {
        markdown.push("## Teaching Narrative".to_string());
        markdown.push(String::new());
        if response.teaching_narrative.is_empty() {
            markdown.push("No teaching narrative was provided.".to_string());
        } else {
            push_markdown_blocks(&mut markdown, &response.teaching_narrative);
        }
        markdown.push(String::new());
    }
    markdown.push("## Quiz".to_string());
    markdown.push(String::new());
    if response.quiz_groups.is_empty() {
        markdown.push("No quiz questions were generated.".to_string());
    } else {
        markdown.push(render_quiz_groups_markdown(&response.quiz_groups, 3));
    }
    markdown.push(String::new());
    if sections.reviewed_external_sources {
        markdown.push("## Reviewed External Sources".to_string());
        if response.reviewed_sources.is_empty() {
            markdown.push("No external sources were reviewed during generation.".to_string());
        } else {
            for source in &response.reviewed_sources {
                markdown.push(format!("### {}", source.url));
                markdown.push(source.summary.clone());
                markdown.push(String::new());
                markdown.push(format!("Why it mattered: {}", source.why_it_matters));
                markdown.push(String::new());
            }
        }
    }
    if sections.referenced_urls {
        markdown.push("## Referenced URLs".to_string());
        if external_urls.is_empty() {
            markdown.push("No external URLs were referenced in the session.".to_string());
        } else {
            for url in external_urls {
                markdown.push(format!("- {}", url));
            }
        }
    }
    markdown.join("\n")
}

fn build_research_plan_prompt(bundle: &SessionResearchBundle) -> String {
    let first_user_prompt = truncate_for_prompt(
        bundle
            .session
            .first_user_prompt
            .as_deref()
            .unwrap_or("No initial user prompt was captured."),
        MAX_FIRST_PROMPT_CHARS,
    );
    let event_details = if bundle.selected_events.is_empty() {
        "- No qualifying events were captured.".to_string()
    } else {
        bundle
            .selected_events
            .iter()
            .map(format_event_for_research_plan)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let urls = if bundle.external_urls.is_empty() {
        "No external URLs were referenced.".to_string()
    } else {
        bundle
            .external_urls
            .iter()
            .enumerate()
            .map(|(index, url)| format!("{}. {}", index + 1, url))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "Plan a session deep dive using this session transcript digest.\n\nSession source: {}\nSession date: {}\nSession id: {}\nProject: {}\nWorking directory: {}\nFirst user prompt:\n{}\n\nSelected session events:\n{}\n\nReferenced external URLs:\n{}\n",
        bundle.session_source,
        bundle.session.date,
        bundle.session.id,
        bundle.project_name,
        bundle.project_cwd,
        first_user_prompt,
        event_details,
        urls
    )
}

fn build_final_deep_dive_prompt(
    bundle: &SessionResearchBundle,
    plan: &DeepDiveResearchPlan,
    fetched_sources: &[FetchedSource],
    fetch_failures: &[String],
    min_quiz_questions: usize,
) -> String {
    let accomplishments = if plan.candidate_accomplishments.is_empty() {
        "None provided.".to_string()
    } else {
        plan.candidate_accomplishments
            .iter()
            .map(|item| format!("- {}", item))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let learnings = if plan.candidate_interesting_learnings.is_empty() {
        "None provided.".to_string()
    } else {
        plan.candidate_interesting_learnings
            .iter()
            .map(|item| format!("- {}", item))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let teaching_angles = if plan.teaching_angles.is_empty() {
        "None provided.".to_string()
    } else {
        plan.teaching_angles
            .iter()
            .map(|item| format!("- {}", item))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let fetched = if fetched_sources.is_empty() {
        "No external sources were fetched.".to_string()
    } else {
        fetched_sources
            .iter()
            .map(|source| {
                format!(
                    "URL: {}\nContent digest:\n{}\n",
                    source.url,
                    truncate_for_prompt(&source.summary_text, MAX_FETCHED_SOURCE_CHARS)
                )
            })
            .collect::<Vec<_>>()
            .join("\n---\n")
    };
    let failures = if fetch_failures.is_empty() {
        "No fetch failures.".to_string()
    } else {
        fetch_failures
            .iter()
            .map(|failure| format!("- {}", failure))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "Write the final structured session deep dive.\n\nFormat requirements:\n- Keep the goal concise and specific.\n- Keep accomplishments and learnings scannable.\n- For teaching_narrative, do not return one large wall of text.\n- Structure teaching_narrative as markdown-friendly blocks with 3 to 5 short sections.\n- Prefer using a `###` subheading followed by a short paragraph for each section.\n- If needed, use an empty string item between sections to preserve spacing.\n- Populate quiz_groups using the same grouped quiz structure LearnChain uses for standalone quizzes.\n- Return at least {} quiz questions overall across quiz_groups.\n- Quiz questions should focus on the code, libraries, frameworks, tools, or APIs from the session rather than LearnChain's own implementation details.\n- Each quiz question should include answer options, exactly one correct answer, and any relevant supporting resources.\n\nInferred goal: {}\nCandidate accomplishments:\n{}\n\nCandidate interesting learnings:\n{}\n\nTeaching angles:\n{}\n\nSession URL inventory:\n{}\n\nFetched source digests:\n{}\n\nFetch failures:\n{}\n",
        min_quiz_questions,
        plan.inferred_goal,
        accomplishments,
        learnings,
        teaching_angles,
        if bundle.external_urls.is_empty() {
            "No URLs were referenced.".to_string()
        } else {
            bundle.external_urls.join("\n")
        },
        fetched,
        failures
    )
}

fn deep_dive_plan_preamble() -> &'static str {
    "You are preparing a quick first-pass session deep-dive research plan. Do not spend time on hidden planning or exhaustive analysis. Use the provided digest as-is, make the best immediate inference, keep items concise, and prefer returning fewer items over deliberating longer. Choose at most five URLs from the provided inventory for follow-up review. Never invent URLs and never select a URL that is not in the provided inventory."
}

fn deep_dive_final_preamble() -> &'static str {
    "You are writing a precise, educational deep dive for a coding session. Base your answer on the provided session digest and fetched source notes. Explain what the user would have learned by implementing the feature themselves. The teaching narrative must be easy to read, with short paragraphs and markdown subheadings when appropriate. Include embedded quiz_groups that follow the same structure as LearnChain's standalone quiz flow. Only reference URLs from the provided session inventory."
}

fn format_event_for_research_plan(event: &SessionEvent) -> String {
    let mut parts = Vec::new();
    let text_digest = event
        .content_texts
        .iter()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if !text_digest.is_empty() {
        parts.push(format!(
            "texts={}",
            truncate_for_prompt(&text_digest, MAX_PLAN_EVENT_TEXT_CHARS)
        ));
    }
    if let Some(arguments) = event.arguments.as_deref().map(str::trim)
        && !arguments.is_empty()
    {
        parts.push(format!(
            "arguments={}",
            truncate_for_prompt(arguments, MAX_PLAN_ARGUMENT_CHARS)
        ));
    }
    if let Some(output) = event.output.as_deref().map(str::trim)
        && !output.is_empty()
    {
        parts.push(format!(
            "output={}",
            truncate_for_prompt(output, MAX_PLAN_OUTPUT_CHARS)
        ));
    }

    if parts.is_empty() {
        format!("- {} | {}", event.timestamp, event.payload_type)
    } else {
        format!(
            "- {} | {} | {}",
            event.timestamp,
            event.payload_type,
            parts.join(" | ")
        )
    }
}

fn build_fallback_research_plan(bundle: &SessionResearchBundle) -> DeepDiveResearchPlan {
    let mut candidate_accomplishments = Vec::new();
    if !bundle.session.summary.trim().is_empty() {
        push_unique_bounded(
            &mut candidate_accomplishments,
            truncate_for_prompt(bundle.session.summary.trim(), MAX_PLAN_EVENT_TEXT_CHARS),
            MAX_FALLBACK_ACCOMPLISHMENTS,
        );
    }

    for event in &bundle.selected_events {
        for candidate in event
            .content_texts
            .iter()
            .map(|text| text.trim())
            .chain(event.output.iter().map(|text| text.trim()))
        {
            if candidate.is_empty() || candidate.starts_with("cwd:") {
                continue;
            }
            push_unique_bounded(
                &mut candidate_accomplishments,
                truncate_for_prompt(candidate, MAX_PLAN_EVENT_TEXT_CHARS),
                MAX_FALLBACK_ACCOMPLISHMENTS,
            );
            if candidate_accomplishments.len() == MAX_FALLBACK_ACCOMPLISHMENTS {
                break;
            }
        }
        if candidate_accomplishments.len() == MAX_FALLBACK_ACCOMPLISHMENTS {
            break;
        }
    }

    if candidate_accomplishments.is_empty() {
        candidate_accomplishments.push("The transcript captured concrete implementation work, but no concise accomplishment summary was available.".to_string());
    }

    let mut candidate_interesting_learnings = Vec::new();
    if !bundle.external_urls.is_empty() {
        push_unique_bounded(
            &mut candidate_interesting_learnings,
            "The session cross-checked implementation details against external references mentioned in the transcript.".to_string(),
            MAX_FALLBACK_LEARNINGS,
        );
    }
    if bundle
        .selected_events
        .iter()
        .any(|event| event.payload_type.contains("function"))
    {
        push_unique_bounded(
            &mut candidate_interesting_learnings,
            "The workflow alternated between reasoning steps and tool-driven actions instead of staying purely conceptual.".to_string(),
            MAX_FALLBACK_LEARNINGS,
        );
    }
    push_unique_bounded(
        &mut candidate_interesting_learnings,
        "The session can be explained as a sequence of small implementation decisions rather than a single large code dump.".to_string(),
        MAX_FALLBACK_LEARNINGS,
    );

    let mut teaching_angles = Vec::new();
    push_unique_bounded(
        &mut teaching_angles,
        format!(
            "Explain how the session moved from the initial request to concrete changes in {}.",
            bundle.project_name
        ),
        MAX_FALLBACK_TEACHING_ANGLES,
    );
    push_unique_bounded(
        &mut teaching_angles,
        "Highlight the implementation tradeoffs and why the chosen path fit the session constraints.".to_string(),
        MAX_FALLBACK_TEACHING_ANGLES,
    );
    if !bundle.external_urls.is_empty() {
        push_unique_bounded(
            &mut teaching_angles,
            "Connect the implementation back to the external references that appeared in the session.".to_string(),
            MAX_FALLBACK_TEACHING_ANGLES,
        );
    }

    DeepDiveResearchPlan {
        inferred_goal: infer_fallback_goal(bundle),
        candidate_accomplishments,
        candidate_interesting_learnings,
        teaching_angles,
        selected_urls: bundle
            .external_urls
            .iter()
            .take(MAX_REVIEW_URLS)
            .cloned()
            .collect(),
    }
}

fn infer_fallback_goal(bundle: &SessionResearchBundle) -> String {
    if let Some(prompt) = bundle.session.first_user_prompt.as_deref() {
        let trimmed = prompt.trim();
        if !trimmed.is_empty() {
            return truncate_for_prompt(trimmed, MAX_FIRST_PROMPT_CHARS);
        }
    }
    if !bundle.session.summary.trim().is_empty() {
        return truncate_for_prompt(bundle.session.summary.trim(), MAX_FIRST_PROMPT_CHARS);
    }
    format!(
        "Summarize the implementation work captured in session {} for {}.",
        bundle.session.id, bundle.project_name
    )
}

fn push_unique_bounded(items: &mut Vec<String>, candidate: String, max_items: usize) {
    if items.len() >= max_items {
        return;
    }
    if items.iter().any(|existing| existing == &candidate) {
        return;
    }
    items.push(candidate);
}

fn sanitize_selected_urls(inventory: &[String], selected: &[String]) -> Vec<String> {
    let inventory: BTreeSet<String> = inventory.iter().cloned().collect();
    let mut result = Vec::new();
    for url in selected {
        let Some(normalized) = normalize_url(url) else {
            continue;
        };
        if inventory.contains(&normalized) && !result.contains(&normalized) {
            result.push(normalized);
        }
        if result.len() == MAX_REVIEW_URLS {
            break;
        }
    }
    result
}

fn sanitize_reviewed_sources(
    inventory: &[String],
    reviewed_sources: Vec<DeepDiveReviewedSource>,
) -> Vec<DeepDiveReviewedSource> {
    let inventory: BTreeSet<String> = inventory.iter().cloned().collect();
    let mut sanitized = Vec::new();
    for source in reviewed_sources {
        let Some(normalized) = normalize_url(&source.url) else {
            continue;
        };
        if inventory.contains(&normalized)
            && !sanitized
                .iter()
                .any(|item: &DeepDiveReviewedSource| item.url == normalized)
        {
            sanitized.push(DeepDiveReviewedSource {
                url: normalized,
                summary: source.summary.trim().to_string(),
                why_it_matters: source.why_it_matters.trim().to_string(),
            });
        }
    }
    sanitized
}

async fn fetch_selected_sources(selected_urls: &[String]) -> (Vec<FetchedSource>, Vec<String>) {
    if selected_urls.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let client = match Client::builder()
        .redirect(Policy::limited(5))
        .user_agent("learnchain/0.1")
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            return (
                Vec::new(),
                vec![format!("Failed to build HTTP client: {}", err)],
            );
        }
    };

    let mut fetched = Vec::new();
    let mut failures = Vec::new();
    for url in selected_urls {
        match fetch_single_source(&client, url).await {
            Ok(summary_text) => fetched.push(FetchedSource {
                url: url.clone(),
                summary_text,
            }),
            Err(err) => failures.push(format!("{} ({})", url, err)),
        }
    }
    (fetched, failures)
}

async fn fetch_single_source(client: &Client, url: &str) -> Result<String> {
    let request = client.get(url);
    let response = tokio::time::timeout(FETCH_TIMEOUT, request.send())
        .await
        .map_err(|_| {
            eyre!(
                "request timed out after {} seconds",
                FETCH_TIMEOUT.as_secs()
            )
        })?
        .map_err(|err| eyre!("request failed: {}", err))?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let bytes = read_body_with_limit(response).await?;
    body_to_text(content_type.as_deref(), &bytes)
}

async fn read_body_with_limit(mut response: reqwest::Response) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| eyre!("failed to read body chunk: {}", err))?
    {
        let remaining = MAX_FETCH_BYTES.saturating_sub(bytes.len());
        if remaining == 0 {
            break;
        }
        let take = remaining.min(chunk.len());
        bytes.extend_from_slice(&chunk[..take]);
        if bytes.len() >= MAX_FETCH_BYTES {
            break;
        }
    }
    Ok(bytes)
}

fn body_to_text(content_type: Option<&str>, bytes: &[u8]) -> Result<String> {
    let content_type = content_type.unwrap_or("").to_ascii_lowercase();
    if content_type.contains("text/html") || looks_like_html(bytes) {
        return html2text::from_read(bytes, 100)
            .map_err(|err| eyre!("failed to convert HTML to text: {}", err));
    }
    if content_type.contains("application/json")
        || content_type.contains("text/plain")
        || content_type.contains("text/markdown")
        || content_type.contains("application/xml")
        || content_type.contains("text/xml")
        || content_type.is_empty()
    {
        return Ok(String::from_utf8_lossy(bytes).to_string());
    }
    Err(eyre!("unsupported content type: {}", content_type))
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let snippet = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).to_ascii_lowercase();
    snippet.contains("<html") || snippet.contains("<!doctype html")
}

fn send_progress(sender: &Sender<AiTaskMessage>, message: &str, percent: u8) {
    let _ = sender.send(AiTaskMessage::Progress(
        AiTaskKind::SessionDeepDive,
        message.to_string(),
        percent,
    ));
}

fn merge_usage(
    left: Option<crate::llm::types::LlmUsage>,
    right: Option<crate::llm::types::LlmUsage>,
) -> Option<crate::llm::types::LlmUsage> {
    match (left, right) {
        (Some(left), Some(right)) => Some(crate::llm::types::LlmUsage {
            input_tokens: left.input_tokens + right.input_tokens,
            output_tokens: left.output_tokens + right.output_tokens,
            total_tokens: left.total_tokens + right.total_tokens,
        }),
        (Some(usage), None) | (None, Some(usage)) => Some(usage),
        (None, None) => None,
    }
}

fn extract_urls_from_text(text: &str) -> Vec<String> {
    static URL_REGEX: OnceLock<Regex> = OnceLock::new();
    let regex = URL_REGEX.get_or_init(|| Regex::new(r#"https?://[^\s<>"]+"#).unwrap());
    let mut urls = Vec::new();
    for capture in regex.find_iter(text) {
        if let Some(url) = normalize_url(capture.as_str()) {
            urls.push(url);
        }
    }
    urls
}

fn normalize_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim_end_matches(|ch: char| {
        matches!(ch, ')' | ']' | '}' | ',' | '.' | ';' | ':' | '\'' | '\"')
    });
    let mut url = Url::parse(trimmed).ok()?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return None,
    }
    if url.fragment().is_none() && url.path().is_empty() {
        url.set_path("/");
    }
    Some(url.to_string())
}

fn truncate_for_prompt(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

fn push_bullets(markdown: &mut Vec<String>, items: &[String]) {
    if items.is_empty() {
        markdown.push("- No entries provided.".to_string());
        return;
    }
    for item in items {
        markdown.push(format!("- {}", item));
    }
}

fn push_markdown_blocks(markdown: &mut Vec<String>, blocks: &[String]) {
    let mut needs_separator = false;
    for block in blocks {
        let trimmed = block.trim();
        if trimmed.is_empty() {
            if !markdown.last().is_some_and(|line| line.is_empty()) {
                markdown.push(String::new());
            }
            needs_separator = false;
            continue;
        }

        if needs_separator && !markdown.last().is_some_and(|line| line.is_empty()) {
            markdown.push(String::new());
        }
        markdown.push(trimmed.to_string());
        needs_separator = true;
    }
}

fn normalize_teaching_narrative(
    teaching_narrative: Vec<String>,
    teaching_angles: &[String],
) -> Vec<String> {
    let normalized = teaching_narrative
        .into_iter()
        .flat_map(|block| split_markdown_blocks(&block))
        .collect::<Vec<_>>();

    let has_headings = normalized
        .iter()
        .any(|block| block.trim_start().starts_with('#'));
    if has_headings || normalized.len() != 1 {
        return normalized;
    }

    let paragraphs = split_dense_paragraphs(&normalized[0]);
    if paragraphs.len() <= 1 {
        return normalized;
    }

    let mut structured = Vec::new();
    for (index, paragraph) in paragraphs.into_iter().enumerate() {
        if let Some(angle) = teaching_angles.get(index) {
            let heading = angle.trim().trim_end_matches('.');
            if !heading.is_empty() {
                structured.push(format!("### {}", heading));
            }
        }
        structured.push(paragraph);
    }
    structured
}

fn split_markdown_blocks(block: &str) -> Vec<String> {
    block
        .split("\n\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn split_dense_paragraphs(text: &str) -> Vec<String> {
    let sentences = split_sentences(text);
    if sentences.len() < 4 {
        return vec![text.trim().to_string()];
    }

    let chunk_size = if sentences.len() >= 9 { 3 } else { 2 };
    sentences
        .chunks(chunk_size)
        .map(|chunk| chunk.join(" "))
        .collect()
}

fn split_sentences(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let mut sentences = Vec::new();
    let mut start = 0;
    let mut index = 0;

    while index < chars.len() {
        let current = chars[index];
        let is_terminal = matches!(current, '.' | '!' | '?');
        let next_is_boundary = index + 1 == chars.len() || chars[index + 1].is_whitespace();

        if is_terminal && next_is_boundary {
            let sentence: String = chars[start..=index].iter().collect();
            let sentence = sentence.trim();
            if !sentence.is_empty() {
                sentences.push(sentence.to_string());
            }

            index += 1;
            while index < chars.len() && chars[index].is_whitespace() {
                index += 1;
            }
            start = index;
            continue;
        }

        index += 1;
    }

    if start < chars.len() {
        let sentence: String = chars[start..].iter().collect();
        let sentence = sentence.trim();
        if !sentence.is_empty() {
            sentences.push(sentence.to_string());
        }
    }

    if sentences.is_empty() {
        vec![trimmed.to_string()]
    } else {
        sentences
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AiProvider, ResolvedLlmConfig};
    use crate::llm::DeepDiveArtifactMetadata;
    use crate::llm::types::{KnowledgeResponse, QuizItem, QuizOption};
    use crate::session_analytics::{
        AdjustmentKind, AdjustmentMarker, ExternalResourceKind, ExternalResourceRef,
        SessionAnalytics,
    };
    use crate::session_sources::SessionEventKind;
    use std::path::PathBuf;

    fn sample_session() -> Session {
        Session {
            id: "session-123".to_string(),
            date: "2026-03-06".to_string(),
            timestamp: "2026-03-06T12:00:00Z".to_string(),
            cwd: "/workspace/learnchain".to_string(),
            summary: "Add deep-dive support".to_string(),
            first_user_prompt: Some(
                "Use docs at https://docs.rs/reqwest and https://ratatui.rs".to_string(),
            ),
            source_file: PathBuf::from("/tmp/session.jsonl"),
            source_label: "Codex CLI".to_string(),
            analytics: SessionAnalytics::default(),
            events: vec![
                SessionEvent {
                    timestamp: "2026-03-06T12:00:00Z".to_string(),
                    payload_type: "message".to_string(),
                    event_kind: SessionEventKind::Message,
                    call_id: None,
                    tool_name: None,
                    arguments: Some(
                        "{\"url\":\"https://docs.rs/reqwest/latest/reqwest/\"}".to_string(),
                    ),
                    output: Some("See https://github.com/0xPlaygrounds/rig".to_string()),
                    result_metadata: None,
                    content_texts: vec![
                        "Read https://ratatui.rs/examples/".to_string(),
                        "cwd: /workspace/learnchain".to_string(),
                    ],
                },
                SessionEvent {
                    timestamp: "2026-03-06T12:05:00Z".to_string(),
                    payload_type: "function_call".to_string(),
                    event_kind: SessionEventKind::ToolCall,
                    call_id: None,
                    tool_name: Some("shell".to_string()),
                    arguments: None,
                    output: Some("Done".to_string()),
                    result_metadata: None,
                    content_texts: vec!["Updated output manager".to_string()],
                },
            ],
        }
    }

    #[test]
    fn extract_external_urls_deduplicates_and_normalizes() {
        let urls = extract_external_urls(&sample_session());
        assert!(urls.contains(&"https://docs.rs/reqwest/latest/reqwest/".to_string()));
        assert!(urls.contains(&"https://ratatui.rs/".to_string()));
        assert!(urls.contains(&"https://github.com/0xPlaygrounds/rig".to_string()));
        assert_eq!(urls.len(), 5);
    }

    #[test]
    fn select_balanced_events_spans_session_timeline() {
        let mut events = Vec::new();
        for index in 0..10 {
            events.push(SessionEvent {
                timestamp: format!("{}", index),
                payload_type: "message".to_string(),
                event_kind: SessionEventKind::Message,
                call_id: None,
                tool_name: None,
                arguments: None,
                output: None,
                result_metadata: None,
                content_texts: vec![format!("event {}", index)],
            });
        }

        let selected = select_balanced_events(&events, 4);
        let timestamps: Vec<String> = selected.into_iter().map(|event| event.timestamp).collect();
        assert_eq!(timestamps, vec!["0", "3", "6", "9"]);
    }

    #[test]
    fn render_deep_dive_markdown_uses_expected_section_order() {
        let metadata = DeepDiveArtifactMetadata {
            artifact_type: "session_deep_dive".to_string(),
            title: "Deep Dive".to_string(),
            generated_at: "2026-03-06T12:00:00Z".to_string(),
            session_source: "Codex CLI".to_string(),
            session_id: "session-123".to_string(),
            session_timestamp: "2026-03-06T12:00:00Z".to_string(),
            session_date: "2026-03-06".to_string(),
            project_name: "learnchain".to_string(),
            project_cwd: "/workspace/learnchain".to_string(),
            source_file: "/tmp/session.jsonl".to_string(),
            referenced_url_count: 2,
            reviewed_url_count: 1,
            session_analytics: SessionAnalytics {
                total_tool_calls: 4,
                successful_tool_calls: 3,
                failed_tool_calls: 1,
                unknown_outcome_tool_calls: 0,
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
        };
        let response = StructuredDeepDiveResponse {
            title: "Deep Dive".to_string(),
            goal: "Ship the feature".to_string(),
            accomplishments: vec!["Added a picker".to_string()],
            interesting_learnings: vec!["Learned about scroll handling".to_string()],
            teaching_narrative: vec!["The implementation used shared state.".to_string()],
            reviewed_sources: vec![DeepDiveReviewedSource {
                url: "https://ratatui.rs/".to_string(),
                summary: "Docs summary".to_string(),
                why_it_matters: "Explains scrolling.".to_string(),
            }],
            quiz_groups: vec![KnowledgeResponse {
                knowledge_type_group: "State management".to_string(),
                summary: "App state drives the picker and viewer.".to_string(),
                knowledge_type_language: "Rust".to_string(),
                quiz: vec![QuizItem {
                    question: "Which layer owns the deep-dive scroll offset?".to_string(),
                    options: vec![
                        QuizOption {
                            selection: "The app state".to_string(),
                            is_correct_answer: true,
                        },
                        QuizOption {
                            selection: "The renderer only".to_string(),
                            is_correct_answer: false,
                        },
                    ],
                    resources: vec!["src/main.rs".to_string()],
                }],
            }],
        };

        let markdown = render_deep_dive_markdown(
            &metadata,
            &response,
            &["https://ratatui.rs/".to_string()],
            &DeepDiveSectionsConfig::default(),
        );
        let goal = markdown.find("## Goal").unwrap();
        let accomplished = markdown.find("## What Was Accomplished").unwrap();
        let learnings = markdown
            .find("## Interesting or Unexpected Learnings")
            .unwrap();
        let narrative = markdown.find("## Teaching Narrative").unwrap();
        let quiz = markdown.find("## Quiz").unwrap();
        let reviewed = markdown.find("## Reviewed External Sources").unwrap();
        let urls = markdown.find("## Referenced URLs").unwrap();
        let analytics = markdown.find("## Session Analytics").unwrap();
        let resources = markdown.find("### External Resources").unwrap();
        let adjustments = markdown.find("### Adjustments Detected").unwrap();
        assert!(analytics < goal);
        assert!(resources < goal);
        assert!(adjustments < goal);
        assert!(goal < accomplished);
        assert!(accomplished < learnings);
        assert!(learnings < narrative);
        assert!(narrative < quiz);
        assert!(quiz < reviewed);
        assert!(reviewed < urls);
        assert!(markdown.contains("- Tool calls: 3 / 4 successful"));
        assert!(markdown.contains("- rust iterators x2"));
        assert!(markdown.contains(
            "- shell -> web.search_query (pivot after failure): cmd=cat missing.txt -> rust iterators"
        ));
        assert!(markdown.contains("### State management"));
        assert!(markdown.contains("#### Question 1"));
    }

    #[test]
    fn render_deep_dive_markdown_omits_disabled_sections() {
        let metadata = DeepDiveArtifactMetadata {
            artifact_type: "session_deep_dive".to_string(),
            title: "Deep Dive".to_string(),
            generated_at: "2026-03-06T12:00:00Z".to_string(),
            session_source: "Codex CLI".to_string(),
            session_id: "session-123".to_string(),
            session_timestamp: "2026-03-06T12:00:00Z".to_string(),
            session_date: "2026-03-06".to_string(),
            project_name: "learnchain".to_string(),
            project_cwd: "/workspace/learnchain".to_string(),
            source_file: "/tmp/session.jsonl".to_string(),
            referenced_url_count: 2,
            reviewed_url_count: 1,
            session_analytics: SessionAnalytics::default(),
        };
        let response = StructuredDeepDiveResponse {
            title: "Deep Dive".to_string(),
            goal: "Ship the feature".to_string(),
            accomplishments: vec!["Added a picker".to_string()],
            interesting_learnings: vec!["Learned about scroll handling".to_string()],
            teaching_narrative: vec!["The implementation used shared state.".to_string()],
            reviewed_sources: vec![DeepDiveReviewedSource {
                url: "https://ratatui.rs/".to_string(),
                summary: "Docs summary".to_string(),
                why_it_matters: "Explains scrolling.".to_string(),
            }],
            quiz_groups: vec![KnowledgeResponse {
                knowledge_type_group: "Session selection".to_string(),
                summary: "Selection drives deep-dive generation.".to_string(),
                knowledge_type_language: "Rust".to_string(),
                quiz: vec![QuizItem {
                    question: "Which target starts a deep dive?".to_string(),
                    options: vec![
                        QuizOption {
                            selection: "SessionSelectionTarget::DeepDive".to_string(),
                            is_correct_answer: true,
                        },
                        QuizOption {
                            selection: "AppView::Learning".to_string(),
                            is_correct_answer: false,
                        },
                    ],
                    resources: Vec::new(),
                }],
            }],
        };
        let sections = DeepDiveSectionsConfig {
            session_metadata: false,
            goal: true,
            accomplishments: false,
            interesting_learnings: true,
            teaching_narrative: false,
            reviewed_external_sources: false,
            referenced_urls: true,
        };

        let markdown = render_deep_dive_markdown(
            &metadata,
            &response,
            &["https://ratatui.rs/".to_string()],
            &sections,
        );

        assert!(markdown.contains("# Deep Dive"));
        assert!(markdown.contains("## Goal"));
        assert!(markdown.contains("## Interesting or Unexpected Learnings"));
        assert!(markdown.contains("## Quiz"));
        assert!(markdown.contains("### Session selection"));
        assert!(markdown.contains("## Referenced URLs"));
        assert!(!markdown.contains("## Session Metadata"));
        assert!(!markdown.contains("## What Was Accomplished"));
        assert!(!markdown.contains("## Teaching Narrative"));
        assert!(!markdown.contains("## Reviewed External Sources"));
    }

    #[test]
    fn format_event_for_research_plan_skips_empty_fields() {
        let event = SessionEvent {
            timestamp: "2026-03-06T12:00:00Z".to_string(),
            payload_type: "message".to_string(),
            event_kind: SessionEventKind::Message,
            call_id: None,
            tool_name: None,
            arguments: Some(String::new()),
            output: None,
            result_metadata: None,
            content_texts: vec!["First change".to_string(), "  ".to_string()],
        };

        let formatted = format_event_for_research_plan(&event);

        assert!(formatted.contains("texts=First change"));
        assert!(!formatted.contains("arguments="));
        assert!(!formatted.contains("output="));
    }

    #[test]
    fn fallback_research_plan_limits_urls_and_uses_prompt_as_goal() {
        let mut session = sample_session();
        session.first_user_prompt = Some("Ship the session deep-dive flow".to_string());
        session.summary = "Deep-dive generation work".to_string();
        session.events.push(SessionEvent {
            timestamp: "2026-03-06T12:10:00Z".to_string(),
            payload_type: "message".to_string(),
            event_kind: SessionEventKind::Message,
            call_id: None,
            tool_name: None,
            arguments: None,
            output: Some("Added timeout fallback".to_string()),
            result_metadata: None,
            content_texts: vec!["Updated Codex CLI defaults".to_string()],
        });

        let bundle = build_session_research_bundle("Codex CLI", &session);
        let plan = build_fallback_research_plan(&bundle);

        assert_eq!(plan.inferred_goal, "Ship the session deep-dive flow");
        assert!(!plan.candidate_accomplishments.is_empty());
        assert!(plan.selected_urls.len() <= MAX_REVIEW_URLS);
    }

    #[test]
    fn codex_cli_skips_llm_research_plan() {
        let backend = LlmBackend::from_config(
            ResolvedLlmConfig {
                provider: AiProvider::CodexCli,
                model_name: "codex-exec".to_string(),
                model_label: "CLI default".to_string(),
                api_key: String::new(),
            },
            "output",
        )
        .unwrap();

        assert!(should_skip_llm_research_plan(&backend));
    }

    #[test]
    fn deep_dive_plan_preamble_requests_fast_first_pass() {
        let preamble = deep_dive_plan_preamble();

        assert!(preamble.contains("quick first-pass"));
        assert!(preamble.contains("Do not spend time on hidden planning"));
    }

    #[test]
    fn final_prompt_requests_structured_teaching_narrative() {
        let bundle = build_session_research_bundle("Codex CLI", &sample_session());
        let prompt = build_final_deep_dive_prompt(
            &bundle,
            &DeepDiveResearchPlan {
                inferred_goal: "Ship a deep dive".to_string(),
                candidate_accomplishments: vec!["Added formatting".to_string()],
                candidate_interesting_learnings: vec!["Markdown needs spacing".to_string()],
                teaching_angles: vec!["Explain the implementation path".to_string()],
                selected_urls: Vec::new(),
            },
            &[],
            &[],
            5,
        );

        assert!(prompt.contains("do not return one large wall of text"));
        assert!(prompt.contains("3 to 5 short sections"));
        assert!(prompt.contains("`###` subheading"));
        assert!(prompt.contains("quiz_groups"));
        assert!(prompt.contains("at least 5 quiz questions overall"));
    }

    #[test]
    fn render_deep_dive_markdown_preserves_teaching_narrative_spacing() {
        let metadata = DeepDiveArtifactMetadata {
            artifact_type: "session_deep_dive".to_string(),
            title: "Deep Dive".to_string(),
            generated_at: "2026-03-06T12:00:00Z".to_string(),
            session_source: "Codex CLI".to_string(),
            session_id: "session-123".to_string(),
            session_timestamp: "2026-03-06T12:00:00Z".to_string(),
            session_date: "2026-03-06".to_string(),
            project_name: "learnchain".to_string(),
            project_cwd: "/workspace/learnchain".to_string(),
            source_file: "/tmp/session.jsonl".to_string(),
            referenced_url_count: 0,
            reviewed_url_count: 0,
            session_analytics: SessionAnalytics::default(),
        };
        let response = StructuredDeepDiveResponse {
            title: "Deep Dive".to_string(),
            goal: "Ship the feature".to_string(),
            accomplishments: Vec::new(),
            interesting_learnings: Vec::new(),
            teaching_narrative: vec![
                "### Architecture".to_string(),
                "The session mapped the feature to the right seam.".to_string(),
                "### Outcome".to_string(),
                "That kept the app loop simpler.".to_string(),
            ],
            reviewed_sources: Vec::new(),
            quiz_groups: vec![KnowledgeResponse {
                knowledge_type_group: "Narrative structure".to_string(),
                summary: "Readable structure supports learning.".to_string(),
                knowledge_type_language: "Markdown".to_string(),
                quiz: vec![QuizItem {
                    question: "Why split the narrative into sections?".to_string(),
                    options: vec![
                        QuizOption {
                            selection: "To improve readability".to_string(),
                            is_correct_answer: true,
                        },
                        QuizOption {
                            selection: "To hide content from the user".to_string(),
                            is_correct_answer: false,
                        },
                    ],
                    resources: Vec::new(),
                }],
            }],
        };

        let markdown = render_deep_dive_markdown(
            &metadata,
            &response,
            &[],
            &DeepDiveSectionsConfig::default(),
        );

        assert!(markdown.contains(
            "## Teaching Narrative\n\n### Architecture\n\nThe session mapped the feature to the right seam.\n\n### Outcome\n\nThat kept the app loop simpler."
        ));
        assert!(markdown.contains(
            "## Quiz\n\n### Narrative structure\n- Language: Markdown\n\nReadable structure supports learning.\n\n#### Question 1"
        ));
    }

    #[test]
    fn normalize_teaching_narrative_splits_dense_single_block() {
        let narrative = normalize_teaching_narrative(
            vec!["The first lesson is to inspect existing seams before adding logic. That keeps changes local and easier to reason about. The second lesson is to prefer intermediate representations that support multiple downstream views. That makes generated artifacts more reliable. The final lesson is to validate the output format, not just the raw content, because readability affects how useful the artifact is.".to_string()],
            &[
                "Inspect the right seam".to_string(),
                "Preserve reusable structure".to_string(),
                "Validate the final artifact".to_string(),
            ],
        );

        assert!(
            narrative
                .iter()
                .any(|block| block == "### Inspect the right seam")
        );
        assert!(
            narrative
                .iter()
                .any(|block| block == "### Preserve reusable structure")
        );
        assert!(narrative.iter().any(|block| {
            block.contains("The second lesson is to prefer intermediate representations")
        }));
    }
}
