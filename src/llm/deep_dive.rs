use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
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
        DeepDiveReviewedSource, DeepDiveTakeawayCard, LlmBackend, StructuredDeepDiveResponse,
    },
    markdown_rules::MarkdownRules,
    output_manager::{OutputManager, render_quiz_groups_markdown},
    session_sources::{Session, SessionEvent, SessionEventKind},
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
const TAKEAWAY_CARD_COUNT: usize = 5;
const MAX_TAKEAWAY_TITLE_CHARS: usize = 72;
const MAX_TAKEAWAY_TEXT_CHARS: usize = 180;
const MAX_SESSION_FILE_REFERENCES: usize = 8;
const MAX_FILE_SNIPPET_LINES: usize = 18;
const MAX_FILE_INVENTORY_SNIPPET_CHARS: usize = 700;

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

#[derive(Debug, Clone)]
struct SessionFileReference {
    relative_path: String,
    absolute_path: String,
    link_label: String,
    language: String,
    snippet: String,
    evidence: String,
    file_name_lower: String,
    file_stem_lower: String,
    relative_path_lower: String,
    search_terms: BTreeSet<String>,
    priority: u8,
}

#[derive(Debug, Clone, Default)]
struct PatchFileSection {
    path: String,
    hint_lines: Vec<String>,
    diff_lines: Vec<String>,
    deleted: bool,
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
    deep_dive_context: Option<&str>,
    progress_sender: impl Into<Option<&Sender<AiTaskMessage>>>,
) -> Result<DeepDiveGenerationResult> {
    let sender = progress_sender.into();
    let bundle = build_session_research_bundle(session_source, &session);
    let request_options = LlmRequestOptions::session_deep_dive();
    let file_references = build_session_file_references(&bundle.session);

    if let Some(sender) = sender {
        send_progress(sender, "Preparing session research bundle...", 20);
    }

    if let Some(sender) = sender {
        send_progress(
            sender,
            "Scanning session files for deep-dive references...",
            28,
        );
    }

    let plan_prompt = build_research_plan_prompt(&bundle, &file_references, deep_dive_context);
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
        &file_references,
        min_quiz_questions,
        deep_dive_context,
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
    let key_takeaways = std::mem::take(&mut response.key_takeaways);
    response.key_takeaways = normalize_key_takeaways(
        key_takeaways,
        &bundle.external_urls,
        response.goal.as_str(),
        &response.accomplishments,
        &response.interesting_learnings,
        &response.reviewed_sources,
        &plan,
    );

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

    let markdown = enrich_deep_dive_markdown_with_file_references(
        &render_deep_dive_markdown(&metadata, &response, &bundle.external_urls, &sections),
        &file_references,
    );
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
    markdown.push("## Key Takeaways".to_string());
    for (index, takeaway) in response.key_takeaways.iter().enumerate() {
        markdown.push(format!("### {}. {}", index + 1, takeaway.title));
        markdown.push(format!("- Category: {}", takeaway.category));
        markdown.push(format!("- Summary: {}", takeaway.summary));
        markdown.push(format!("- Why it matters: {}", takeaway.why_it_matters));
        if !takeaway.source_url.is_empty() {
            markdown.push(format!("- Source: {}", takeaway.source_url));
        }
        markdown.push(String::new());
    }
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

fn build_session_file_references(session: &Session) -> Vec<SessionFileReference> {
    let mut references = Vec::new();

    for event in &session.events {
        references.extend(extract_claude_tool_file_references(event, &session.cwd));

        if event.event_kind != SessionEventKind::ToolCall {
            continue;
        }

        let command = extract_command_invocation(event.arguments.as_deref());
        references.extend(extract_patch_references_from_event(
            event,
            command.as_ref(),
            &session.cwd,
        ));

        if let Some((command_text, workdir)) = command {
            references.extend(extract_command_file_references(
                &command_text,
                workdir.as_deref().unwrap_or(&session.cwd),
                &session.cwd,
            ));
        }
    }

    merge_session_file_references(references)
}

fn extract_patch_references_from_event(
    event: &SessionEvent,
    command: Option<&(String, Option<String>)>,
    session_cwd: &str,
) -> Vec<SessionFileReference> {
    let patch_text = if event.tool_name.as_deref() == Some("apply_patch") {
        event.arguments.clone()
    } else {
        command.and_then(|(command_text, _)| extract_patch_text_from_command(command_text))
    };
    let patch_cwd = command
        .and_then(|(_, workdir)| workdir.clone())
        .unwrap_or_else(|| session_cwd.to_string());

    let Some(patch_text) = patch_text else {
        return Vec::new();
    };

    parse_apply_patch_sections(&patch_text)
        .into_iter()
        .filter_map(|section| reference_from_patch_section(&section, &patch_cwd, session_cwd))
        .collect()
}

fn extract_command_file_references(
    command: &str,
    command_cwd: &str,
    session_cwd: &str,
) -> Vec<SessionFileReference> {
    if command.contains("*** Begin Patch") {
        return Vec::new();
    }

    let mut references = Vec::new();
    static SED_RANGE_REGEX: OnceLock<Regex> = OnceLock::new();
    let sed_range_regex = SED_RANGE_REGEX
        .get_or_init(|| Regex::new(r#"sed\s+-n\s+['"]?(\d+),(\d+)p['"]?\s+([^\s]+)"#).unwrap());
    for capture in sed_range_regex.captures_iter(command) {
        let Some(path) = capture
            .get(3)
            .map(|value| trim_wrapping_quotes(value.as_str()))
        else {
            continue;
        };
        let start_line = capture
            .get(1)
            .and_then(|value| value.as_str().parse::<usize>().ok())
            .unwrap_or(1);
        let end_line = capture
            .get(2)
            .and_then(|value| value.as_str().parse::<usize>().ok())
            .unwrap_or(start_line);
        if let Some(reference) = reference_from_line_span(
            path,
            command_cwd,
            session_cwd,
            start_line,
            end_line,
            "Referenced during the session via sed -n".to_string(),
            1,
        ) {
            references.push(reference);
        }
    }

    static CAT_REGEX: OnceLock<Regex> = OnceLock::new();
    let cat_regex = CAT_REGEX.get_or_init(|| Regex::new(r#"(?m)(?:^|\s)cat\s+([^\s]+)"#).unwrap());
    for capture in cat_regex.captures_iter(command) {
        let Some(path) = capture
            .get(1)
            .map(|value| trim_wrapping_quotes(value.as_str()))
        else {
            continue;
        };
        if let Some(reference) = reference_from_line_span(
            path,
            command_cwd,
            session_cwd,
            1,
            MAX_FILE_SNIPPET_LINES,
            "Referenced during the session via cat".to_string(),
            1,
        ) {
            references.push(reference);
        }
    }

    references
}

fn extract_claude_tool_file_references(
    event: &SessionEvent,
    session_cwd: &str,
) -> Vec<SessionFileReference> {
    let Some(tool_name) = event.tool_name.as_deref() else {
        return Vec::new();
    };
    if !matches!(tool_name, "Read" | "Edit" | "Write" | "MultiEdit") {
        return Vec::new();
    }

    let Some(arguments) = event.arguments.as_deref() else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return Vec::new();
    };
    let event_cwd = extract_event_cwd(event).unwrap_or(session_cwd);

    match tool_name {
        "Read" => reference_from_claude_read_args(&parsed, event_cwd, session_cwd)
            .into_iter()
            .collect(),
        "Edit" => reference_from_claude_edit_args(&parsed, event_cwd, session_cwd)
            .into_iter()
            .collect(),
        "Write" => reference_from_claude_write_args(&parsed, event_cwd, session_cwd)
            .into_iter()
            .collect(),
        "MultiEdit" => references_from_claude_multi_edit_args(&parsed, event_cwd, session_cwd),
        _ => Vec::new(),
    }
}

fn extract_event_cwd<'a>(event: &'a SessionEvent) -> Option<&'a str> {
    event.content_texts.iter().find_map(|line| {
        line.strip_prefix("cwd: ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn reference_from_claude_read_args(
    parsed: &serde_json::Value,
    command_cwd: &str,
    session_cwd: &str,
) -> Option<SessionFileReference> {
    let raw_path = json_string_field(parsed, "file_path")?;
    let evidence = "Referenced during the session via Claude Code Read".to_string();
    let start_line = json_usize_field(parsed, "offset").unwrap_or(1).max(1);
    let line_count = json_usize_field(parsed, "limit")
        .unwrap_or(MAX_FILE_SNIPPET_LINES)
        .max(1);
    let end_line = start_line.saturating_add(line_count.saturating_sub(1));

    reference_from_line_span(
        raw_path,
        command_cwd,
        session_cwd,
        start_line,
        end_line,
        evidence.clone(),
        1,
    )
    .or_else(|| {
        reference_from_line_span(
            raw_path,
            command_cwd,
            session_cwd,
            1,
            MAX_FILE_SNIPPET_LINES,
            evidence,
            1,
        )
    })
}

fn reference_from_claude_edit_args(
    parsed: &serde_json::Value,
    command_cwd: &str,
    session_cwd: &str,
) -> Option<SessionFileReference> {
    let raw_path = json_string_field(parsed, "file_path")?;
    let hints = collect_json_string_fields(parsed, &["new_string", "old_string"]);
    reference_from_claude_file_hints(
        raw_path,
        command_cwd,
        session_cwd,
        &hints,
        "Updated during the session via Claude Code Edit".to_string(),
        2,
    )
}

fn reference_from_claude_write_args(
    parsed: &serde_json::Value,
    command_cwd: &str,
    session_cwd: &str,
) -> Option<SessionFileReference> {
    let raw_path = json_string_field(parsed, "file_path")?;
    let hints = collect_json_string_fields(parsed, &["content"]);
    reference_from_claude_file_hints(
        raw_path,
        command_cwd,
        session_cwd,
        &hints,
        "Updated during the session via Claude Code Write".to_string(),
        2,
    )
}

fn references_from_claude_multi_edit_args(
    parsed: &serde_json::Value,
    command_cwd: &str,
    session_cwd: &str,
) -> Vec<SessionFileReference> {
    let root_path = json_string_field(parsed, "file_path").map(|value| value.to_string());
    let mut hints_by_path: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let root_hints = collect_json_string_fields(parsed, &["new_string", "old_string", "content"]);
    if let Some(root_path) = root_path.as_ref()
        && !root_hints.is_empty()
    {
        hints_by_path.insert(root_path.clone(), root_hints);
    }

    for field in ["edits", "changes"] {
        let Some(edits) = parsed.get(field).and_then(|value| value.as_array()) else {
            continue;
        };
        for edit in edits {
            let Some(raw_path) = json_string_field(edit, "file_path").or(root_path.as_deref())
            else {
                continue;
            };
            let hints = collect_json_string_fields(edit, &["new_string", "old_string", "content"]);
            hints_by_path
                .entry(raw_path.to_string())
                .or_default()
                .extend(hints);
        }
    }

    if hints_by_path.is_empty() {
        return root_path
            .as_deref()
            .and_then(|raw_path| {
                reference_from_claude_file_hints(
                    raw_path,
                    command_cwd,
                    session_cwd,
                    &[],
                    "Updated during the session via Claude Code MultiEdit".to_string(),
                    2,
                )
            })
            .into_iter()
            .collect();
    }

    hints_by_path
        .into_iter()
        .filter_map(|(raw_path, hints)| {
            reference_from_claude_file_hints(
                &raw_path,
                command_cwd,
                session_cwd,
                &hints,
                "Updated during the session via Claude Code MultiEdit".to_string(),
                2,
            )
        })
        .collect()
}

fn reference_from_claude_file_hints(
    raw_path: &str,
    command_cwd: &str,
    session_cwd: &str,
    hints: &[String],
    evidence: String,
    priority: u8,
) -> Option<SessionFileReference> {
    let path = resolve_session_path(command_cwd, raw_path);
    let snippet = build_snippet_from_hints(&path, hints)
        .or_else(|| inline_snippet_from_hints(hints))
        .or_else(|| read_line_span_snippet(&path, 1, MAX_FILE_SNIPPET_LINES))?;
    build_session_file_reference(&path, snippet, evidence, priority, session_cwd)
}

fn json_string_field<'a>(value: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    value
        .get(field)
        .and_then(|inner| inner.as_str())
        .map(str::trim)
        .filter(|inner| !inner.is_empty())
}

fn json_usize_field(value: &serde_json::Value, field: &str) -> Option<usize> {
    value
        .get(field)
        .and_then(|inner| inner.as_u64())
        .and_then(|inner| usize::try_from(inner).ok())
}

fn collect_json_string_fields(value: &serde_json::Value, fields: &[&str]) -> Vec<String> {
    fields
        .iter()
        .filter_map(|field| json_string_field(value, field))
        .map(ToString::to_string)
        .collect()
}

fn merge_session_file_references(
    references: Vec<SessionFileReference>,
) -> Vec<SessionFileReference> {
    let mut merged: BTreeMap<String, SessionFileReference> = BTreeMap::new();

    for reference in references {
        match merged.get_mut(&reference.absolute_path) {
            Some(existing) => {
                let mut search_terms = existing.search_terms.clone();
                search_terms.extend(reference.search_terms.iter().cloned());
                let should_replace = reference.priority > existing.priority
                    || (reference.priority == existing.priority
                        && reference.snippet.len() > existing.snippet.len());
                if should_replace {
                    let mut replacement = reference;
                    replacement.search_terms = search_terms;
                    *existing = replacement;
                } else {
                    existing.search_terms = search_terms;
                }
            }
            None => {
                merged.insert(reference.absolute_path.clone(), reference);
            }
        }
    }

    let mut references = merged.into_values().collect::<Vec<_>>();
    references.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    references.truncate(MAX_SESSION_FILE_REFERENCES);
    references
}

fn reference_from_patch_section(
    section: &PatchFileSection,
    command_cwd: &str,
    session_cwd: &str,
) -> Option<SessionFileReference> {
    let path = resolve_session_path(command_cwd, &section.path);
    let (snippet, diff_fallback) = if section.deleted {
        (build_diff_fallback_snippet(section), true)
    } else {
        let primary = build_patch_snippet(&path, section);
        let diff_fallback = primary.is_none();
        (
            primary.or_else(|| build_diff_fallback_snippet(section)),
            diff_fallback,
        )
    };
    let mut reference = build_session_file_reference(
        &path,
        snippet?,
        "Updated during the session via apply_patch".to_string(),
        2,
        session_cwd,
    )?;
    if diff_fallback {
        reference.language = code_fence_language_for_path(&path, true);
    }
    Some(reference)
}

fn reference_from_line_span(
    raw_path: &str,
    command_cwd: &str,
    session_cwd: &str,
    start_line: usize,
    end_line: usize,
    evidence: String,
    priority: u8,
) -> Option<SessionFileReference> {
    let path = resolve_session_path(command_cwd, raw_path);
    let snippet = read_line_span_snippet(&path, start_line, end_line)?;
    let mut reference =
        build_session_file_reference(&path, snippet, evidence, priority, session_cwd)?;
    reference.link_label = format!(
        "{} (around line {})",
        reference.relative_path,
        start_line.max(1)
    );
    Some(reference)
}

fn build_session_file_reference(
    path: &Path,
    snippet: String,
    evidence: String,
    priority: u8,
    session_cwd: &str,
) -> Option<SessionFileReference> {
    let snippet = snippet.trim().to_string();
    if snippet.is_empty() {
        return None;
    }

    let absolute_path = path.display().to_string();
    let relative_path = display_session_path(path, session_cwd);
    let file_name_lower = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let file_stem_lower = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let relative_path_lower = relative_path.to_ascii_lowercase();

    Some(SessionFileReference {
        relative_path: relative_path.clone(),
        absolute_path,
        link_label: relative_path.clone(),
        language: code_fence_language_for_path(path, false),
        snippet: limit_snippet_lines(&snippet, MAX_FILE_SNIPPET_LINES),
        evidence,
        file_name_lower,
        file_stem_lower,
        relative_path_lower,
        search_terms: tokenize_search_terms(&format!("{relative_path}\n{snippet}")),
        priority,
    })
}

fn extract_command_invocation(arguments: Option<&str>) -> Option<(String, Option<String>)> {
    let raw = arguments?;
    let parsed = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let workdir = parsed
        .get("workdir")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());

    if let Some(command) = parsed.get("cmd").and_then(|value| value.as_str()) {
        return Some((command.to_string(), workdir));
    }

    match parsed.get("command") {
        Some(serde_json::Value::String(command)) => Some((command.to_string(), workdir)),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|value| value.as_str())
            .last()
            .map(|value| (value.to_string(), workdir)),
        _ => None,
    }
}

fn extract_patch_text_from_command(command: &str) -> Option<String> {
    let start = command.find("*** Begin Patch")?;
    let end = command.rfind("*** End Patch")?;
    let end = end + "*** End Patch".len();
    Some(command[start..end].to_string())
}

fn parse_apply_patch_sections(patch_text: &str) -> Vec<PatchFileSection> {
    let mut sections = Vec::new();
    let mut current: Option<PatchFileSection> = None;

    for line in patch_text.lines() {
        let next_section = line
            .strip_prefix("*** Update File: ")
            .or_else(|| line.strip_prefix("*** Add File: "))
            .or_else(|| line.strip_prefix("*** Delete File: "));
        if let Some(path) = next_section {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some(PatchFileSection {
                path: path.trim().to_string(),
                deleted: line.starts_with("*** Delete File: "),
                ..PatchFileSection::default()
            });
            continue;
        }

        if line.starts_with("*** Move to: ") || line.starts_with("*** End Patch") {
            continue;
        }

        let Some(section) = current.as_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            continue;
        }
        if matches!(line.chars().next(), Some('+') | Some('-') | Some(' ')) {
            section.diff_lines.push(line.to_string());
        }
        if let Some(content) = line.strip_prefix('+').or_else(|| line.strip_prefix(' ')) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                section.hint_lines.push(trimmed.to_string());
            }
        }
    }

    if let Some(section) = current {
        sections.push(section);
    }

    sections
        .into_iter()
        .filter(|section| !section.path.is_empty())
        .collect()
}

fn resolve_session_path(base_dir: &str, raw_path: &str) -> PathBuf {
    let trimmed = trim_wrapping_quotes(raw_path);
    let path = Path::new(trimmed);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(base_dir).join(path)
    }
}

fn trim_wrapping_quotes(value: &str) -> &str {
    value.trim().trim_matches('"').trim_matches('\'')
}

fn display_session_path(path: &Path, session_cwd: &str) -> String {
    let cwd = Path::new(session_cwd);
    match path.strip_prefix(cwd) {
        Ok(relative) => relative.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

fn code_fence_language_for_path(path: &Path, diff_fallback: bool) -> String {
    if diff_fallback {
        return "diff".to_string();
    }

    match path.extension().and_then(|value| value.to_str()) {
        Some("rs") => "rust",
        Some("ts") => "ts",
        Some("tsx") => "tsx",
        Some("js") | Some("mjs") | Some("cjs") => "js",
        Some("jsx") => "jsx",
        Some("py") => "python",
        Some("toml") => "toml",
        Some("json") => "json",
        Some("md") => "md",
        Some("yml") | Some("yaml") => "yaml",
        Some("swift") => "swift",
        Some("java") => "java",
        Some("rb") => "ruby",
        Some("go") => "go",
        Some("sh") => "bash",
        _ => "text",
    }
    .to_string()
}

fn read_line_span_snippet(path: &Path, start_line: usize, end_line: usize) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let start_index = start_line
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));
    let mut end_index = end_line.max(start_line).min(lines.len());
    if end_index.saturating_sub(start_index) > MAX_FILE_SNIPPET_LINES {
        end_index = start_index + MAX_FILE_SNIPPET_LINES;
    }
    Some(lines[start_index..end_index].join("\n"))
}

fn build_patch_snippet(path: &Path, section: &PatchFileSection) -> Option<String> {
    build_snippet_from_hints(path, &section.hint_lines)
}

fn build_snippet_from_hints(path: &Path, hints: &[String]) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let candidates = collect_hint_candidates(hints);
    if candidates.is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    for candidate in &candidates {
        if let Some(index) = lines.iter().position(|line| line.trim() == candidate) {
            matches.push(index);
        }
        if matches.len() >= 6 {
            break;
        }
    }

    if matches.is_empty() {
        for candidate in &candidates {
            if let Some(index) = lines.iter().position(|line| line.contains(candidate)) {
                matches.push(index);
            }
            if matches.len() >= 6 {
                break;
            }
        }
    }

    let Some(first_match) = matches.iter().min().copied() else {
        return None;
    };
    let last_match = matches.iter().max().copied().unwrap_or(first_match);
    let start = first_match.saturating_sub(2);
    let end = (last_match + 4).min(lines.len());
    Some(lines[start..end].join("\n"))
}

fn collect_hint_candidates(hints: &[String]) -> Vec<String> {
    let mut candidates = Vec::new();
    for hint in hints {
        for line in hint.lines() {
            let trimmed = line.trim();
            if trimmed.len() >= 3 {
                candidates.push(trimmed.to_string());
            }
        }
    }
    candidates
}

fn inline_snippet_from_hints(hints: &[String]) -> Option<String> {
    hints.iter().find_map(|hint| {
        let trimmed = hint.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(limit_snippet_lines(trimmed, MAX_FILE_SNIPPET_LINES))
        }
    })
}

fn build_diff_fallback_snippet(section: &PatchFileSection) -> Option<String> {
    if section.diff_lines.is_empty() {
        return None;
    }
    Some(limit_snippet_lines(
        &section.diff_lines.join("\n"),
        MAX_FILE_SNIPPET_LINES,
    ))
}

fn limit_snippet_lines(snippet: &str, max_lines: usize) -> String {
    let lines = snippet.lines().collect::<Vec<_>>();
    if lines.len() <= max_lines {
        return snippet.to_string();
    }

    lines[..max_lines].join("\n")
}

fn format_session_file_inventory(
    references: &[SessionFileReference],
    include_snippets: bool,
) -> String {
    if references.is_empty() {
        return "No high-confidence session file references were identified.".to_string();
    }

    references
        .iter()
        .enumerate()
        .map(|(index, reference)| {
            if include_snippets {
                format!(
                    "{}. File: {}\n   Link target: {}\n   Evidence: {}\n   Relevant snippet:\n```{}\n{}\n```",
                    index + 1,
                    reference.relative_path,
                    reference.absolute_path,
                    reference.evidence,
                    reference.language,
                    truncate_for_prompt(&reference.snippet, MAX_FILE_INVENTORY_SNIPPET_CHARS)
                )
            } else {
                format!(
                    "{}. {} ({})",
                    index + 1,
                    reference.relative_path,
                    reference.evidence
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn enrich_deep_dive_markdown_with_file_references(
    markdown: &str,
    references: &[SessionFileReference],
) -> String {
    if references.is_empty() {
        return markdown.to_string();
    }

    let lines = markdown.lines().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut used_paths = BTreeSet::new();
    let mut index = 0;
    let mut section = String::new();

    while index < lines.len() {
        let line = lines[index];
        if line.starts_with("## ") {
            section = line.to_string();
            output.push(line.to_string());
            index += 1;
            continue;
        }

        match section.as_str() {
            "## What Was Accomplished" | "## Interesting or Unexpected Learnings" => {
                output.push(line.to_string());
                if line.starts_with("- ")
                    && let Some(reference) =
                        best_session_file_reference_for_text(line, references, &used_paths)
                {
                    append_file_reference_block(&mut output, reference);
                    used_paths.insert(reference.absolute_path.clone());
                }
                index += 1;
            }
            "## Key Takeaways" | "## Goal" | "## Teaching Narrative" => {
                if line.trim().is_empty() {
                    output.push(String::new());
                    index += 1;
                    continue;
                }

                let mut block = Vec::new();
                while index < lines.len()
                    && !lines[index].starts_with("## ")
                    && !lines[index].trim().is_empty()
                {
                    block.push(lines[index].to_string());
                    index += 1;
                }
                let block_text = block.join("\n");
                output.extend(block);
                if let Some(reference) =
                    best_session_file_reference_for_text(&block_text, references, &used_paths)
                {
                    append_file_reference_block(&mut output, reference);
                    used_paths.insert(reference.absolute_path.clone());
                }
            }
            _ => {
                output.push(line.to_string());
                index += 1;
            }
        }
    }

    output.join("\n")
}

fn best_session_file_reference_for_text<'a>(
    text: &str,
    references: &'a [SessionFileReference],
    used_paths: &BTreeSet<String>,
) -> Option<&'a SessionFileReference> {
    let terms = tokenize_search_terms(text);
    references
        .iter()
        .filter(|reference| !used_paths.contains(&reference.absolute_path))
        .map(|reference| {
            (
                reference,
                score_session_file_reference(text, &terms, reference),
            )
        })
        .filter(|(_, score)| *score >= 10)
        .max_by(
            |(left_reference, left_score), (right_reference, right_score)| {
                left_score
                    .cmp(right_score)
                    .then_with(|| right_reference.priority.cmp(&left_reference.priority))
            },
        )
        .map(|(reference, _)| reference)
}

fn score_session_file_reference(
    text: &str,
    terms: &BTreeSet<String>,
    reference: &SessionFileReference,
) -> usize {
    let lower = text.to_ascii_lowercase();
    let mut score = 0;

    if lower.contains(&reference.relative_path_lower) {
        score += 100;
    }
    if !reference.file_name_lower.is_empty() && lower.contains(&reference.file_name_lower) {
        score += 60;
    }
    if reference.file_stem_lower.len() >= 4 && lower.contains(&reference.file_stem_lower) {
        score += 20;
    }

    let overlap = reference
        .search_terms
        .iter()
        .filter(|term| terms.contains(*term))
        .count();
    score += overlap * 4;

    if reference.priority > 1
        && (lower.contains("updated")
            || lower.contains("change")
            || lower.contains("added")
            || lower.contains("patched"))
    {
        score += 4;
    }

    score
}

fn append_file_reference_block(output: &mut Vec<String>, reference: &SessionFileReference) {
    output.push(String::new());
    output.push(format!(
        "[{}]({})",
        reference.link_label, reference.absolute_path
    ));
    output.push(format!("```{}", reference.language));
    output.extend(reference.snippet.lines().map(ToString::to_string));
    output.push("```".to_string());
    output.push(String::new());
}

fn tokenize_search_terms(text: &str) -> BTreeSet<String> {
    static TERM_REGEX: OnceLock<Regex> = OnceLock::new();
    let term_regex = TERM_REGEX.get_or_init(|| Regex::new(r"[A-Za-z0-9]{3,}").unwrap());
    const STOP_WORDS: &[&str] = &[
        "the", "and", "with", "from", "into", "that", "this", "were", "when", "then", "they",
        "their", "there", "about", "because", "while", "where", "which", "added", "using", "used",
        "make", "made", "keep", "keeps", "just", "more", "than", "have", "has", "your", "code",
        "file", "session", "deep", "dive",
    ];

    term_regex
        .find_iter(text)
        .map(|value| value.as_str().to_ascii_lowercase())
        .filter(|term| !STOP_WORDS.contains(&term.as_str()))
        .collect()
}

fn build_research_plan_prompt(
    bundle: &SessionResearchBundle,
    file_references: &[SessionFileReference],
    deep_dive_context: Option<&str>,
) -> String {
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
        "Plan a session deep dive using this session transcript digest.\n\nSession source: {}\nSession date: {}\nSession id: {}\nProject: {}\nWorking directory: {}\n{}First user prompt:\n{}\n\nSelected session events:\n{}\n\nRelevant session files:\n{}\n\nReferenced external URLs:\n{}\n",
        bundle.session_source,
        bundle.session.date,
        bundle.session.id,
        bundle.project_name,
        bundle.project_cwd,
        format_deep_dive_context_block(deep_dive_context),
        first_user_prompt,
        event_details,
        format_session_file_inventory(file_references, false),
        urls
    )
}

fn build_final_deep_dive_prompt(
    bundle: &SessionResearchBundle,
    plan: &DeepDiveResearchPlan,
    fetched_sources: &[FetchedSource],
    fetch_failures: &[String],
    file_references: &[SessionFileReference],
    min_quiz_questions: usize,
    deep_dive_context: Option<&str>,
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
        "Write the final structured session deep dive.\n\nFormat requirements:\n- Keep the goal concise and specific.\n- Keep accomplishments and learnings scannable.\n- For teaching_narrative, do not return one large wall of text.\n- Structure teaching_narrative as markdown-friendly blocks with 3 to 5 short sections.\n- Prefer using a `###` subheading followed by a short paragraph for each section.\n- If needed, use an empty string item between sections to preserve spacing.\n- Populate key_takeaways with exactly {} concise cards.\n- Vary the key_takeaways categories when the session supports it, prioritizing concrete APIs, useful external docs, codebase insights, tooling workflows, and architecture lessons.\n- Each key_takeaways item must include title, category, summary, why_it_matters, and optional source_url.\n- When a source_url is present, it must come from the provided session URL inventory.\n- Populate quiz_groups using the same grouped quiz structure LearnChain uses for standalone quizzes.\n- Return at least {} quiz questions overall across quiz_groups.\n- Quiz questions should focus on the code, libraries, frameworks, tools, or APIs from the session rather than LearnChain's own implementation details.\n- Each quiz question should include answer options, exactly one correct answer, and any relevant supporting resources.\n- When you mention a concrete file or update, prefer using a file path from the provided session file inventory so the renderer can attach the right snippet later.\n- Do not invent file paths or claim file updates that are not supported by the session file inventory.\n{}\nInferred goal: {}\nCandidate accomplishments:\n{}\n\nCandidate interesting learnings:\n{}\n\nTeaching angles:\n{}\n\nSession file inventory:\n{}\n\nSession URL inventory:\n{}\n\nFetched source digests:\n{}\n\nFetch failures:\n{}\n",
        TAKEAWAY_CARD_COUNT,
        min_quiz_questions,
        format_deep_dive_context_block(deep_dive_context),
        plan.inferred_goal,
        accomplishments,
        learnings,
        teaching_angles,
        format_session_file_inventory(file_references, true),
        if bundle.external_urls.is_empty() {
            "No URLs were referenced.".to_string()
        } else {
            bundle.external_urls.join("\n")
        },
        fetched,
        failures
    )
}

fn format_deep_dive_context_block(deep_dive_context: Option<&str>) -> String {
    if let Some(context) = deep_dive_context
        .map(str::trim)
        .filter(|context| !context.is_empty())
    {
        format!(
            "Requested deep-dive focus:\n{}\nUse this focus to steer the goal, teaching angles, key takeaways, and quiz emphasis, but do not invent work that is not supported by the session.\n",
            truncate_for_prompt(context, MAX_FIRST_PROMPT_CHARS)
        )
    } else {
        String::new()
    }
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

fn normalize_key_takeaways(
    key_takeaways: Vec<DeepDiveTakeawayCard>,
    external_urls: &[String],
    goal: &str,
    accomplishments: &[String],
    interesting_learnings: &[String],
    reviewed_sources: &[DeepDiveReviewedSource],
    plan: &DeepDiveResearchPlan,
) -> Vec<DeepDiveTakeawayCard> {
    let allowed_urls: BTreeSet<String> = external_urls.iter().cloned().collect();
    let mut normalized = Vec::new();
    let mut seen_titles = BTreeSet::new();

    for takeaway in key_takeaways {
        push_takeaway_card(
            &mut normalized,
            &mut seen_titles,
            DeepDiveTakeawayCard {
                title: normalize_takeaway_title(&takeaway.title),
                category: normalize_takeaway_category(&takeaway.category),
                summary: normalize_takeaway_text(&takeaway.summary),
                why_it_matters: normalize_takeaway_why(&takeaway.why_it_matters, &takeaway.summary),
                source_url: normalize_takeaway_source_url(&takeaway.source_url, &allowed_urls),
            },
        );
        if normalized.len() == TAKEAWAY_CARD_COUNT {
            return normalized;
        }
    }

    for fallback in build_fallback_takeaway_cards(
        &allowed_urls,
        goal,
        accomplishments,
        interesting_learnings,
        reviewed_sources,
        plan,
    ) {
        push_takeaway_card(&mut normalized, &mut seen_titles, fallback);
        if normalized.len() == TAKEAWAY_CARD_COUNT {
            break;
        }
    }

    while normalized.len() < TAKEAWAY_CARD_COUNT {
        let index = normalized.len() + 1;
        push_takeaway_card(
            &mut normalized,
            &mut seen_titles,
            DeepDiveTakeawayCard {
                title: format!("Session takeaway {}", index),
                category: "Session Insight".to_string(),
                summary: "The transcript captured a useful implementation lesson worth revisiting."
                    .to_string(),
                why_it_matters:
                    "These cards keep the most actionable points visible before the full narrative."
                        .to_string(),
                source_url: String::new(),
            },
        );
    }

    normalized
}

fn build_fallback_takeaway_cards(
    allowed_urls: &BTreeSet<String>,
    goal: &str,
    accomplishments: &[String],
    interesting_learnings: &[String],
    reviewed_sources: &[DeepDiveReviewedSource],
    plan: &DeepDiveResearchPlan,
) -> Vec<DeepDiveTakeawayCard> {
    let mut cards = Vec::new();

    for source in reviewed_sources {
        cards.push(DeepDiveTakeawayCard {
            title: takeaway_title_from_url(&source.url),
            category: "External Docs".to_string(),
            summary: normalize_takeaway_text(&source.summary),
            why_it_matters: normalize_takeaway_why(&source.why_it_matters, &source.summary),
            source_url: normalize_takeaway_source_url(&source.url, allowed_urls),
        });
    }

    for (index, accomplishment) in accomplishments.iter().enumerate() {
        cards.push(DeepDiveTakeawayCard {
            title: format!("Implementation highlight {}", index + 1),
            category: "Implementation".to_string(),
            summary: normalize_takeaway_text(accomplishment),
            why_it_matters: "This marks a concrete change or outcome from the session.".to_string(),
            source_url: String::new(),
        });
    }

    for (index, learning) in interesting_learnings.iter().enumerate() {
        cards.push(DeepDiveTakeawayCard {
            title: format!("Codebase insight {}", index + 1),
            category: "Codebase Insight".to_string(),
            summary: normalize_takeaway_text(learning),
            why_it_matters: "This is a reusable lesson that should transfer to similar work."
                .to_string(),
            source_url: String::new(),
        });
    }

    if !goal.trim().is_empty() {
        cards.push(DeepDiveTakeawayCard {
            title: "Session goal".to_string(),
            category: "Goal".to_string(),
            summary: normalize_takeaway_text(goal),
            why_it_matters: "Keeping the original objective in view makes the rest of the deep dive easier to interpret.".to_string(),
            source_url: String::new(),
        });
    }

    for (index, angle) in plan.teaching_angles.iter().enumerate() {
        cards.push(DeepDiveTakeawayCard {
            title: format!("Architecture thread {}", index + 1),
            category: "Architecture".to_string(),
            summary: normalize_takeaway_text(angle),
            why_it_matters:
                "This points to the system seam or design constraint that shaped the implementation."
                    .to_string(),
            source_url: String::new(),
        });
    }

    for url in &plan.selected_urls {
        cards.push(DeepDiveTakeawayCard {
            title: takeaway_title_from_url(url),
            category: "Reference".to_string(),
            summary:
                "The session called out this external reference strongly enough to review it directly."
                    .to_string(),
            why_it_matters:
                "The linked material likely clarifies an API, framework behavior, or implementation constraint."
                    .to_string(),
            source_url: normalize_takeaway_source_url(url, allowed_urls),
        });
    }

    cards
}

fn push_takeaway_card(
    items: &mut Vec<DeepDiveTakeawayCard>,
    seen_titles: &mut BTreeSet<String>,
    candidate: DeepDiveTakeawayCard,
) {
    if items.len() >= TAKEAWAY_CARD_COUNT {
        return;
    }
    if candidate.title.is_empty()
        || candidate.summary.is_empty()
        || candidate.why_it_matters.is_empty()
    {
        return;
    }

    let key = candidate.title.to_ascii_lowercase();
    if !seen_titles.insert(key) {
        return;
    }

    items.push(candidate);
}

fn normalize_takeaway_title(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    truncate_for_prompt(trimmed, MAX_TAKEAWAY_TITLE_CHARS)
}

fn normalize_takeaway_category(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "Session Insight".to_string()
    } else {
        truncate_for_prompt(trimmed, 32)
    }
}

fn normalize_takeaway_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        truncate_for_prompt(trimmed, MAX_TAKEAWAY_TEXT_CHARS)
    }
}

fn normalize_takeaway_why(value: &str, summary: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        if summary.trim().is_empty() {
            String::new()
        } else {
            "This shaped how the session moved from exploration into implementation.".to_string()
        }
    } else {
        truncate_for_prompt(trimmed, MAX_TAKEAWAY_TEXT_CHARS)
    }
}

fn normalize_takeaway_source_url(value: &str, allowed_urls: &BTreeSet<String>) -> String {
    let Some(normalized) = normalize_url(value) else {
        return String::new();
    };
    if allowed_urls.contains(&normalized) {
        normalized
    } else {
        String::new()
    }
}

fn takeaway_title_from_url(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return "External reference".to_string();
    };
    let host = parsed.host_str().unwrap_or("reference");
    format!("Reference from {}", host)
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
    let regex = URL_REGEX.get_or_init(|| Regex::new(r#"https?://[^\s<>"'`{}\\$]+"#).unwrap());
    let mut urls = Vec::new();
    for capture in regex.find_iter(text) {
        if let Some(url) = normalize_url(capture.as_str()) {
            urls.push(url);
        }
    }
    urls
}

fn normalize_url(raw: &str) -> Option<String> {
    if contains_suspicious_url_artifact(raw) {
        return None;
    }

    let trimmed = raw.trim_end_matches(|ch: char| {
        matches!(
            ch,
            ')' | ']' | '}' | ',' | '.' | ';' | ':' | '\'' | '"' | '`' | '+'
        )
    });
    let mut url = Url::parse(trimmed).ok()?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return None,
    }
    if !has_plausible_url_host(&url) || has_suspicious_url_component(&url) {
        return None;
    }
    normalize_trailing_slashes(&mut url);
    if url.fragment().is_none() && url.path().is_empty() {
        url.set_path("/");
    }
    Some(url.to_string())
}

fn contains_suspicious_url_artifact(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    raw.contains("${")
        || raw.contains("#{")
        || raw.contains('\\')
        || raw.contains('`')
        || lower.contains("###")
        || lower.contains("%60")
        || lower.contains("%7b")
        || lower.contains("%7d")
}

fn has_plausible_url_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    if host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }

    is_plausible_domain(host)
}

fn is_plausible_domain(domain: &str) -> bool {
    if domain.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if domain.is_empty()
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
    {
        return false;
    }
    if !domain
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
    {
        return false;
    }

    domain.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().any(|ch| ch.is_ascii_alphanumeric())
    })
}

fn has_suspicious_url_component(url: &Url) -> bool {
    let serialized = url.as_str().to_ascii_lowercase();
    serialized.contains("%60")
        || serialized.contains("%7b")
        || serialized.contains("%7d")
        || serialized.contains("%5c")
}

fn normalize_trailing_slashes(url: &mut Url) {
    let path = url.path();
    if path.len() <= 1 {
        return;
    }

    let trimmed = path.trim_end_matches('/');
    if trimmed != path {
        url.set_path(&format!("{}/", trimmed));
    }
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
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

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
    fn normalize_url_rejects_template_and_markdown_artifacts() {
        for raw in [
            "https://${trimmed}`;/n+",
            "https://)/",
            "https://./n",
            "https://example.com))./n/n###",
            "https://github.com/normand1/HeyJamie/releases/download/v#{version}/HeyJamie_#{version}_#{arch}.dmg\\",
            "https://arxiv.org/list/cs.AI/recent`",
        ] {
            assert_eq!(
                normalize_url(raw),
                None,
                "expected {:?} to be rejected",
                raw
            );
        }
    }

    #[test]
    fn normalize_url_collapses_redundant_trailing_slashes() {
        assert_eq!(
            normalize_url("https://docs.rs/reqwest/latest/reqwest//"),
            Some("https://docs.rs/reqwest/latest/reqwest/".to_string())
        );
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
            key_takeaways: vec![DeepDiveTakeawayCard {
                title: "A focused API mattered".to_string(),
                category: "API".to_string(),
                summary: "The session depended on ratatui scrolling behavior.".to_string(),
                why_it_matters: "That behavior directly shaped the final implementation."
                    .to_string(),
                source_url: "https://ratatui.rs/".to_string(),
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
        let takeaways = markdown.find("## Key Takeaways").unwrap();
        let resources = markdown.find("### External Resources").unwrap();
        let adjustments = markdown.find("### Adjustments Detected").unwrap();
        assert!(takeaways < analytics);
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
        assert!(markdown.contains("### 1. A focused API mattered"));
        assert!(markdown.contains("- Category: API"));
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
            key_takeaways: vec![DeepDiveTakeawayCard {
                title: "Scrolling stays visible".to_string(),
                category: "Codebase Insight".to_string(),
                summary: "The document now surfaces takeaways above the narrative.".to_string(),
                why_it_matters: "Readers get the highest-signal points first.".to_string(),
                source_url: String::new(),
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
        assert!(markdown.contains("## Key Takeaways"));
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
    fn claude_code_cli_does_not_skip_llm_research_plan() {
        let backend = LlmBackend::from_config(
            ResolvedLlmConfig {
                provider: AiProvider::ClaudeCodeCli,
                model_name: "claude-code-print".to_string(),
                model_label: "CLI default".to_string(),
                api_key: String::new(),
            },
            "output",
        )
        .unwrap();

        assert!(!should_skip_llm_research_plan(&backend));
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
            &[],
            5,
            None,
        );

        assert!(prompt.contains("do not return one large wall of text"));
        assert!(prompt.contains("3 to 5 short sections"));
        assert!(prompt.contains("`###` subheading"));
        assert!(prompt.contains("key_takeaways"));
        assert!(prompt.contains("exactly 5 concise cards"));
        assert!(prompt.contains("quiz_groups"));
        assert!(prompt.contains("at least 5 quiz questions overall"));
        assert!(prompt.contains("Session file inventory"));
        assert!(prompt.contains("Do not invent file paths"));
    }

    #[test]
    fn final_prompt_includes_requested_deep_dive_focus_when_provided() {
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
            &[],
            5,
            Some("Focus on architecture tradeoffs and data flow."),
        );

        assert!(prompt.contains("Requested deep-dive focus"));
        assert!(prompt.contains("architecture tradeoffs and data flow"));
    }

    #[test]
    fn research_plan_prompt_includes_compact_session_file_inventory() {
        let root = unique_temp_dir("research-plan-files");
        let file_path = root.join("src/main.rs");
        fs::create_dir_all(file_path.parent().expect("parent dir")).expect("create dirs");
        fs::write(
            &file_path,
            ["fn main() {", "    println!(\"deep dive\");", "}"].join("\n"),
        )
        .expect("write file");

        let reference = build_session_file_reference(
            &file_path,
            fs::read_to_string(&file_path).expect("read file"),
            "Referenced during the session via Claude Code Read".to_string(),
            1,
            &root.display().to_string(),
        )
        .expect("reference");

        let bundle = build_session_research_bundle("Claude Code", &sample_session());
        let prompt = build_research_plan_prompt(&bundle, &[reference], None);

        assert!(prompt.contains("Relevant session files"));
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("Claude Code Read"));
        assert!(!prompt.contains("Relevant snippet"));
    }

    #[test]
    fn build_session_file_references_prefers_patch_backed_snippets() {
        let root = unique_temp_dir("file-reference-inventory");
        let file_path = root.join("src/llm/deep_dive.rs");
        fs::create_dir_all(file_path.parent().expect("parent dir")).expect("create dirs");
        fs::write(
            &file_path,
            [
                "fn helper() {}",
                "",
                "fn build_final_deep_dive_prompt() {",
                "    let session_file_inventory = true;",
                "    let renderer_context = \"attached\";",
                "}",
            ]
            .join("\n"),
        )
        .expect("write file");

        let session = Session {
            id: "session-123".to_string(),
            date: "2026-03-10".to_string(),
            timestamp: "2026-03-10T12:00:00Z".to_string(),
            cwd: root.display().to_string(),
            summary: "Improve the deep dive".to_string(),
            first_user_prompt: None,
            source_file: root.join("session.jsonl"),
            source_label: "Codex CLI".to_string(),
            analytics: SessionAnalytics::default(),
            events: vec![
                SessionEvent {
                    timestamp: "2026-03-10T12:00:01Z".to_string(),
                    payload_type: "custom_tool_call".to_string(),
                    event_kind: SessionEventKind::ToolCall,
                    call_id: Some("call_patch".to_string()),
                    tool_name: Some("apply_patch".to_string()),
                    arguments: Some(format!(
                        "*** Begin Patch\n*** Update File: {}\n@@\n-fn build_final_deep_dive_prompt() {{\n-    let session_file_inventory = false;\n+fn build_final_deep_dive_prompt() {{\n+    let session_file_inventory = true;\n+    let renderer_context = \"attached\";\n }}\n*** End Patch",
                        file_path.display()
                    )),
                    output: None,
                    result_metadata: None,
                    content_texts: Vec::new(),
                },
                SessionEvent {
                    timestamp: "2026-03-10T12:00:02Z".to_string(),
                    payload_type: "function_call".to_string(),
                    event_kind: SessionEventKind::ToolCall,
                    call_id: Some("call_read".to_string()),
                    tool_name: Some("exec_command".to_string()),
                    arguments: Some(
                        serde_json::json!({
                            "cmd": "sed -n '3,6p' src/llm/deep_dive.rs",
                            "workdir": root.display().to_string(),
                        })
                        .to_string(),
                    ),
                    output: None,
                    result_metadata: None,
                    content_texts: Vec::new(),
                },
            ],
        };

        let references = build_session_file_references(&session);
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].relative_path, "src/llm/deep_dive.rs");
        assert!(
            references[0]
                .snippet
                .contains("session_file_inventory = true")
        );
        assert!(references[0].evidence.contains("apply_patch"));
    }

    #[test]
    fn build_session_file_references_extracts_claude_read_reference() {
        let root = unique_temp_dir("claude-read-reference");
        let file_path = root.join("src/session_sources/claude.rs");
        fs::create_dir_all(file_path.parent().expect("parent dir")).expect("create dirs");
        fs::write(
            &file_path,
            [
                "line 1",
                "line 2",
                "fn parse_claude_session_file() {",
                "    let file_path = true;",
                "    let tool_input = true;",
                "}",
            ]
            .join("\n"),
        )
        .expect("write file");

        let session = Session {
            id: "session-claude-read".to_string(),
            date: "2026-03-10".to_string(),
            timestamp: "2026-03-10T12:00:00Z".to_string(),
            cwd: root.display().to_string(),
            summary: "Inspect Claude session files".to_string(),
            first_user_prompt: None,
            source_file: root.join("claude-session.jsonl"),
            source_label: "Claude Code".to_string(),
            analytics: SessionAnalytics::default(),
            events: vec![SessionEvent {
                timestamp: "2026-03-10T12:00:01Z".to_string(),
                payload_type: "tool_use: Read".to_string(),
                event_kind: SessionEventKind::Message,
                call_id: Some("tool_read".to_string()),
                tool_name: Some("Read".to_string()),
                arguments: Some(
                    serde_json::json!({
                        "file_path": file_path.display().to_string(),
                        "offset": 3,
                        "limit": 4,
                    })
                    .to_string(),
                ),
                output: None,
                result_metadata: None,
                content_texts: vec!["tool: Read".to_string(), format!("cwd: {}", root.display())],
            }],
        };

        let references = build_session_file_references(&session);
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].relative_path, "src/session_sources/claude.rs");
        assert!(references[0].link_label.contains("around line 3"));
        assert!(
            references[0]
                .snippet
                .contains("fn parse_claude_session_file()")
        );
        assert!(references[0].evidence.contains("Claude Code Read"));
    }

    #[test]
    fn build_session_file_references_extracts_claude_edit_reference() {
        let root = unique_temp_dir("claude-edit-reference");
        let file_path = root.join("src/App.tsx");
        fs::create_dir_all(file_path.parent().expect("parent dir")).expect("create dirs");
        fs::write(
            &file_path,
            [
                "const browserosInFlightRef = React.useRef(false);",
                "const browserosRunStartedAtRef = React.useRef(0);",
                "const browserosRunPromiseRef = React.useRef(null);",
                "const browserosRunCancelledRef = React.useRef(false);",
            ]
            .join("\n"),
        )
        .expect("write file");

        let session = Session {
            id: "session-claude-edit".to_string(),
            date: "2026-03-10".to_string(),
            timestamp: "2026-03-10T12:00:00Z".to_string(),
            cwd: root.display().to_string(),
            summary: "Update browser state".to_string(),
            first_user_prompt: None,
            source_file: root.join("claude-session.jsonl"),
            source_label: "Claude Code".to_string(),
            analytics: SessionAnalytics::default(),
            events: vec![SessionEvent {
                timestamp: "2026-03-10T12:00:02Z".to_string(),
                payload_type: "tool_use: Edit".to_string(),
                event_kind: SessionEventKind::Message,
                call_id: Some("tool_edit".to_string()),
                tool_name: Some("Edit".to_string()),
                arguments: Some(
                    serde_json::json!({
                        "replace_all": false,
                        "file_path": file_path.display().to_string(),
                        "old_string": "const browserosInFlightRef = React.useRef(false);\nconst browserosRunPromiseRef = React.useRef(null);",
                        "new_string": "const browserosInFlightRef = React.useRef(false);\nconst browserosRunStartedAtRef = React.useRef(0);\nconst browserosRunPromiseRef = React.useRef(null);",
                    })
                    .to_string(),
                ),
                output: None,
                result_metadata: None,
                content_texts: vec!["tool: Edit".to_string(), format!("cwd: {}", root.display())],
            }],
        };

        let references = build_session_file_references(&session);
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].relative_path, "src/App.tsx");
        assert!(references[0].snippet.contains("browserosRunStartedAtRef"));
        assert!(references[0].evidence.contains("Claude Code Edit"));
    }

    #[test]
    fn markdown_enrichment_inserts_link_and_snippet_for_semantic_match() {
        let root = unique_temp_dir("markdown-enrichment");
        let file_path = root.join("src/llm/deep_dive.rs");
        fs::create_dir_all(file_path.parent().expect("parent dir")).expect("create dirs");
        fs::write(
            &file_path,
            [
                "fn build_final_deep_dive_prompt() {",
                "    let session_file_inventory = true;",
                "    let renderer_context = \"attached\";",
                "}",
            ]
            .join("\n"),
        )
        .expect("write file");

        let reference = build_session_file_reference(
            &file_path,
            fs::read_to_string(&file_path).expect("read file"),
            "Updated during the session via apply_patch".to_string(),
            2,
            &root.display().to_string(),
        )
        .expect("reference");

        let markdown = [
            "# Deep Dive",
            "",
            "## What Was Accomplished",
            "- The final prompt now carries session file inventory context for the renderer.",
            "",
        ]
        .join("\n");

        let enriched = enrich_deep_dive_markdown_with_file_references(&markdown, &[reference]);
        assert!(enriched.contains("[src/llm/deep_dive.rs]("));
        assert!(enriched.contains("```rust"));
        assert!(enriched.contains("let session_file_inventory = true;"));
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
            key_takeaways: vec![DeepDiveTakeawayCard {
                title: "Readable structure wins".to_string(),
                category: "Workflow".to_string(),
                summary: "Short sections make the deep dive easier to use.".to_string(),
                why_it_matters:
                    "Formatting quality affects whether the document teaches effectively."
                        .to_string(),
                source_url: String::new(),
            }],
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
    fn normalize_key_takeaways_fills_missing_cards_from_session_context() {
        let takeaways = normalize_key_takeaways(
            vec![DeepDiveTakeawayCard {
                title: "Existing takeaway".to_string(),
                category: "API".to_string(),
                summary: "A concrete API choice shaped the work.".to_string(),
                why_it_matters: "That choice determined how the feature was implemented."
                    .to_string(),
                source_url: "https://ratatui.rs/".to_string(),
            }],
            &["https://ratatui.rs/".to_string()],
            "Ship the feature",
            &["Added the top-level document section".to_string()],
            &["The codebase already had a stable parsing seam".to_string()],
            &[DeepDiveReviewedSource {
                url: "https://ratatui.rs/".to_string(),
                summary: "Ratatui docs clarified the expected behavior.".to_string(),
                why_it_matters: "The scrolling and layout details came directly from the docs."
                    .to_string(),
            }],
            &DeepDiveResearchPlan {
                inferred_goal: "Ship the feature".to_string(),
                candidate_accomplishments: Vec::new(),
                candidate_interesting_learnings: Vec::new(),
                teaching_angles: vec!["Explain the rendering seam".to_string()],
                selected_urls: vec!["https://ratatui.rs/".to_string()],
            },
        );

        assert_eq!(takeaways.len(), TAKEAWAY_CARD_COUNT);
        assert!(
            takeaways
                .iter()
                .any(|item| item.category == "External Docs")
        );
        assert!(
            takeaways
                .iter()
                .any(|item| item.category == "Implementation")
        );
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

    fn unique_temp_dir(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("learnchain-{label}-{suffix}"))
    }
}
