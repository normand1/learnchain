# 2026-03-06: Session Deep Dives and Shared Session Picker

Status: Accepted

## Context

LearnChain already supported quiz generation from recent or selected sessions, but it had no way to produce a durable, narrative artifact that explained what happened in a session, what was learned, and which external references mattered.

The new requirement added three related needs:

- a second generation action that starts from the same project/session picker as quiz generation
- a saved markdown artifact for each deep dive, with future in-app browsing
- a Rig-backed workflow that can stay grounded in the session transcript while revisiting a small subset of cited URLs

## Decision

LearnChain now treats session picking as a shared flow and deep dives as a first-class generation mode.

### Shared picker

- `AppView::SessionPicker` owns the project -> session selection UI
- `SessionSelectionTarget` decides whether a selected session launches quiz generation or deep-dive generation
- `SessionPickerManager` owns the picker interactions so `LearningManager` only handles quiz behavior

### Shared LLM backend

- `src/llm/backend.rs` now exposes a shared `LlmBackend`
- quiz generation and deep-dive generation both reuse the same provider construction, timeout handling, and typed Rig extraction path
- task orchestration in `src/llm/mod.rs` routes by `AiTaskKind`

### Deep-dive workflow

Deep dives use a two-step Rig workflow:

1. build a deterministic session research bundle from metadata, a balanced slice of session events, and the deduped URL inventory
2. ask Rig for a research plan, fetch only the planner-selected URLs, then ask Rig for the final structured deep dive

Markdown assembly is performed in Rust so the saved file shape is stable and testable.

### Persistence

- deep dives are always saved under `output/deep-dives/`
- each file includes TOML front matter for history scanning
- deep-dive history is file-backed, not SQLite-backed
- the existing config toggle still controls quiz JSON persistence only

## Consequences

### Positive

- quiz generation and deep dives can share the same session entry flow
- saved deep dives are durable and can be reopened without re-running the model
- the URL review step stays bounded and predictable

### Tradeoffs

- deep-dive history depends on filesystem scanning rather than indexed storage
- revisited source material is limited to a planner-selected subset of cited URLs, not broad live web search
- the deep-dive view currently shows file paths rather than opening OS-native hyperlinks
