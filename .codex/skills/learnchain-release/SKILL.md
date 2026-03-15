---
name: learnchain-release
description: Prepare and publish a new LearnChain release for this repository. Use when a maintainer asks to bump a version, cut a tag, publish to npm, monitor the release workflow, or verify the GitHub release and npm package for LearnChain.
---

# LearnChain Release

Use this skill only for maintainer release work in the LearnChain repository. Do
not use it for general coding tasks or for end-user skill installation.

This skill relies on:

- `scripts/prepare_release.sh`
- `docs/releasing.md`
- `gh`
- `npm`
- a clean understanding of which local changes belong in the release

## Workflow

1. Read `docs/releasing.md` before publishing if the workflow or expected checks are unclear.
2. Confirm the requested version number and use:
   `scripts/prepare_release.sh X.Y.Z`
   or:
   `scripts/prepare_release.sh X.Y.Z --date YYYY-MM-DD`
3. Replace the generated `TBD` placeholders in `CHANGELOG.md` and `docs/releases/vX.Y.Z.md` with the actual release summary.
4. Run the local preflight checks:
   - `cargo fmt`
   - `cargo test -- --nocapture`
   - `npm pack --dry-run`
5. Stage only the release-related files and the intended code or workflow changes for that release. Do not sweep unrelated dirty files into the release commit.
6. Commit with:
   `git commit -m "Prepare vX.Y.Z release"`
7. Create and push the annotated tag:
   - `git tag -a vX.Y.Z -m "vX.Y.Z"`
   - `git push origin master`
   - `git push origin vX.Y.Z`
8. Monitor the `Release` GitHub Actions workflow until it finishes.
9. Verify both the GitHub release and npm publication:
   - `gh release view vX.Y.Z`
   - `npm view learnchain version`
   - `npm view learnchain@X.Y.Z version`
10. If any step fails, report the concrete failing command or workflow step and stop rather than guessing.

## Response Format

When the release succeeds, report:

- release version
- release commit SHA
- tag name
- GitHub release URL
- npm package/version confirmation
- any manual follow-up still required

When the release does not succeed, report:

- the exact failed step
- the relevant error output
- whether the tag, GitHub release, or npm publish partially completed
