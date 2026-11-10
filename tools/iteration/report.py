#!/usr/bin/env python3
"""How fast work on The One App goes from started to done, and where the time goes, for the last day.

Reads this machine's Claude Code sessions in the repo and its worktrees, Agent Memory and git, and writes a report
to ~/one-export/iteration/<date>.md. Sessions on other machines and other harnesses aren't in it, and the report says
so; Agent Memory and git are everyone's. Memories have a date and no time, so it reads whole days.

    python3 -I tools/iteration/report.py [--days 1]
"""

import argparse, datetime as dt, glob, json, os, re, subprocess, sys
from collections import Counter, defaultdict

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SESSIONS = os.path.expanduser("~/.claude/projects/*the-one-app*/*.jsonl")
OUT = os.path.expanduser("~/one-export/iteration")

EDITS = ("Edit", "Write", "MultiEdit", "NotebookEdit")
LOOP = "sleep or until loop"
PLUMBING = set("""cd echo printf tail head grep rg sed awk cut sort uniq tr wc cat ls find tee xargs jq true false test exit date pwd
    export set unset mkdir rm cp mv touch chmod ln stat basename dirname which command source read hostname fi done then else for if
    while do case esac bash sh python python3 pgrep pkill kill ps lsof df du file readlink diff nl sha256sum wait""".split())
WRAPPERS = ("nice", "time", "timeout", "nohup", "env", "sudo", "ionice", "exec", "stdbuf")
SUBCOMMANDS = ("cargo", "git", "go", "bun", "one", "one-dev", "ssh", "gh")
CHANGES_THE_WAIT = ("--until", "--release")
QUICK = re.compile(r"git \S+|gh \S+|one (memory|board|status|version|logs|agent-devices)|one-dev (signoff|changes|unfinished|status)|ssh \S+")


def when(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(len(xs) * p))] if xs else None


def one(*args):
    """A one command's output. If it fails the report does too, rather than counting nothing as zero."""
    r = subprocess.run(["one", *args], capture_output=True, text=True, stdin=subprocess.DEVNULL)
    if r.returncode != 0:
        sys.exit(f"one {' '.join(args)} failed: {r.stderr.strip()[:300]}")
    return r.stdout


def label(words):
    """A program's name with the words naming the job, from one command in a shell line."""
    prog = {"ptc": "one", "ptc-dev": "one-dev"}.get(os.path.basename(words[0]), os.path.basename(words[0]))
    if prog == "cargo-test-when-quiet":  # ~/.local/bin's cargo test, once the machine is quiet
        return "cargo test"
    if words[1:2] == ["-C"]:
        del words[1:3]
    args = [w for w in words[1:] if not w.startswith("-") and "=" not in w]
    take = 3 if args[:2] == ["throwaway", "client"] else 2 if args[:1] == ["throwaway"] else 1
    return " ".join([prog, *(args[:take] if prog in SUBCOMMANDS else []), *(f for f in CHANGES_THE_WAIT if f in words)])


def kind(c):
    """What a shell command was for: its last real job, or its last program if all were quick. None if all plumbing."""
    c = re.sub(r"<<-?\s*['\"]?(\w+)['\"]?.*?^\s*\1$", "", c, flags=re.S | re.M)
    c = re.sub(r"'[^']*'|\"[^\"]*\"|\$\([^)]*\)|`[^`]*`", "''", c)
    if re.search(r"\bsleep \d|\buntil\b.*\bdo\b", c, re.S):
        return LOOP
    found = []
    for part in re.split(r"&&|\|\||;|\||\n|&(?=\s)", c):
        words = re.sub(r"\d?>&?\s*\S+|<\s*\S+", " ", part).strip(" (){}").split()
        while words and (re.match(r"\w+=", words[0]) or words[0] in WRAPPERS or re.match(r"-|\d+\w?$", words[0])):
            words.pop(0)
        if words and re.fullmatch(r"[\w./~-]+", words[0]) and os.path.basename(words[0]) not in PLUMBING:
            found.append(label(words))
    jobs = [k for k in found if not QUICK.fullmatch(k)]
    return (jobs or found or [None])[-1]


# What a tool call was for, as time is split.
def phase(name, inp):
    if name in ("Read", "Grep", "Glob") + EDITS:
        return "reading and editing code"
    if "computer_" in name or "agent-devices_" in name or "preview_" in name:
        return "checking it works"
    if name != "Bash":
        return "other"
    k = kind(inp.get("command", ""))
    if k is None or re.match(r"git (log|show|diff|status|blame)", k):
        return "reading and editing code"
    if k == LOOP:
        return "sleep or until loops"
    if re.match(r"cargo|xcodebuild|gradle|go (test|build)|build\.sh|bun", k):
        return "building and testing"
    if re.match(r"git (push|pull|fetch|rebase|commit)|one(-dev)? (update|deploy|release|ship|signoff|publish|build|retire)|cloudflare", k):
        return "shipping"
    if re.match(r"one(-dev)? (try|throwaway|status|version|logs|agent-devices|computer)|ssh|curl|xcrun|idb|adb|screencapture", k):
        return "checking it works"
    return "other"


def sessions(since):
    split, commands, started, stuck = Counter(), defaultdict(list), [], []
    theirs = cut = n = 0
    for f in glob.glob(SESSIONS):
        if os.path.getmtime(f) < since.timestamp():
            continue
        uses, background, last = {}, {}, None
        whole, first, edit, push, context, calls = True, None, None, None, None, 0
        for line in open(f, errors="replace"):
            try:
                e = json.loads(line)
            except ValueError:
                continue
            if e.get("isSidechain") or not e.get("timestamp"):
                continue
            t = when(e["timestamp"])
            if t < since:
                whole = False
                continue
            done = re.search(r"<tool-use-id>([^<]+)</tool-use-id>.*?<status>(\w+)</status>", line)
            if done and done[1] in background:
                t0, k = background.pop(done[1])
                if done[2] in ("completed", "failed"):
                    commands[k].append((t - t0).total_seconds())
            message = e.get("message") or {}
            if e.get("type") == "assistant" and context is None and message.get("usage"):
                u = message["usage"]
                context = u.get("input_tokens", 0) + u.get("cache_read_input_tokens", 0) + u.get("cache_creation_input_tokens", 0)
            content = message.get("content")
            if e.get("type") == "user" and (isinstance(content, str) or any(isinstance(c, dict) and c.get("type") == "text" for c in content or [])):
                last = None  # a user's message: the time before it was theirs
                continue
            if not isinstance(content, list):
                continue
            for c in content:
                if not isinstance(c, dict):
                    continue
                if c.get("type") == "tool_use":
                    if c["name"] == "StructuredOutput":  # T3 Code asking for a branch name: no work was done
                        continue
                    if first is None:
                        first, n = t, n + 1
                    if last and (t - last).total_seconds() < 1800:
                        split["the model thinking and writing"] += (t - last).total_seconds()
                    last = None  # calls sent together follow the same thinking
                    calls += edit is None
                    if edit is None and c["name"] in EDITS:
                        edit = t
                    if edit and push is None and c["name"] == "Bash" and re.search(r"\bgit push\b", c["input"].get("command", "")):
                        push = t
                    uses[c["id"]] = (t, c["name"], c.get("input", {}))
                elif c.get("type") == "tool_result" and c.get("tool_use_id") in uses:
                    t0, name, inp = uses.pop(c["tool_use_id"])
                    d = (t - t0).total_seconds()
                    last = t
                    # a call the user had to answer or approve runs on their clock, not an agent's
                    if name == "AskUserQuestion" or (e.get("permissionDecision") or {}).get("source", "").startswith("user"):
                        theirs += d
                    elif e.get("toolDenialKind") == "interrupted":  # the session ended: its result is written when it resumes
                        cut += d
                    elif d > 3600:
                        stuck.append((d, name, inp.get("command") or json.dumps(inp)))
                    else:
                        split[phase(name, inp)] += d
                        k = kind(inp["command"]) if name == "Bash" else None
                        if k and inp.get("run_in_background"):
                            background[c["tool_use_id"]] = (t0, k)
                        elif k:
                            commands[k].append(d)
        if whole and edit:
            started.append(((edit - first).total_seconds(), calls, (push - edit).total_seconds() if push else None, context or 0))
    return dict(n=n, split=split, commands=commands, started=started, stuck=stuck, theirs=theirs, cut=cut)


MEMORY = re.compile(r"#(\d+) (\d{4}-\d\d-\d\d) (.*)$")


def memories(since):
    """Memories since since's day that correct an earlier report or say Friction, oldest first: (id, date, text).
    Asked a day at a time, as recall gives only the newest matches that fit."""
    out, day = [], since.date()
    while day <= dt.datetime.now(dt.timezone.utc).date():
        text = one("memory", "recall", f"^#\\d+ {day:%Y-%m-%d} (correcting|.*friction)")
        if "Narrow the regex" in text:
            sys.exit(f"Agent Memory has more matches for {day} than recall gives: the report would undercount")
        out += [(int(m.group(1)), m.group(2), m.group(3)) for m in map(MEMORY.match, text.splitlines()) if m]
        day += dt.timedelta(days=1)
    return out


def commits(since):
    return subprocess.run(["git", "-C", REPO, "rev-list", "--count", f"--since={since:%Y-%m-%dT%H:%M:%SZ}", "HEAD"], capture_output=True, text=True).stdout.strip()


def hours(s):
    return f"{s / 3600:.1f}h"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--days", type=float, default=1)
    a = ap.parse_args()
    now = dt.datetime.now(dt.timezone.utc)
    since = now - dt.timedelta(days=a.days)

    s = sessions(since)
    posts = memories(since)
    corrected = [p for p in posts if p[2].lower().startswith("correcting")]
    friction = [p for p in posts if re.search(r"\bFriction\b[^:.]{0,12}:", p[2])]
    total = sum(s["split"].values()) or 1

    lines = [f"# Iteration, {since:%Y-%m-%d %H:%M} to {now:%Y-%m-%d %H:%M} UTC: {commits(since)} commits to main", "",
             f"Times and commands are Claude Code sessions on {os.uname().nodename} only: other machines and harnesses aren't in them.", ""]
    lines += [f"## Where agents' time went ({s['n']} sessions, {hours(total)})", ""]
    lines += [f"- {k}: {hours(v)} ({100 * v / total:.0f}%)" for k, v in s["split"].most_common()]
    lines += ["", f"Not counted: {hours(s['theirs'])} waiting on the user, {hours(s['cut'])} cut off when a session ended."]
    if s["stuck"]:
        lines += ["", "Also not counted, over an hour:", ""] + [f"- {hours(d)} {name}: {' '.join(what.split())[:120]}" for d, name, what in sorted(s["stuck"], reverse=True)]

    edited = s["started"]
    lines += ["", f"## From a session's start to its first push ({len(edited)} sessions that edited)", ""]
    pushed = [x[2] for x in edited if x[2] is not None]
    lines += [f"- first tool call to first edit: median {(pct([x[0] for x in edited], .5) or 0) / 60:.0f} min, {pct([x[1] for x in edited], .5) or 0} calls",
              f"- first edit to first push: median {(pct(pushed, .5) or 0) / 60:.0f} min ({len(pushed)} pushed)",
              f"- context at the first turn: median {pct([x[3] for x in edited], .5) or 0:,} tokens"]

    slow = sorted(((k, v) for k, v in s["commands"].items() if len(v) >= 3 and pct(v, .9) >= 60), key=lambda kv: -sum(kv[1]))[:12]
    lines += ["", "## Slowest commands, foreground and background", "", "| command | runs | median | p90 | total |", "| --- | ---: | ---: | ---: | ---: |"]
    lines += [f"| {k} | {len(v)} | {pct(v, .5):.0f}s | {pct(v, .9):.0f}s | {hours(sum(v))} |" for k, v in slow]

    lines += ["", f"## Reports corrected afterwards ({len(corrected)})", ""] + [f"- #{n} {text[:200]}" for n, _, text in corrected]
    lines += ["", f"## Friction ({len(friction)})", ""] + [f"- #{n} {text}" for n, _, text in friction]
    report = "\n".join(lines) + "\n"

    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, f"{now:%Y-%m-%d}.md")
    open(path, "w").write(report)
    print(report)


if __name__ == "__main__":
    sys.exit(main())
