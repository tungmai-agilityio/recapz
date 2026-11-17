#!/usr/bin/env bash
# PreToolUse (Bash): blocks plain force pushes, pushes naming main and ways to skip the pre-push hook.
# --force-with-lease to other branches is allowed. scripts/git-hooks/pre-push is the real guard for main.
cmd=$(jq -r '.tool_input.command // empty')
git_cmd='^([A-Za-z_]+=[^ ]* +)*git( |$)'
push_cmd='^([A-Za-z_]+=[^ ]* +)*git( +-[Cc] +[^ ]+| +--[^ ]+)* +push( |$)'
block() { echo "guard-push: $1; blocked: $part" >&2; exit 2; }
has() { printf '%s\n' "$1" | grep -qE -e "$2"; }
while IFS= read -r part; do
  part="${part#"${part%%[![:space:]]*}"}"
  has "$part" "$git_cmd" || continue
  case "$part" in
    *hooksPath*)
      case "$part" in
        *--get* | "git config core.hooksPath scripts/git-hooks") ;;
        *) block "do not change core.hooksPath" ;;
      esac ;;
  esac
  has "$part" "$push_cmd" || continue
  args=" ${part#*push} "
  has "$args" ' --no-verify ' && block "do not skip the pre-push hook"
  has "$args" ' (--force|--mirror|-[A-Za-z]*f[A-Za-z]*) ' && block "use --force-with-lease"
  has "$args" ' \+' && block "use --force-with-lease, not a + refspec"
  has "$args" ' \+?([^ ]*:)?(refs/heads/)?main ' && block "never push to main; merge through a PR"
done < <(printf '%s\n' "$cmd" | sed -E 's/(&&|\|\||;|\|)/\n/g')
exit 0
