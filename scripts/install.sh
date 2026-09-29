#!/usr/bin/env bash
# Install prebuilt binaries from a portable bundle (scripts/portable.sh) into target/release,
# choosing x86-64-v3 when the CPU supports it and x86-64-v2 otherwise. Run from the unpacked bundle
# (or a checkout that has target/dist-v2 and target/dist-v3 builds). Then: edit .env, scripts/start.sh.
#   scripts/install.sh            install the best level for this CPU
#   LEVEL=v2 scripts/install.sh   force a level
set -euo pipefail
cd "$(dirname "$0")/.."

cpu_level() {
  local flags
  flags=" $(grep -m1 '^flags' /proc/cpuinfo | cut -d: -f2) "
  local f
  for f in avx avx2 bmi1 bmi2 f16c fma abm movbe xsave; do
    [[ "$flags" == *" $f "* ]] || { v2_or_none "$flags"; return; }
  done
  echo v3
}
v2_or_none() {
  local f
  for f in cx16 lahf_lm popcnt sse4_1 sse4_2 ssse3; do
    [[ "$1" == *" $f "* ]] || { echo none; return; }
  done
  echo v2
}

[ "$(uname -m)" = x86_64 ] || { echo "Prebuilt bundles are x86-64 only; build from source with scripts/setup.sh." >&2; exit 1; }
level="${LEVEL:-$(cpu_level)}"
[ "$level" != none ] || { echo "CPU lacks x86-64-v2 (SSE4.2/POPCNT); build from source with scripts/setup.sh." >&2; exit 1; }

if [ -d "bin/x86-64-$level" ]; then
  src="bin/x86-64-$level"
elif [ -d "target/dist-$level/release" ]; then
  src="target/dist-$level/release"
else
  echo "No x86-64-$level binaries here (run scripts/portable.sh first)." >&2
  exit 1
fi

if [ -f MANIFEST ]; then
  need=$(grep '^glibc=' MANIFEST | cut -d= -f2)
  have=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')
  have=${have:-0}
  if [ -n "$need" ] && [ "$(printf '%s\n%s\n' "$need" "$have" | sort -V | head -1)" != "$need" ]; then
    echo "This bundle needs glibc >= $need (found $have); build from source with scripts/setup.sh." >&2
    exit 1
  fi
fi

mkdir -p target/release artifacts/logs artifacts/backups
for b in sv10-bot learner analyst sim probe tables review calibrate ingest archive; do
  [ -x "$src/$b" ] || continue
  # Copy then rename, like scripts/release.sh: running processes keep their inode and hot-swap.
  cp "$src/$b" "target/release/.$b.new" && mv -f "target/release/.$b.new" "target/release/$b"
done
./target/release/sv10-bot --version
echo "Installed x86-64-$level binaries into target/release."

if [ ! -f .env ] && [ -f .env.example ]; then
  cp .env.example .env && chmod 600 .env
  echo "Created .env from .env.example: add your openpoker.ai API key(s), then run scripts/start.sh."
fi
echo "Hardware profile:"; ./target/release/probe --hardware 2>/dev/null || true
