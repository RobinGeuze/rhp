#!/usr/bin/env python3
"""Run frameworks' own PHPUnit suites (tests/suites/suites.toml) under php and rhp.

    python3 tests/suites/suites.py setup [SUITE...]
    python3 tests/suites/suites.py run SUITE --engine php|rhp|BIN --out DIR [--label L] [--part P]...
    python3 tests/suites/suites.py compare SUITE --ref DIR [--ref DIR]... --cand DIR
            [--check-known] [--write-known [FILE]] [--write-flaky] [--summary FILE]
    python3 tests/suites/suites.py matrix

`setup` clones the suite at its pinned commit into the work dir
($RHP_SUITES_DIR/<suite>, default tests/suites/work/<suite>), copies the
committed overlay (lock file, configs), unpacks the pinned downloads, runs
`composer install` from the lock and the suite's setup commands with php.

`run` runs every part (one PHPUnit process each) with ENGINE and writes
<out>/<label>/: per part the TeamCity log (read as it streams, so a crash
only loses the rest of that part), the JUnit log and stdout/stderr, plus
results.json, the outcome of every test: pass, fail, skip or crash (started,
never finished). The label defaults to the engine name.

`compare` reads results.json of one or more reference runs (php) and of the
candidate (rhp). A test is passing when it passed in every reference run and
in the candidate. With --check-known, a test in <suite>/known-passing.txt
that php passes but the candidate doesn't is a regression (exit 1); one php
doesn't pass either is reported, not failed (the environment, not rhp).
--write-known writes the passing tests (to the list, or to FILE).
With several reference runs, tests whose outcome differs between them are
flaky: --write-flaky records them in <suite>/flaky.txt, which every
comparison leaves out.

php is $RHP_PHP or `php`; rhp is $RHP or target/release/rhp; composer is
$COMPOSER or `composer`. Needs python >= 3.11 (tomllib).
"""

import argparse
import collections
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import time
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.dont_write_bytecode = True  # no __pycache__ in tests/apps
sys.path.insert(0, str(HERE.parent / "apps"))
import apps  # noqa: E402  (fetch/unpack of pinned downloads)

STATUSES = ("pass", "skip", "fail", "crash")  # worst last
LIST_HEADER = (
    "# Tests of this suite that pass under php and rhp (tests/suites/README.md).\n"
    "# One `Class::test` per line, sorted; regenerate with `suites.py compare --write-known`.\n"
)
FLAKY_HEADER = (
    "# Tests whose outcome under php differs between runs; every comparison leaves them out.\n"
    "# Regenerate with `suites.py compare --ref A --ref B --ref C --write-flaky`.\n"
)


def load():
    with open(HERE / "suites.toml", "rb") as f:
        return tomllib.load(f)["suite"]


def select(suites, names):
    if not names:
        return suites
    by = {s["name"]: s for s in suites}
    bad = [n for n in names if n not in by]
    if bad:
        sys.exit(f"unknown suite(s): {', '.join(bad)} (known: {', '.join(by)})")
    return [by[n] for n in names]


def work_dir(s):
    base = os.environ.get("RHP_SUITES_DIR")
    return (Path(base) if base else HERE / "work") / s["name"]


def sh(cmd, cwd, env=None):
    print(f"[{cwd.name}] $ {cmd if isinstance(cmd, str) else shlex.join(cmd)}", flush=True)
    subprocess.run(cmd, cwd=cwd, env=env, check=True, shell=isinstance(cmd, str))


# ---------------------------------------------------------------- setup


def cmd_setup(suites):
    php = os.environ.get("RHP_PHP", "php")
    composer = os.environ.get("COMPOSER") or "composer"
    for s in suites:
        d = work_dir(s)
        g = s.get("git")
        if g:
            if not (d / ".git").is_dir():
                d.parent.mkdir(parents=True, exist_ok=True)
                sh(["git", "clone", "--quiet", "--depth", "1", "--branch", g["ref"], g["url"], str(d)], d.parent)
            head = subprocess.run(["git", "-C", str(d), "rev-parse", "HEAD"], capture_output=True, text=True, check=True)
            if head.stdout.strip() != g["commit"]:
                sys.exit(f"{s['name']}: {d} is at {head.stdout.strip()}, the manifest pins {g['commit']}")
        d.mkdir(parents=True, exist_ok=True)
        for f in s.get("overlay", []):
            shutil.copyfile(HERE / s["name"] / f, d / f)
        for dl in s.get("download", []):
            src = apps.fetch(dl["url"], dl["sha256"])
            dest = d / dl["to"]
            dest.parent.mkdir(parents=True, exist_ok=True)
            print(f"[{s['name']}] unpack {src.name} -> {dl['to']}", flush=True)
            apps.unpack(src, dest)
        env = dict(os.environ)
        if s.get("install"):
            cmd = shlex.split(s["install"])
            if cmd[0] == "composer":
                cmd[0] = composer
            sh(cmd, d, env)
        for c in s.get("setup", []):
            argv = shlex.split(c)
            if argv[0] == "php":
                argv[0] = php
            sh(argv, d, env)


# ---------------------------------------------------------------- TeamCity

_TC_LINE = re.compile(r"^##teamcity\[(\w+)((?: \w+='(?:[^'|]|\|.)*')*)\s*\]\s*$")
_TC_ATTR = re.compile(r"(\w+)='((?:[^'|]|\|.)*)'")
_TC_ESC = {"'": "'", "n": "\n", "r": "\r", "|": "|", "[": "[", "]": "]", "x": "\u0085", "l": "\u2028", "p": "\u2029"}


def tc_unescape(v):
    return re.sub(r"\|(0x[0-9a-fA-F]{4}|.)", lambda m: chr(int(m.group(1)[2:], 16)) if m.group(1).startswith("0x") else _TC_ESC.get(m.group(1), m.group(1)), v)


def _class_of(hint):
    # php_qn://<file>::\Ns\Class[::method...]
    i = hint.find("::\\")
    if i < 0:
        return None
    return hint[i + 3 :].split("::", 1)[0]


def parse_teamcity(text):
    """Map `Class::name` -> (status, message) from PHPUnit's TeamCity log.

    A test that started but never finished (the process died) is a crash.
    When a test is reported more than once, the worst outcome wins.
    """
    tests = {}
    suite_class = []
    current = None  # [key, name, outcome or None]

    def record(key, status, msg):
        old = tests.get(key)
        if old is None or STATUSES.index(status) > STATUSES.index(old[0]):
            tests[key] = (status, msg)

    def key_of(attrs):
        cls = _class_of(attrs.get("locationHint", "")) or (suite_class[-1] if suite_class and suite_class[-1] else "?")
        return f"{cls}::{attrs.get('name', '')}"

    for line in text.splitlines():
        m = _TC_LINE.match(line)
        if not m:
            continue
        ev = m.group(1)
        attrs = {k: tc_unescape(v) for k, v in _TC_ATTR.findall(m.group(2))}
        name = attrs.get("name", "")
        if ev == "testSuiteStarted":
            suite_class.append(_class_of(attrs.get("locationHint", "")) or (suite_class[-1] if suite_class else None))
        elif ev == "testSuiteFinished":
            if suite_class:
                suite_class.pop()
        elif ev == "testStarted":
            if current:
                record(current[0], "crash", "started, never finished")
            current = [key_of(attrs), name, None]
        elif ev in ("testFailed", "testIgnored"):
            st = "fail" if ev == "testFailed" else "skip"
            msg = attrs.get("message") or attrs.get("details", "")
            if current and current[1] == name:
                if current[2] is None or STATUSES.index(st) > STATUSES.index(current[2][0]):
                    current[2] = (st, msg)
            else:
                record(key_of(attrs), st, msg)
        elif ev == "testFinished":
            if current and current[1] == name:
                record(current[0], *(current[2] or ("pass", "")))
                current = None
    if current:
        record(current[0], "crash", "started, never finished")
    return tests


# ---------------------------------------------------------------- run


def engine_cmd(engine):
    if engine == "php":
        return [os.environ.get("RHP_PHP", "php")]
    if engine == "rhp":
        return [os.environ.get("RHP") or str(REPO / "target/release/rhp")]
    return shlex.split(engine)


def run_part(s, d, eng, part, out):
    name = re.sub(r"[^A-Za-z0-9_.-]+", "_", part)
    tc, junit, log = out / f"{name}.teamcity", out / f"{name}.junit.xml", out / f"{name}.log"
    for p in (tc, junit):
        p.unlink(missing_ok=True)
    for r in s.get("reset", []):
        shutil.rmtree(d / r, ignore_errors=True)
        (d / r).mkdir(parents=True, exist_ok=True)
    home = out / "home"
    tmp = out / "tmp"
    home.mkdir(exist_ok=True)
    tmp.mkdir(exist_ok=True)
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(home),
        "TMPDIR": str(tmp),
        "TZ": "UTC",
        "LANG": "C",
        "LC_ALL": "C",
        "COLUMNS": "120",
        "NO_COLOR": "1",
        "TERM": "dumb",
        # The same php.ini for both engines and their subprocesses.
        "PHPRC": str(HERE / "php.ini"),
        "WP_PHP_BINARY": shlex.join(eng),
        **s.get("env", {}),
    }
    argv = [*eng, s["phpunit"], *s.get("args", []), *shlex.split(s["part_args"].format(part=part)),
            "--log-teamcity", str(tc), "--log-junit", str(junit)]
    start = time.monotonic()
    timeout = s.get("timeout", 1800)
    timed_out = False
    with open(log, "wb") as lf:
        lf.write(f"$ {shlex.join(argv)}\n".encode())
        lf.flush()
        p = subprocess.Popen(argv, cwd=d, env=env, stdin=subprocess.DEVNULL, stdout=lf, stderr=subprocess.STDOUT,
                             start_new_session=True)
        try:
            code = p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(p.pid, signal.SIGKILL)
            code = p.wait()
    secs = round(time.monotonic() - start, 1)
    tests = parse_teamcity(tc.read_text(errors="replace")) if tc.exists() else {}
    tail = log.read_bytes()[-2000:].decode(errors="replace").strip().splitlines()
    return {"exit": code, "seconds": secs, "timeout": timed_out, "tail": tail[-5:],
            "tests": {k: list(v) for k, v in tests.items()}}


def cmd_run(s, engine, out_root, label, only):
    d = work_dir(s)
    if not d.is_dir():
        sys.exit(f"{s['name']}: no work dir {d}; run `suites.py setup {s['name']}` first")
    eng = engine_cmd(engine)
    label = label or (engine if engine in ("php", "rhp") else "cand")
    out = (Path(out_root) / s["name"] / label).resolve()
    out.mkdir(parents=True, exist_ok=True)
    parts = [p for p in s["parts"] if not only or p in only]
    res = {"suite": s["name"], "engine": shlex.join(eng), "parts": {}}
    for part in parts:
        r = run_part(s, d, eng, part, out)
        res["parts"][part] = r
        c = collections.Counter(st for st, _ in r["tests"].values())
        extra = " TIMEOUT" if r["timeout"] else ""
        print(f"{label:4} {s['name']} {part}: exit={r['exit']}{extra} {r['seconds']}s "
              + " ".join(f"{k}={c[k]}" for k in STATUSES if c[k]), flush=True)
        (out / "results.json").write_text(json.dumps(res, indent=1, sort_keys=True) + "\n")
    return res


# ---------------------------------------------------------------- compare


def flat(res):
    """test -> (status, message, part); worst outcome when a test ran in several parts."""
    t = {}
    for part, r in res["parts"].items():
        for k, (st, msg) in r["tests"].items():
            k = list_key(k)
            old = t.get(k)
            if old is None or STATUSES.index(st) > STATUSES.index(old[0]):
                t[k] = (st, msg, part)
    return t


def list_key(k):
    """A test name as one line of a list (data set names can hold newlines)."""
    return k.replace("\n", "\\n").replace("\r", "\\r")


def read_list(p):
    if not p.exists():
        return []
    return [l for l in p.read_text().splitlines() if l and not l.startswith("#")]


def write_list(p, header, keys):
    p.write_text(header + "".join(k + "\n" for k in sorted(set(keys))))


_NORM = [
    (re.compile(r"(/[^\s'\"():]+)+/(work|tmp)/[^\s'\"():]*"), "<path>"),
    (re.compile(r"/(private/)?(tmp|var/folders)/[^\s'\"():]+"), "<tmp>"),
    (re.compile(r"#\d+"), "#N"),
    (re.compile(r"\b0x[0-9a-f]+\b"), "0x…"),
]


def reason(msg):
    first = next((l.strip() for l in (msg or "").splitlines() if l.strip()), "")
    for rx, rep in _NORM:
        first = rx.sub(rep, first)
    return first[:200] or "<no message>"


def compare(suite_name, refs, cand, known, flaky):
    """The comparison as data; see cmd_compare."""
    rt = [flat(r) for r in refs]
    ct = flat(cand)
    flaky = set(flaky)
    all_ref = set().union(*rt) if rt else set()
    unstable = sorted(k for k in all_ref if len({(t.get(k) or ("missing",))[0] for t in rt}) > 1)
    skip = flaky | set(unstable)
    ref_pass = {k for k in rt[0] if all(t.get(k, ("x",))[0] == "pass" for t in rt)} - skip if rt else set()
    cand_pass = {k for k, v in ct.items() if v[0] == "pass"} - skip
    passing = ref_pass & cand_pass
    known = set(known)
    regressions = sorted(k for k in known & ref_pass if k not in cand_pass)
    env = sorted(k for k in known - ref_pass - skip if k not in cand_pass)
    new = sorted(passing - known)
    # Why the candidate doesn't pass what php passes: the part's crash or
    # timeout when the test never ran, else the first line of its message.
    reasons = collections.Counter()
    example = {}
    for k in sorted(ref_pass - cand_pass):
        if k in ct:
            st, msg, _ = ct[k]
            r = f"{st}: {reason(msg)}"
        else:
            part = rt[0][k][2]
            pr = cand["parts"].get(part)
            if pr is None:
                r = f"part {part} not run"
            elif pr["timeout"]:
                r = f"part {part}: timeout"
            else:
                r = f"part {part} exited {pr['exit']}: {reason(' '.join(pr['tail'][-1:]))}"
        reasons[r] += 1
        example.setdefault(r, k)
    counts = {
        "ref": collections.Counter(v[0] for v in rt[0].values()) if rt else collections.Counter(),
        "cand": collections.Counter(v[0] for v in ct.values()),
    }
    return {
        "suite": suite_name, "ref_pass": len(ref_pass), "cand_pass": len(cand_pass), "passing": sorted(passing),
        "regressions": regressions, "env": env, "new": new, "unstable": unstable,
        "reasons": reasons.most_common(), "example": example, "counts": counts,
        "ref_total": len(rt[0]) if rt else 0, "cand_total": len(ct),
    }


def load_results(s, d):
    """results.json of a run: DIR is the run's own dir or the --out of `run`."""
    d = Path(d)
    for p in (d / "results.json", d / s["name"] / "results.json"):
        if p.exists():
            return json.loads(p.read_text())
    sys.exit(f"{s['name']}: no results.json in {d}")


def cmd_compare(s, refs, cand, check_known, write_known, write_flaky, summary, top):
    sd = HERE / s["name"]
    known_p, flaky_p = sd / "known-passing.txt", sd / "flaky.txt"
    c = compare(s["name"], [load_results(s, r) for r in refs], load_results(s, cand), read_list(known_p), read_list(flaky_p))
    rc, cc = c["counts"]["ref"], c["counts"]["cand"]
    lines = [
        f"{s['name']} ({s['title']} {s['version']}): php passes {c['ref_pass']}, rhp passes {len(c['passing'])} of them",
        "  php: " + " ".join(f"{k}={rc[k]}" for k in STATUSES if rc[k]) + f" (of {c['ref_total']})",
        "  rhp: " + " ".join(f"{k}={cc[k]}" for k in STATUSES if cc[k]) + f" (of {c['cand_total']})",
    ]
    if len(refs) > 1:
        lines.append(f"  unstable under php across {len(refs)} runs: {len(c['unstable'])}")
    if check_known:
        lines.append(f"  known-passing: {len(read_list(known_p))}, regressions: {len(c['regressions'])}, "
                     f"not passing under php either: {len(c['env'])}, newly passing: {len(c['new'])}")
    if c["reasons"]:
        lines.append("  why rhp doesn't pass what php passes:")
        for r, n in c["reasons"][:top]:
            lines.append(f"    {n:6d}  {r}\n            e.g. {c['example'][r]}")
    for title, keys in (("REGRESSION", c["regressions"]), ("not passing under php either", c["env"])):
        if check_known and keys:
            lines.append(f"  {title}:")
            lines += [f"    {k}" for k in keys[:200]]
    text = "\n".join(lines)
    print(text, flush=True)
    if summary:
        with open(summary, "a") as f:
            f.write(f"### {s['name']}\n\n```\n{text}\n```\n\n")
    if write_flaky:
        write_list(flaky_p, FLAKY_HEADER, c["unstable"] + read_list(flaky_p))
        print(f"  wrote {flaky_p} ({len(read_list(flaky_p))} tests)")
    if write_known is not None:
        p = Path(write_known) if write_known else known_p
        write_list(p, LIST_HEADER, c["passing"])
        print(f"  wrote {p} ({len(c['passing'])} tests)")
    return 1 if check_known and c["regressions"] else 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("setup")
    p.add_argument("suites", nargs="*")
    p = sub.add_parser("run")
    p.add_argument("suite")
    p.add_argument("--engine", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--label")
    p.add_argument("--part", action="append", default=[])
    p = sub.add_parser("compare")
    p.add_argument("suite")
    p.add_argument("--ref", action="append", required=True)
    p.add_argument("--cand", required=True)
    p.add_argument("--check-known", action="store_true")
    p.add_argument("--write-known", nargs="?", const="", default=None)
    p.add_argument("--write-flaky", action="store_true")
    p.add_argument("--summary")
    p.add_argument("--top", type=int, default=40)
    sub.add_parser("matrix")
    a = ap.parse_args(argv)
    suites = load()
    if a.cmd == "setup":
        cmd_setup(select(suites, a.suites))
    elif a.cmd == "run":
        cmd_run(select(suites, [a.suite])[0], a.engine, a.out, a.label, a.part)
    elif a.cmd == "compare":
        s = select(suites, [a.suite])[0]
        return cmd_compare(s, a.ref, a.cand, a.check_known, a.write_known, a.write_flaky, a.summary, a.top)
    elif a.cmd == "matrix":
        print(json.dumps([{"suite": s["name"]} for s in suites]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
