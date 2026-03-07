# 2026-03-06: Rig-Backed Learning Generation

Status: Accepted

## Context

LearnChain previously invoked LLM providers through a custom `AiManager` built on raw `reqwest` calls. That design had a few problems:

- provider-specific request/response parsing was mixed into application code
- extending provider support required touching orchestration code and transport code at the same time
- structured quiz generation relied on prompt-level JSON instructions and manual parsing
- token usage and provider behavior were not exposed through a consistent internal interface

The project also needed to preserve existing user-facing behavior:

- the current TUI config flow
- the existing provider list: OpenAI, Anthropic, and OpenRouter
- the persisted quiz payload shape used by the knowledge store
- the current background-thread generation model

## Decision

LearnChain now uses a dedicated `src/llm/` module as the application boundary for quiz generation.

### Boundary and ownership

- `src/llm/mod.rs` owns app-facing orchestration:
  - task startup
  - progress messages
  - success/error handling
  - persistence of generated quiz output
- `src/llm/backend.rs` owns provider clients and request execution
- `src/llm/types.rs` owns the structured learning payload and token-usage types

The application talks to a `LearningGenerator` instead of a transport-specific manager.

### Provider and config resolution

Provider, model, and API key selection are resolved before request execution through `ResolvedLlmConfig` in `src/config.rs`.

This keeps provider selection logic in one place and allows the UI, CLI, and runtime app startup to share the same resolution rules.

### Structured output contract

The structured learning response types were moved into `src/llm/types.rs` and now derive:

- `Serialize`
- `Deserialize`
- `Default`
- `Clone`
- `Debug`
- `JsonSchema`

Each structured type also uses `#[serde(deny_unknown_fields)]`.

This preserves the stored quiz shape while making the response contract explicit and stricter. Unknown fields now fail deserialization instead of silently being ignored.

### Rig as the provider integration layer

LearnChain now uses `rig-core` as the provider abstraction instead of direct HTTP calls.

The internal dispatch is:

- OpenAI: Rig client with typed structured output
- Anthropic: Rig client with typed structured output
- OpenRouter: Rig extractor path with retries

Although the app-facing boundary is standardized, the provider call paths are intentionally not identical.

OpenAI and Anthropic use Rig's typed prompt path because the extractor-based path caused the quiz flow to stall in practice. OpenRouter stays on the extractor path because that provider path is the current working route for structured extraction in this codebase.

The standardization target is the internal service boundary, not forced parity in the underlying provider API call shape.

### Prompting and validation

The system prompt is now extraction-oriented rather than "return raw JSON" oriented.

Key rules preserved in the prompt:

- use the session summary as the source of truth
- produce at least the configured minimum number of quiz questions
- ask language- and tool-specific questions, not LearnChain implementation trivia
- fill every required field in the structured response

The user prompt is intentionally simple: it supplies the session summary without embedding a schema blob.

### Runtime model

The existing concurrency model was preserved:

- the TUI starts a background thread for each generation task
- that thread creates a Tokio runtime
- the runtime performs the async provider request

This avoided a larger refactor while still allowing the LLM layer to change cleanly.

### Observability and control

Generation results now return `LearningGenerationResult`, which includes:

- the structured response
- optional token usage

The app surfaces token totals in the success status when available and logs provider/model usage details through `log_util`.

Requests now use a 180-second timeout so the UI fails explicitly instead of hanging indefinitely.

## Consequences

### Positive

- adding another provider no longer requires touching the learning-view orchestration flow
- the transport layer is isolated from app state management
- structured response validation is stricter and easier to test
- token usage is available through one internal result type
- provider-specific failures are easier to localize to `src/llm/backend.rs`

### Tradeoffs

- OpenAI/Anthropic and OpenRouter do not share one exact Rig invocation pattern
- the per-request Tokio runtime remains a pragmatic compromise rather than a fully shared async runtime
- live provider verification is still required when adding new models because unit tests only cover the local contract

## Files to Start With

- `src/llm/mod.rs`
- `src/llm/backend.rs`
- `src/llm/types.rs`
- `src/config.rs`
- `src/main.rs`

## Follow-On Guidance

Future LLM work should preserve these boundaries:

- keep provider/model/key resolution in `src/config.rs`
- keep app orchestration in `src/llm/mod.rs`
- keep provider-specific client logic in `src/llm/backend.rs`
- keep persisted quiz response types stable unless a deliberate storage migration is planned

If a future provider can use a cleaner Rig path, prefer changing only `src/llm/backend.rs` and leaving the rest of the app untouched.
