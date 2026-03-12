---
name: learnchain-deep-dive
description: Generate a LearnChain deep-dive document for the active Claude Code session. Use when the user asks for a deep dive, learning writeup, session summary document, or retrospective based on the current Claude Code conversation, especially when LearnChain should produce the markdown artifact automatically from the latest Claude session.
argument-hint: "[optional focus text or session:<id>]"
disable-model-invocation: true
user-invocable: true
---

# LearnChain Deep Dive

Generate the deep dive by running LearnChain's headless Claude flow instead of recreating the document manually.

If the user asks to emphasize a specific topic, subsystem, teaching angle, or question, pass that request through with `--context "<requested focus>"`.

## Workflow

1. Prefer:
   `learnchain deep-dive generate claude --export`
2. If `learnchain` is not in PATH but the current workspace is the LearnChain repo:
   `cargo run -- deep-dive generate claude --export`
3. If the user supplied a specific Claude session id, append:
   `--session-id "<requested session id>"`
4. If the user supplied focus/context, append:
   `--context "<requested focus>"`
5. If the user did not supply focus/context or a specific session id, do not invent either one.
6. On failure, surface the LearnChain error directly.

## Response Format

After success, report:

- saved markdown path
- deep-dive title
- exported repository name
- exported repository URL when one is returned
- if the returned export URL matches `http://localhost:3000/api/documents/<id>` or `<site>/api/documents/<id>`, present the human-readable dashboard URL instead as `http://localhost:3000/dashboard/documents/<id>` or `<site>/dashboard/documents/<id>`
- short summary based on the goal and accomplishment bullets
