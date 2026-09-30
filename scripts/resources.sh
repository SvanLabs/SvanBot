#!/usr/bin/env bash
# Source from the checkout root before building. Explicit operator job limits take precedence.
if [ -z "${CARGO_BUILD_JOBS:-}" ]; then
  resource_jobs=$(python3 scripts/host_resources.py --jobs) || return 1
  # The gate/release overlap two compiler stages; share the budget between them.
  resource_stages=${SVANBOT_BUILD_STAGES:-1}
  export CARGO_BUILD_JOBS=$((resource_jobs / resource_stages > 0 ? resource_jobs / resource_stages : 1))
fi
python3 scripts/host_resources.py --check-disk "${CARGO_TARGET_DIR:-target/dev}" artifacts
