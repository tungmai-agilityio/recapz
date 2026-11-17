"""PreToolUse (test-runner only): strict allowlist of exact command shapes. Exit 2 blocks.

The command is tokenised with shlex (so quotes are respected and operators are separate tokens), split on
&& || ; |, and every command must match one allowed shape. The only redirections allowed are
`> /tmp/recapz-tests/<dir>/<name>.log` and `2>&1`. Anything else (background &, <, >>, process substitution,
other redirect targets, unlisted commands or options) is blocked.
"""
import json
import re
import shlex
import sys

PATH_RE = re.compile(r"^[A-Za-z0-9_./-]+$")
LOG = re.compile(r"^/tmp/recapz-tests/[A-Za-z0-9_][A-Za-z0-9._-]*/[A-Za-z0-9_][A-Za-z0-9._-]*\.log$")
CRATE = re.compile(r"^[A-Za-z0-9_-]+$")
FILTER = re.compile(r"^[A-Za-z0-9_:]+$")
NUM = re.compile(r"^-?[0-9]+$")
GIT = [["status"], ["status", "--porcelain"], ["rev-parse", "--short", "HEAD"], ["diff", "--stat"], ["diff", "--name-only"]]


class PATH:
    """Plain path characters and no `.` or `..` component."""

    @staticmethod
    def match(t):
        return bool(PATH_RE.match(t)) and not {".", ".."} & set(t.split("/"))


def unquote(t):
    return t[1:-1] if len(t) >= 2 and t[0] == t[-1] and t[0] in "'\"" else t


def cargo_test(a):
    i = 2
    while i < len(a) and a[i] != "--":
        if a[i] == "--workspace":
            i += 1
        elif a[i] == "-p" and i + 1 < len(a) and CRATE.match(a[i + 1]):
            i += 2
        else:
            return False
    return all(t == "--nocapture" or FILTER.match(t) for t in a[i + 1:])


def allowed(a):
    if not a:
        return True
    if a[:1] == ["make"]:
        return a[1:] in (["test"], ["lint"])
    if a[:2] == ["cargo", "test"]:
        return cargo_test(a)
    if a == ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]:
        return True
    if a == ["cargo", "fmt", "--check"]:
        return True
    if a[:3] == ["pnpm", "-C", "web"]:
        rest = a[3:]
        if rest in (["test"], ["lint"], ["exec", "tsc", "--noEmit"]):
            return True
        return rest[:3] == ["exec", "playwright", "test"] and all(PATH.match(t) for t in rest[3:])
    if a[0] == "cd":
        return len(a) == 2 and bool(PATH.match(a[1]))
    if a[0] == "ls":
        return all(t == "-la" or PATH.match(t) for t in a[1:])
    if a[0] in ("head", "tail"):
        rest = a[1:]
        if rest[:1] == ["-n"]:
            rest = rest[1:]
            if not rest or not NUM.match(rest[0]):
                return False
            rest = rest[1:]
        elif rest and NUM.match(rest[0]):
            rest = rest[1:]
        return len(rest) == 1 and bool(PATH.match(rest[0]))
    if a[:2] == ["wc", "-l"]:
        return len(a) == 3 and bool(PATH.match(a[2]))
    if a[:2] == ["mkdir", "-p"]:
        return len(a) == 3 and a[2].startswith("/tmp/recapz-tests/") and PATH.match(a[2])
    if a[0] == "grep":
        rest = a[1:]
        while rest and re.match(r"^-[A-Za-z]+$", rest[0]):
            rest = rest[1:]
        return len(rest) >= 2 and not unquote(rest[0]).startswith("-") and all(PATH.match(t) for t in rest[1:])
    if a[0] == "git":
        rest = a[1:]
        if rest[:1] == ["-C"]:
            if len(rest) < 2 or not PATH.match(rest[1]):
                return False
            rest = rest[2:]
        return rest in GIT
    return False


def check(cmd):
    """Return None when the command is allowed, else the reason it is blocked."""
    if not isinstance(cmd, str) or not cmd.strip():
        return "empty or non-text command"
    if re.search(r"[\n\r\\$`]", cmd):
        return "no newlines, backslashes, variables or command substitution"
    # The only fd redirect allowed is a literal ` 2>&1` as its own word; any other `>&` stays and is blocked below.
    cmd = re.sub(r"(?<=\s)2>&1(?=\s|;|$)", " ", cmd)
    try:
        # posix=False keeps quotes on words, so a quoted '&&' is a plain word, not an operator, as in the shell.
        lex = shlex.shlex(cmd, posix=False, punctuation_chars=True)
        lex.whitespace_split = True
        toks = list(lex)
    except ValueError as e:
        return f"cannot parse: {e}"
    cur, i = [], 0
    while i <= len(toks):
        t = toks[i] if i < len(toks) else ";"
        if t in ("&&", "||", ";", "|"):
            if not allowed(cur):
                return "not an allowed command: " + " ".join(cur)
            cur = []
        elif t == ">":
            if i + 1 >= len(toks) or not LOG.match(toks[i + 1]):
                return "redirect only to /tmp/recapz-tests/<dir>/<name>.log"
            i += 1
        elif re.fullmatch(r"[&|<>()]+", t):
            return f"operator not allowed: {t}"
        else:
            cur.append(t)
        i += 1
    return None


def main():
    try:
        reason = check(json.load(sys.stdin).get("tool_input", {}).get("command"))
    except BaseException as e:  # fail closed: any other exit code lets the command run
        reason = f"hook error ({type(e).__name__})"
    if reason:
        print(f"test-runner may only run tests, lint or read logs; {reason}", file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
