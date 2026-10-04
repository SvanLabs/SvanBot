# Source with $repo set. The wrappers (release.sh, update.sh, rollback.sh) exec the Rust installer, and a
# fixture checkout has no Rust sources, so the suites hand them a built one: the gate profile, the build
# the workspace tests share, unless the caller (check.sh, a developer) already set SV10_RELEASE_BIN.
if [ -z "${SV10_RELEASE_BIN:-}" ]; then
  release_target=${CARGO_TARGET_DIR:-$repo/target/dev}
  case $release_target in /*) ;; *) release_target=$repo/$release_target ;; esac
  (cd "$repo" && CARGO_TARGET_DIR="$release_target" cargo build --profile gate -p sv10-release -q) || {
    echo "could not build sv10-release for the wrapper tests" >&2
    exit 1
  }
  export SV10_RELEASE_BIN="$release_target/gate/sv10-release"
fi
