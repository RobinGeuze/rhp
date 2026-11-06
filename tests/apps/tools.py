#!/usr/bin/env python3
"""Run PHP's dev tools (tests/apps/tools.toml) under php and under rhp, and compare.

    python3 tests/apps/tools.py setup              # composer install in tools/project
    python3 tests/apps/tools.py list               # tools, versions and command counts
    python3 tests/apps/tools.py diff [TOOL...] [--candidate BIN] [--only N,...]

`diff` runs each command of each TOOL (all tools when none is named) on a
fresh copy of the fixture project, first under php and then under rhp. A
command is SAME when stdout, stderr and the exit code are byte-identical after
normalisation (NORMALISE below), and the files it left in the project
(vendor/ and var/ aside) are identical too; otherwise DIFF, with a unified
diff of what differed. The summary line per tool is `identical/total`.
Exit code 0 when every command was SAME.

  --candidate BIN  the engine compared against php (default $RHP or
                   target/release/rhp); `--candidate "$(command -v php)"`
                   self-checks the normalisation
  --only N,...     run only these command numbers (as printed by `diff`)
  --quiet          print the DIFF details only into the output directory

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
import subprocess
import sys
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
PROJECT = HERE / "tools" / "project"
WORK = HERE / "tools" / "work"
TIMEOUT = 900

# Normalisation: only what php itself varies from run to run, plus the
# engine's binary path. Each rule is (name, pattern, replacement); the README
# (tests/apps/README.md, "Dev tools") documents each one.
NORMALISE = [
    # The path of the engine binary (PHP_BINARY, and $_SERVER['_'] in
    # diagnostics) differs by construction between the two engines.
    ("binary", None, "<PHP_BINARY>"),
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
    (run_dir / "tmp").mkdir()


def snapshot(run_dir):
    """Files the command left in the project (outside vendor/, var/, home/, tmp/)."""
    out = {}
    for p in sorted(run_dir.rglob("*")):
        rel = p.relative_to(run_dir)
        if rel.parts[0] in ("vendor", "var", "home", "tmp", "out") or not p.is_file():
            continue
        out[str(rel)] = p.read_bytes()
    return out


def normalise(text, binaries):
    for name, pat, rep in NORMALISE:
        if pat is None:
            for b in binaries:
                text = text.replace(b, rep)
        else:
            text = pat.sub(rep, text)
    return text


def run_one(engine, tool, args, run_dir, files, binaries):
    reset(run_dir, files)
    # A minimal, fixed environment: no CI variables (php-cs-fixer's and
    # PHPStan's output depends on them), no colours, a fixed width, and HOME
    # and TMPDIR inside the run directory (same path for both engines).
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(run_dir / "home"),
        "TMPDIR": str(run_dir / "tmp"),
        "TZ": "UTC",
        "LANG": "C",
        "LC_ALL": "C",
        "COLUMNS": "120",
        "NO_COLOR": "1",
        "TERM": "dumb",
    }
    argv = [engine, tool["bin"], *shlex.split(args)]
    try:
        p = subprocess.run(argv, cwd=run_dir, env=env, capture_output=True, timeout=TIMEOUT, stdin=subprocess.DEVNULL)
        out, err, code = p.stdout, p.stderr, p.returncode
    except subprocess.TimeoutExpired as e:
        out, err, code = e.stdout or b"", (e.stderr or b"") + f"\n<TIMEOUT after {TIMEOUT}s>".encode(), "timeout"
    dec = lambda b: normalise(b.decode("utf-8", "replace"), binaries)
    return {"stdout": dec(out), "stderr": dec(err), "code": code, "files": snapshot(run_dir)}


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


def cmd_diff(tools, candidate, only, quiet):
    php = os.environ.get("RHP_PHP", "php")
    real_php = subprocess.run([php, "-r", "echo PHP_BINARY;"], capture_output=True, text=True).stdout
    real_cand = subprocess.run([candidate, "-r", "echo PHP_BINARY;"], capture_output=True, text=True).stdout
    binaries = sorted({b for b in (php, shutil.which(php), real_php, candidate, real_cand) if b}, key=len, reverse=True)
    files = fixture_files()
    summary = []
    for t in tools:
        run_dir = WORK / t["name"]
        out_dir = WORK / f"{t['name']}-out"
        shutil.rmtree(out_dir, ignore_errors=True)
        out_dir.mkdir(parents=True)
        same_count = total = 0
        for i, args in enumerate(t["commands"]):
            if only and i not in only:
                continue
            ref = run_one(php, t, args, run_dir, files, binaries)
            cand = run_one(candidate, t, args, run_dir, files, binaries)
            for who, r in (("php", ref), ("cand", cand)):
                (out_dir / f"{i:02}.{who}.txt").write_text(
                    f"exit {r['code']}\n--stdout--\n{r['stdout']}--stderr--\n{r['stderr']}"
                )
            same = ref == cand
            total += 1
            same_count += same
            label = f"{t['name']} {args}".rstrip()
            print(f"{'SAME' if same else 'DIFF'}  php={ref['code']} rhp={cand['code']}  {i:02} {label}", flush=True)
            if not same:
                d = describe_diff(ref, cand)
                (out_dir / f"{i:02}.diff").write_text(d)
                if not quiet:
                    lines = d.splitlines()
                    print("      " + "\n      ".join(lines[:40]))
                    if len(lines) > 40:
                        print(f"      ... ({len(lines) - 40} more lines in {out_dir / f'{i:02}.diff'})")
        summary.append((t, same_count, total))
        print(f"== {t['name']} {t['version']}: {same_count}/{total} commands identical (outputs in {out_dir})", flush=True)
    if len(summary) > 1:
        for t, s, n in summary:
            print(f"{t['name']:<14} {t['version']:<10} {s}/{n}")
    return 0 if all(s == n for _, s, n in summary) else 1


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    sub, rest = argv[0], argv[1:]
    candidate = os.environ.get("RHP") or str(REPO / "target/release/rhp")
    only, quiet, names = None, False, []
    it = iter(rest)
    for a in it:
        if a == "--candidate":
            candidate = next(it)
        elif a == "--only":
            only = {int(x) for x in next(it).split(",")}
        elif a == "--quiet":
            quiet = True
        else:
            names.append(a)
    tools = load()
    if sub == "setup":
        cmd_setup()
    elif sub == "list":
        for t in tools:
            print(f"{t['name']:<14} {t['version']:<10} {len(t['commands'])} commands")
    elif sub == "diff":
        return cmd_diff(select(tools, names), candidate, only, quiet)
    else:
        sys.exit(f"unknown command {sub!r}; see --help")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
