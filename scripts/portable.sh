#!/usr/bin/env bash
# Portable release bundle for x86-64 Linux machines without a Rust toolchain (0064): the workspace
# binaries built for baseline x86-64-v1, x86-64-v2 (SSE4.2/POPCNT, any x86-64 CPU since ~2009) and x86-64-v3
# (AVX2/BMI2/FMA, Haswell and later), plus scripts, the built dashboard and docs. On the target,
# scripts/install.sh picks the level from the CPU flags and installs into target/release.
#   scripts/portable.sh        -> target/dist/svanbot-<version>-<commit>-x86_64-linux-gnu.tar.gz
# The fleet on this box keeps its native build (target-cpu=native); v3 benchmarks within ~1% of it
# and every level reproduces the paired-sim reference exactly.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
source scripts/resources.sh

version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
commit=$(git rev-parse --short HEAD)
name="svanbot-$version-$commit-x86_64-linux-gnu"
out="target/dist/$name"
rm -rf "$out" && mkdir -p "$out/bin"
for level in v1 v2 v3; do
  echo "== building x86-64-$level"
  # RUSTFLAGS sets the instruction set for this build, overriding the workspace default.
  cpu="x86-64-$level"
  [ "$level" != v1 ] || cpu=x86-64
  RUSTFLAGS="-C target-cpu=$cpu" nice -n 10 cargo build --release --workspace --bins --target-dir "target/dist-$level"
  mkdir -p "$out/bin/x86-64-$level"
  for b in sv10-bot learner analyst sim probe tables review monitor calibrate ingest archive; do
    cp "target/dist-$level/release/$b" "$out/bin/x86-64-$level/"
  done
  # Debug info stays in target/dist-*; the bundle ships stripped binaries.
  strip "$out/bin/x86-64-$level"/*
done
echo "== dashboard"
(cd web && npm run build >/dev/null)
mkdir -p "$out/web"
cp -r web/dist "$out/web/"
cp -r scripts docs .env.example AGENTS.md CLAUDE.md "$out/"
glibc=$(objdump -T "$out"/bin/x86-64-v1/* | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1 | cut -d_ -f2)
printf 'version=%s\ncommit=%s\nglibc=%s\nbuilt=%s\n' "$version" "$commit" "$glibc" "$(date -u +%FT%TZ)" > "$out/MANIFEST"
tar -C target/dist -czf "target/dist/$name.tar.gz" "$name"
echo "Bundle: target/dist/$name.tar.gz (needs glibc >= $glibc)"
