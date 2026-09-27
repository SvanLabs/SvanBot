#!/usr/bin/env bash
# Git pre-commit hook: the commit gate. Install: ln -sf ../../scripts/pre-commit.sh .git/hooks/pre-commit
exec "$(git rev-parse --show-toplevel)/scripts/check.sh" commit
