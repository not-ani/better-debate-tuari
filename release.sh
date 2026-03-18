#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULT_BRANCH="main"
RELEASE_BRANCH="release"

usage() {
  cat <<'EOF'
Usage: ./release.sh [options]

Push the current HEAD to the default branch and to the release branch.
The release branch push triggers the Tex GitHub Actions release workflow.

Options:
  --skip-main         Do not push HEAD to main before pushing release
  --allow-dirty       Allow running with uncommitted changes
  --yes               Skip the confirmation prompt
  -h, --help          Show this help
EOF
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: missing required command '$1'" >&2
    exit 1
  fi
}

SKIP_MAIN=0
ALLOW_DIRTY=0
ASSUME_YES=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-main)
      SKIP_MAIN=1
      shift
      ;;
    --allow-dirty)
      ALLOW_DIRTY=1
      shift
      ;;
    --yes)
      ASSUME_YES=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option '$1'" >&2
      usage >&2
      exit 1
      ;;
  esac
done

require_command git
require_command python3

cd "$ROOT_DIR"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "error: not inside a git repository" >&2
  exit 1
fi

if ! git remote get-url origin >/dev/null 2>&1; then
  echo "error: remote 'origin' is not configured" >&2
  exit 1
fi

CURRENT_BRANCH="$(git branch --show-current)"
HEAD_SHA="$(git rev-parse --short HEAD)"
VERSION="$(
  python3 - <<'PY'
import json
from pathlib import Path

config = json.loads(Path("apps/tex/src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
print(config["version"])
PY
)"

if [[ "$ALLOW_DIRTY" -ne 1 ]] && [[ -n "$(git status --short --untracked-files=all)" ]]; then
  echo "error: working tree is dirty; commit or stash changes first, or use --allow-dirty" >&2
  exit 1
fi

echo "Preparing Tex release"
echo "  version: ${VERSION}"
echo "  head:    ${HEAD_SHA}"
echo "  branch:  ${CURRENT_BRANCH:-detached}"
echo "  remote:  $(git remote get-url origin)"
echo
echo "This will:"
if [[ "$SKIP_MAIN" -eq 1 ]]; then
  echo "  - skip pushing ${DEFAULT_BRANCH}"
else
  echo "  - push HEAD to ${DEFAULT_BRANCH}"
fi
echo "  - push HEAD to ${RELEASE_BRANCH}"
echo "  - trigger the publish-tex workflow on GitHub"

if [[ "$ASSUME_YES" -ne 1 ]]; then
  printf "Continue? [y/N] "
  read -r response
  if [[ ! "$response" =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 1
  fi
fi

git fetch origin --prune

if [[ "$SKIP_MAIN" -ne 1 ]]; then
  echo "Pushing ${DEFAULT_BRANCH}..."
  git push origin "HEAD:refs/heads/${DEFAULT_BRANCH}"
fi

echo "Pushing ${RELEASE_BRANCH}..."
git push origin "HEAD:refs/heads/${RELEASE_BRANCH}"

echo
echo "Release push complete."
echo "GitHub Actions:"
echo "  https://github.com/$(git remote get-url origin | sed -E 's#(git@github.com:|https://github.com/)##; s#\.git$##')/actions"
