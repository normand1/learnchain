# Releasing LearnChain

This document describes the current release flow for LearnChain, the exact steps
to cut a release, and the changes worth making to simplify the pipeline.

## Current release flow

The release path now uses one GitHub Actions workflow:

1. `Release`
   - Triggered by pushing a `v*` tag or by manual `workflow_dispatch` with a tag input.
   - Resolves the release tag once at the start of the run.
   - Runs the reusable cross-platform test workflow on:
     - `ubuntu-latest`
     - `macos-latest`
   - Builds release binaries for:
     - `aarch64-apple-darwin`
     - `x86_64-apple-darwin`
     - `x86_64-unknown-linux-gnu`
   - Creates or updates the GitHub release with those binary assets.
   - Publishes the npm package from the same workflow run using the same binary artifacts.

## Release checklist

Use this checklist when cutting a new version such as `0.4.7`.

### 1. Run the release prep script

Run:

```bash
scripts/prepare_release.sh X.Y.Z
```

Or pin the changelog date explicitly:

```bash
scripts/prepare_release.sh X.Y.Z --date YYYY-MM-DD
```

The script updates:

- `Cargo.toml`
- `Cargo.lock`
- `package.json`
- `package-lock.json`
- `CHANGELOG.md`
- `docs/releases/vX.Y.Z.md`

Then replace the `TBD` placeholders in the changelog entry and release notes
with the actual release summary.

### 2. Validate locally

Run:

```bash
cargo fmt
cargo test -- --nocapture
npm pack --dry-run
```

These three checks catch most release blockers:

- formatting drift
- failing Rust tests
- broken npm package contents

### 3. Commit the release

Use the current convention:

```bash
git add \
  Cargo.toml Cargo.lock package.json package-lock.json \
  CHANGELOG.md docs/releases/vX.Y.Z.md \
  <any code or workflow changes included in the release>
git commit -m "Prepare vX.Y.Z release"
```

Note: CI workflows intentionally skip some normal push checks for commits with
this release message pattern.

### 4. Create and push the tag

```bash
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin master
git push origin vX.Y.Z
```

If the default branch is `main` in the future, replace `master` accordingly.

### 5. Monitor GitHub Actions

Check:

- `Release`

Useful commands:

```bash
gh run list --limit 10
gh run watch <run-id> --exit-status
gh release view vX.Y.Z
```

Expected order:

1. Cross-platform tests finish
2. Binary builds finish
3. GitHub release appears with binary assets
4. npm package verification runs
5. npm publish completes successfully

### 6. Verify the release externally

Confirm both GitHub and npm:

```bash
gh release view vX.Y.Z
npm view learnchain version
npm view learnchain@X.Y.Z version
```

## Current pain points

The current flow works, but it is more complicated than it needs to be.

### Release notes still require human editing

The prep script creates the right files and placeholders, but the release
summary still needs to be written by hand.

### Release monitoring is manual

The operator still has to watch the workflow and verify npm manually.

## Recommended simplifications

These changes would reduce the number of moving parts materially.

### 1. Keep release validation small and explicit

Recommended release preflight:

- `cargo fmt`
- `cargo test -- --nocapture`
- `npm pack --dry-run`

Avoid layering extra release-only checks on top unless they catch a real class
of failures not already covered by CI.

### 2. Consider a manual release dispatch workflow later

If you want fewer local git steps, a future manual release workflow could accept
a version input and do some combination of:

- validate that the tree is clean
- create the tag
- push the tag
- run the release pipeline

I would not start there. The single release workflow and local prep script are
already in place.

## Should this become a skill?

Yes.

A skill is a good fit because this process is:

- repetitive
- repo-specific
- operationally sensitive
- easy to partially complete

The skill should not just be prose. It should pair a short `SKILL.md` with a
bundled script for the mechanical parts.

Recommended skill scope:

- verify the repo is in a releasable state
- run the local preflight checks
- bump version files
- scaffold release notes
- create the release commit and tag
- push branch and tag
- monitor GitHub Actions
- verify GitHub release assets and npm publication

Recommended skill dependencies:

- `git`
- `gh`
- `npm`
- the local release-prep script

Recommended timing:

- create the skill now that the release flow is a single pipeline
- keep the script as the mechanical backbone so the skill stays short and reliable

The repo-local maintainer skill lives at:

- `.codex/skills/learnchain-release/SKILL.md`

## Recommended next steps

In order:

1. Keep the Windows removal in place so release latency stays reasonable.
2. Create a repo-local release skill that wraps the process.
