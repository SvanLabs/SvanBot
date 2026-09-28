#!/usr/bin/env bash
# First-run setup for a new machine: checks toolchains, creates .env, builds, tests and installs
# a release natively for this CPU, and prints the hardware profile the bot will tune itself to.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

say() { printf '\n==> %s\n' "$*"; }

say "Checking toolchains"
if ! command -v cargo >/dev/null; then
  echo "Rust is not installed. Install it with:"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  exit 1
fi
cargo --version
if ! command -v npm >/dev/null; then
  echo "Node.js/npm is not installed (needed only to build the dashboard). Install Node 20+ and rerun."
  exit 1
fi
node --version
command -v sqlite3 >/dev/null || echo "(optional) sqlite3 CLI not found; scripts/status.sh uses it for totals."

say "Configuration"
if [ ! -f .env ]; then
  cp .env.example .env
  chmod 600 .env
  TOKEN=$(head -c 24 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32)
  sed -i "s/^SVANBOT_WEB__OPERATOR_TOKEN=.*/SVANBOT_WEB__OPERATOR_TOKEN=${TOKEN}/" .env
  echo "Created .env with a random dashboard password. Add your openpoker.ai API key(s) and bot name(s):"
  echo "  $(pwd)/.env"
else
  echo ".env already exists; leaving it untouched."
fi
mkdir -p artifacts/logs artifacts/backups

say "Dashboard dependencies"
(cd web && { [ -d node_modules ] || npm ci; })

# Only release.sh writes target/release and web/dist (LESSONS 31): a build made here had no commit
# identity, so the first release after setup refused to overwrite it (#15).
say "Building, testing and installing (optimized for this CPU via .cargo/config.toml target-cpu=native; scripts/portable.sh builds portable per-CPU-level bundles for other machines)"
scripts/release.sh

say "Systemd units (start on boot)"
scripts/units.sh --setup

say "Hardware profile (the bot applies this automatically at every start)"
./target/release/probe --hardware

if grep -q "^SVANBOT_API_KEY=$" .env; then
  say "Next: put your API key in .env, then run scripts/start.sh"
else
  say "Ready. Start with scripts/start.sh; dashboard on the host/port set in .env"
fi
