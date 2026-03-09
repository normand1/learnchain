---
name: learnchain-deep-dive
description: Generate a LearnChain deep-dive document for the active Codex session. Use when the user asks for a deep dive, learning writeup, session summary document, or retrospective based on the current Codex conversation, especially when LearnChain should produce the markdown artifact automatically from `CODEX_THREAD_ID`.
---

# LearnChain Deep Dive

Generate the deep dive by running LearnChain's headless Codex flow instead of recreating the document manually.

## Workflow

1. Prefer:
   `learnchain --generate-codex-deep-dive --export-to-document-repository --codex-thread-id "$CODEX_THREAD_ID"`
2. If `learnchain` is not in PATH but the current workspace is the LearnChain repo:
   `cargo run -- --generate-codex-deep-dive --export-to-document-repository --codex-thread-id "$CODEX_THREAD_ID"`
3. On failure, surface the LearnChain error directly.

## Response Format

After success, report:

- saved markdown path
- deep-dive title
- exported repository name
- exported repository URL when one is returned
- if the returned export URL matches `http://localhost:3000/api/documents/<id>` or `<site>/api/documents/<id>`, present the human-readable dashboard URL instead as `http://localhost:3000/dashboard/documents/<id>` or `<site>/dashboard/documents/<id>`
- short summary based on the goal and accomplishment bullets

If the command reports a fallback note about `CODEX_THREAD_ID`, include it.
