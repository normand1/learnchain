# 2026-03-06: Runtime Debug Logging Flag

Status: Accepted

## Context

The TUI needed a low-friction way to troubleshoot runtime issues, especially LLM generation stalls and provider-specific failures.

Before this change, debug log writes depended on `write_output_artifacts`, which made ad hoc troubleshooting awkward:

- operators had to change config before reproducing an issue
- logs were not guaranteed to exist for a one-off debugging run
- it was harder to capture the exact sequence of app and LLM state transitions

## Decision

LearnChain now supports a CLI debug flag:

- `cargo run -- --debug`
- `learnchain --debug`

When enabled, the app forces timestamped debug logging to `output/learnchain-debug.log`.

### Logging behavior

- the log file is truncated at the start of each debug run
- runtime debug logging is enabled through an in-memory override in `src/log_util.rs`
- the override works even if `write_output_artifacts` is disabled in config
- log write failures are reported to stderr only, so logging does not take down the TUI

### Why a flag instead of a permanent config switch

The flag was chosen because it supports incident-style debugging without requiring config edits or persistent verbose logging. The default app behavior stays quiet, but a user can request a clean debug trace for a single run.

## Consequences

### Positive

- easier reproduction and inspection of generation failures
- no need to mutate persisted config for one-off troubleshooting
- each run starts with a clean log file

### Tradeoffs

- the debug log is still plain text rather than structured JSON
- the log is local-only and not tied to a request identifier
- sensitive values still must not be logged by future changes

## Files to Start With

- `src/main.rs`
- `src/log_util.rs`

## Follow-On Guidance

If future troubleshooting work expands logging, prefer:

- adding more high-signal lifecycle messages instead of dumping raw provider payloads by default
- keeping secrets and full API keys out of log output
- preserving the current property that `--debug` works without changing saved config
