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
# The install tree is `~/.local/lib/svanbot` unless given, which is where `deploy/svanbot@.service`
# execs `current/svanbot` from. Run `svanbot-release status` to read what is installed and running.
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

# The staged release the updater verifies: the program, its checksum, and the manifest naming the
# version it is. Read by `svanbot_runtime::release`, installed by `svanbot-release install`.
staged="$tree/staged"
rm -rf "$staged"
mv "$work/release" "$staged"
chmod +x "$staged/svanbot"

echo "install.sh: installed $bindir/svanbot-release"
echo "install.sh: staged $(awk -F'\t' '$1 == "version" { print $2 }' "$staged/version") under $staged"
echo "install.sh: now run \`svanbot-release install $staged\`, then one bot at a time"
echo "install.sh:           \`svanbot-release update\` (SVANBOT_CANARY first, then --fleet)"
