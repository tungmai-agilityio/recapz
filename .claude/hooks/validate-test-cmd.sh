#!/usr/bin/env bash
# PreToolUse (test-runner only): allow only exact test, check and log-reading commands; exit 2 blocks the rest.
# The allowlist lives in validate_test_cmd.py. Any exit code other than 2 lets the command run, so every failure
# here (python3 missing, a crash) must exit 2. python3 -I ignores the working directory and environment.
command -v python3 >/dev/null || { echo "test-runner hook: python3 is required" >&2; exit 2; }
python3 -I -B "${BASH_SOURCE[0]%/*}/validate_test_cmd.py" && exit 0
exit 2
