#!/usr/bin/env python3
"""How fast work on The One App goes from started to done, and where the time goes, for the last day.

Reads this machine's Claude Code sessions in the repo and its worktrees, Agent Memory and git, and writes a report
to ~/one-export/iteration/<date>.md. Sessions on other machines and other harnesses aren't in it; Agent Memory and
git are everyone's. Memories have a date and no time, so it reads whole days.

    python3 -I tools/iteration/report.py [--days 1]
"""

import argparse, datetime as dt, glob, json, os, re, subprocess, sys, time
from collections import Counter

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SESSIONS = os.path.expanduser("~/.claude/projects/*the-one-app*/*.jsonl")
OUT = os.path.expanduser("~/one-export/iteration")


def when(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(len(xs) * p))] if xs else None


def one(*args):
    """A one command's output. A node just starting can't answer yet, so it's asked again for a few minutes;
    then the report fails rather than counting nothing as zero."""
    for wait in (0, 30, 60, 120):
        time.sleep(wait)
        r = subprocess.run(["one", *args], capture_output=True, text=True, stdin=subprocess.DEVNULL)
        if r.returncode == 0:
            return r.stdout
    sys.exit(f"one {' '.join(args)} failed: {r.stderr.strip()[:300]}")


# What a tool call was for, as time is split.
def phase(name, inp):
    if name in ("Read", "Grep", "Glob"):
        return "reading code"
    if name in ("Edit", "Write", "MultiEdit", "NotebookEdit"):
        return "editing"
    if "computer_" in name or "agent-devices_" in name or "preview_" in name:
        return "checking it works"
    if "memory_" in name or name.startswith("mcp__t3-code__") or name in ("Agent", "SendMessage", "ToolSearch", "AskUserQuestion"):
        return "memory and delegating"
    if name != "Bash":
        return "other"
    c = inp.get("command", "")
    if "<<'EOF'" in c and re.search(r"python3? -|cat >", c):
        return "editing"
    if re.search(r"\bsleep \d|\buntil\b.*\bdo\b", c):
        return "waiting"
    if re.search(r"cargo (test|build|check|clippy)|xcodebuild|gradle|go test|build\.sh", c):
        return "building and testing"
    if re.search(r"one(-dev)? update|git (push|pull|rebase|commit)|one deploy|one(-dev)? release|cloudflare deploy", c):
        return "shipping"
    if re.search(r"one (try|throwaway)|ssh mac|simctl|idb |screencapture|curl|adb |one (status|version)", c):
        return "checking it works"
    if re.search(r"one memory|t3 thread", c):
        return "memory and delegating"
    return "reading code" if re.match(r"(cd [^&]*&& )?(grep|rg|sed -n|cat|head|tail|ls|find|wc|git (log|show|diff|status|blame))", c) else "other"


def waited_for(c):
    for kind, p in [("the Mac", r"ssh mac|xcode|simctl|idb"), ("an update", r"one(-dev)? (update|build|publish)"),
                    ("a node or the Workers", r"curl|healthz|one status|cloudflare|/api/"),
                    ("an agent device", r"adb|vnc|agent-devices|emulator|qemu|VM ready"), ("one-dev throwaway", r"one(-dev)? (try|throwaway)"),
                    ("tests or builds", r"cargo|gradle|build")]:
        if re.search(p, c, re.I):
            return kind
    return "its own background job"


def sessions(since):
    split, waits, tests, builds, release = Counter(), Counter(), [], 0, 0
    calls = errors = bounced = steps = 0
    n = 0
    for f in glob.glob(SESSIONS):
        if os.path.getmtime(f) < since.timestamp():
            continue
        uses, last, counted = {}, None, False
        for line in open(f, errors="replace"):
            try:
                e = json.loads(line)
            except ValueError:
                continue
            if e.get("isSidechain") or not e.get("timestamp"):
                continue
            t = when(e["timestamp"])
            if t < since:
                continue
            content = (e.get("message") or {}).get("content")
            if e.get("type") == "user" and (isinstance(content, str) or any(isinstance(c, dict) and c.get("type") == "text" for c in content or [])):
                last = None  # a user's message: the time before it was theirs
                continue
            if not isinstance(content, list):
                continue
            for c in content:
                if not isinstance(c, dict):
                    continue
                if c.get("type") == "tool_use":
                    if not counted:
                        n, counted = n + 1, True
                    if last and (t - last).total_seconds() < 1800:
                        split["the model thinking and writing"] += (t - last).total_seconds()
                    steps += 1
                    uses[c["id"]] = (t, c["name"], c.get("input", {}))
                elif c.get("type") == "tool_result" and c.get("tool_use_id") in uses:
                    t0, name, inp = uses.pop(c["tool_use_id"])
                    d = (t - t0).total_seconds()
                    last = t
                    if d > 3600:  # a command left hanging, counted apart
                        split["commands left hanging (over an hour)"] += d
                        continue
                    p = phase(name, inp)
                    split[p] += d
                    calls += 1
                    text = json.dumps(c.get("content"))[:2000]
                    if c.get("is_error"):
                        errors += 1
                    if "too long" in text and "memory" in name:
                        bounced += 1
                    cmd = inp.get("command", "") if name == "Bash" else ""
                    if p == "waiting" and not inp.get("run_in_background"):
                        waits[waited_for(cmd)] += d
                    if re.search(r"(^|&& |; )(\S+=\S+ )*(nice \S* )?cargo test", cmd) and not inp.get("run_in_background"):
                        tests.append(d)
                    if re.search(r"cargo build", cmd):
                        builds += 1
                        release += "--release" in cmd
    return dict(n=n, split=split, waits=waits, tests=tests, builds=builds, release=release, calls=calls, errors=errors,
                bounced=bounced, steps=steps)


MEMORY = re.compile(r"#(\d+) (\d{4}-\d\d-\d\d) (.*)$")


def memories(since):
    """Agent Memory's memories from since's day on, oldest first: (id, date, text)."""
    days = [since.date() + dt.timedelta(days=i) for i in range((dt.datetime.now(dt.timezone.utc).date() - since.date()).days + 1)]
    out = one("memory", "recall", "^#\\d+ (" + "|".join(f"{d:%Y-%m-%d}" for d in days) + ") ")
    return [(int(m.group(1)), m.group(2), m.group(3)) for m in map(MEMORY.match, out.splitlines()) if m]


def broken(posts):
    """Memories saying something that went out was broken, found after."""
    return [p for p in posts if re.search(r"\bBug since\b|\bbroke\b|\bdidn't (build|parse|load|start) since\b|\bfailed since\b|\bsince [0-9a-f]{7}\b.*\b(fail|broke|didn't)", p[2], re.I)]


def git(*args):
    return subprocess.run(["git", "-C", REPO, *args], capture_output=True, text=True, errors="replace").stdout


def code(since):
    out = git("log", f"--since={since:%Y-%m-%dT%H:%M:%SZ}", "--numstat", "--format=tformat:@@", "--", ".", ":!vendor", ":!Cargo.lock", ":!icons", ":!*.png", ":!*.svg")
    commits = out.count("@@")
    added = removed = 0
    for line in out.splitlines():
        parts = line.split("\t")
        if len(parts) == 3 and parts[0].isdigit():
            added, removed = added + int(parts[0]), removed + int(parts[1])
    return commits, added, removed


def survival(day):
    """Of the lines committed on `day`, how many are still in main."""
    out = git("log", f"--since={day:%Y-%m-%dT%H:%M:%SZ}", f"--until={day + dt.timedelta(days=1):%Y-%m-%dT%H:%M:%SZ}", "--numstat", "--format=", "--",
              ".", ":!vendor", ":!Cargo.lock", ":!icons", ":!*.png", ":!*.svg")
    added = sum(int(l.split("\t")[0]) for l in out.splitlines() if l.split("\t")[0].isdigit())
    if not added:
        return None
    kept = 0
    files = [f for f in git("ls-files").splitlines() if not re.search(r"^(vendor|icons)/|\.(png|svg|lock|jpg|ico)$", f)]
    lo, hi = day.timestamp(), day.timestamp() + 86400
    for f in files:
        for line in git("blame", "--line-porcelain", "--", f).splitlines():
            if line.startswith("committer-time "):
                kept += lo <= int(line.split()[1]) < hi
    return kept, added


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
    bad = broken(posts)
    friction = [p for p in posts if p[2].lower().startswith("friction:")]
    commits, added, removed = code(since)
    day = (now - dt.timedelta(days=3)).replace(hour=0, minute=0, second=0, microsecond=0)
    kept = survival(day)
    total = sum(s["split"].values()) or 1

    lines = [f"# Iteration, {since:%Y-%m-%d %H:%M} to {now:%Y-%m-%d %H:%M} UTC", ""]
    lines += [f"## Where agents' time went ({s['n']} sessions on this machine, {hours(total)})", ""]
    lines += [f"- {k}: {hours(v)} ({100 * v / total:.0f}%)" for k, v in s["split"].most_common()]
    lines.append(f"- steps: {s['steps']}, {s['steps'] // max(s['n'], 1)} a session; {s['errors']} of {s['calls']} tool calls failed ({100 * s['errors'] / max(s['calls'], 1):.0f}%); {s['bounced']} memories bounced as too long")
    lines += ["", "Waiting, for:", ""] + [f"- {k}: {hours(v)}" for k, v in s["waits"].most_common()]
    t = s["tests"]
    lines += ["", "## Builds", "", f"- cargo test: {len(t)} runs, median {pct(t, .5) or 0:.0f}s, p90 {pct(t, .9) or 0:.0f}s",
              f"- cargo build: {s['builds']}, {s['release']} of them release",
              f"- load now: {open('/proc/loadavg').read().split()[0]} on {os.cpu_count()} cores"]
    lines += ["", "## Code", "", f"- {commits} commits, +{added} −{removed}"]
    if kept:
        lines.append(f"- of {day:%b %d}'s {kept[1]} lines, {kept[0]} ({100 * kept[0] / kept[1]:.0f}%) are still in main")
    lines += ["", f"## Broken, found after it went out ({len(bad)})", ""] + [f"- #{n} {text[:200]}" for n, _, text in bad]
    lines += ["", f"## Friction ({len(friction)})", ""] + [f"- #{n} {text}" for n, _, text in friction]
    report = "\n".join(lines) + "\n"

    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, f"{now:%Y-%m-%d}.md")
    open(path, "w").write(report)
    print(report)


if __name__ == "__main__":
    sys.exit(main())
