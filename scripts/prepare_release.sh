#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/prepare_release.sh <version> [--date YYYY-MM-DD]

Prepares a LearnChain release by:
- updating Cargo.toml
- updating Cargo.lock
- updating package.json
- updating package-lock.json
- prepending a CHANGELOG.md entry template
- creating docs/releases/v<version>.md

Example:
  scripts/prepare_release.sh 0.4.7
  scripts/prepare_release.sh 0.4.7 --date 2026-03-15
EOF
}

if [[ ${1:-} == "-h" || ${1:-} == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -lt 1 || $# -gt 3 ]]; then
  usage >&2
  exit 1
fi

version="$1"
shift

release_date="$(date +%F)"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --date)
      if [[ $# -lt 2 ]]; then
        echo "--date requires a value" >&2
        exit 1
      fi
      release_date="$2"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Version must use semver like 0.4.7" >&2
  exit 1
fi

if ! [[ "$release_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
  echo "Date must use YYYY-MM-DD" >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

for required in Cargo.toml Cargo.lock package.json package-lock.json CHANGELOG.md; do
  if [[ ! -f "$required" ]]; then
    echo "Missing required file: $required" >&2
    exit 1
  fi
done

current_cargo_version="$(
  perl -0ne '
    if (/\[package\]\n(?:(?!\n\[).*\n)*?version = "([^"]+)"/s) {
      print "$1\n";
      exit 0;
    }
    exit 1;
  ' Cargo.toml
)"
current_package_version="$(node -p "require('./package.json').version")"

if [[ "$current_cargo_version" != "$current_package_version" ]]; then
  echo "Cargo.toml version ($current_cargo_version) does not match package.json version ($current_package_version)" >&2
  exit 1
fi

if [[ "$current_cargo_version" == "$version" ]]; then
  echo "Version $version is already current" >&2
  exit 1
fi

release_notes_path="docs/releases/v${version}.md"
if [[ -e "$release_notes_path" ]]; then
  echo "Release notes already exist: $release_notes_path" >&2
  exit 1
fi

if grep -Fq "## [${version}] -" CHANGELOG.md; then
  echo "CHANGELOG.md already contains version $version" >&2
  exit 1
fi

OLD_VERSION="$current_cargo_version" NEW_VERSION="$version" perl -0pi -e 'my $old = $ENV{OLD_VERSION}; my $new = $ENV{NEW_VERSION}; my $count = s/(\[package\]\n(?:(?!\n\[).*\n)*?version = ")\Q$old\E(")/$1 . $new . $2/se; die "Failed to update Cargo.toml version\n" if $count != 1;' Cargo.toml

OLD_VERSION="$current_cargo_version" NEW_VERSION="$version" perl -0pi -e 'my $old = $ENV{OLD_VERSION}; my $new = $ENV{NEW_VERSION}; my $count = s/(name = "learnchain"\nversion = ")\Q$old\E(")/$1 . $new . $2/se; die "Failed to update Cargo.lock version\n" if $count != 1;' Cargo.lock

npm version "$version" --no-git-tag-version >/dev/null

tmp_changelog="$(mktemp)"
cat >"$tmp_changelog" <<EOF
# Changelog

All notable changes to this project will be documented in this file.

## [$version] - $release_date

### Added

- TBD

### Changed

- TBD

### Fixed

- TBD

EOF
tail -n +4 CHANGELOG.md >>"$tmp_changelog"
mv "$tmp_changelog" CHANGELOG.md

cat >"$release_notes_path" <<EOF
# LearnChain v$version

This release includes:

## Added

- TBD

## Changed

- TBD

## Fixed

- TBD
EOF

printf 'Prepared v%s for %s\n' "$version" "$release_date"
printf 'Updated: Cargo.toml, Cargo.lock, package.json, package-lock.json, CHANGELOG.md\n'
printf 'Created: %s\n' "$release_notes_path"
