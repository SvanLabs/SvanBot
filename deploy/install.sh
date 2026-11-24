#!/bin/sh
# Install a release bundle on a box with no Rust toolchain, and point the fleet at it.
#
# The fleet never builds: it runs the program CI built, verified and checksummed. This script is
# the first half of that (V11-10) — it stages a bundle, verifies it, and promotes it into the tree
# — and the second half is `svanbot-release update`, which swaps `current` between hands, canary
# first. This script never touches `current`: a release can be staged long before a window is
# found, and staging is not shipping.
#
#   deploy/install.sh <bundle.tar.gz> [install tree]
#
# The install tree is `~/.local/lib/svanbot` unless given, which is where the supervisor
# (`svanbot-bot`, `scripts/bot.sh`) execs `current/svanbot` from. Run `svanbot-release status` to read what is installed and running.
set -eu

usage() {
  echo "usage: install.sh <bundle.tar.gz> [install tree]" >&2
  exit 2
}

[ $# -ge 1 ] || usage
[ -f "$1" ] || { echo "install.sh: no bundle at $1" >&2; exit 1; }

bundle=$1
tree=${2:-"${HOME:?HOME is not set}/.local/lib/svanbot"}

# One bundle names its own version and instruction level, because both are facts a caller would
# only restate. `tar` and `sha256sum` are the tools every machine here already has; the verifier
# inside `svanbot-release` re-checks every byte after this, and agrees or refuses.
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM
tar -xzf "$bundle" -C "$work"

# The bundle publishes the archive's checksum beside it. It is checked when the publisher shipped
# one; a bundle copied by hand without it is staged anyway, and `svanbot-release install` still
# verifies the program against the checksum inside the release.
if [ -f "$bundle.sha256" ]; then
  ( cd "$(dirname -- "$bundle")" && sha256sum -c "$(basename -- "$bundle").sha256" ) >/dev/null
  echo "install.sh: $(basename -- "$bundle") matches its published checksum"
fi

[ -d "$work/release" ] || { echo "install.sh: the bundle has no release/ to stage" >&2; exit 1; }

# The instruction level the program was built for, against the one this CPU has. A program built
# above its CPU dies the first time it reaches an instruction the CPU lacks, so it is refused here,
# before anything is staged. A bundle that does not name its level was built for the x86-64
# baseline, which every 64-bit CPU has. `SVANBOT_CPUINFO` stands in for /proc/cpuinfo in a test.
level_of() {
  case "$1" in
    x86-64) echo 1 ;;
    x86-64-v2) echo 2 ;;
    x86-64-v3) echo 3 ;;
    x86-64-v4) echo 4 ;;
    *) echo 0 ;;
  esac
}
flags=" $(grep -m1 '^flags' "${SVANBOT_CPUINFO:-/proc/cpuinfo}" | cut -d: -f2) "
has_all() {
  for flag in "$@"; do
    case "$flags" in *" $flag "*) ;; *) return 1 ;; esac
  done
}
cpu_level=1
if has_all cx16 lahf_lm popcnt sse4_1 sse4_2 ssse3; then
  cpu_level=2
  if has_all avx avx2 bmi1 bmi2 f16c fma abm movbe xsave; then
    cpu_level=3
    if has_all avx512f avx512bw avx512cd avx512dq avx512vl; then cpu_level=4; fi
  fi
fi
built_for=x86-64
[ -f "$work/UPGRADE/cpu" ] && built_for=$(head -n1 "$work/UPGRADE/cpu")
built_level=$(level_of "$built_for")
[ "$built_level" -gt 0 ] || {
  echo "install.sh: the bundle was built for $built_for, a level this installer does not know" >&2
  exit 1
}
[ "$built_level" -le "$cpu_level" ] || {
  machine=x86-64
  [ "$cpu_level" -gt 1 ] && machine="x86-64-v$cpu_level"
  echo "install.sh: the bundle was built for $built_for, and this CPU is $machine: refused" >&2
  exit 1
}
[ -f "$work/UPGRADE/svanbot-release" ] || {
  echo "install.sh: the bundle has no svanbot-release to install" >&2
  exit 1
}
mkdir -p "$tree"

# The operator's tool, installed once beside the tree and never swapped: it is what does the
# swapping. `~/.local/bin` is on the PATH the runbook uses.
bindir="${HOME:?HOME is not set}/.local/bin"
mkdir -p "$bindir"
install -m 755 "$work/UPGRADE/svanbot-release" "$bindir/svanbot-release"
# The supervisor the updater stops and starts bots through. A bundle from before V11-52 has none.
if [ -f "$work/UPGRADE/svanbot-bot" ]; then
  install -m 755 "$work/UPGRADE/svanbot-bot" "$bindir/svanbot-bot"
fi

# The staged release the updater verifies: the program, its checksum, and the manifest naming the
# version it is. Read by `svanbot_runtime::release`, installed by `svanbot-release install`.
staged="$tree/staged"
rm -rf "$staged"
mv "$work/release" "$staged"
chmod +x "$staged/svanbot"

echo "install.sh: installed $bindir/svanbot-release"
[ -f "$bindir/svanbot-bot" ] && echo "install.sh: installed $bindir/svanbot-bot"
echo "install.sh: staged $(awk -F'\t' '$1 == "version" { print $2 }' "$staged/version") under $staged"
echo "install.sh: now run \`svanbot-release install $staged\`, then one bot at a time"
echo "install.sh:           \`svanbot-release update\` (SVANBOT_CANARY first, then --fleet)"
