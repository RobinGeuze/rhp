#!/usr/bin/env python3
"""Run Composer's commands under php and under rhp, and compare.

    python3 tests/apps/composer_diff.py [--work DIR] [--suite NAME,...] [COMMAND...]

Runs suites of Composer commands (SUITES below; all of them, or those named
with --suite) through `php composer` and then `rhp composer`, and compares
each command's stdout + stderr and exit code, with timings, memory figures,
dates and build-environment facts masked (mask() below), and prints SAME or
DIFF, with the CPU time and peak RSS under each engine. After a suite it
compares the files it lists (composer.lock, vendor/, ...) as the two runs
left them.

  basic         a small project that requires monolog/monolog and
                psr/container: 29 commands, from `about` to `archive`
  stream        the same project without curl_multi (Composer's own https
                client on rhp's stream layer and openssl)
  ziparchive    the same project with no `unzip`/`7z` on PATH (Composer
                extracts with ZipArchive, as on a minimal CI image)
  symfony-7.4   the fixture app's composer.json and lock: `install` runs
                Symfony Flex (a plugin) and the auto-scripts (`cache:clear`)
                under the engine; then `dump-autoload -o`, `run-script`
  laravel-13    the same for Laravel: `install`, `dump-autoload -o`,
                `run-script post-autoload-dump` (`artisan package:discover`)

A DIFF listed in KNOWN (with its reason) prints KNOWN and does not fail the
run; a KNOWN entry that matched prints FIXED, a cue to drop it. Exit code 0
when everything else matched.

Both engines run in the same directories with the same, initially empty
COMPOSER_HOME and cache, and through a link to their binary at the same path
(WORK/bin/php), so PHP_BINARY is the same and the text Symfony Console wraps
is the same length. `show --platform` is compared on the extensions both
engines load.

Composer is $COMPOSER or `composer` from PATH (a phar or its launcher), php is
$RHP_PHP or `php`, rhp is $RHP or target/release/rhp. Needs network access
(packagist.org). Each engine's outputs are kept in WORK/out/<suite>.
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

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.dont_write_bytecode = True  # no __pycache__ in the tree
sys.path.insert(0, str(HERE))
from tools import spawn  # noqa: E402

TIMEOUT = 900

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

# A step is a Composer command (a list) or ("php", [script args]).
USE = ("php", ["use.php"])
NO_CURL = ["-d", "disable_functions=curl_multi_exec,curl_multi_init"]

SUITES = {
    "basic": {"project": "basic", "steps": CMDS, "compare": ["composer.lock", "vendor"]},
    "stream": {
        "project": "basic",
        "engine_args": NO_CURL,
        "steps": [["install", "--no-cache"], USE, ["show", "--tree"],
                  ["create-project", "--no-cache", "psr/log", "cp-log", "3.0.2"], ["audit"]],
        "compare": ["composer.lock", "vendor", "cp-log"],
    },
    "ziparchive": {
        "project": "basic",
        "no_unzip": True,
        "steps": [["install", "--no-cache"], USE, ["create-project", "--no-cache", "psr/log", "cp-log", "3.0.2"]],
        "compare": ["composer.lock", "vendor", "cp-log"],
    },
    "symfony-7.4": {
        "project": "symfony-7.4",
        "steps": [["install"], ["validate"], ["dump-autoload", "-o"], ["run-script", "auto-scripts"],
                  ("php", ["bin/console", "--version"])],
        # var/cache (what cache:clear wrote) is the request harness's business.
        "compare": ["composer.lock", "symfony.lock", "vendor"],
    },
    "laravel-13": {
        "project": "laravel-13",
        "steps": [["install"], ["dump-autoload", "-o"], ["run-script", "post-autoload-dump"],
                  ("php", ["artisan", "--version"])],
        "compare": ["composer.lock", "vendor", "bootstrap/cache/packages.php", "bootstrap/cache/services.php"],
    },
}

# Differences with a known cause in rhp, by (suite, step name); a step name
# of None covers the whole suite. Remove an entry once it prints FIXED.
SOCKETS = "counts the ext-sockets suggestion: rhp does not report sockets loaded"
KNOWN = {
    ("basic", "composer install"): SOCKETS,
    ("stream", "composer install --no-cache"): SOCKETS,
    ("ziparchive", "composer install --no-cache"): SOCKETS,
    ("basic", "composer show --platform"): "no lib-* entries from ReflectionExtension::info(), which prints nothing",
    ("laravel-13", None): "league/flysystem-local requires ext-fileinfo, which rhp lacks",
}

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

# What a build environment decides, not the engine: the OpenSSL headers each
# engine was compiled against (Homebrew's php: 4.0.3, rhp: what it links).
OPENSSL = [
    (re.compile(r"^(OpenSSL version: ).*$", re.M), r"\1<OPENSSL>"),
    (re.compile(r"^(lib-openssl\s+).*$", re.M), r"\1<OPENSSL>"),
    (re.compile(r'("name": "lib-openssl",\s*"version": )"[^"]*"'), r'\1"<OPENSSL>"'),
]

# The library entries of `show --platform` and the extension that reports
# them (Composer's PlatformRepository), where the name does not say it.
LIB_OWNER = {"lib-icu": "intl", "lib-libxml": "libxml", "lib-libsodium": "sodium", "lib-libxslt": "xsl",
             "lib-openssl": "openssl", "lib-pcre": "pcre", "lib-zlib": "zlib", "lib-bz2": "bz2",
             "lib-iconv": "iconv", "lib-gmp": "gmp", "lib-curl": "curl"}


def mask(text, binaries):
    for b in binaries:
        text = text.replace(b, "<PHP_BINARY>")
    text = re.sub(r"Memory usage: [\d.]+MiB \(peak: [\d.]+MiB\), time: [\d.]+s", "<MEM>", text)
    text = re.sub(r"\d+(\.\d+)?\s*(ms|s|seconds)\b", "<T>", text)
    for pat, rep in OPENSSL:
        text = pat.sub(rep, text)
    # Random per run: the secrets a recipe or create-project writes, and the
    # session id Symfony Flex sends to its endpoint.
    text = re.sub(r"\b(APP_SECRET|APP_KEY)=\S+", r"\1=<SECRET>", text)
    text = re.sub(r"(Flex session id:? ?)\S+", r"\1<ID>", text)
    return re.sub(r"\b\d{4}-\d\d-\d\d[ T]\d\d:\d\d:\d\d\b", "<DATE>", text)


def platform_owner(name):
    """The extension a `show --platform` entry belongs to, or None."""
    if name.startswith("ext-"):
        return name[4:].lower()
    if name.startswith("lib-"):
        for lib, ext in LIB_OWNER.items():
            if name == lib or name.startswith(lib + "-"):
                return ext
        return name[4:].split("-")[0].lower()
    return None


def only_common(text, common):
    """`show --platform` with the entries of extensions that only one engine
    loads left out, and the columns' padding (set by the longest entry)
    collapsed to one space."""
    out = []
    for line in text.splitlines(keepends=True):
        m = re.match(r"^((?:ext|lib)-\S+)\s", line)
        if m and platform_owner(m.group(1)) not in common:
            continue
        if re.match(r"^(ext|lib|php|composer)\S*\s", line):
            line = re.sub(r"  +", " ", line)
        out.append(line)
    return "".join(out)


def loaded_extensions(engine):
    out = subprocess.run([engine, "-r", "echo implode(',', get_loaded_extensions());"],
                         capture_output=True, text=True).stdout
    return {e.lower().replace(' ', '-') for e in out.split(",") if e}


def engine_binary(engine):
    """The engine's executable (PHP_BINARY): not a wrapper script around it."""
    real = subprocess.run([engine, "-r", "echo PHP_BINARY;"], capture_output=True, text=True).stdout
    return real if real and os.path.isfile(real) else shutil.which(engine) or engine


def link_binary(engine, bindir):
    """WORK/bin/php: a hard link to (or copy of) the engine, so both engines
    run as the same path. A symlink would not do: both resolve PHP_BINARY
    through realpath()."""
    bindir.mkdir(parents=True, exist_ok=True)
    dst = bindir / "php"
    if dst.exists() or dst.is_symlink():
        dst.unlink()
    src = engine_binary(engine)
    try:
        os.link(src, dst)
    except OSError:
        shutil.copy2(src, dst)
    return str(dst)


def no_unzip_path(work):
    """A PATH holding the usual tools, without unzip and 7z."""
    d = work / "pathbin"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    for tool in ("sh", "bash", "env", "git", "tar", "gzip", "bzip2", "xz", "uname", "which", "cat", "ls",
                 "find", "grep", "sort", "head", "md5", "md5sum", "id", "hg", "svn"):
        p = shutil.which(tool)
        if p:
            (d / tool).symlink_to(p)
    return str(d)


def make_project(name, proj):
    if name == "basic":
        (proj / "src").mkdir(parents=True)
        (proj / "initdir").mkdir()
        (proj / "composer.json").write_text(json.dumps(COMPOSER_JSON, indent=4) + "\n")
        (proj / "src" / "Hello.php").write_text(
            "<?php\nnamespace App;\nclass Hello { public static function hi(): string { return 'hi'; } }\n"
        )
        (proj / "use.php").write_text(USE_PHP)
        return
    # A fixture app: its committed files, without vendor/ and caches.
    app = HERE / name
    files = subprocess.run(["git", "ls-files", "-z", "."], cwd=app, capture_output=True, check=True).stdout
    for rel in filter(None, files.decode().split("\0")):
        (proj / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(app / rel, proj / rel)


def run_suite(suite, engine, composer, work, only, binaries, common):
    proj, home = work / "project", work / "home"
    for d in (proj, home):
        shutil.rmtree(d, ignore_errors=True)
    proj.mkdir(parents=True)
    home.mkdir()
    make_project(suite["project"], proj)
    php = link_binary(engine, work / "bin")
    env = dict(os.environ, COMPOSER_HOME=str(home), COMPOSER_CACHE_DIR=str(home / "cache"),
               COMPOSER_NO_INTERACTION="1", NO_COLOR="1", COLUMNS="120")
    for k in ("COMPOSER", "COMPOSER_VENDOR_DIR", "COMPOSER_BIN_DIR"):
        env.pop(k, None)
    if suite.get("no_unzip"):
        env["PATH"] = no_unzip_path(work)
    pre = [php, *suite.get("engine_args", [])]
    results = []
    for step in suite["steps"]:
        if isinstance(step, tuple):
            name, argv = "(php " + " ".join(step[1]) + ")", [php, *step[1]]
        else:
            name, argv = "composer " + " ".join(step), [*pre, composer, "--no-ansi", "--no-interaction", *step]
        if only and name.split()[1] not in only:
            continue
        out, err, code, cpu, rss = spawn(argv, proj, env, TIMEOUT)
        text = out.decode("utf-8", "replace") + "\n--stderr--\n" + err.decode("utf-8", "replace")
        if step[:2] == ["show", "--platform"]:
            text = only_common(text, common)
        results.append((name, code, mask(text, binaries), cpu, rss))
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


def same_path(a, b):
    if a.is_dir() and b.is_dir():
        return same_tree(a, b)
    if not a.exists() and not b.exists():
        return True
    return a.is_file() and b.is_file() and filecmp.cmp(a, b, shallow=False)


def verdict(suite_name, name, same, counts):
    known = KNOWN.get((suite_name, name)) or KNOWN.get((suite_name, None))
    if same:
        counts["same"] += 1
        return "FIXED" if known and (suite_name, name) in KNOWN else "SAME"
    if known:
        counts["known"] += 1
        return "KNOWN"
    counts["diff"] += 1
    return "DIFF"


def main(argv):
    work, suites, only = None, list(SUITES), []
    it = iter(argv)
    for a in it:
        if a == "--work":
            work = Path(next(it)).resolve()
        elif a == "--suite":
            suites = next(it).split(",")
        else:
            only.append(a)
    bad = [s for s in suites if s not in SUITES]
    if bad:
        sys.exit(f"composer_diff: unknown suite(s) {', '.join(bad)} (known: {', '.join(SUITES)})")
    work = work or Path(tempfile.mkdtemp(prefix="composer-diff-"))
    work.mkdir(parents=True, exist_ok=True)
    php = os.environ.get("RHP_PHP", "php")
    rhp = os.environ.get("RHP", str(REPO / "target/release/rhp"))
    composer = os.environ.get("COMPOSER") or shutil.which("composer")
    if not composer:
        sys.exit("composer_diff: no composer on PATH (set COMPOSER)")
    binaries = sorted({b for b in (rhp, engine_binary(php), shutil.which(php)) if b and os.path.isabs(b)},
                      key=len, reverse=True)
    common = loaded_extensions(php) & loaded_extensions(rhp)
    counts = {"same": 0, "known": 0, "diff": 0}
    for sname in suites:
        suite = SUITES[sname]
        print(f"== {sname}", flush=True)
        ref, ref_final = run_suite(suite, php, composer, work, only, binaries, common)
        cand, cand_final = run_suite(suite, rhp, composer, work, only, binaries, common)
        out = work / "out" / sname
        shutil.rmtree(out, ignore_errors=True)
        out.mkdir(parents=True)
        for i, ((name, pc, po, pcpu, prss), (_, rc, ro, rcpu, rrss)) in enumerate(zip(ref, cand)):
            (out / f"{i:02}.php.txt").write_text(po)
            (out / f"{i:02}.rhp.txt").write_text(ro)
            v = verdict(sname, name, pc == rc and po == ro, counts)
            usage = f"cpu {pcpu:.1f}s/{rcpu:.1f}s rss {prss:.0f}/{rrss:.0f}MB"
            print(f"{v:<5}  php={pc} rhp={rc}  {usage:<34} {i:02} {name}", flush=True)
        if only:
            continue
        for rel in suite["compare"]:
            v = verdict(sname, rel, same_path(ref_final / rel, cand_final / rel), counts)
            print(f"{v:<5}  {rel} after the last command", flush=True)
    for (s, name), why in KNOWN.items():
        if s in suites:
            print(f"known: {s} {name or '(whole suite)'}: {why}")
    print(f"{counts['same']} same, {counts['known']} known, {counts['diff']} different; outputs in {work / 'out'}")
    return 1 if counts["diff"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
