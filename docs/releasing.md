# Releasing LearnChain

This document describes the current LearnChain release flow and the exact steps
to cut a release with GitHub Actions and npm trusted publishing.

## Current release flow

The release path now uses two GitHub Actions workflows:

1. `Cross-platform Tests`
   - Runs automatically for:
     - labeled PRs with the `ready to merge` label
     - pushes to `main`, `master`, and `develop`
   - Can also be run manually with a `ref` input for release preflight checks.
   - Runs the reusable cross-platform matrix on:
     - `ubuntu-latest`
     - `macos-latest`

2. `Release`
   - Triggered manually with:
     - `version` such as `0.4.9`
     - optional `ref`, which defaults to `master`
   - Validates the checked-out ref before tagging:
     - `Cargo.toml` and `package.json` versions must match
     - both must already equal the requested release version
     - `CHANGELOG.md` must already contain the release entry
     - `docs/releases/vX.Y.Z.md` must already exist
     - the tag must not already exist locally or on origin
   - Runs the reusable cross-platform test workflow against the selected ref.
   - Builds release binaries for:
     - `aarch64-apple-darwin`
     - `x86_64-apple-darwin`
     - `x86_64-unknown-linux-gnu`
   - Creates and pushes the release tag after validation and builds succeed.
   - Creates or updates the GitHub release with those binary assets.
   - Publishes the npm package from the same workflow run using npm trusted publishing over OIDC.

## Trusted publisher configuration

The package uses npm trusted publishing over OIDC.

If the trusted publisher ever needs to be recreated, configure it on npm with:

1. Open the package settings on npmjs.com.
2. Add a GitHub Actions trusted publisher with:
   - owner: `normand1`
   - repository: `learnchain`
   - workflow filename: `release.yml`
3. Leave the environment blank unless the workflow later adds a GitHub
   environment gate.

Notes:

- npm trusted publishing requires a GitHub-hosted runner.
- The publish job uses Node `24`.
- The publish job runs `npm ci` before packaging and publishing.
- The publish job verifies npm CLI `11.5.1` or newer before publishing.
- After trusted publishing is verified, remove the old `NPM_TOKEN` secret from
  the GitHub repository.

## Release checklist

Use this checklist when cutting a new version such as `0.4.9`.

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

### 3. Commit and merge the release-prepared change

Use the current convention:

```bash
git add \
  Cargo.toml Cargo.lock package.json package-lock.json \
  CHANGELOG.md docs/releases/vX.Y.Z.md \
  <any code or workflow changes included in the release>
git commit -m "Prepare vX.Y.Z release"
```

Merge that release-prepared commit to `master` before running the publish
workflow. The release workflow expects the target ref to already contain the
final version metadata, changelog entry, and release notes.

Note: some CI workflows intentionally skip normal push checks for commits with
this release message pattern.

### 4. Optionally run manual cross-platform preflight

Before finalizing the release, you can run the `Cross-platform Tests` workflow
manually against:

- `master`
- a release-prepared branch
- a specific commit SHA

This is the way to validate macOS and Linux behavior before any tag or GitHub
release exists.

### 5. Run the release workflow

From the GitHub Actions UI, run `Release` with:

- `version=X.Y.Z`
- optional `ref=master`

The workflow will:

1. validate the release metadata on the chosen ref
2. run cross-platform tests
3. build release binaries
4. create and push tag `vX.Y.Z`
5. create the GitHub release
6. publish `learnchain@X.Y.Z` to npm

Useful monitoring commands:

```bash
gh run list --limit 10
gh run watch <run-id> --exit-status
gh release view vX.Y.Z
```

### 6. Verify the release externally

Confirm both GitHub and npm:

```bash
gh release view vX.Y.Z
npm view learnchain version
npm view learnchain@X.Y.Z version
```

If the publish job was already verified with trusted publishing, remove the old
`NPM_TOKEN` secret from the repository settings so npm publication is fully
OIDC-based.
