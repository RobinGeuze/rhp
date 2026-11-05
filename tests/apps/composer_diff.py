#!/usr/bin/env python3
"""Run Composer's commands under php and under rhp, and compare.

    python3 tests/apps/composer_diff.py [--work DIR] [COMMAND...]

Creates a small project that requires monolog/monolog and psr/container, and
runs each command of CMDS (or only those named) through `php composer` and
then `rhp composer`. Both engines run in the same directory with the same,
initially empty COMPOSER_HOME and cache, so paths match. After each command it
compares stdout + stderr and the exit code, with timings, memory figures and
dates masked, and prints SAME or DIFF. After `install` it also runs a script
that uses monolog. Last, it compares the composer.lock and vendor/ the two
runs left. Exit code 0 when everything matched.

Composer is $COMPOSER or `composer` from PATH (a phar or its launcher), php is
$RHP_PHP or `php`, rhp is $RHP or target/release/rhp. Needs network access
(packagist.org). Each engine's outputs are kept in WORK/out.
"""

import filecmp
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent

CMDS = [
    ["--version"],
    ["about"],
    ["list"],
    ["validate"],
    ["install"],
    ["show"],
    ["show", "--tree"],
    ["show", "monolog/monolog"],
    ["show", "--platform"],
    ["licenses"],
    ["depends", "psr/log"],
    ["why-not", "psr/log", "1.0"],
    ["check-platform-reqs"],
    ["dump-autoload", "-o"],
    ["config", "--list"],
    ["status"],
    ["diagnose"],
    ["require", "psr/clock:^1.0"],
    ["remove", "psr/clock"],
    ["update"],
    ["outdated", "--direct"],
    ["audit"],
    ["run-script", "hello"],
    ["exec", "--list"],
    ["reinstall", "psr/log"],
    ["search", "monolog", "--only-name"],
    ["fund"],
    ["archive", "--format=zip", "--dir=dist", "--file=proj"],
    ["init", "--name=x/y", "--working-dir=initdir"],
]

COMPOSER_JSON = {
    "name": "rhp/composer-test",
    "description": "A small project for running Composer under rhp",
    "type": "project",
    "license": "MIT",
    "require": {"monolog/monolog": "^3.9", "psr/container": "^2.0"},
    "autoload": {"psr-4": {"App\\": "src/"}},
    "scripts": {"hello": "@php -r \"echo PHP_VERSION, ' ', PHP_SAPI, PHP_EOL;\""},
}

USE_PHP = """<?php
require __DIR__ . '/vendor/autoload.php';
$log = new Monolog\\Logger('t');
$h = new Monolog\\Handler\\StreamHandler('php://stdout');
$h->setFormatter(new Monolog\\Formatter\\LineFormatter("%channel%.%level_name%: %message% %context%\\n"));
$log->pushHandler($h);
$log->info('hello', ['a' => 1]);
echo App\\Hello::hi(), "\\n";
"""


def mask(text, binaries):
    for b in binaries:
        text = text.replace(b, "<PHP_BINARY>")
    text = re.sub(r"Memory usage: [\d.]+MiB \(peak: [\d.]+MiB\), time: [\d.]+s", "<MEM>", text)
    text = re.sub(r"\d+(\.\d+)?\s*(ms|s|seconds)\b", "<T>", text)
    return re.sub(r"\b\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d\b", "<DATE>", text)


def run_engine(engine, composer, work, only, binaries):
    proj, home = work / "project", work / "home"
    for d in (proj, home):
        shutil.rmtree(d, ignore_errors=True)
    (proj / "src").mkdir(parents=True)
    (proj / "initdir").mkdir()
    home.mkdir()
    (proj / "composer.json").write_text(json.dumps(COMPOSER_JSON, indent=4) + "\n")
    (proj / "src" / "Hello.php").write_text(
        "<?php\nnamespace App;\nclass Hello { public static function hi(): string { return 'hi'; } }\n"
    )
    (proj / "use.php").write_text(USE_PHP)
    env = dict(os.environ, COMPOSER_HOME=str(home), COMPOSER_CACHE_DIR=str(home / "cache"),
               COMPOSER_NO_INTERACTION="1", NO_COLOR="1", COLUMNS="120")
    results = []

    def run(name, argv):
        try:
            p = subprocess.run(argv, cwd=proj, env=env, capture_output=True, text=True,
                               errors="replace", timeout=600)
            out, code = p.stdout + "\n--stderr--\n" + p.stderr, p.returncode
        except subprocess.TimeoutExpired:
            out, code = "TIMEOUT", -1
        results.append((name, code, mask(out, binaries)))

    for c in CMDS:
        if only and c[0] not in only:
            continue
        run("composer " + " ".join(c), [engine, composer, "--no-ansi", "--no-interaction"] + c)
        if c[0] == "install":
            run("(use.php: monolog)", [engine, "use.php"])
    final = work / ("final-" + Path(engine).name)
    shutil.rmtree(final, ignore_errors=True)
    shutil.copytree(proj, final, symlinks=True)
    return results, final


def same_tree(a, b):
    cmp = filecmp.dircmp(a, b)
    if cmp.left_only or cmp.right_only or cmp.funny_files:
        return False
    _, mismatch, errors = filecmp.cmpfiles(a, b, cmp.common_files, shallow=False)
    return not mismatch and not errors and all(same_tree(a / d, b / d) for d in cmp.common_dirs)


def main(argv):
    work = None
    if argv[:1] == ["--work"]:
        work, argv = Path(argv[1]).resolve(), argv[2:]
    work = work or Path(tempfile.mkdtemp(prefix="composer-diff-"))
    work.mkdir(parents=True, exist_ok=True)
    php = os.environ.get("RHP_PHP", "php")
    rhp = os.environ.get("RHP", str(REPO / "target/release/rhp"))
    composer = os.environ.get("COMPOSER") or shutil.which("composer")
    if not composer:
        sys.exit("composer_diff: no composer on PATH (set COMPOSER)")
    real_php = subprocess.run([php, "-r", "echo PHP_BINARY;"], capture_output=True, text=True).stdout
    binaries = sorted({rhp, real_php, shutil.which(php) or php}, key=len, reverse=True)
    ref, ref_final = run_engine(php, composer, work, argv, binaries)
    cand, cand_final = run_engine(rhp, composer, work, argv, binaries)
    out = work / "out"
    out.mkdir(exist_ok=True)
    failed = 0
    for i, ((name, pc, po), (_, rc, ro)) in enumerate(zip(ref, cand)):
        (out / f"{i:02}.php.txt").write_text(po)
        (out / f"{i:02}.rhp.txt").write_text(ro)
        same = pc == rc and po == ro
        failed += not same
        print(f"{'SAME' if same else 'DIFF'}  php={pc} rhp={rc}  {i:02} {name}")
    for rel in ("composer.lock", "vendor"):
        a, b = ref_final / rel, cand_final / rel
        if a.is_dir() and b.is_dir():
            same = same_tree(a, b)
        else:
            same = a.is_file() and b.is_file() and filecmp.cmp(a, b, shallow=False)
        failed += not same
        print(f"{'SAME' if same else 'DIFF'}  {rel} after the last command")
    print(f"outputs in {out}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
