#!/usr/bin/env bash
# PreToolUse (Bash): blocks plain force pushes, pushes naming main and ways to skip the pre-push hook.
# --force-with-lease to other branches is allowed. scripts/git-hooks/pre-push is the real guard for main.
# ponytail: line-based parsing stops accidental commands, not `bash -c "..."` or quoted env values.
command -v jq >/dev/null || { echo "guard-push: jq is required" >&2; exit 2; }
cmd=$(jq -r '.tool_input.command // empty')
git_cmd='^([A-Za-z_]+=[^ ]* +)*([^ ]*/)?git( |$)'
push_cmd='^([A-Za-z_]+=[^ ]* +)*([^ ]*/)?git( +-[Cc] +[^ ]+| +--[^ ]+)* +push( |$)'
block() { echo "guard-push: $1; blocked: $part" >&2; exit 2; }
has() { printf '%s\n' "$1" | grep -qiE -e "$2"; }
while IFS= read -r part; do
  part=$(printf '%s' "$part" | tr '\t' ' ' | sed -E 's/^( *([({]|(then|do|else|env|exec|xargs|command)( |$)))* *//')
  has "$part" "$git_cmd" || continue
  has "$part" ' (-c +|--config-env[= ])core\.hooksPath' && block "do not change core.hooksPath"
  if has "$part" ' config( |$)' && has "$part" ' core\.hooksPath( |$)' &&
    ! has "$part" ' (--get|--list|-l)( |$)' && ! has "$part" ' config core\.hooksPath scripts/git-hooks$'; then
    block "do not change core.hooksPath"
  fi
  has "$part" "$push_cmd" || continue
  args=" $(printf '%s' "${part#*push}" | tr -d "\"'") "
  has "$args" ' --no-verify ' && block "do not skip the pre-push hook"
  has "$args" ' (--force|--mirror|-[A-Za-z]*f[A-Za-z]*) ' && block "use --force-with-lease"
  has "$args" ' \+' && block "use --force-with-lease, not a + refspec"
  has "$args" ' \+?([^ ]*:)?(refs/heads/)?main ' && block "never push to main; merge through a PR"
done < <(printf '%s\n' "$cmd" | sed -E 's/(&&|\|\||;|\||&)/\n/g')
exit 0
