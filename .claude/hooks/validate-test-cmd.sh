#!/usr/bin/env bash
# PreToolUse (test-runner only): allow only exact test, check and log-reading commands; exit 2 blocks the rest.
# The allowlist lives in validate_test_cmd.py; python3 -I ignores the working directory and environment.
exec python3 -I "$(dirname "$0")/validate_test_cmd.py"
