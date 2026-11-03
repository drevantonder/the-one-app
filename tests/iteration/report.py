#!/usr/bin/env python3
"""How fast work on The One App goes from started to done, and where the time goes, for the last day.

Reads this machine's Claude Code sessions in the repo and its worktrees, the board's the-one-app topic and git, and
writes a report to ~/one-export/iteration/<date>.md. With --post, also posts its headline to the-one-app/speed.
Sessions on other machines and other harnesses aren't in it; the board and git are everyone's.

    python3 -I tests/iteration/report.py [--days 1] [--post]
"""

import argparse, datetime as dt, glob, json, os, re, statistics, subprocess, sys, time
from collections import Counter

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SESSIONS = os.path.expanduser("~/.claude/projects/*the-one-app*/*.jsonl")
OUT = os.path.expanduser("~/one-export/iteration")
WHO = "iteration report (tests/iteration/report.py, personal)"


def when(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(len(xs) * p))] if xs else None


def one(*args):
    return subprocess.run(["one", *args], capture_output=True, text=True, stdin=subprocess.DEVNULL).stdout


# What a tool call was for, as time is split.
def phase(name, inp):
    if name in ("Read", "Grep", "Glob"):
        return "reading code"
    if name in ("Edit", "Write", "MultiEdit", "NotebookEdit"):
        return "editing"
    if "computer_" in name or "agent-devices_" in name or "preview_" in name:
        return "checking it works"
    if "board_" in name or name.startswith("mcp__t3-code__") or name in ("Agent", "SendMessage", "ToolSearch", "AskUserQuestion"):
        return "board and delegating"
    if name != "Bash":
        return "other"
    c = inp.get("command", "")
    if "<<'EOF'" in c and re.search(r"python3? -|cat >", c):
        return "editing"
    if re.search(r"\bsleep \d|\buntil\b.*\bdo\b", c):
        return "waiting"
    if re.search(r"cargo (test|build|check|clippy)|xcodebuild|gradle|go test|build\.sh", c):
        return "building and testing"
    if re.search(r"one update|git (push|pull|rebase|commit)|one deploy|one release|kernel deploy", c):
        return "shipping"
    if re.search(r"one try|ssh mac|simctl|idb |screencapture|curl|adb |one (status|version)", c):
        return "checking it works"
    if re.search(r"one board|t3 thread", c):
        return "board and delegating"
    return "reading code" if re.match(r"(cd [^&]*&& )?(grep|rg|sed -n|cat|head|tail|ls|find|wc|git (log|show|diff|status|blame))", c) else "other"


def waited_for(c):
    for kind, p in [("the Mac", r"ssh mac|xcode|simctl|idb"), ("an update", r"one update|after-update"),
                    ("a node or the kernel", r"curl|healthz|one status|kernel|/v1/"),
                    ("an agent device", r"adb|vnc|agent-devices|emulator|qemu|VM ready"), ("one try", r"one try"),
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
                last = None  # a person's message: the time before it was theirs
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
                    if "too long" in text and "board" in name:
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


POST = re.compile(r"#(\d+) (\d{4}-\d\d-\d\d \d\d:\d\d)Z (.*)$")


def board(topic, since):
    """The topic's posts since then, oldest first: (n, time, who, text)."""
    posts, before = {}, None
    while True:
        text = one("board", "read", "--topic", topic, *(["--before", str(before)] if before else []))
        page = []
        for line in text.splitlines():
            m = POST.match(line.strip())
            if m:
                who, _, said = m.group(3).partition(": ")
                page.append((int(m.group(1)), dt.datetime.strptime(m.group(2), "%Y-%m-%d %H:%M").replace(tzinfo=dt.timezone.utc), who, said))
        if not page:
            break
        for p in page:
            posts[p[0]] = p
        oldest = min(p[0] for p in page)
        if min(p[1] for p in page) < since or oldest <= 1 or oldest == before:
            break
        before = oldest
    return [p for _, p in sorted(posts.items()) if p[1] >= since]


def tasks(posts, now):
    """Started work: how long each took to its end, and those with no word since."""
    took, open_ = [], []
    for i, (n, t, who, said) in enumerate(posts):
        if not re.match(r"Starting", said):
            continue
        tid = re.search(r"t3-[0-9a-f]{8}", who + " " + said)
        end = next((q for q in posts[i + 1:] if re.match(r"(Done|Fixed|Stopped|Gave up|Parked)", q[3])
                    and (f"#{n}" in q[3] or q[2] == who or (tid and tid.group(0) in q[2] + q[3]))), None)
        if end:
            took.append((end[1] - t).total_seconds() / 60)
        else:
            later = [q for q in posts[i + 1:] if q[2] == who]
            if (now - (later[-1][1] if later else t)).total_seconds() > 3 * 3600:
                open_.append((n, t, who, said))
    return took, open_


def broken(posts):
    """Posts saying something that went out was broken, found after."""
    return [p for p in posts if re.search(r"\bBug since\b|\bbroke\b|\bdidn't (build|parse|load|start) since\b|\bfailed since\b|\bsince [0-9a-f]{7}\b.*\b(fail|broke|didn't)", p[3], re.I)]


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
    ap.add_argument("--post", action="store_true")
    a = ap.parse_args()
    now = dt.datetime.now(dt.timezone.utc)
    since = now - dt.timedelta(days=a.days)

    s = sessions(since)
    posts = board("the-one-app", since)
    took, open_ = tasks(posts, now)
    bad = broken(posts)
    friction = board("the-one-app/friction", since)
    commits, added, removed = code(since)
    day = (now - dt.timedelta(days=3)).replace(hour=0, minute=0, second=0, microsecond=0)
    kept = survival(day)
    total = sum(s["split"].values()) or 1

    lines = [f"# Iteration, {since:%Y-%m-%d %H:%M} to {now:%Y-%m-%d %H:%M} UTC", ""]
    lines += ["## Started to done", ""]
    if took:
        lines.append(f"{len(took)} pieces of work: median {statistics.median(took):.0f} min, p90 {pct(took, .9):.0f} min.")
    lines.append(f"{len(open_)} started with no word for 3 hours or more (a start's end is found by its number, its t3 id, or the same who):" if open_ else "Nothing started is without word for 3 hours.")
    lines += [f"- #{n} {t:%H:%M} {who}: {said[:160]}" for n, t, who, said in open_]
    lines += ["", f"## Where agents' time went ({s['n']} sessions on this machine, {hours(total)})", ""]
    lines += [f"- {k}: {hours(v)} ({100 * v / total:.0f}%)" for k, v in s["split"].most_common()]
    lines.append(f"- steps: {s['steps']}, {s['steps'] // max(s['n'], 1)} a session; {s['errors']} of {s['calls']} tool calls failed ({100 * s['errors'] / max(s['calls'], 1):.0f}%); {s['bounced']} board posts bounced as too long")
    lines += ["", "Waiting, for:", ""] + [f"- {k}: {hours(v)}" for k, v in s["waits"].most_common()]
    t = s["tests"]
    lines += ["", "## Builds", "", f"- cargo test: {len(t)} runs, median {pct(t, .5) or 0:.0f}s, p90 {pct(t, .9) or 0:.0f}s",
              f"- cargo build: {s['builds']}, {s['release']} of them release",
              f"- load now: {open('/proc/loadavg').read().split()[0]} on {os.cpu_count()} cores"]
    lines += ["", "## Code", "", f"- {commits} commits, +{added} −{removed}"]
    if kept:
        lines.append(f"- of {day:%b %d}'s {kept[1]} lines, {kept[0]} ({100 * kept[0] / kept[1]:.0f}%) are still in main")
    lines += ["", f"## Broken, found after it went out ({len(bad)})", ""] + [f"- #{n} {who}: {said[:200]}" for n, _, who, said in bad]
    lines += ["", f"## Friction ({len(friction)})", ""] + [f"- {who}: {said}" for _, _, who, said in friction]
    report = "\n".join(lines) + "\n"

    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, f"{now:%Y-%m-%d}.md")
    open(path, "w").write(report)
    print(report)
    if a.post:
        top = ", ".join(f"{k} {100 * v / total:.0f}%" for k, v in s["split"].most_common(3))
        head = (f"Last {a.days:g}d: {len(took)} done, median {statistics.median(took) if took else 0:.0f} min (p90 {pct(took, .9) or 0:.0f}); "
                f"{len(open_)} starts silent 3h+; {len(bad)} broken found later; {len(friction)} friction. Time: {top}. "
                f"cargo test median {pct(t, .5) or 0:.0f}s. {path}")
        while len(head.encode()) > 280:
            head = head[: -(len(head.encode()) - 278)] + "…"
        one("board", "post", "--topic", "the-one-app/speed", "--who", WHO, head)


if __name__ == "__main__":
    sys.exit(main())
