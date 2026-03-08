# Codex Session Analytics Data Model

This note documents two things:

1. the internal Rust data model LearnChain uses to derive Codex session analytics
2. the exported deep-dive markdown front matter that another application should consume

## Scope and intended consumer

This repository is the LearnChain Rust application.

- Internal runtime code lives in `src/*.rs`
- The built-in UI is the Ratatui terminal UI in `src/ui_renderer.rs`
- Deep-dive markdown artifacts are written by `src/output_manager.rs`

If another coding agent is working in a different repository, the supported integration contract is the generated deep-dive markdown file, not the in-memory Rust `Session` object.

That means:

- inside LearnChain, read `Session.analytics`
- outside LearnChain, read deep-dive front matter

Do not treat this document as an API contract. The exported contract is the deep-dive markdown format.

## Source of truth inside LearnChain

LearnChain still computes analytics from Codex JSONL logs and stores them on the in-memory session:

- `src/session_sources/mod.rs`
  - `Session`
    - `source_label: String`
    - `analytics: SessionAnalytics`

The analytics model lives in:

- `src/session_analytics.rs`

Important types:

- `SessionAnalytics`
  - `total_tool_calls`
  - `successful_tool_calls`
  - `failed_tool_calls`
  - `unknown_outcome_tool_calls`
  - `mcp_tool_calls`
  - `external_lookup_calls`
  - `adjust_course_count`
  - `external_resources: Vec<ExternalResourceRef>`
  - `adjustments: Vec<AdjustmentMarker>`
- `ExternalResourceRef`
  - `kind: ExternalResourceKind`
  - `tool_name: String`
  - `label: String`
  - `count: u32`
- `AdjustmentMarker`
  - `kind: AdjustmentKind`
  - `from_tool_name: String`
  - `to_tool_name: String`

Event enrichment used to derive analytics lives in:

- `src/session_sources/mod.rs`
  - `SessionEvent`
    - `event_kind: SessionEventKind`
    - `tool_name: Option<String>`
    - `result_metadata: Option<ToolResultMetadata>`

## How analytics are produced

### Codex parsing

Codex parsing in `src/session_sources/codex.rs` now preserves:

- `response_item.function_call`
  - `SessionEventKind::ToolCall`
  - `tool_name`
  - normalized `arguments`
- `response_item.function_call_output`
  - `SessionEventKind::ToolResult`
  - formatted `output`
  - `result_metadata.exit_code`
  - `result_metadata.duration_seconds`
- `event_msg.agent_reasoning`
  - `SessionEventKind::AgentReasoning`

### Analytics derivation

Session analytics are derived in:

- `src/session_analytics.rs`
  - `analyze(session: &Session) -> SessionAnalytics`

Sessions are built and analytics are attached in:

- `src/session_sources/mod.rs`
  - `group_events_by_session_with_metadata(...)`

That function:

1. builds the `Session`
2. sets `source_label`
3. calls `session_analytics::analyze(&session)`
4. stores the result on `session.analytics`

## Exported document contract

The deep-dive markdown file is now the external integration contract for analytics.

The writer lives in:

- `src/output_manager.rs`
  - `write_deep_dive_markdown(...)`

The deep-dive metadata type lives in:

- `src/llm/deep_dive_types.rs`
  - `DeepDiveArtifactMetadata`

`DeepDiveArtifactMetadata` now includes:

- `session_analytics: SessionAnalytics`

This means generated deep-dive files contain analytics in TOML front matter at the top of the markdown document.

## Deep-dive front matter fields

The deep-dive front matter now includes the existing metadata plus a nested `session_analytics` table.

Top-level fields still include:

- `artifact_type`
- `title`
- `generated_at`
- `session_source`
- `session_id`
- `session_timestamp`
- `session_date`
- `project_name`
- `project_cwd`
- `source_file`
- `referenced_url_count`
- `reviewed_url_count`

New nested field:

- `session_analytics`

Nested analytics fields:

- `total_tool_calls`
- `successful_tool_calls`
- `failed_tool_calls`
- `unknown_outcome_tool_calls`
- `mcp_tool_calls`
- `external_lookup_calls`
- `adjust_course_count`
- `external_resources`
- `adjustments`

Nested resource item fields:

- `kind`
- `tool_name`
- `label`
- `count`

Nested adjustment item fields:

- `kind`
- `from_tool_name`
- `to_tool_name`

Enum values serialize in snake_case:

- `ExternalResourceKind`
  - `web`
  - `mcp`
- `AdjustmentKind`
  - `post_failure_pivot`
  - `retry_with_different_arguments`

## Example shape

The exact TOML formatting is produced by the serializer, but the logical shape is:

```toml
artifact_type = "session_deep_dive"
title = "Session Deep Dive - 2026-03-08"
generated_at = "2026-03-08T12:00:00Z"
session_source = "Codex CLI"
session_id = "session-123"
session_timestamp = "2026-03-08T12:00:00Z"
session_date = "2026-03-08"
project_name = "learnchain"
project_cwd = "/workspace/learnchain"
source_file = "/tmp/session.jsonl"
referenced_url_count = 2
reviewed_url_count = 1

[session_analytics]
total_tool_calls = 4
successful_tool_calls = 2
failed_tool_calls = 1
unknown_outcome_tool_calls = 1
mcp_tool_calls = 1
external_lookup_calls = 2
adjust_course_count = 1

[[session_analytics.external_resources]]
kind = "web"
tool_name = "web.search_query"
label = "rust iterators"
count = 2

[[session_analytics.adjustments]]
kind = "post_failure_pivot"
from_tool_name = "shell"
to_tool_name = "web.search_query"
```

## Deep-dive markdown body

The rendered deep-dive body now also includes a human-readable analytics section.

Rendering lives in:

- `src/llm/deep_dive.rs`
  - `render_deep_dive_markdown(...)`

When analytics are present, the body includes:

- `## Session Analytics`
- summary bullets for counts
- `### External Resources`
- `### Adjustments Detected`

This body section is for humans. External consumers should parse the front matter, not the prose section.

## What another UI should read

If the UI is in another repository or another app:

- read the generated deep-dive markdown file
- parse the TOML front matter
- consume `metadata.session_analytics`

Do not:

- parse the prose analytics section in the markdown body
- parse raw Codex JSONL directly
- re-derive analytics from tool transcripts if the deep-dive file already exists
- assume the LearnChain in-memory `Session` type exists in the other repository

## Backward compatibility

Older deep-dive files without `session_analytics` still load correctly because the new field defaults to an empty analytics value during deserialization.

That compatibility behavior is implemented through:

- `#[serde(default)]` on metadata types
- fallback parsing in `src/output_manager.rs`

## Current built-in UI usage

Inside LearnChain itself, the existing Events view still reads:

- `Session.analytics`

Specifically:

- `src/ui_renderer.rs`
  - `UiRenderer::session_details_text(session: &Session) -> String`

That is an internal renderer convenience. It is not the cross-repo contract.

## Extension guidance

If analytics are later added to Claude or another coding tool:

1. normalize that source into `SessionEvent`
2. reuse `session_analytics::analyze(...)`
3. let the deep-dive writer serialize the resulting `Session.analytics`

The intended long-term layering is:

- parsers normalize raw source logs
- LearnChain derives analytics once
- deep-dive artifacts serialize those analytics
- other applications consume the deep-dive front matter
