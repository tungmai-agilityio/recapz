#!/usr/bin/env bash
# PreToolUse (test-runner only): allow test, check and log-reading commands; exit 2 blocks the rest.
cmd=$(jq -r '.tool_input.command // empty')
case "$cmd" in *'$('* | *'`'* | *'<('* | *'>('*) echo "test-runner: no command or process substitution" >&2; exit 2 ;; esac
case "$cmd" in *--output*) echo "test-runner: no --output (writes files)" >&2; exit 2 ;; esac
while IFS= read -r part; do
  part="${part#"${part%%[![:space:]]*}"}"
  [ -z "$part" ] && continue
  # `git -C <dir>` is allowed only for the same read-only subcommands as plain git below.
  [[ "$part" =~ ^git\ -C\ [^\ ]+\ (rev-parse|status|diff)(\ |$) ]] && continue
  case "$part" in
    "make test"* | "make lint"* | "cargo test"* | "cargo clippy"* | "cargo fmt --check"* | \
    "pnpm -C web test"* | "pnpm -C web lint"* | "pnpm -C web exec tsc --noEmit"* | "pnpm -C web exec playwright"* | \
    "cd "* | "ls"* | "tail "* | "head "* | "grep "* | "wc "* | "mkdir -p "* | \
    "git status"* | "git diff"* | "git rev-parse"*) ;;
    *) echo "test-runner may only run tests, lint or read logs; blocked: $part" >&2; exit 2 ;;
  esac
done < <(printf '%s\n' "$cmd" | sed -E -e 's/[0-9]*>&[0-9-]*//g' -e 's/&>>?/>/g' -e 's/(&&|\|\||;|\||&)/\n/g')
exit 0
