#!/usr/bin/env bash
# Offline source builds (0064): copy every crates.io dependency into vendor/ (gitignored) and write
# .cargo/vendor.toml, which points cargo at it. Afterwards the workspace builds with no network:
#   cargo build --release --offline --config .cargo/vendor.toml
# Re-run after Cargo.lock changes. Checked with `cargo check --offline` against the vendored tree.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
# A checkout has no `.cargo/` until something creates it, and the redirect below needs it to exist.
mkdir -p .cargo
cargo vendor --locked --versioned-dirs vendor > .cargo/vendor.toml.new
# cargo vendor prints the source-replacement stanza; keep only the TOML.
sed -n '/^\[source/,$p' .cargo/vendor.toml.new > .cargo/vendor.toml
rm -f .cargo/vendor.toml.new
if [ "${CHECK:-1}" = 1 ]; then
  CARGO_TARGET_DIR=target/vendor-check nice -n 10 cargo check --release --workspace --offline --config .cargo/vendor.toml
fi
echo "vendor/ ready ($(du -sh vendor | cut -f1)); build offline with: cargo build --release --offline --config .cargo/vendor.toml"
