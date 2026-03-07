# 2026-03-06: External Document Export API Integration Requirements

Status: Proposed

## Context

LearnChain can already export library artifacts directly to Notion from the terminal UI.

Current behavior:

- exports start from the Library view
- the selected artifact is either a saved deep-dive markdown document or a saved quiz artifact
- the active document repository is selected in config
- the current supported repository is `notion`
- repository-specific configuration currently lives in `config/app_config.toml`

This document defines the requirements for a separate service or API that another Codex instance could build so LearnChain can delegate document publishing to that service instead of talking to each repository directly.

It is intentionally written as an integration contract, not an implementation for the current binary.

## Current LearnChain Contract

The external API must preserve these runtime assumptions from the app:

- export is initiated from the Library view against a single selected artifact
- artifacts are immutable saved outputs, not in-progress generation buffers
- the export flow must support both:
  - deep dives
  - quizzes
- the export target is repository-specific and user-configurable
- the export operation must be safe to run asynchronously from the UI
- success must produce a user-visible destination URL when the repository supports one
- failure must return a short actionable error message suitable for terminal display

### Current config surface

LearnChain currently stores:

- `document_repository`
- `document_repository_target`
- `notion_api_token`

Today that means:

- `document_repository = "notion"`
- `document_repository_target` is a Notion page/database/data source identifier or full URL
- `notion_api_token` is an internal integration token

An external API integration should avoid breaking this shape in its first version.

## Required Artifact Inputs

The API must accept enough data to publish either library artifact type without needing direct filesystem access to the LearnChain workspace.

Minimum required fields:

- `artifact_type`
  - valid initial values: `deep_dive`, `quiz`
- `title`
- `markdown`
- `source_path`
  - absolute LearnChain path for observability only
- `repository_kind`
  - valid initial value: `notion`
- `repository_target`
- `request_id`
  - unique caller-generated id for tracing and deduplication

Recommended optional fields:

- `artifact_generated_at`
- `session_date`
- `session_id`
- `project_name`
- `metadata`
  - arbitrary JSON object for future repository-specific formatting

### Artifact formatting expectations

The API should treat `markdown` as the canonical portable payload.

LearnChain can already derive markdown for both artifact types:

- deep dives use the saved markdown body directly
- quizzes are rendered into markdown before export

The API should not require raw quiz JSON or raw deep-dive TOML front matter in order to operate.

## Required API Shape

The first version should expose a single synchronous create endpoint:

- `POST /v1/document-exports`

Recommended request body:

```json
{
  "request_id": "4c9b7c2a-fcb8-4d4f-b0e1-2d17db40f4d9",
  "artifact_type": "deep_dive",
  "title": "Session Deep Dive - Rust Export Flow",
  "markdown": "# Session Deep Dive\n\n...",
  "source_path": "/Users/davidnorman/learnchain/output/deep-dives/deep-dive-....md",
  "repository_kind": "notion",
  "repository_target": "https://www.notion.so/31c0f905b7ec802bb0befe7ddebe4c9b",
  "credentials": {
    "notion_api_token": "ntn_..."
  },
  "metadata": {
    "session_date": "2026-03-06",
    "project_name": "learnchain"
  }
}
```

Recommended success response:

```json
{
  "status": "success",
  "repository_kind": "notion",
  "document_title": "Session Deep Dive - Rust Export Flow",
  "remote_id": "31d0f905-b7ec-80d2-9c11-d6ce8fd4a22b",
  "remote_url": "https://www.notion.so/...",
  "message": "Document exported successfully."
}
```

Recommended failure response:

```json
{
  "status": "error",
  "error_code": "repository_access_denied",
  "message": "The Notion integration does not have access to the configured destination."
}
```

## Authentication Requirements

The API service itself should have its own authentication boundary separate from the document repository token.

Recommended service auth:

- `Authorization: Bearer <learnchain_export_api_key>`

Repository credentials should be provided in one of two ways:

1. Initial pragmatic version:
   LearnChain passes repository credentials in the request body or encrypted headers.

2. Better long-term version:
   LearnChain sends a credential reference and the export API resolves the actual secret server-side.

For the first version, it is acceptable to mirror LearnChain’s current config shape and pass:

- `notion_api_token`

If the external API is multi-tenant, raw repository credentials should not be logged.

## Repository-Specific Requirements

### Notion

The service must support exporting to:

- a page target
- a database/data source target

The service must:

- accept either a raw Notion ID or a full Notion URL
- normalize Notion IDs internally
- distinguish between:
  - invalid identifier
  - target not found
  - integration lacks access
  - unsupported linked database/data source case
- create a new child page under the configured target
- render markdown into Notion-compatible blocks
- return the created page URL on success

The service should not require LearnChain to understand Notion block schemas.

## Error Contract

Error responses must be short and actionable because LearnChain surfaces them in a TUI status area.

Required properties:

- stable `error_code`
- one-line human-readable `message`

Recommended initial error codes:

- `invalid_request`
- `unsupported_repository`
- `invalid_repository_target`
- `repository_access_denied`
- `repository_target_not_found`
- `repository_rate_limited`
- `repository_upstream_error`
- `authentication_failed`
- `service_unavailable`

The API should not return giant stack traces or HTML error bodies to the client.

## Idempotency and Retry Requirements

The service should assume LearnChain may retry after network errors or UI restarts.

Required behavior:

- accept a caller-provided `request_id`
- treat repeated requests with the same `request_id` as idempotent when practical
- return the original success payload when the export already completed

At minimum, duplicate requests must not create obviously duplicated pages if the service can prevent it.

## Latency and Execution Model

LearnChain currently treats export as a background task and expects a single success or failure event.

The API should therefore:

- respond in less than 30 seconds for normal-sized documents
- avoid long polling requirements in the first version
- keep the first version synchronous if repository operations are short enough

If asynchronous server-side jobs are needed later, add:

- `POST /v1/document-exports`
- `GET /v1/document-exports/{request_id}`

But that is not required for the initial integration.

## Content Requirements

The API must accept markdown documents at least as large as current LearnChain artifacts.

Initial minimum expectations:

- support at least 250 KB request bodies
- preserve headings, paragraphs, bullets, numbered lists, and code fences
- tolerate plain markdown without front matter
- tolerate markdown generated from quizzes

The API may down-convert unsupported markdown features, but it must not fail just because a document includes:

- fenced code blocks
- long bullet lists
- plain URLs

## Observability Requirements

The service should log:

- `request_id`
- repository kind
- repository target hash or redacted form
- artifact type
- document title
- result status
- upstream latency

The service should not log:

- raw repository tokens
- full markdown bodies by default

## LearnChain Integration Points

A future LearnChain client for this API would replace or wrap the current direct repository code at:

- `src/view_managers/library_manager.rs`
  - export trigger from the selected library item
- `src/document_repository.rs`
  - current direct Notion implementation
- `src/config.rs`
  - repository selection and target config

The current caller behavior that must remain true:

- user selects a library item
- user presses export
- LearnChain starts a background task
- LearnChain displays either:
  - a success status with destination URL
  - a compact failure message

## Acceptance Criteria For The External API

Another Codex instance implementing the service should be considered done when all of the following are true:

- it accepts a deep-dive markdown artifact and exports it successfully to Notion
- it accepts a quiz-derived markdown artifact and exports it successfully to Notion
- it accepts both raw Notion IDs and full Notion URLs as targets
- it returns a stable remote URL on success
- it returns short structured errors on failure
- it does not require filesystem access to the LearnChain workspace
- it does not expose repository tokens in logs

## Recommended First Increment

The first implementation should stay narrow:

1. Support only `repository_kind = notion`
2. Support only `POST /v1/document-exports`
3. Accept `markdown`, `title`, `repository_target`, and `notion_api_token`
4. Return `remote_url` and a short status message
5. Keep retries and idempotency simple but explicit through `request_id`

That is enough to let LearnChain switch from direct Notion export to service-backed export without widening scope into generic repository orchestration too early.
