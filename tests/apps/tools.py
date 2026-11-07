#!/usr/bin/env python3
"""Run PHP's dev tools (tests/apps/tools.toml) under php and under rhp, and compare.

    python3 tests/apps/tools.py setup              # composer install in tools/project
    python3 tests/apps/tools.py list               # tools, versions and command counts
    python3 tests/apps/tools.py diff [TOOL...] [--big] [--candidate BIN] [--only N,...]

`diff` runs each command of each TOOL (all tools when none is named) on a
fresh copy of the fixture project, first under php and then under rhp. A
command is SAME when stdout, stderr and the exit code are byte-identical after
normalisation (NORMALISE below), and the files it left in the project
(vendor/ and var/ aside) are identical too; otherwise DIFF, with a unified
diff of what differed. Commands marked `unordered` in tools.toml (parallel
runs, which list files in worker completion order) compare their per-file
blocks as a set. Each line also shows CPU time (user + sys of the whole
process tree) and peak RSS under php and the candidate. The summary line per
tool is `identical/total`. A command marked `known` in tools.toml prints
KNOWN when it differs and FIXED when it matches (drop the mark then). Exit
code 0 when every other command was SAME.

Both engines read tools/php.ini (PHPRC), a php.ini like a distribution's.

  --candidate BIN  the engine compared against php (default $RHP or
                   target/release/rhp); `--candidate "$(command -v php)"`
                   self-checks the normalisation
  --only N,...     run only these command numbers (as printed by `diff`)
  --big            run the big tier instead (each tool's `big` commands:
                   PHPStan, Psalm, Rector and CS-Fixer on vendor code,
                   PHPUnit's synthetic suite with process isolation);
                   minutes per command under rhp, so CI runs it nightly
  --quiet          print the DIFF details only into the output directory
  --work DIR       the work directory (default tools/work)

php is $RHP_PHP or `php`; composer (for setup) is $COMPOSER or `composer`.
Each tool runs in tools/work/<tool>/ (vendor/ is cloned there from
tools/project/vendor once), so different tools can run concurrently. Raw
outputs are kept in tools/work/<tool>/out/. Needs python >= 3.11 (tomllib).
"""

import difflib
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import threading
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
PROJECT = HERE / "tools" / "project"
WORK = HERE / "tools" / "work"
PHP_INI = HERE / "tools" / "php.ini"
TIMEOUT = 900
BIG_TIMEOUT = 3600  # a big-tier command takes minutes of CPU under rhp

# Normalisation: only what php itself varies from run to run, plus the
# engine's binary path. Each rule is (name, pattern, replacement); the README
# (tests/apps/README.md, "Dev tools") documents each one.
NORMALISE = [
    # The absolute path of the engine binary (PHP_BINARY, and $_SERVER['_']
    # in diagnostics) differs by construction between the two engines.
    ("binary", None, "<PHP_BINARY>"),
    # PHPStan forks its workers only with its native phpstan_turbo extension,
    # which php loads and rhp cannot (a Zend .so); otherwise it spawns them.
    ("phpstan-worker", re.compile(r"\((the|largest of \d+) (forked|spawned) (worker|workers)\)"), r"(\1 \3)"),
    # PHPUnit's "Time: 00:00.047, Memory: 18.00 MB" summary line.
    ("phpunit-time", re.compile(r"Time: \d\d:\d\d(\.\d+)?, Memory: [\d.]+ MB"), "Time: <T>, Memory: <M>"),
    # PHPUnit's event log: "[00:00:00.012345678 / 00:00:00.000123456] ..." and memory figures.
    ("phpunit-event-time", re.compile(r"\[\d\d:\d\d:\d\d\.\d+ / \d\d:\d\d:\d\d\.\d+\]"), "[<T> / <T>]"),
    ("phpunit-event-memory", re.compile(r"(memory|Memory)(: | usage: | peak: )[\d.]+ ?(MB|bytes)"), r"\1\2<M>"),
    # Durations in JUnit/TeamCity/JSON reports (time="0.0123", duration='12', "time":0.08).
    ("report-time", re.compile(r"""\b(time|duration)=(["'])[\d.]+\2"""), r"\1=\2<T>\2"),
    ("xml-time", re.compile(r'<total value="[\d.]+"/>'), '<total value="<T>"/>'),
    ("xml-memory", re.compile(r'<memory value="[\d.]+"'), '<memory value="<M>"'),
    # TeamCity's flowId is the process id.
    ("teamcity-flow", re.compile(r"flowId='\d+'"), "flowId='<PID>'"),
    # Symfony progress bars redraw at most every 0.1 s, so which intermediate
    # steps get printed depends on timing: keep only the first and last step.
    (
        "progress-steps",
        re.compile(r"\n *(\d+)/(\d+) \[[░▓=>-]*\] *\d+%[^\n]*"),
        lambda m: "" if 0 < int(m.group(1)) < int(m.group(2)) else m.group(0),
    ),
    ("json-time",re.compile(r'"(time|total|duration)": ?[\d.]+'), r'"\1": <T>'),
    ("json-memory", re.compile(r'"memory": ?[\d.]+'), '"memory": <M>'),
    # Elapsed time and memory in summaries: "in 0.088 seconds, 18.00 MB memory
    # used", "Checks took 14.64 seconds and used 123.789MB of memory",
    # "Elapsed time: 1.2 seconds", "Used memory: 64 MB".
    ("elapsed", re.compile(r"\b\d+(\.\d+)?\s*(ms|s|sec|secs|seconds?)\b"), "<T>"),
    ("memory", re.compile(r"\b\d+(\.\d+)?\s*(B|KB|KiB|MB|MiB|GB|GiB)\b"), "<M>"),
]


def load():
    with open(HERE / "tools.toml", "rb") as f:
        tools = tomllib.load(f)["tool"]
    lock = json.loads((PROJECT / "composer.lock").read_text())
    versions = {p["name"]: p["version"] for p in lock["packages"] + lock["packages-dev"]}
    for t in tools:
        t["version"] = versions.get(t["package"], "?")
        # A command is an argument string or {args = "...", unordered = true}.
        for tier in ("commands", "big"):
            t[tier] = [c if isinstance(c, dict) else {"args": c} for c in t.get(tier, [])]
    return tools


def select(tools, names):
    if not names:
        return tools
    by = {t["name"]: t for t in tools}
    bad = [n for n in names if n not in by]
    if bad:
        sys.exit(f"unknown tool(s): {', '.join(bad)} (known: {', '.join(by)})")
    return [by[n] for n in names]


def cmd_setup():
    composer = os.environ.get("COMPOSER") or "composer"
    cmd = [composer, "install", "--no-scripts", "--no-interaction"]
    print(f"[tools/project] $ {shlex.join(cmd)}", flush=True)
    subprocess.run(cmd, cwd=PROJECT, check=True)


def fixture_files():
    """The committed fixture: every file of tools/project but vendor/ and var/."""
    out = []
    for p in sorted(PROJECT.rglob("*")):
        rel = p.relative_to(PROJECT)
        if rel.parts[0] in ("vendor", "var") or not p.is_file():
            continue
        out.append(rel)
    return out


def reset(run_dir, files):
    """Make run_dir a fresh copy of the fixture, keeping its cloned vendor/."""
    run_dir.mkdir(parents=True, exist_ok=True)
    for p in run_dir.iterdir():
        if p.name == "vendor":
            continue
        if p.is_dir() and not p.is_symlink():
            shutil.rmtree(p)
        else:
            p.unlink()
    vendor = run_dir / "vendor"
    if not vendor.is_dir():
        if not (PROJECT / "vendor").is_dir():
            sys.exit("tools/project/vendor is missing; run `python3 tests/apps/tools.py setup`")
        # APFS clones are instant; fall back to a plain copy elsewhere.
        if sys.platform == "darwin":
            subprocess.run(["cp", "-cR", str(PROJECT / "vendor"), str(vendor)], check=True)
        else:
            shutil.copytree(PROJECT / "vendor", vendor, symlinks=True)
    for rel in files:
        (run_dir / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(PROJECT / rel, run_dir / rel)
    (run_dir / "home").mkdir()
    tmp = tmp_dir(run_dir)
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir(parents=True)


def tmp_dir(run_dir):
    """TMPDIR for a run: short, as Psalm's and Rector's workers put unix
    sockets there and macOS allows socket paths of only 104 bytes."""
    return Path("/tmp") / f"rhp-tools-{os.getuid()}" / run_dir.name


def snapshot(run_dir):
    """Files the command left in the project (outside vendor/, var/, home/, tmp/)."""
    out = {}
    for p in sorted(run_dir.rglob("*")):
        rel = p.relative_to(run_dir)
        if rel.parts[0] in ("vendor", "var", "home", "tmp", "out") or not p.is_file():
            continue
        out[str(rel)] = p.read_bytes()
    return out


# Parallel runs print one numbered block per file ("   1) src/Item.php",
# "1) src/Item.php:12") in the order the workers finish, which php does not
# keep stable either. The text ends at a summary line.
BLOCK = re.compile(r"^\s*\d+\) \S")
TAIL = re.compile(r"^ ?\[(OK|ERROR|WARNING|NOTE|INFO|CAUTION)\] |^(Found|Fixed) \d+ of \d+ files")
FILE_KEYS = ("name", "file", "file_name", "path")


def sort_json(v, key=""):
    if isinstance(v, dict):
        return {k: sort_json(x, k) for k, x in v.items()}
    if isinstance(v, list):
        v = [sort_json(x) for x in v]
        if v and all(isinstance(x, dict) for x in v):
            k = next((k for k in FILE_KEYS if all(k in x for x in v)), None)
            if k:
                v.sort(key=lambda x: str(x[k]))
        elif "file" in key and all(isinstance(x, str) for x in v):
            v.sort()
    return v


def unorder(text):
    """The per-file blocks of a parallel run in a fixed order (see BLOCK)."""
    try:
        data = json.loads(text)
    except ValueError:
        data = None
    if isinstance(data, (dict, list)):
        return json.dumps(sort_json(data), indent=4, ensure_ascii=False) + "\n"
    lines = text.splitlines(keepends=True)
    starts = [i for i, l in enumerate(lines) if BLOCK.match(l)]
    if not starts:
        return text
    end = next((i for i in range(starts[-1], len(lines)) if TAIL.match(lines[i])), len(lines))
    blocks = []
    for a, b in zip(starts, starts[1:] + [end]):
        block = "".join(lines[a:b]).rstrip("\n") + "\n"
        blocks.append(re.sub(r"^(\s*)\d+\)", r"\1#)", block))
    return "".join(lines[: starts[0]]) + "\n".join(sorted(blocks)) + "\n" + "".join(lines[end:])


def normalise(text, binaries):
    for name, pat, rep in NORMALISE:
        if pat is None:
            for b in binaries:
                text = text.replace(b, rep)
        else:
            text = pat.sub(rep, text)
    return text


def spawn(argv, cwd, env, timeout=TIMEOUT):
    """Run argv with stdin closed; return stdout, stderr, exit code (or
    "timeout"), CPU seconds and peak RSS in MB of the process and the
    children it waited for (composer_diff.py uses it too)."""
    p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, start_new_session=True)
    bufs = {}

    def drain(name, f):
        bufs[name] = f.read()

    readers = [threading.Thread(target=drain, args=(n, f)) for n, f in (("out", p.stdout), ("err", p.stderr))]
    for r in readers:
        r.start()
    timed_out = []

    def kill():
        timed_out.append(True)
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    timer = threading.Timer(timeout, kill)
    timer.start()
    _, status, ru = os.wait4(p.pid, 0)
    timer.cancel()
    p.returncode = os.waitstatus_to_exitcode(status)
    if timed_out:
        # Leftover workers would keep the pipes open.
        kill()
    for r in readers:
        r.join()
    p.stdout.close()
    p.stderr.close()
    out, err, code = bufs.get("out", b""), bufs.get("err", b""), p.returncode
    if timed_out:
        err += f"\n<TIMEOUT after {timeout}s>".encode()
        code = "timeout"
    rss = ru.ru_maxrss / (1 << 20 if sys.platform == "darwin" else 1 << 10)
    return out, err, code, ru.ru_utime + ru.ru_stime, rss


def run_one(engine, tool, cmd, run_dir, files, binaries, timeout=TIMEOUT):
    reset(run_dir, files)
    # A minimal, fixed environment: no CI variables (php-cs-fixer's and
    # PHPStan's output depends on them), no colours, a fixed width, and HOME
    # inside the run directory and a short TMPDIR (same paths for both engines).
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(run_dir / "home"),
        "TMPDIR": str(tmp_dir(run_dir)),
        "TZ": "UTC",
        "LANG": "C",
        "LC_ALL": "C",
        "COLUMNS": "120",
        "NO_COLOR": "1",
        "TERM": "dumb",
        "PHPRC": str(PHP_INI),
    }
    argv = [engine, tool["bin"], *shlex.split(cmd["args"])]
    out, err, code, cpu, rss = spawn(argv, run_dir, env, timeout)

    def dec(b):
        text = b.decode("utf-8", "replace")
        return normalise(unorder(text) if cmd.get("unordered") else text, binaries)

    return {"stdout": dec(out), "stderr": dec(err), "code": code, "files": snapshot(run_dir), "cpu": cpu, "rss": rss}


def same(ref, cand):
    return all(ref[k] == cand[k] for k in ("stdout", "stderr", "code", "files"))


def describe_diff(ref, cand):
    parts = []
    if ref["code"] != cand["code"]:
        parts.append(f"exit code: php={ref['code']} candidate={cand['code']}\n")
    for stream in ("stdout", "stderr"):
        if ref[stream] != cand[stream]:
            d = difflib.unified_diff(
                ref[stream].splitlines(keepends=True),
                cand[stream].splitlines(keepends=True),
                f"php {stream}",
                f"candidate {stream}",
                n=2,
            )
            parts.append("".join(d))
    for name in sorted(set(ref["files"]) | set(cand["files"])):
        a, b = ref["files"].get(name), cand["files"].get(name)
        if a == b:
            continue
        if a is None or b is None:
            parts.append(f"file {name}: only written by {'candidate' if a is None else 'php'}\n")
            continue
        d = difflib.unified_diff(
            a.decode("utf-8", "replace").splitlines(keepends=True),
            b.decode("utf-8", "replace").splitlines(keepends=True),
            f"php {name}",
            f"candidate {name}",
            n=2,
        )
        parts.append("".join(d))
    return "".join(p if p.endswith("\n") else p + "\n" for p in parts)


def binary_paths(php, candidate):
    """The absolute paths under which each engine can appear in output. Never
    a bare name such as `php`: it would replace every "php" in the text."""
    out = set()
    for engine in (php, candidate):
        which = shutil.which(engine)
        real = subprocess.run([engine, "-n", "-r", "echo PHP_BINARY;"], capture_output=True, text=True).stdout
        for b in (engine, which, which and os.path.realpath(which), real):
            if b and os.path.isabs(b):
                out.add(b)
    return sorted(out, key=len, reverse=True)


def cmd_diff(tools, candidate, only, quiet, big):
    php = os.environ.get("RHP_PHP", "php")
    binaries = binary_paths(php, candidate)
    files = fixture_files()
    tier = "big" if big else "commands"
    summary = []
    for t in tools:
        if not t[tier]:
            continue
        name = f"{t['name']}-big" if big else t["name"]
        run_dir = WORK / name
        out_dir = WORK / f"{name}-out"
        shutil.rmtree(out_dir, ignore_errors=True)
        out_dir.mkdir(parents=True)
        same_count = total = failed = 0
        cpu = [0.0, 0.0]
        for i, cmd in enumerate(t[tier]):
            if only and i not in only:
                continue
            timeout = BIG_TIMEOUT if big else TIMEOUT
            ref = run_one(php, t, cmd, run_dir, files, binaries, timeout)
            cand = run_one(candidate, t, cmd, run_dir, files, binaries, timeout)
            for who, r in (("php", ref), ("cand", cand)):
                (out_dir / f"{i:02}.{who}.txt").write_text(
                    f"exit {r['code']}\n--stdout--\n{r['stdout']}--stderr--\n{r['stderr']}"
                )
            ok = same(ref, cand)
            total += 1
            same_count += ok
            cpu[0] += ref["cpu"]
            cpu[1] += cand["cpu"]
            label = f"{t['name']} {cmd['args']}".rstrip()
            usage = f"cpu {ref['cpu']:.1f}s/{cand['cpu']:.1f}s rss {ref['rss']:.0f}/{cand['rss']:.0f}MB"
            # A command with `known = "reason"` differs for a known cause in rhp.
            verdict = ("FIXED" if cmd.get("known") else "SAME") if ok else ("KNOWN" if cmd.get("known") else "DIFF")
            failed += verdict == "DIFF"
            print(f"{verdict:<5} php={ref['code']} rhp={cand['code']}  {usage:<34} {i:02} {label}", flush=True)
            if verdict == "KNOWN":
                print(f"      known: {cmd['known']}")
            if not ok:
                d = describe_diff(ref, cand)
                (out_dir / f"{i:02}.diff").write_text(d)
                if not quiet:
                    lines = d.splitlines()
                    print("      " + "\n      ".join(lines[:40]))
                    if len(lines) > 40:
                        print(f"      ... ({len(lines) - 40} more lines in {out_dir / f'{i:02}.diff'})")
        summary.append((t, same_count, total, cpu, failed))
        print(
            f"== {t['name']} {t['version']}: {same_count}/{total} commands identical, "
            f"cpu php {cpu[0]:.1f}s rhp {cpu[1]:.1f}s (outputs in {out_dir})",
            flush=True,
        )
    if len(summary) > 1:
        for t, s, n, c, _ in summary:
            ratio = f"{c[1] / c[0]:.1f}x" if c[0] else "-"
            print(f"{t['name']:<14} {t['version']:<10} {s}/{n}  cpu {c[0]:.1f}s -> {c[1]:.1f}s ({ratio})")
    return 1 if any(f for *_, f in summary) else 0


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    sub, rest = argv[0], argv[1:]
    candidate = os.environ.get("RHP") or str(REPO / "target/release/rhp")
    global WORK
    only, quiet, big, names = None, False, False, []
    it = iter(rest)
    for a in it:
        if a == "--candidate":
            candidate = next(it)
        elif a == "--only":
            only = {int(x) for x in next(it).split(",")}
        elif a == "--quiet":
            quiet = True
        elif a == "--big":
            big = True
        elif a == "--work":
            WORK = Path(next(it)).resolve()
        else:
            names.append(a)
    tools = load()
    if sub == "setup":
        cmd_setup()
    elif sub == "list":
        for t in tools:
            print(f"{t['name']:<14} {t['version']:<10} {len(t['commands'])} commands, {len(t['big'])} big")
    elif sub == "diff":
        return cmd_diff(select(tools, names), candidate, only, quiet, big)
    else:
        sys.exit(f"unknown command {sub!r}; see --help")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
