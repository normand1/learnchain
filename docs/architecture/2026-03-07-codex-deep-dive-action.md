# 2026-03-07: Codex deep-dive action

## Context

LearnChain already supported session deep dives inside the TUI, but Codex users had no direct way to invoke that flow from the active conversation. The existing CLI only handled configuration flags, and Codex session loading still relied on heuristic grouping that often collapsed session ids to `"unknown"`.

## Decision

LearnChain now exposes a headless Codex-specific deep-dive command:

- `learnchain deep-dive generate codex`
- `learnchain deep-dive generate codex --thread-id <id>`
- `learnchain deep-dive generate codex --export`
- `learnchain action print codex`
- `learnchain skill install codex`

The headless path resolves the active session from `CODEX_THREAD_ID` when present, falls back to the most recent Codex session when that env var is stale or missing, and writes the same markdown artifact to `output/deep-dives/` as the TUI flow.

Codex session parsing now reads `session_meta` records so LearnChain can preserve a stable session id, timestamp, and cwd even when function-call events do not carry explicit `session:` markers.

## Why this shape

We intentionally ship both a real skill installer and a copy/paste custom-command template instead of trying to register commands directly through undocumented Codex internals. The local Codex app does not expose a stable, documented file format for custom commands, so treating a hidden on-disk convention as public API would create brittle setup instructions.

The bundled skill installer gives users a supported self-serve setup path from `cargo run`, the compiled binary, or npm, while the template generator still covers environments where users prefer explicit command bodies. The export flag lets the skill hand the same generated artifact to the configured repository without teaching Codex a second export workflow. Both flows are portable and easy to support without coupling LearnChain to undocumented Codex internals.

## Consequences

- Codex users can trigger a deep dive from chat without opening the TUI first.
- Users can install the LearnChain Codex skill directly from LearnChain without copying files by hand.
- The skill can publish the generated deep dive to the configured repository in the same invocation.
- LearnChain can target the active Codex session deterministically.
- Session metadata is more reliable across the session picker, project grouping, and deep-dive export flows.
