#!/usr/bin/env bash
# Offline checks for scripts/deploy-recv.sh: what the CI deploy key may and may not ask for.
cd "$(dirname "$0")/.." || exit 1
d=$(mktemp -d); trap 'rm -rf "$d"' EXIT
fail=0
# stand-in for deploy-release.sh: records its arguments and a copy of the binary
cat >"$d/rel" <<'S'
#!/usr/bin/env bash
if [ "$1" = --rollback ]; then echo --rollback >>"$D/calls"; else echo "$(basename "$1") $2" >>"$D/calls"; cp "$1" "$D/got"; fi
S
chmod +x "$d/rel"
printf '\177ELFpayload' >"$d/elf"; printf 'not elf' >"$d/txt"
good=$(shasum -a 256 "$d/elf" | cut -d' ' -f1)
run() { # command stdin-file
  : >"$d/calls"; rm -f "$d/got"
  SSH_ORIGINAL_COMMAND="$1" D="$d" RECAPZ_DEPLOY_RELEASE="$d/rel" RECAPZ_LINK="$d/link" ./scripts/deploy-recv.sh <"${2:-/dev/null}" >/dev/null 2>&1
}
ok() { local w=$1; shift; run "$@" || { echo "want success: $w"; fail=1; }; }
no() { local w=$1; shift; run "$@" && { echo "want refusal: $w"; fail=1; }; [ ! -s "$d/calls" ] || { echo "refused but ran the deploy: $w"; fail=1; }; }

ok "deploy" "deploy v0.4.0 $good" "$d/elf"
[ "$(cat "$d/calls")" = "recapz v0.4.0" ] || { echo "deploy args: $(cat "$d/calls")"; fail=1; }
cmp -s "$d/elf" "$d/got" || { echo "binary not passed through"; fail=1; }
ok "rollback" "rollback"
[ "$(cat "$d/calls")" = "--rollback" ] || { echo "rollback args: $(cat "$d/calls")"; fail=1; }
ln -s /opt/x/recapz "$d/link"; ok "status" "status"
no "wrong sha" "deploy v1 $(printf 0%.0s $(seq 64))" "$d/elf"
no "not an ELF" "deploy v1 $(shasum -a 256 "$d/txt" | cut -d' ' -f1)" "$d/txt"
no "empty binary" "deploy v1 $good"
no "path in name" "deploy ../x $good" "$d/elf"
no "dot name" "deploy .x $good" "$d/elf"
no "extra word" "deploy v1 $good rm" "$d/elf"
no "short sha" "deploy v1 abc" "$d/elf"
no "rollback with args" "rollback now"
no "shell" "bash -c id"
no "newline" $'status\nrollback'
no "empty command" ""
[ $fail = 0 ] && echo "deploy-recv: all checks passed"; exit $fail
