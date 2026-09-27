#!/usr/bin/env bash
# Bootstrap for Claude Code cloud sessions (claude.ai/code); CLAUDE.md tells a cloud session to
# run it first. Idempotent and never fatal: it installs
# what scripts/check.sh needs (pre-commit hook, cargo-deny, web deps). Rust comes from
# rust-toolchain.toml via rustup. Runtime data is opt-in: scripts/fetch-data.sh.
# Cloud sessions never run the live fleet (no .env, no API keys): no start.sh, no release.sh.
cd "$(dirname "$0")/.." || exit 0
export PATH="$HOME/.cargo/bin:$PATH"
log() { echo "cloud-setup: $*" >&2; }

ln -sf ../../scripts/pre-commit.sh .git/hooks/pre-commit 2>/dev/null || log "could not link the pre-commit hook"
mkdir -p artifacts/logs artifacts/backups

if ! command -v cargo-deny >/dev/null 2>&1 && command -v cargo >/dev/null 2>&1; then
  tag=$(curl -fsSL https://api.github.com/repos/EmbarkStudios/cargo-deny/releases/latest 2>/dev/null |
    sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
  if [ -n "$tag" ]; then
    name="cargo-deny-${tag}-x86_64-unknown-linux-musl"
    mkdir -p "$HOME/.cargo/bin"
    curl -fsSL "https://github.com/EmbarkStudios/cargo-deny/releases/download/${tag}/${name}.tar.gz" |
      tar -xz -C "$HOME/.cargo/bin" --strip-components=1 "${name}/cargo-deny" 2>/dev/null ||
      log "cargo-deny download failed; install with: cargo install --locked cargo-deny"
  else
    log "cargo-deny release lookup failed; install with: cargo install --locked cargo-deny"
  fi
fi

if command -v npm >/dev/null 2>&1 && [ ! -d web/node_modules ]; then
  (cd web && npm ci --no-audit --no-fund >/dev/null 2>&1) || log "npm ci failed in web/"
fi

# The board-strength tables are a build artifact and not data: `tables build` computes them exactly,
# and a pair generated from scratch is byte-identical to the shipped one (checked 2026-09-27). They
# are not in git (96 MB) and `fetch-data.sh` does not have to supply them. Without them
# `equity::tables::loaded` returns None and every board's strengths are recomputed — about 15x
# slower sims (docs/OPERATIONS.md), and one learner test's outcome changes with the speed. Build
# them once, into `artifacts/tables` explicitly rather than through `tables_dir`'s search.
if [ ! -f artifacts/tables/strengths-turn.sv10tbl ] && command -v cargo >/dev/null 2>&1; then
  log "building the board-strength tables (~1 min, once)"
  CARGO_TARGET_DIR=target/dev cargo run --release -p sv10-core --bin tables -- build artifacts/tables >/dev/null 2>&1 ||
    log "table build failed; sims will recompute board strengths, ~15x slower (rerun: CARGO_TARGET_DIR=target/dev cargo run --release -p sv10-core --bin tables -- build artifacts/tables)"
fi

if [ ! -f artifacts/svanbot10.db ]; then
  log "no runtime data; run scripts/fetch-data.sh for the latest snapshot from the GitHub release"
fi
exit 0
