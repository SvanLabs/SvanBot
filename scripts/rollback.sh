#!/usr/bin/env bash
# Hash-verified, failure-safe installation snapshots and code rollback.
set -euo pipefail

script_root=$(cd "$(dirname "$0")/.." && pwd -P)
release_root=${SV10_RELEASE_ROOT:-$script_root}
cleanup_stage=
cleanup_aux=
operation_lock_fd=

die() {
  echo "rollback: $*" >&2
  exit 1
}

cleanup() {
  [ -z "$cleanup_stage" ] || rm -rf -- "$cleanup_stage"
  [ -z "$cleanup_aux" ] || rm -rf -- "$cleanup_aux"
}
trap cleanup EXIT

validate_root() {
  [ -n "$release_root" ] || die "release root is empty"
  [ -d "$release_root" ] || die "release root does not exist: $release_root"
  release_root=$(cd "$release_root" && pwd -P)
  local home_root
  home_root=$(cd "${HOME:?HOME is not set}" && pwd -P)
  case "$release_root" in /|"$home_root") die "refusing unsafe release root: $release_root" ;; esac
  git -C "$release_root" rev-parse --git-dir >/dev/null 2>&1 || die "release root is not a git repository"
}

assert_managed_path() {
  local rel=$1 current=$release_root part resolved
  case "$rel" in ""|/*|*..*) die "unsafe managed path: $rel" ;; esac
  IFS=/ read -r -a parts <<< "$rel"
  for part in "${parts[@]}"; do
    current="$current/$part"
    [ ! -L "$current" ] || die "managed path contains a symlink: $rel"
    if [ -e "$current" ]; then
      resolved=$(readlink -f -- "$current")
      case "$resolved" in "$release_root"|"$release_root"/*) ;; *) die "managed path escapes repository: $rel" ;; esac
    fi
  done
}

validate_layout() {
  local rel
  for rel in target target/release web web/dist artifacts artifacts/release-snapshots artifacts/release-operation.lock artifacts/release.lock artifacts/release.log artifacts/release-swap.journal; do
    assert_managed_path "$rel"
  done
}

validate_source_clean() {
  local dirty
  dirty=$(git -C "$release_root" status --porcelain --untracked-files=all -- \
    crates Cargo.toml Cargo.lock build.rs .cargo rust-toolchain rust-toolchain.toml web)
  if [ -n "$dirty" ]; then
    printf '%s\n' "$dirty" >&2
    die "uncommitted or untracked Rust/web build inputs"
  fi
}

swap_journal() { printf '%s/artifacts/release-swap.journal' "$release_root"; }

# The directory renames that make an install are five steps with no transaction around them: a SIGKILL
# (OOM, a stop, power loss) between any two leaves the tree half installed — after the first one there
# is no `target/release` at all and the supervisors crash-loop. This journal is written and flushed
# before the first rename, so every kill point is accounted for and the next operation — or
# keepalive's repair, or `rollback.sh --repair` — can put the previous verified sets back (issue #726).
write_swap_journal() {
  local staging=$1 old_release=$2 old_web=$3 have_release=$4 have_web=$5 tmp
  tmp="$release_root/artifacts/release-swap.journal.new.$$"
  {
    printf 'release_root=%s\n' "$release_root"
    printf 'staging=%s\n' "$staging"
    printf 'old_release=%s\n' "$old_release"
    printf 'old_web=%s\n' "$old_web"
    printf 'have_release=%s\n' "$have_release"
    printf 'have_web=%s\n' "$have_web"
  } > "$tmp"
  mv -T "$tmp" "$(swap_journal)"
  # One flush of the filesystem holding the journal, so the renames below cannot outlive their record.
  sync -f "$(swap_journal)" 2>/dev/null || sync 2>/dev/null || true
}

# Restore whatever an interrupted swap left behind. Idempotent: the journal is removed only after the
# previous sets are back, and a run killed mid-repair is repaired again by the next one.
repair_interrupted_swap() {
  local journal root= line key value staging= old_release= old_web= have_release= have_web= restored= path
  journal=$(swap_journal)
  [ -f "$journal" ] || return 0
  while IFS= read -r line; do
    key=${line%%=*}
    value=${line#*=}
    case "$key" in
      release_root) root=$value ;;
      staging) staging=$value ;;
      old_release) old_release=$value ;;
      old_web) old_web=$value ;;
      have_release) have_release=$value ;;
      have_web) have_web=$value ;;
    esac
  done < "$journal"
  [ "${root:-}" = "$release_root" ] || die "swap journal names another release root: ${root:-<none>}"
  for path in "$staging" "$old_release" "$old_web"; do
    case "$path" in "$release_root"/*) ;; *) die "swap journal names a path outside the release root: ${path:-<none>}" ;; esac
  done
  case "$staging" in
    "$release_root"/target/.install-*|"$release_root"/target/.rollback-*) ;;
    *) die "swap journal names an unexpected staging directory: ${staging:-<none>}" ;;
  esac
  echo "rollback: repairing an interrupted release swap (journal $journal)" >&2
  # A killed run's EXIT trap never ran, so its staging tree — and the copy about to be installed — is
  # still there. Removing both staged sets is what leaves the tree consistent either way.
  rm -rf -- "$staging"
  if [ "$have_release" = 1 ] && [ -e "$old_release" ]; then
    rm -rf -- "$release_root/target/release"
    mv -- "$old_release" "$release_root/target/release"
    restored=1
  elif [ "$have_release" = 0 ] && [ -e "$release_root/target/release" ]; then
    # A first install that never finished: there was no previous set, so none is installed again.
    rm -rf -- "$release_root/target/release"
    restored=1
  fi
  if [ "$have_web" = 1 ] && [ -e "$old_web" ]; then
    rm -rf -- "$release_root/web/dist"
    mv -- "$old_web" "$release_root/web/dist"
    restored=1
  elif [ "$have_web" = 0 ] && [ -e "$release_root/web/dist" ]; then
    rm -rf -- "$release_root/web/dist"
    restored=1
  fi
  rm -f -- "$journal"
  [ -z "$restored" ] || echo "rollback: restored the previous install; installed commit is now $(installed_commit || echo unidentified)" >&2
}

acquire_operation_lock() {
  local lock="$release_root/artifacts/release-operation.lock" inherited target
  mkdir -p "$release_root/artifacts"
  if [[ ${SV10_RELEASE_LOCK_FD:-} =~ ^[0-9]+$ ]]; then
    inherited=$SV10_RELEASE_LOCK_FD
    target=$(readlink -f "/proc/$$/fd/$inherited" 2>/dev/null || true)
    [ "$target" = "$lock" ] || die "inherited release lock does not identify the managed lock file"
    flock -n "$inherited" || die "inherited release operation lock is not held"
  else
    exec {operation_lock_fd}> "$lock"
    flock -n "$operation_lock_fd" || die "another release, snapshot, or rollback operation is active"
  fi
  repair_interrupted_swap
}

resolve_commit() {
  local requested=$1 resolved
  [ -n "$requested" ] || die "commit is empty"
  resolved=$(git -C "$release_root" rev-parse --verify "${requested}^{commit}" 2>/dev/null) ||
    die "commit does not resolve in this repository: $requested"
  git -C "$release_root" rev-parse --short=7 "$resolved"
}

installed_commit() {
  local marker="$release_root/target/release/.sv10-installed-commit" log candidate
  if [ -f "$marker" ]; then
    candidate=$(tr -d '[:space:]' < "$marker")
    resolve_commit "$candidate"
    return
  fi
  log="$release_root/artifacts/releases.log"
  [ -f "$log" ] || return 1
  while IFS= read -r candidate; do
    [ -n "$candidate" ] || continue
    if git -C "$release_root" rev-parse --verify "${candidate}^{commit}" >/dev/null 2>&1; then
      resolve_commit "$candidate"
      return
    fi
  done < <(awk 'NF >= 2 { print $2 }' "$log" | tac)
  return 1
}

health_commit() {
  local url
  local port
  # The dashboard port lives in .env (SVANBOT_WEB_PORT); read only that line.
  port=${SVANBOT_WEB_PORT:-$(sed -n 's/^SVANBOT_WEB_PORT=\([0-9]*\)$/\1/p' "$release_root/.env" 2>/dev/null | tail -1)}
  url=${SV10_HEALTH_URL:-http://127.0.0.1:${port:-5000}/api/health}
  python3 - "$url" <<'PY'
import json, sys, urllib.request
with urllib.request.urlopen(sys.argv[1], timeout=3) as response:
    value = json.load(response)
commit = value.get("commit")
if not isinstance(commit, str) or not commit:
    raise SystemExit("health response has no commit")
print(commit)
PY
}

# The install is complete only once the fleet serves the installed commit, not when the files land
# (issue #726): a binary that answers `--version` and dies at startup is installed by a files-only
# check and then crash-loops. Bounded: the hot swap itself can take the settle window (20 s) plus the
# wait for a mid-turn bot (up to 90 s) before the process even restarts, so the default is generous;
# past it the caller rolls back. `SV10_HEALTH_TIMEOUT` is a whole number of seconds.
await_health() {
  local expected=$1 timeout=${SV10_HEALTH_TIMEOUT:-240} deadline now got last=
  [[ $timeout =~ ^[0-9]+$ ]] || die "SV10_HEALTH_TIMEOUT must be a whole number of seconds"
  expected=$(resolve_commit "$expected")
  deadline=$(($(date +%s) + timeout))
  while :; do
    got=$(health_commit 2>/dev/null || true)
    if [ -n "$got" ] && [ "$(resolve_commit "$got" 2>/dev/null || true)" = "$expected" ]; then
      echo "Health reports the installed commit $expected"
      return 0
    fi
    [ -n "$got" ] && last="; health last reported $got"
    now=$(date +%s)
    [ "$now" -lt "$deadline" ] || break
    sleep 3
  done
  die "the fleet did not answer /api/health with commit $expected within ${timeout}s$last"
}

# Free space the release path needs before it stages anything (LESSONS 22: snapshots and hourly
# backups filled the root once already). Fails closed here, before a snapshot or a build copy is
# half-written, rather than after. `SV10_MIN_FREE_MB` overrides the default for small volumes.
check_space() {
  local required=${1:-${SV10_MIN_FREE_MB:-4096}} avail_kb
  [[ $required =~ ^[0-9]+$ ]] || die "SV10_MIN_FREE_MB must be a whole number of megabytes"
  avail_kb=$(df -Pk "$release_root" | awk 'NR == 2 { print $4 }')
  [[ $avail_kb =~ ^[0-9]+$ ]] || die "could not read free space for $release_root"
  [ "$avail_kb" -ge $((required * 1024)) ] ||
    die "only $((avail_kb / 1024)) MB free on $release_root and a release needs about ${required} MB (SV10_MIN_FREE_MB)"
  echo "$((avail_kb / 1024)) MB free on $release_root (>= ${required} MB)"
}

running_installed_bot_is_verified() {
  local pidfile pid
  for pidfile in "$release_root/artifacts/bot.pid" "$release_root/artifacts/head.pid" "$release_root"/artifacts/worker-*.pid; do
    [ -f "$pidfile" ] || continue
    pid=$(tr -d '[:space:]' < "$pidfile")
    [[ $pid =~ ^[0-9]+$ ]] || continue
    [ -e "/proc/$pid/exe" ] || continue
    [ "$release_root/target/release/sv10-bot" -ef "/proc/$pid/exe" ] && return 0
  done
  return 1
}

adopt_legacy_identity() {
  local requested=$1 expected logged health output name program version build extra marker tmp
  expected=$(resolve_commit "$requested")
  validate_layout
  acquire_operation_lock
  marker="$release_root/target/release/.sv10-installed-commit"
  [ ! -e "$marker" ] || die "installed commit marker already exists"
  logged=$(installed_commit || true)
  [ "$logged" = "$expected" ] || die "release log does not identify legacy commit $expected"
  for name in sv10-bot learner analyst; do
    [ -f "$release_root/target/release/$name" ] && [ -x "$release_root/target/release/$name" ] ||
      die "legacy required binary missing or non-executable: $name"
    output=$("$release_root/target/release/$name" --version 2>/dev/null) || die "$name --version failed"
    read -r program version build extra <<< "$output"
    [ "$program" = "$name" ] && [ -n "$version" ] && [ -z "${build:-}" ] && [ -z "${extra:-}" ] ||
      die "$name is not an unmarked legacy binary"
  done
  running_installed_bot_is_verified || die "no running process matches the installed legacy sv10-bot inode"
  health=$(health_commit) || die "could not read legacy build identity from local health"
  health=$(resolve_commit "$health")
  [ "$health" = "$expected" ] || die "running health identifies $health, expected $expected"
  tmp="$release_root/target/release/.sv10-installed-commit.new.$$"
  printf '%s\n' "$expected" > "$tmp"
  mv "$tmp" "$marker"
  require_binaries "$release_root/target/release" "$expected"
  echo "Adopted verified legacy installed identity $expected"
}

# One-time escape for an installed build that carries no commit identity (e.g. `11.0.0 dev`,
# built straight into target/release): it cannot become a commit-keyed snapshot, so keep a
# verified copy under artifacts/unidentified-builds/ instead. Only while a verified rollback
# snapshot exists, so the next release still has somewhere safe to roll back to. Read-only on
# the installed tree.
preserve_unidentified() {
  local parent stamp dest src rel commit verified= name
  validate_layout
  acquire_operation_lock
  [ ! -e "$release_root/target/release/.sv10-installed-commit" ] ||
    die "installed release is identified; snapshot it with --snapshot instead"
  [ -d "$release_root/web/dist" ] || die "installed dashboard missing: web/dist"
  reject_special_files "$release_root/target/release"
  reject_special_files "$release_root/web/dist"
  for src in "$release_root"/artifacts/release-snapshots/*/; do
    [ -d "$src" ] || continue
    commit=$(basename "$src")
    if (verify_snapshot "$commit") >/dev/null 2>&1; then verified=$commit; break; fi
  done
  [ -n "$verified" ] || die "no verified rollback snapshot exists; refusing to release over an unidentified build"
  parent="$release_root/artifacts/unidentified-builds"
  mkdir -p "$parent"
  stamp=$(date -u +%Y%m%dT%H%M%SZ)
  dest="$parent/$stamp"
  [ ! -e "$dest" ] || die "preserved build already exists: $dest"
  cleanup_stage=$(mktemp -d "$parent/.${stamp}.tmp.XXXXXX")
  mkdir -p "$cleanup_stage/target/release" "$cleanup_stage/web/dist"
  while IFS= read -r -d '' src; do
    rel=${src#"$release_root/"}
    cp -p "$src" "$cleanup_stage/$rel"
  done < <(find "$release_root/target/release" -maxdepth 1 -type f -perm /111 ! -name '.*' -print0)
  cp -a "$release_root/web/dist/." "$cleanup_stage/web/dist/"
  for name in sv10-bot learner analyst; do
    [ -x "$cleanup_stage/target/release/$name" ] || die "unidentified build lacks $name"
    "$cleanup_stage/target/release/$name" --version >> "$cleanup_stage/VERSIONS" 2>&1 || die "$name --version failed"
  done
  write_manifest "$cleanup_stage"
  verify_tree_against_manifest "$cleanup_stage" "$cleanup_stage/SHA256SUMS"
  mv -T "$cleanup_stage" "$dest"
  cleanup_stage=
  echo "Rollback target stays verified snapshot $verified"
  echo "Preserved unidentified build at $dest"
}

require_binaries() {
  local base=$1 expected=${2:-} identity_mode=${3:-allow-marker} marker= output name program version build extra
  [ -z "$expected" ] || expected=$(resolve_commit "$expected")
  [ ! -f "$base/.sv10-installed-commit" ] || marker=$(tr -d '[:space:]' < "$base/.sv10-installed-commit")
  for name in sv10-bot learner analyst; do
    [ -f "$base/$name" ] || die "required binary missing: $name"
    [ -x "$base/$name" ] || die "required binary is not executable: $name"
    output=$("$base/$name" --version 2>/dev/null) || die "$name --version failed"
    read -r program version build extra <<< "$output"
    [ "$program" = "$name" ] && [ -n "$version" ] && [ -z "${extra:-}" ] || die "$name --version is malformed"
    if [ -n "$expected" ]; then
      if [ -n "${build:-}" ]; then
        [ "$build" = "$expected" ] || die "$name identifies build $build, expected $expected"
      elif [ "$identity_mode" = strict ]; then
        die "$name does not identify its build commit"
      else
        [ "$marker" = "$expected" ] || die "$name has no build identity and no matching installed marker"
      fi
    fi
  done
}

reject_special_files() {
  local base=$1 bad
  bad=$(find "$base" \( -type l -o \( ! -type f ! -type d \) \) -print -quit)
  [ -z "$bad" ] || die "symlink or special file is not allowed in a release tree: $bad"
}

write_manifest() {
  local base=$1 rel
  (
    cd "$base"
    find target/release web/dist -type f -print | LC_ALL=C sort | while IFS= read -r rel; do
      case "$rel" in *[[:space:]]*) die "release path contains whitespace: $rel" ;; esac
      sha256sum "$rel"
    done > SHA256SUMS
    [ -s SHA256SUMS ] || die "release manifest would be empty"
    sha256sum --check --strict SHA256SUMS >/dev/null
  )
}

verify_tree_against_manifest() {
  local base=$1 manifest=$2 listed actual
  cleanup_aux=$(mktemp -d)
  listed="$cleanup_aux/listed"
  actual="$cleanup_aux/actual"
  sed 's/^[0-9a-fA-F]\{64\} [ *]//' "$manifest" | sort > "$listed"
  (
    cd "$base"
    find target/release web/dist -type f -print | sort > "$actual"
    diff -u "$listed" "$actual" >/dev/null || die "release manifest is incomplete"
    sha256sum --check --strict "$manifest" >/dev/null || die "release hash verification failed"
  )
  rm -rf -- "$cleanup_aux"
  cleanup_aux=
}

manifest_paths_are_safe() {
  local manifest=$1 hash path
  while read -r hash path; do
    [[ $hash =~ ^[0-9a-fA-F]{64}$ ]] || die "invalid snapshot hash entry"
    path=${path#\*}
    case "$path" in target/release/*|web/dist/*) ;; *) die "unsafe snapshot manifest path: $path" ;; esac
    case "$path" in *..*|/*) die "unsafe snapshot manifest path: $path" ;; esac
  done < "$manifest"
}

verify_snapshot() {
  local commit=$1 snapshot manifest
  snapshot="$release_root/artifacts/release-snapshots/$commit"
  manifest="$snapshot/SHA256SUMS"
  [ -d "$snapshot" ] && [ ! -L "$snapshot" ] || die "snapshot does not exist safely: $commit"
  [ -f "$manifest" ] && [ ! -L "$manifest" ] || die "snapshot manifest missing: $commit"
  reject_special_files "$snapshot/target/release"
  reject_special_files "$snapshot/web/dist"
  manifest_paths_are_safe "$manifest"
  verify_tree_against_manifest "$snapshot" "$manifest"
  require_binaries "$snapshot/target/release" "$commit"
  echo "Verified release snapshot $commit"
}

create_snapshot() {
  local requested=$1 commit parent snapshot src rel
  commit=$(resolve_commit "$requested")
  validate_layout
  acquire_operation_lock
  require_binaries "$release_root/target/release" "$commit"
  [ -d "$release_root/web/dist" ] || die "installed dashboard missing: web/dist"
  reject_special_files "$release_root/web/dist"
  parent="$release_root/artifacts/release-snapshots"
  mkdir -p "$parent"
  snapshot="$parent/$commit"
  if [ -e "$snapshot" ]; then
    verify_snapshot "$commit"
    echo "Release snapshot $commit already exists; keeping it unchanged"
    return
  fi
  cleanup_stage=$(mktemp -d "$parent/.${commit}.tmp.XXXXXX")
  mkdir -p "$cleanup_stage/target/release" "$cleanup_stage/web/dist"
  while IFS= read -r -d '' src; do
    rel=${src#"$release_root/"}
    cp -p "$src" "$cleanup_stage/$rel"
  done < <(find "$release_root/target/release" -maxdepth 1 -type f -perm /111 ! -name '.*' -print0)
  printf '%s\n' "$commit" > "$cleanup_stage/target/release/.sv10-installed-commit"
  cp -a "$release_root/web/dist/." "$cleanup_stage/web/dist/"
  reject_special_files "$cleanup_stage/web/dist"
  write_manifest "$cleanup_stage"
  require_binaries "$cleanup_stage/target/release" "$commit"
  mv -T "$cleanup_stage" "$snapshot"
  cleanup_stage=
  verify_snapshot "$commit"
  echo "Created release snapshot $commit"
  prune_snapshots "$commit"
}

# Keep the newest SV10_KEEP_SNAPSHOTS release snapshots (default 5, ~1.3 GB); every release adds
# ~250 MB and 26 had piled up by 2026-09-24, when the disk filled (LESSONS 22). The snapshot just
# made is never removed; hidden staging directories are not snapshots.
prune_snapshots() {
  local keep_commit=$1 keep=${SV10_KEEP_SNAPSHOTS:-5} parent dir n=0
  parent="$release_root/artifacts/release-snapshots"
  [[ "$keep" =~ ^[0-9]+$ ]] && [ "$keep" -ge 2 ] || die "SV10_KEEP_SNAPSHOTS must be a whole number of at least 2"
  while IFS= read -r dir; do
    n=$((n + 1))
    [ "$n" -gt "$keep" ] || continue
    [ "$(basename "$dir")" != "$keep_commit" ] || continue
    rm -rf -- "$dir"
    echo "Pruned release snapshot $(basename "$dir")"
  done < <(find "$parent" -mindepth 1 -maxdepth 1 -type d ! -name '.*' -printf '%T@ %p\n' | sort -rn | cut -d' ' -f2-)
}

restore_old_install() {
  local failed_release=$1 old_release=$2 failed_web=$3 old_web=$4
  [ ! -e "$release_root/target/release" ] || mv "$release_root/target/release" "$failed_release"
  [ ! -e "$old_release" ] || mv "$old_release" "$release_root/target/release"
  if [ -e "$old_web" ]; then
    [ ! -e "$release_root/web/dist" ] || mv "$release_root/web/dist" "$failed_web"
    mv "$old_web" "$release_root/web/dist"
  fi
  rm -rf -- "$failed_release" "$failed_web"
}

swap_install() {
  local staged_release=$1 staged_web=$2 staging old_release old_web failed_release failed_web path have_release=0 have_web=0
  old_release="$release_root/target/.release.before-swap.$$"
  old_web="$release_root/web/.dist.before-swap.$$"
  failed_release="$release_root/target/.release.failed-swap.$$"
  failed_web="$release_root/web/.dist.failed-swap.$$"
  [ "$(stat -c %d "$release_root/target")" = "$(stat -c %d "$release_root/web")" ] ||
    die "target and web are on different filesystems; atomic directory swap is unavailable"
  if [ "${SV10_RELEASE_TEST_FAIL_AFTER_BIN_SWAP:-0}" = 1 ] || [ "${SV10_RELEASE_TEST_KILL_AFTER_BIN_SWAP:-0}" = 1 ]; then
    [ "$release_root" != "$script_root" ] || die "test failure injection is forbidden on the real root"
  fi
  for path in "$old_release" "$old_web" "$failed_release" "$failed_web"; do [ ! -e "$path" ] || die "swap scratch path exists: $path"; done
  [ ! -d "$release_root/target/release" ] || have_release=1
  [ ! -d "$release_root/web/dist" ] || have_web=1
  [ "$have_release" = "$have_web" ] || die "installed executable and dashboard sets are incomplete"
  # The staging tree both callers build (mktemp -d target/.install-<commit>.XXXXXX); the journal names
  # it so a killed run's leftovers are removed with the repair.
  staging=$(dirname "$(dirname "$staged_release")")
  write_swap_journal "$staging" "$old_release" "$old_web" "$have_release" "$have_web"
  if [ "$have_release" = 0 ]; then
    if ! mv "$staged_release" "$release_root/target/release"; then
      fail_swap "could not install the first executable set"
    fi
    if ! mv "$staged_web" "$release_root/web/dist"; then
      mv "$release_root/target/release" "$staged_release"
      fail_swap "could not install the first dashboard set"
    fi
    rm -f -- "$(swap_journal)"
    return
  fi
  mv "$release_root/target/release" "$old_release"
  if [ "${SV10_RELEASE_TEST_KILL_AFTER_BIN_SWAP:-0}" = 1 ]; then
    kill -9 "$$" # a real SIGKILL at the worst point: no trap, no cleanup, the journal is the record
  fi
  if ! mv "$staged_release" "$release_root/target/release"; then
    mv "$old_release" "$release_root/target/release"
    fail_swap "could not install staged executable set"
  fi
  if [ "${SV10_RELEASE_TEST_FAIL_AFTER_BIN_SWAP:-0}" = 1 ]; then
    restore_old_install "$failed_release" "$old_release" "$failed_web" "$old_web"
    fail_swap "injected failure after executable swap"
  fi
  if ! mv "$release_root/web/dist" "$old_web"; then
    restore_old_install "$failed_release" "$old_release" "$failed_web" "$old_web"
    fail_swap "could not stage the previous dashboard for replacement"
  fi
  if ! mv "$staged_web" "$release_root/web/dist"; then
    restore_old_install "$failed_release" "$old_release" "$failed_web" "$old_web"
    fail_swap "could not install staged dashboard set"
  fi
  # The swap is complete: drop the record before the old sets, so a kill from here on leaves the new
  # build installed rather than a journal that would roll it back.
  rm -f -- "$(swap_journal)"
  rm -rf -- "$old_release" "$old_web"
}

# A swap failure that restored the previous sets by itself is not interrupted: drop the journal so the
# next operation does not repair a tree that is already consistent.
fail_swap() {
  rm -f -- "$(swap_journal)"
  die "$@"
}

install_release() {
  local binary_source=$1 web_source=$2 requested=$3 commit source resolved src
  commit=$(resolve_commit "$requested")
  validate_layout
  acquire_operation_lock
  for source in "$binary_source" "$web_source"; do
    [ -d "$source" ] || die "install source is not a directory: $source"
    [ ! -L "$source" ] || die "install source is a symlink: $source"
    resolved=$(readlink -f -- "$source")
    case "$resolved" in "$release_root"/*) ;; *) die "install source escapes repository: $source" ;; esac
    reject_special_files "$source"
  done
  cleanup_stage=$(mktemp -d "$release_root/target/.install-${commit}.XXXXXX")
  mkdir -p "$cleanup_stage/target/release" "$cleanup_stage/web/dist"
  while IFS= read -r -d '' src; do
    cp -p "$src" "$cleanup_stage/target/release/$(basename "$src")"
  done < <(find "$binary_source" -maxdepth 1 -type f -perm /111 ! -name '.*' -print0)
  printf '%s\n' "$commit" > "$cleanup_stage/target/release/.sv10-installed-commit"
  cp -a "$web_source/." "$cleanup_stage/web/dist/"
  reject_special_files "$cleanup_stage/target/release"
  reject_special_files "$cleanup_stage/web/dist"
  write_manifest "$cleanup_stage"
  require_binaries "$cleanup_stage/target/release" "$commit" strict
  swap_install "$cleanup_stage/target/release" "$cleanup_stage/web/dist"
  rm -rf -- "$cleanup_stage"
  cleanup_stage=
  echo "Installed verified release set $commit"
}

# Data format a build reads (0229): the DATA_FORMAT line of its store codec, 1 for a build from
# before the codec (text columns only).
build_data_format() {
  local format
  format=$(git -C "$release_root" show "$1:crates/libs/store/src/packed.rs" 2>/dev/null |
    sed -n 's/^pub const DATA_FORMAT: u32 = \([0-9][0-9]*\);$/\1/p' | head -1)
  echo "${format:-1}"
}

# The highest data format written next to the databases (artifacts/data-format; 1 when absent).
store_data_format() {
  local format
  format=$(tr -d '[:space:]' < "$release_root/artifacts/data-format" 2>/dev/null || true)
  [[ $format =~ ^[0-9]+$ ]] && echo "$format" || echo 1
}

# A build that cannot read the stored data never gets installed: it would fail on every packed row.
require_readable_store() {
  local commit=$1 build store
  build=$(build_data_format "$commit")
  store=$(store_data_format)
  [ "$build" -ge "$store" ] ||
    die "build $commit reads data format $build but the databases hold format $store (compressed columns, 0229); stop the fleet, run ./target/release/archive unpack, then roll back"
}

restore_snapshot() {
  local requested=$1 commit previous snapshot
  commit=$(resolve_commit "$requested")
  validate_layout
  acquire_operation_lock
  verify_snapshot "$commit"
  require_readable_store "$commit"
  previous=$(installed_commit || true)
  [ -n "$previous" ] || die "current installed commit cannot be resolved"
  snapshot="$release_root/artifacts/release-snapshots/$commit"
  cleanup_stage=$(mktemp -d "$release_root/target/.rollback-${commit}.XXXXXX")
  mkdir -p "$cleanup_stage/target/release" "$cleanup_stage/web/dist"
  cp -a "$snapshot/target/release/." "$cleanup_stage/target/release/"
  cp -a "$snapshot/web/dist/." "$cleanup_stage/web/dist/"
  verify_tree_against_manifest "$cleanup_stage" "$snapshot/SHA256SUMS"
  require_binaries "$cleanup_stage/target/release" "$commit"
  swap_install "$cleanup_stage/target/release" "$cleanup_stage/web/dist"
  rm -rf -- "$cleanup_stage"
  cleanup_stage=
  printf '%s %s rollback from %s\n' "$(date +%FT%T%:z)" "$commit" "$previous" >> "$release_root/artifacts/releases.log"
  echo "Restored $commit from verified snapshot (previous $previous). Watch the hot-swap log."
}

validate_root
case ${1:-} in
  --validate-layout)
    [ "$#" -eq 1 ] || die "usage: scripts/rollback.sh --validate-layout"
    validate_layout
    ;;
  --validate-source-clean)
    [ "$#" -eq 1 ] || die "usage: scripts/rollback.sh --validate-source-clean"
    validate_source_clean
    ;;
  --snapshot)
    [ "$#" -eq 2 ] || die "usage: scripts/rollback.sh --snapshot <commit>"
    create_snapshot "$2"
    ;;
  --verify)
    [ "$#" -eq 2 ] || die "usage: scripts/rollback.sh --verify <commit>"
    verify_snapshot "$(resolve_commit "$2")"
    ;;
  --check-space)
    [ "$#" -eq 1 ] || die "usage: scripts/rollback.sh --check-space"
    check_space
    ;;
  --await-health)
    [ "$#" -eq 2 ] || die "usage: scripts/rollback.sh --await-health <commit>"
    await_health "$2"
    ;;
  --fleet-running)
    [ "$#" -eq 1 ] || die "usage: scripts/rollback.sh --fleet-running"
    running_installed_bot_is_verified
    ;;
  --repair)
    [ "$#" -eq 1 ] || die "usage: scripts/rollback.sh --repair"
    [ -f "$(swap_journal)" ] || echo "No interrupted release swap to repair"
    validate_layout
    acquire_operation_lock
    ;;
  --data-format)
    [ "$#" -eq 2 ] || die "usage: scripts/rollback.sh --data-format <commit>"
    echo "$(build_data_format "$(resolve_commit "$2")") $(store_data_format)"
    ;;
  --installed-commit)
    [ "$#" -eq 1 ] || die "usage: scripts/rollback.sh --installed-commit"
    installed_commit || die "no valid installed commit metadata"
    ;;
  --preserve-unidentified)
    [ "$#" -eq 1 ] || die "usage: scripts/rollback.sh --preserve-unidentified"
    preserve_unidentified
    ;;
  --adopt-legacy)
    [ "$#" -eq 2 ] || die "usage: scripts/rollback.sh --adopt-legacy <commit>"
    adopt_legacy_identity "$2"
    ;;
  --install)
    [ "$#" -eq 4 ] || die "usage: scripts/rollback.sh --install <binary-dir> <web-dir> <commit>"
    install_release "$2" "$3" "$4"
    ;;
  *)
    [ "$#" -eq 1 ] || die "usage: scripts/rollback.sh <commit>"
    restore_snapshot "$1"
    ;;
esac
