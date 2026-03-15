# Changelog

All notable changes to this project will be documented in this file.

## [0.4.7] - 2026-03-14

### Added

- Added `scripts/prepare_release.sh` to update version files and scaffold release notes for new versions.
- Added a repo-local maintainer release skill at `.codex/skills/learnchain-release/` for repeatable tag-and-publish operations.

### Changed

- Collapsed the old two-workflow release handoff into a single `Release` GitHub Actions workflow that tests, builds, creates the GitHub release, and publishes to npm in one run.
- Switched release gating and cross-platform checks to Linux plus macOS, removing Windows from the release-critical path to reduce publish latency.
- Documented the maintainer release process in `docs/releasing.md` and linked it from the README.

### Fixed

- Updated LearnChain export URLs to point at `/dashboard/documents/<id>` instead of the raw `/api/documents/<id>` endpoint.


## [0.4.6] - 2026-03-14

### Changed

- Added a configurable deep-dive request timeout to the AI settings and carried that timeout through the LLM backend.
- Updated the configuration UI and saved-config feedback so the active deep-dive timeout is visible while tuning providers.

### Fixed

- Canonicalized LearnChain uploads to `https://www.learnchain.co` so document exports do not fail on the production `307` redirect from the apex host.

## [0.4.2] - 2026-03-12

### Changed

- Moved `Generate session deep dive` to the top of the main action list so the document workflow is the default menu path.
- Added the app version to the main menu header so the running build is visible inside the TUI.

### Fixed

- Made Claude deep-dive reference extraction tests platform-safe by generating valid JSON tool arguments on Windows and Unix.

## [0.4.0] - 2026-03-12

### Added

- Added headless CLI commands for config updates, deep-dive generation, bundled skill installation, and Codex action template printing.
- Added first-time LearnChain account setup, persisted LearnChain authentication, and LearnChain document export support alongside existing repository flows.
- Added bundled Claude Code skill support, Claude deep-dive generation, and broader Claude Code CLI integration across session parsing and LLM execution.

### Changed

- Expanded deep-dive generation and document export handling with richer research bundles, configurable sections, and stronger repository plumbing.
- Reworked the terminal UI and menu/config flows to surface onboarding, skill installation, and publish-ready guidance more clearly.
- Refreshed the README, bundled skills, and install assets to document the new CLI and LearnChain setup workflows.

### Fixed

- Hardened the npm publish workflow with release-tag version alignment and OTP-aware publishing steps.

## [0.3.0] - 2026-03-09

### Added

- Added saved deep-dive generation for current or selected Codex sessions, including optional export to configured document repositories.
- Added library browsing and export flows for generated deep dives and quiz artifacts.
- Added bundled Codex deep-dive skill installation and a printable custom-command template for Codex users.
- Added richer Codex and Claude session ingestion plus deterministic session analytics and dashboard views.

### Changed

- Replaced the previous AI manager with Rig-backed LLM workflows for quiz and deep-dive generation.
- Expanded configuration and terminal UI flows for provider setup, document repositories, session selection, and learning history review.
- Updated the public documentation to cover deep dives, analytics, document exports, and Codex integration.

### Fixed

- Fixed the npm publish workflow so releases derive the expected package version and avoid publish-time 404 failures.
- Cleaned up release packaging, deep-dive defaults, document UI behavior, and configuration panel hierarchy.
