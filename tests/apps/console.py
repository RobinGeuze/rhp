#!/usr/bin/env python3
"""Run each fixture app's console commands under php and under rhp, and compare.

    python3 tests/apps/apps.py console [APP...] [--write-known] [-- OPTIONS]
    python3 tests/apps/console.py APP [--write-known] [OPTIONS]

The commands are in the app's console list (`console_list` in apps.toml,
default console.toml; format below). Every command starts from the same
pristine copy of the app, first under `php -n`, then under `rhp -n`. A
command is SAME when the exit code, stdout, stderr and the files it wrote
(their permission bits too; SQLite databases compared by their SQL dump) are
identical after the masks.

Without --write-known the run checks the app's ratchet (`known_console` in
apps.toml, default known-passing-console.txt): it fails only when a listed
command is not SAME. Commands newly SAME are printed as hints.

OPTIONS
  --candidate BIN  the engine compared against php (default $RHP or
                   target/release/rhp); `--candidate "$(command -v php)"`
                   self-checks the list and its masks
  --only ID,...    run only these command ids
  --network        also run the commands marked `network` (the nightly does)
  --quiet          write the diffs only into the output directory

php is $RHP_PHP or `php`. The work directory is target/apps-console/<app>/
($RHP_APPS_WORK/<app> when set):
`run/` (where commands run; its path is the same for both engines), the
pristine copy, `home/` and `tmp/` (HOME and TMPDIR, emptied before each run)
and `out/` (each command's outputs and diff). Needs python >= 3.11.

Console list (TOML), paths relative to the app directory:

  shared = ["vendor"]       copied into run/ once and treated as read-only:
                            neither reset nor compared (keep big trees here)
  clear = ["var"]           removed from the copy before `prepare`
  prepare = ["bin/console cache:warmup"]
                            run with `php -n` in run/ before the pristine
                            copy is taken (the console entry is not implied)
  ini = ["memory_limit=512M"]   -d settings for both engines
  ignore = ["storage/logs/*"]   written files left out of the comparison

  [[mask]]                  a regex replacement (python `re` syntax)
  name, pattern, replace
  in = ["stdout", "stderr", "files", "paths"]   where (default: the first three;
                            "paths" rewrites the names of written files)
  commands = ["id", ...]    only for these commands (default: all)
  builtin = "now-epoch" | "now-datetime"
                            instead of pattern: 10-digit Unix times, or
                            Y-m-d H:i:s (UTC) datetimes, taken during the
                            command's run or a whole number of minutes after
                            it (expiries, schedules): <NOW>, <NOW+3600s>

  [[command]]
  id = "about"              unique, the ratchet's key
  args = "debug:router hello"   shell-split arguments to the console entry
  argv = ["eval", "echo 1;"]    or the arguments as a list
  script = "console/x.php"  run this script instead of the console entry
  network = true            needs the internet: run only with --network
                            (a listed one stays listed when skipped)
"""

import datetime
import difflib
import fnmatch
import hashlib
import os
import re
import shlex
import shutil
import sqlite3
import stat
import subprocess
import sys
import time
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
TIMEOUT = 300
# A time "of the run" is within SLACK seconds of the command's start and end,
# or a whole number of minutes (a TTL, a schedule) up to HORIZON after it.
SLACK = 2
HORIZON = 400 * 86400


# --- the console list -------------------------------------------------------


def load(app):
    path = HERE / app["name"] / app.get("console_list", "console.toml")
    with open(path, "rb") as f:
        spec = tomllib.load(f)
    ids = [c["id"] for c in spec.get("command", [])]
    dup = {i for i in ids if ids.count(i) > 1}
    if dup:
        sys.exit(f"{path}: duplicate command id(s): {', '.join(sorted(dup))}")
    for c in spec.get("command", []):
        if "args" in c and "argv" in c:
            sys.exit(f"{path}: command {c['id']}: give args or argv, not both")
        c["argv"] = c["argv"] if "argv" in c else shlex.split(c.get("args", ""))
    for m in spec.get("mask", []):
        b = m.get("builtin")
        if b:
            if b not in BUILTINS:
                sys.exit(f"{path}: mask {m['name']}: unknown builtin {b!r}")
            m["re"], m["timed"] = BUILTINS[b]
        else:
            m["re"], m["sub"] = re.compile(m["pattern"]), m["replace"]
        m.setdefault("in", ["stdout", "stderr", "files"])
    return spec


def run_offset(t, window):
    """The offset of time t from the run `window` (start, end): 0 when t is
    within it, else a whole number of minutes ahead that puts it there (an
    expiry, a schedule); None when t is no time of the run. Only ahead: a run
    that spans a minute boundary would also take fixed past dates on the
    minute (post dates) for times of the run."""
    start, end = window[0] - SLACK, window[1] + SLACK
    if start <= t <= end:
        return 0
    d = round((t - (start + end) / 2) / 60) * 60
    if 0 < d <= HORIZON and start <= t - d <= end:
        return d
    return None


def now_tag(tag, t, window, orig):
    d = run_offset(t, window)
    if d is None:
        return orig
    return f"<{tag}>" if d == 0 else f"<{tag}{d:+d}s>"


def now_epoch(window):
    return lambda m: now_tag("NOW", int(m.group(0)), window, m.group(0))


def now_datetime(window):
    def sub(m):
        try:
            t = datetime.datetime.strptime(m.group(1) + " " + m.group(2), "%Y-%m-%d %H:%M:%S")
        except ValueError:
            return m.group(0)
        return now_tag("NOWDT", t.replace(tzinfo=datetime.timezone.utc).timestamp(), window, m.group(0))

    return sub


# name -> (regex, replacement factory taking the command's run window)
BUILTINS = {
    "now-epoch": (re.compile(r"(?<!\d)1[789]\d{8}(?!\d)"), now_epoch),
    "now-datetime": (re.compile(r"(\d{4}-\d\d-\d\d)[ T](\d\d:\d\d:\d\d)"), now_datetime),
}


# --- the work directory -----------------------------------------------------


class Tree:
    """The non-shared part of run/: walked, compared and restored by stat."""

    def __init__(self, root, shared):
        self.root = root
        self.shared = {Path(s) for s in shared}
        self.above = {p for s in self.shared for p in s.parents if p != Path(".")}

    def walk(self):
        """{relative path: lstat} of every file, symlink and directory."""
        out = {}
        stack = [Path(".")]
        while stack:
            d = stack.pop()
            try:
                entries = list(os.scandir(self.root / d))
            except FileNotFoundError:
                continue
            for e in entries:
                rel = d / e.name
                if rel in self.shared:
                    continue
                st = e.stat(follow_symlinks=False)
                out[rel] = st
                if stat.S_ISDIR(st.st_mode):
                    stack.append(rel)
        return out


def key(st):
    # Any write, rename or chmod changes the ctime; a replaced file has a new inode.
    return (st.st_ino, st.st_ctime_ns, st.st_size, st.st_mode)


def remove(path):
    if path.is_dir() and not path.is_symlink():
        shutil.rmtree(path)
    elif path.exists() or path.is_symlink():
        path.unlink()


def clone(src, dst):
    """Copy a tree or file, keeping symlinks and times (APFS clone on macOS)."""
    remove(dst)
    dst.parent.mkdir(parents=True, exist_ok=True)
    if sys.platform == "darwin":
        subprocess.run(["cp", "-cpR", str(src), str(dst)], check=True)
    else:
        subprocess.run(["cp", "-a", "--reflink=auto", str(src), str(dst)], check=True)


class Work:
    def __init__(self, app, spec, php):
        self.app, self.spec, self.php = app, spec, php
        self.src = HERE / app["name"]
        self.dir = Path(os.environ.get("RHP_APPS_WORK") or REPO / "target" / "apps-console") / app["name"]
        self.run_dir = self.dir / "run"
        self.pristine = self.dir / "pristine"
        self.out = self.dir / "out"
        self.shared = spec.get("shared", [])
        self.tree = Tree(self.run_dir, self.shared)
        self.ptree = Tree(self.pristine, self.shared)

    def setup(self):
        """run/: shared paths cloned (when changed), the rest copied, prepared."""
        self.dir.mkdir(parents=True, exist_ok=True)
        stamp_file = self.dir / "shared.stamp"
        stamp = repr([self.stamp(s) for s in self.shared])
        copied = all((self.run_dir / s).exists() for s in self.shared)
        if not copied or not stamp_file.is_file() or stamp_file.read_text() != stamp:
            remove(self.run_dir)
            for s in self.shared:
                if not (self.src / s).exists():
                    sys.exit(f"[{self.app['name']}] {s} is missing; run `python3 tests/apps/apps.py setup {self.app['name']}`")
                print(f"[{self.app['name']}] copy {s}", flush=True)
                clone(self.src / s, self.run_dir / s)
            stamp_file.write_text(stamp)
        for rel in sorted(self.tree.walk(), key=lambda p: len(p.parts)):
            if rel not in self.tree.above:
                remove(self.run_dir / rel)
        src_tree = Tree(self.src, self.shared)
        for rel, st in sorted(src_tree.walk().items()):
            dst = self.run_dir / rel
            if rel in self.tree.above:
                continue
            if stat.S_ISDIR(st.st_mode):
                dst.mkdir(parents=True, exist_ok=True)
            else:
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(self.src / rel, dst, follow_symlinks=False)
        for c in self.spec.get("clear", []):
            remove(self.run_dir / c)
        for c in self.spec.get("prepare", []):
            argv = [self.php, "-n", *ini_args(self.spec), *shlex.split(c)]
            print(f"[{self.app['name']}] prepare: {shlex.join(argv)}", flush=True)
            self.fresh_home()
            p = subprocess.run(argv, cwd=self.run_dir, env=self.env(), stdin=subprocess.DEVNULL, capture_output=True)
            if p.returncode != 0:
                sys.exit(f"prepare failed ({p.returncode}):\n{p.stdout.decode(errors='replace')}{p.stderr.decode(errors='replace')}")
        remove(self.pristine)
        self.pristine.mkdir()
        for rel, st in sorted(self.tree.walk().items()):
            dst = self.pristine / rel
            if stat.S_ISDIR(st.st_mode):
                dst.mkdir(parents=True, exist_ok=True)
            else:
                shutil.copy2(self.run_dir / rel, dst, follow_symlinks=False)
        self.index = {rel: key(st) for rel, st in self.tree.walk().items()}
        self.pindex = self.ptree.walk()

    def stamp(self, s):
        p = self.src / s
        if p.is_dir():
            return sorted((e.name, e.stat(follow_symlinks=False).st_mtime_ns) for e in os.scandir(p)) + [p.stat().st_mtime_ns]
        st = p.stat()
        return (st.st_size, st.st_mtime_ns)

    def changes(self):
        """{relative path: lstat or None (removed)} since the last restore."""
        now = self.tree.walk()
        out = {rel: st for rel, st in now.items() if self.index.get(rel) != key(st)}
        out.update({rel: None for rel in self.index if rel not in now})
        return out

    def restore(self, changed):
        """Put back what `changed` (from changes()) touched."""
        for rel in sorted(changed, key=lambda p: len(p.parts)):
            if rel not in self.pindex:
                remove(self.run_dir / rel)
        for rel in sorted(changed, key=lambda p: len(p.parts)):
            if rel not in self.pindex:
                continue
            dst = self.run_dir / rel
            if stat.S_ISDIR(self.pindex[rel].st_mode):
                if not dst.is_dir() or dst.is_symlink():
                    remove(dst)
                    dst.mkdir(parents=True)
                    # A removed directory: put its whole pristine subtree back.
                    for sub, st in self.pindex.items():
                        if rel in sub.parents:
                            sd = self.run_dir / sub
                            if stat.S_ISDIR(st.st_mode):
                                sd.mkdir(parents=True, exist_ok=True)
                            else:
                                sd.parent.mkdir(parents=True, exist_ok=True)
                                shutil.copy2(self.pristine / sub, sd, follow_symlinks=False)
                os.utime(dst, ns=(self.pindex[rel].st_atime_ns, self.pindex[rel].st_mtime_ns))
            else:
                remove(dst)
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(self.pristine / rel, dst, follow_symlinks=False)
        self.index = {rel: key(st) for rel, st in self.tree.walk().items()}
        if set(self.index) != set(self.pindex):
            sys.exit(f"[{self.app['name']}] restoring run/ failed: {sorted(set(self.index) ^ set(self.pindex))[:5]}")

    def fresh_home(self):
        for d in ("home", "tmp"):
            remove(self.dir / d)
            (self.dir / d).mkdir()

    def env(self):
        # A minimal, fixed environment (an agent's or CI's variables change
        # some consoles' output): no colours, a fixed width, UTC.
        return {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(self.dir / "home"),
            "TMPDIR": str(self.dir / "tmp"),
            "TZ": "UTC",
            "LANG": "C",
            "LC_ALL": "C",
            "COLUMNS": "120",
            "NO_COLOR": "1",
            "TERM": "dumb",
        }


def ini_args(spec):
    return [a for i in spec.get("ini", []) for a in ("-d", i)]


# --- one command --------------------------------------------------------------


def file_text(path, st):
    if st is None:
        return "<deleted>"
    if stat.S_ISDIR(st.st_mode):
        return "<dir>"
    if stat.S_ISLNK(st.st_mode):
        return f"<symlink to {os.readlink(path)}>"
    data = path.read_bytes()
    if data.startswith(b"SQLite format 3\0"):
        con = sqlite3.connect(f"file:{path}?mode=ro&immutable=1", uri=True)
        try:
            return "<sqlite dump>\n" + "\n".join(con.iterdump()) + "\n"
        finally:
            con.close()
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return f"<binary, {len(data)} bytes, md5 {hashlib.md5(data).hexdigest()}>"


def masked(text, where, cid, masks, binaries, window=None):
    """`window`: the command's (start, end) Unix times, for the builtin masks."""
    if where != "paths":
        for b in binaries:
            text = text.replace(b, "<PHP_BINARY>")
    for m in masks:
        if where in m["in"] and ("commands" not in m or cid in m["commands"]):
            if "timed" in m:
                if window is not None:
                    text = m["re"].sub(m["timed"](window), text)
            else:
                text = m["re"].sub(m["sub"], text)
    return text


def run_one(work, engine, cmd, binaries):
    spec, cid = work.spec, cmd["id"]
    masks = spec.get("mask", [])
    entry = [cmd["script"]] if "script" in cmd else [work.app["console"]]
    argv = [engine, "-n", *ini_args(spec), *entry, *cmd["argv"]]
    work.fresh_home()
    start = time.time()
    try:
        p = subprocess.run(argv, cwd=work.run_dir, env=work.env(), capture_output=True, stdin=subprocess.DEVNULL, timeout=TIMEOUT)
        out, err, code = p.stdout, p.stderr, p.returncode
    except subprocess.TimeoutExpired as e:
        out, err, code = e.stdout or b"", (e.stderr or b"") + f"\n<TIMEOUT after {TIMEOUT}s>".encode(), "timeout"
    window = (start, time.time())
    changed = work.changes()
    files = {}
    for rel, st in changed.items():
        if st is not None and stat.S_ISDIR(st.st_mode) and rel in work.pindex:
            continue  # a directory whose entries changed: they are listed themselves
        if any(fnmatch.fnmatch(str(rel), g) for g in spec.get("ignore", [])):
            continue
        name = masked(str(rel), "paths", cid, masks, binaries, window)
        text = masked(file_text(work.run_dir / rel, st), "files", cid, masks, binaries, window)
        if st is not None and not stat.S_ISLNK(st.st_mode):
            # Permissions too (mkdir, chmod, tempnam modes).
            text = f"<mode {stat.S_IMODE(st.st_mode):04o}>\n{text}"
        files.setdefault(name, []).append(text)
    work.restore(changed)
    dec = lambda b, where: masked(b.decode("utf-8", "replace"), where, cid, masks, binaries, window)
    return {
        "code": code,
        "stdout": dec(out, "stdout"),
        "stderr": dec(err, "stderr"),
        "files": {k: sorted(v) for k, v in files.items()},
    }


def describe_diff(ref, cand):
    parts = []
    if ref["code"] != cand["code"]:
        parts.append(f"exit code: php={ref['code']} candidate={cand['code']}\n")

    def udiff(a, b, what):
        d = difflib.unified_diff(a.splitlines(keepends=True), b.splitlines(keepends=True), f"php {what}", f"candidate {what}", n=2)
        return "".join(x if x.endswith("\n") else x + "\n" for x in d)

    for stream in ("stdout", "stderr"):
        if ref[stream] != cand[stream]:
            parts.append(udiff(ref[stream], cand[stream], stream))
    for name in sorted(set(ref["files"]) | set(cand["files"])):
        a, b = ref["files"].get(name), cand["files"].get(name)
        if a == b:
            continue
        if a is None or b is None:
            parts.append(f"file {name}: only written by {'candidate' if a is None else 'php'}\n")
            continue
        parts.append(udiff("\n--\n".join(a), "\n--\n".join(b), name))
    return "".join(parts)


# --- the battery ---------------------------------------------------------------


def known_path(app):
    return HERE / app["name"] / app.get("known_console", "known-passing-console.txt")


def read_known(path):
    if not path.is_file():
        return None
    return [l.strip() for l in path.read_text().splitlines() if l.strip() and not l.startswith("#")]


def write_known(path, app, ids):
    head = (
        f"# Console commands of {app['name']} that answer under rhp exactly as under php\n"
        "# (`python3 tests/apps/apps.py console`). Regenerate with --write-known.\n"
    )
    path.write_text(head + "".join(f"{i}\n" for i in sorted(ids)))


def battery(app, candidate, only, quiet, write, network=False):
    name = app["name"]
    spec = load(app)
    php = os.environ.get("RHP_PHP", "php")
    real = lambda b: subprocess.run([b, "-n", "-r", "echo PHP_BINARY;"], capture_output=True, text=True).stdout
    # Absolute paths only: a bare "php" would also match inside "phpdbg".
    found = (php, shutil.which(php), real(php), candidate, shutil.which(candidate), real(candidate))
    binaries = sorted({b for b in found if b and b.startswith("/")}, key=len, reverse=True)
    cmds = spec.get("command", [])
    if only:
        bad = only - {c["id"] for c in cmds}
        if bad:
            sys.exit(f"[{name}] unknown command id(s): {', '.join(sorted(bad))}")
        cmds = [c for c in cmds if c["id"] in only]
    offline = [c["id"] for c in cmds if c.get("network") and not network]
    if offline:
        print(f"[{name}] skipped (need the network; --network runs them): {' '.join(offline)}", flush=True)
        cmds = [c for c in cmds if c["id"] not in offline]
    known = read_known(known_path(app))
    if not write and known is None:
        print(f"[{name}] no {known_path(app).relative_to(REPO)}; create it with `python3 tests/apps/apps.py console {name} --write-known`")
        return False
    if known is not None:
        stale = set(known) - {c["id"] for c in spec.get("command", [])}
        if stale:
            print(f"[{name}] {known_path(app).name} lists unknown command(s): {', '.join(sorted(stale))}")
            return False
    work = Work(app, spec, php)
    work.setup()
    remove(work.out)
    work.out.mkdir(parents=True)
    same_ids = []
    for c in cmds:
        ref = run_one(work, php, c, binaries)
        cand = run_one(work, candidate, c, binaries)
        for who, r in (("php", ref), ("cand", cand)):
            files = "".join(f"--file {k}--\n" + "\n--\n".join(v) for k, v in sorted(r["files"].items()))
            (work.out / f"{c['id']}.{who}.txt").write_text(f"exit {r['code']}\n--stdout--\n{r['stdout']}--stderr--\n{r['stderr']}{files}")
        same = ref == cand
        if same:
            same_ids.append(c["id"])
        what = " ".join(([c["script"]] if "script" in c else []) + c["argv"])
        print(f"{'SAME' if same else 'DIFF'}  php={ref['code']} rhp={cand['code']}  {c['id']:<16} {what[:80]}", flush=True)
        if not same:
            d = describe_diff(ref, cand)
            (work.out / f"{c['id']}.diff").write_text(d)
            if not quiet:
                lines = d.splitlines()
                print("\n".join("      " + (l if len(l) <= 200 else l[:200] + " ...") for l in lines[:30]))
                if len(lines) > 30:
                    print(f"      ... ({len(lines) - 30} more lines in {work.out / (c['id'] + '.diff')})")
    print(f"== {name}: {len(same_ids)}/{len(cmds)} commands identical (outputs in {work.out})", flush=True)
    if write:
        if only:
            sys.exit("--write-known needs the whole list (no --only)")
        # Skipped network commands keep their entries.
        ids = same_ids + [i for i in (known or []) if i in offline]
        write_known(known_path(app), app, ids)
        print(f"[{name}] wrote {known_path(app).relative_to(REPO)} ({len(ids)} commands)")
        return True
    ran = {c["id"] for c in cmds}
    lost = [i for i in known if i in ran and i not in same_ids]
    new = [i for i in same_ids if i not in known]
    if new:
        print(f"[{name}] newly SAME (add to {known_path(app).name}): {' '.join(new)}")
    if lost:
        print(f"[{name}] REGRESSED (listed in {known_path(app).name}, now DIFF): {' '.join(lost)}")
        return False
    return True


def main(apps, argv):
    """`apps.py console` with the selected apps and the remaining arguments."""
    candidate = os.environ.get("RHP") or str(REPO / "target/release/rhp")
    only, quiet, write, network = None, False, False, False
    it = iter(argv)
    for a in it:
        if a == "--candidate":
            candidate = next(it)
        elif a == "--only":
            only = set(next(it).split(","))
        elif a == "--quiet":
            quiet = True
        elif a == "--write-known":
            write = True
        elif a == "--network":
            network = True
        else:
            sys.exit(f"console: unknown option {a!r}; see tests/apps/console.py --help")
    # Commands run in the work directory: a relative path would not resolve there.
    if os.sep in candidate:
        candidate = str(Path(candidate).resolve())
    failed = [a["name"] for a in apps if not battery(a, candidate, only, quiet, write, network)]
    if failed:
        sys.exit(f"console battery failed for: {', '.join(failed)}")


if __name__ == "__main__":
    if len(sys.argv) < 2 or sys.argv[1] in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
    import apps as _apps

    # Option values (--candidate BIN, --only IDS) are not app names.
    rest = sys.argv[1:]
    names, opts, i = [], [], 0
    while i < len(rest):
        if rest[i] in ("--candidate", "--only"):
            opts += rest[i : i + 2]
            i += 2
        elif rest[i].startswith("-"):
            opts.append(rest[i])
            i += 1
        else:
            names.append(rest[i])
            i += 1
    main(_apps.select(_apps.load(), names), opts)
