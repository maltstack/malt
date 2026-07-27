#!/usr/bin/env bash
#
# Maintain a Linux-side mirror of this repo for building and testing.
#
# Why this exists
# ---------------
# Building from /mnt/c under WSL goes through the 9p filesystem and is slow
# enough to change how you work: a full workspace build is minutes rather than
# tens of seconds, and cargo's mtime-based freshness checks are unreliable on
# NTFS (AGENTS.md, "NTFS Caching"). Stale artifacts there produce test
# failures that do not match the source, which has already cost this project
# an afternoon.
#
# The alternative -- waiting for CI -- costs ~3 minutes per bit of information.
# The inverted Unix PTY in docs/briefs/007 was root-caused in WSL in a handful
# of 50-second cycles; it would have taken most of an hour through CI.
#
# How it works
# ------------
# A real git clone on the Linux filesystem (~/malt by default), synced by
# fetching from the Windows checkout and hard-resetting to a chosen ref.
#
# Deliberately NOT a file copy or a post-commit hook:
#   - copying fights target/ (gigabytes, constantly rewritten) and .git
#     locking, and there is no safe moment to copy a repo being written to;
#   - a post-commit hook would push half-finished states and would fire on
#     every commit whether or not a Linux build is wanted.
#
# The mirror is disposable. It is hard-reset on every sync, so never edit
# there and never commit from it -- Windows remains the source of truth.
#
# Usage
# -----
#   scripts/wsl-mirror.sh                 # sync to current HEAD, then build+test
#   scripts/wsl-mirror.sh --ref main      # sync to a specific ref
#   scripts/wsl-mirror.sh --no-test       # sync and build only
#   scripts/wsl-mirror.sh --sync-only     # sync, no cargo
#   scripts/wsl-mirror.sh -- -p malt-daemon --test coordinator
#                                         # everything after -- goes to cargo test
#
# Run from Windows (it re-enters WSL itself) or from inside WSL.

set -euo pipefail

MIRROR="${MALT_WSL_MIRROR:-$HOME/malt}"
# Target dir on the Linux filesystem, never under /mnt/c: this is the whole
# point of the mirror, and pointing it at a Windows path undoes the benefit.
TARGET_DIR="${MALT_WSL_TARGET_DIR:-/tmp/malt-build}"

REF=""
DO_BUILD=1
DO_TEST=1
CARGO_ARGS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --ref)       REF="${2:?--ref needs a value}"; shift 2 ;;
        --no-test)   DO_TEST=0; shift ;;
        --sync-only) DO_TEST=0; DO_BUILD=0; shift ;;
        --)          shift; CARGO_ARGS=("$@"); break ;;
        -h|--help)   sed -n '2,40p' "$0"; exit 0 ;;
        *)           echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

# Re-enter WSL if invoked from Windows, so the same script works from both.
if [[ ! -f /proc/version ]] || ! grep -qi microsoft /proc/version 2>/dev/null; then
    echo "error: run this inside WSL, or via: wsl -e bash scripts/wsl-mirror.sh" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Resolve the real repository root. In a git worktree, .git is a *file* whose
# gitdir: line holds a Windows path (C:/Users/...), which means nothing inside
# WSL -- cloning from a worktree therefore fails with a nonsense path. Walk up
# until we find a directory whose .git is an actual directory; that is the main
# checkout, and it holds every ref including the worktree branches.
SOURCE_DIR="$SCRIPT_DIR"
while [[ ! -d "$SOURCE_DIR/.git" && "$SOURCE_DIR" != "/" ]]; do
    SOURCE_DIR="$(dirname "$SOURCE_DIR")"
done
if [[ ! -d "$SOURCE_DIR/.git" ]]; then
    echo "error: no git repository with a real .git directory above $SCRIPT_DIR" >&2
    exit 1
fi

if [[ -z "$REF" ]]; then
    # HEAD of the tree the script was invoked from, which is what the caller
    # means by "current" -- not the main checkout's HEAD, which may differ.
    #
    # In a worktree, .git is a file holding "gitdir: C:/Users/..." -- a Windows
    # path git cannot use from here. Translate it to /mnt/<drive>/... and point
    # --git-dir at it, so "current HEAD" keeps meaning the worktree's HEAD
    # rather than silently becoming the main checkout's.
    if [[ -f "$SCRIPT_DIR/.git" ]]; then
        WIN_GITDIR="$(sed -n 's/^gitdir: //p' "$SCRIPT_DIR/.git")"
        # Parameter expansion rather than sed: drive-letter and backslash
        # handling is fiddly enough that a regex is the wrong tool here.
        # tr, not ${//}: the parameter-expansion form for this is ambiguous
        # in bash and silently deleted every slash instead of the backslashes.
        GITDIR="$(printf '%s' "$WIN_GITDIR" | tr '\\' '/')"
        GITDRIVE="${GITDIR%%:*}"                      # "C"
        GITREST="${GITDIR#*:}"                        # "/Users/..."
        GITDRIVE="$(printf '%s' "$GITDRIVE" | tr '[:upper:]' '[:lower:]')"
        GITDIR="/mnt/${GITDRIVE}${GITREST}"
        if [[ ! -d "$GITDIR" ]]; then
            echo "error: could not resolve worktree gitdir '$WIN_GITDIR' -> '$GITDIR'" >&2
            echo "       pass --ref explicitly instead" >&2
            exit 1
        fi
        REF="$(git --git-dir="$GITDIR" rev-parse HEAD)"
    else
        REF="$(git -C "$SCRIPT_DIR" rev-parse HEAD)"
    fi
fi

echo "==> source: $SOURCE_DIR"
echo "==> mirror: $MIRROR"
echo "==> ref:    $REF"

if [[ ! -d "$MIRROR/.git" ]]; then
    echo "==> creating mirror (first run; this clone is a one-off cost)"
    git clone --no-checkout "$SOURCE_DIR" "$MIRROR"
fi

git -C "$MIRROR" fetch --prune "$SOURCE_DIR" "+refs/heads/*:refs/remotes/windows/*" --tags --force
# Fetch the exact commit too: HEAD may be detached or on a branch the
# refspec above does not cover (a worktree branch, for instance).
git -C "$MIRROR" fetch "$SOURCE_DIR" "$REF" --force 2>/dev/null || true

# Hard reset, then clean everything untracked EXCEPT the cargo target dir if
# it happens to live inside the mirror -- deleting that would throw away the
# incremental build this script exists to preserve.
git -C "$MIRROR" reset --hard "$REF"
git -C "$MIRROR" clean -fdx --exclude=target

echo "==> mirror now at $(git -C "$MIRROR" rev-parse --short HEAD)"

if [[ "$DO_BUILD" -eq 0 ]]; then
    exit 0
fi

# shellcheck disable=SC1090
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"

if ! command -v vexilc >/dev/null 2>&1; then
    cat >&2 <<'EOF'
error: vexilc is not on PATH.

malt-protocol's build.rs shells out to it, so nothing in this workspace
compiles without it. Install the same pinned revision CI uses:

  cargo install --git https://github.com/vexil-lang/vexil \
      --rev fc8c51f31f1f25f0b2885fc98696ad1c5ee543c7 vexilc

Pinned rather than latest on purpose: a different vexilc can emit different
generated code, so an unpinned local build would compile something other than
what CI compiles.
EOF
    exit 1
fi

export CARGO_TARGET_DIR="$TARGET_DIR"
cd "$MIRROR"

echo "==> cargo build --workspace  (target: $CARGO_TARGET_DIR)"
cargo build --workspace

if [[ "$DO_TEST" -eq 1 ]]; then
    if [[ ${#CARGO_ARGS[@]} -gt 0 ]]; then
        echo "==> cargo test ${CARGO_ARGS[*]}"
        cargo test "${CARGO_ARGS[@]}"
    else
        echo "==> cargo test --workspace"
        cargo test --workspace
    fi
fi
