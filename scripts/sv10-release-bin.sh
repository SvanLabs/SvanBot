#!/usr/bin/env bash
# Source this, then `exec "$(sv10_release_bin)" <surface> "$@"`. release.sh, update.sh and rollback.sh are
# wrappers around the Rust installer (crates/apps/release, #742), and the wrapper runs this checkout's own
# code: update.sh fast-forwards the checkout before it releases, so the checkout is newer than any
# installed set, and a rolled-back set that lacks the binary still has a way in.
#
# `SV10_RELEASE_BIN` names a prebuilt binary and wins (the tests use it, and so does adopt-upstream.sh,
# which runs another checkout's update.sh in a tree that has no Rust sources). Otherwise the binary is
# built into target/dev, never target/release (a release build there would overwrite what the fleet
# hot-swaps from), and only when a source it depends on is newer than it.
sv10_release_bin() {
  if [ -n "${SV10_RELEASE_BIN:-}" ]; then echo "$SV10_RELEASE_BIN"; return; fi
  local bin=target/dev/release/sv10-release
  if [ ! -x "$bin" ] || [ -n "$(find crates/apps/release crates/deps/digest crates/deps/rt Cargo.toml Cargo.lock \
      \( -name '*.rs' -o -name Cargo.toml -o -name Cargo.lock \) -newer "$bin" -print -quit 2>/dev/null)" ]; then
    echo "== building the installer (sv10-release)" >&2
    CARGO_TARGET_DIR=target/dev cargo build --release -p sv10-release -q >&2 || {
      echo "could not build the installer; set SV10_RELEASE_BIN to a built sv10-release" >&2
      return 1
    }
  fi
  echo "$bin"
}
