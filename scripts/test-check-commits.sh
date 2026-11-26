#!/bin/sh
# Exact accept/reject cases for check-commits.sh title and branch rules. Run: ./scripts/test-check-commits.sh
cd "$(dirname "$0")/.." || exit 1
fail=0
t() { # expected-exit title branch
  ./scripts/check-commits.sh HEAD..HEAD "$2" "$3" >/dev/null 2>&1; got=$?
  [ "$got" -eq "$1" ] || { echo "FAIL (want $1, got $got): title='$2' branch='$3'"; fail=1; }
}
t 0 'feat(web): add a thing' 'feat/add-a-thing'
t 0 'T-042: feat(desktop): add a thing' 'feat/T-042-add-a-thing'
t 0 'T-1234: fix(core): handle empty input' 'fix/T-1234-handle-empty-input'
t 0 'docs: update deploy status' 'docs/update-deploy-status'
t 1 'T-042: add a thing' 'feat/T-042-add-a-thing'
t 1 'T-42: feat(web): add a thing' 'feat/add-a-thing'
t 1 'T-042 feat(web): add a thing' 'feat/add-a-thing'
t 1 'T-042: feat(web): add a thing.' 'feat/add-a-thing'
t 1 'feat(web): add a thing' 'feat/T-42-add-a-thing'
t 1 'feat(web): add a thing' 'T-042-add-a-thing'
t 1 'T-042: feat(web): a title that is too long to fit in seventy two characters' 'feat/a'
echo "check-commits tests: $([ $fail -eq 0 ] && echo pass || echo FAIL)"
exit $fail
