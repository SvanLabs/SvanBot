#!/usr/bin/env bash
# Is another cargo build holding this target directory? (#363)
#
# Cargo serialises builds through a lock file per profile directory, `target/<profile>/.cargo-lock`.
# A second cargo that finds it held prints one line — `Blocking waiting for file lock on build
# directory` — and waits, for as long as the holder runs, doing no work at all in the meantime. That
# is invisible from outside and it reads afterwards as the waiter being slow: the release at
# 2026-09-28 05:02 reported a 498 s build stage and compiled no crate, which is not a build.
#
#   scripts/build-lock.sh TARGET_PROFILE_DIR    e.g. target/stage/release
#
# Prints one line naming the held lock, and nothing when there is none, so a caller can put the wait
# in a log where it would otherwise be silent. Advisory: it never fails the caller. Exit 0 means "no
# build holds it or the question could not be answered" — an unanswerable probe is not a held lock,
# and a caller that treated it as one would report a build that is not there.
set -euo pipefail

case "${1:-}" in
  "") echo "usage: scripts/build-lock.sh TARGET_PROFILE_DIR   (e.g. target/stage/release)" >&2; exit 2 ;;
  -h|--help) echo "usage: scripts/build-lock.sh TARGET_PROFILE_DIR   (e.g. target/stage/release)"; exit 0 ;;
esac
dir=$1

# Nothing has built here yet, or the directory is gone: no build can be in progress.
lock="$dir/.cargo-lock"
[ -e "$lock" ] || exit 0
command -v flock >/dev/null 2>&1 || exit 0

# Take the lock and give it straight back: `flock` releases it when `true` exits, so this cannot
# itself start holding the directory it is asking about.
if flock -n "$lock" true 2>/dev/null; then
  exit 0
fi
echo "build-lock: $dir is held by another cargo build (cargo's build lock); a build starting now waits for it"
