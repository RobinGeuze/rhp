#!/usr/bin/env python3
"""Drive the pinned fixture apps listed in tests/apps/apps.toml.

    python3 tests/apps/apps.py list                 # app names
    python3 tests/apps/apps.py matrix               # CI matrix (JSON: app, php-extensions)
    python3 tests/apps/apps.py setup [APP...]       # downloads + composer install + setup
    python3 tests/apps/apps.py smoke [APP...]       # console --version + routes under php -S
    python3 tests/apps/apps.py lint [APP...] [--write-known] [-- LINT ARGS]
    python3 tests/apps/apps.py diff [APP...] [--write-known] [-- APP ARGS]

With no APP, every app in the manifest. `lint` runs `rhp-difftest lint` over
the app's `lint` paths with `--check-known <known_refused>` (or
`--write-known` to regenerate the list). `diff` runs `rhp-difftest app`
(the app's requests.toml through php-cgi and `rhp --cgi`) with
`--check-known <known_requests>` (or `--write-known`). The binary is
$RHP_DIFFTEST or target/release/rhp-difftest. CI runs exactly these commands.

`setup` first fetches the app's pinned `[[app.download]]` files (cached in
$RHP_APPS_CACHE, default ~/.cache/rhp-apps), checks each one's sha256 and
unpacks archives (their single top directory stripped) into `to`.

Needs python >= 3.11 (tomllib), php and composer.
"""

import hashlib
import json
import os
import shlex
import shutil
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
import urllib.error
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent


def load():
    with open(HERE / "apps.toml", "rb") as f:
        apps = tomllib.load(f)["app"]
    names = [a["name"] for a in apps]
    if len(set(names)) != len(names):
        sys.exit("apps.toml: duplicate app name")
    for a in apps:
        if not (HERE / a["name"]).is_dir():
            sys.exit(f"apps.toml: no directory tests/apps/{a['name']}")
    return apps


def select(apps, names):
    if not names:
        return apps
    by = {a["name"]: a for a in apps}
    bad = [n for n in names if n not in by]
    if bad:
        sys.exit(f"unknown app(s): {', '.join(bad)} (known: {', '.join(by)})")
    return [by[n] for n in names]


def run(cmd, cwd):
    print(f"[{cwd.name}] $ {cmd}", flush=True)
    subprocess.run(shlex.split(cmd), cwd=cwd, check=True)


def cache_dir():
    c = os.environ.get("RHP_APPS_CACHE")
    return Path(c) if c else Path.home() / ".cache" / "rhp-apps"


def fetch(url, sha256):
    """The cached file for `url`, downloaded if missing; its sha256 checked."""
    path = cache_dir() / f"{sha256[:16]}-{url.rsplit('/', 1)[-1]}"
    if not path.is_file():
        path.parent.mkdir(parents=True, exist_ok=True)
        print(f"downloading {url}", flush=True)
        tmp = path.with_suffix(path.suffix + ".part")
        with urllib.request.urlopen(url, timeout=120) as r, open(tmp, "wb") as f:
            shutil.copyfileobj(r, f)
        tmp.replace(path)
    got = hashlib.sha256(path.read_bytes()).hexdigest()
    if got != sha256:
        path.unlink()
        sys.exit(f"{url}: sha256 {got}, want {sha256} (removed from the cache)")
    return path


def unpack(archive, dest):
    """Extract `archive` into `dest` (replaced), stripping its top directory."""
    if dest.exists():
        shutil.rmtree(dest)
    with tempfile.TemporaryDirectory(dir=dest.parent) as tmp:
        if archive.name.endswith(".zip"):
            with zipfile.ZipFile(archive) as z:
                z.extractall(tmp)
        else:
            with tarfile.open(archive) as t:
                t.extractall(tmp, filter="data")
        top = list(Path(tmp).iterdir())
        if len(top) != 1 or not top[0].is_dir():
            sys.exit(f"{archive.name}: expected a single top-level directory")
        top[0].rename(dest)


def download(a, d):
    for dl in a.get("download", []):
        src = fetch(dl["url"], dl["sha256"])
        dest = d / dl["to"]
        dest.parent.mkdir(parents=True, exist_ok=True)
        if src.name.endswith((".zip", ".tar.gz", ".tgz")):
            print(f"[{a['name']}] unpack {src.name} -> {dl['to']}", flush=True)
            unpack(src, dest)
        else:
            print(f"[{a['name']}] copy {src.name} -> {dl['to']}", flush=True)
            shutil.copyfile(src, dest)


def cmd_setup(apps):
    for a in apps:
        d = HERE / a["name"]
        download(a, d)
        if a.get("install"):
            run(a["install"], d)
        for c in a.get("setup", []):
            run(c, d)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def cmd_smoke(apps):
    php = os.environ.get("RHP_PHP", "php")
    failed = 0
    for a in apps:
        d = HERE / a["name"]
        out = subprocess.run(
            [php, a["console"], "--version"], cwd=d, capture_output=True, text=True
        )
        print(f"[{a['name']}] {a['console']} --version: {out.stdout.strip()}")
        if out.returncode != 0:
            print(out.stderr)
            failed += 1
        port = free_port()
        # The server's log goes to a temp file, printed if anything fails (a
        # 500 under php -S is otherwise undiagnosable in CI).
        log = tempfile.TemporaryFile()
        # `router = false`: php -S serves the docroot itself, running the
        # .php file a path names and index.php for any other path.
        router = [a["front_controller"]] if a.get("router", True) else []
        srv = subprocess.Popen(
            [php, "-S", f"127.0.0.1:{port}", "-t", a["docroot"], *router],
            cwd=d,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        failed_before = failed
        try:
            for _ in range(100):
                if srv.poll() is not None:
                    break
                try:
                    socket.create_connection(("127.0.0.1", port), timeout=0.2).close()
                    break
                except OSError:
                    time.sleep(0.1)
            for line in a.get("smoke", []):
                method, path, want = line.split()
                req = urllib.request.Request(
                    f"http://127.0.0.1:{port}{path}",
                    method=method,
                    data=b"x=1&y=two" if method == "POST" else None,
                )
                try:
                    with urllib.request.urlopen(req, timeout=30) as r:
                        got = r.status
                except urllib.error.HTTPError as e:
                    got = e.code
                except (urllib.error.URLError, OSError) as e:
                    got = f"error ({e})"
                ok = str(got) == want
                failed += not ok
                print(f"[{a['name']}] {method} {path} -> {got} {'ok' if ok else f'FAIL (want {want})'}")
        finally:
            srv.terminate()
            srv.wait()
            if failed != failed_before:
                log.seek(0)
                print(f"[{a['name']}] php -S log:\n{log.read().decode(errors='replace')}")
            log.close()
    if failed:
        sys.exit(f"{failed} smoke check(s) failed")


def cmd_lint(apps, write_known, extra):
    exe = os.environ.get("RHP_DIFFTEST") or str(REPO / "target/release/rhp-difftest")
    failed = []
    for a in apps:
        d = HERE / a["name"]
        known = d / a["known_refused"]
        if not write_known and not known.is_file():
            print(
                f"[{a['name']}] no {known.relative_to(REPO)}; create it with "
                f"`python3 tests/apps/apps.py lint {a['name']} --write-known`"
            )
            failed.append(a["name"])
            continue
        cmd = [exe, "lint", "-q", *(str(d / p) for p in a["lint"])]
        cmd += ["--write-known" if write_known else "--check-known", str(known)]
        cmd += extra
        print(f"[{a['name']}] $ {shlex.join(cmd)}", flush=True)
        if subprocess.run(cmd).returncode != 0:
            failed.append(a["name"])
    if failed:
        sys.exit(f"lint failed for: {', '.join(failed)}")


def cmd_diff(apps, write_known, extra):
    exe = os.environ.get("RHP_DIFFTEST") or str(REPO / "target/release/rhp-difftest")
    failed = []
    for a in apps:
        d = HERE / a["name"]
        known = d / a.get("known_requests", "known-passing-requests.txt")
        cmd = [exe, "app", str(d)]
        cmd += ["--write-known" if write_known else "--check-known", str(known)]
        cmd += extra
        print(f"[{a['name']}] $ {shlex.join(cmd)}", flush=True)
        if subprocess.run(cmd).returncode != 0:
            failed.append(a["name"])
    if failed:
        sys.exit(f"app diff failed for: {', '.join(failed)}")


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return
    sub, rest = argv[0], argv[1:]
    extra = []
    if "--" in rest:
        i = rest.index("--")
        rest, extra = rest[:i], rest[i + 1 :]
    write_known = "--write-known" in rest
    names = [r for r in rest if r != "--write-known"]
    apps = load()
    if sub == "list":
        print("\n".join(a["name"] for a in apps))
    elif sub == "matrix":
        print(
            json.dumps(
                [
                    {"app": a["name"], "php-extensions": ", ".join(a.get("php_extensions", []))}
                    for a in apps
                ]
            )
        )
    elif sub == "setup":
        cmd_setup(select(apps, names))
    elif sub == "smoke":
        cmd_smoke(select(apps, names))
    elif sub == "lint":
        cmd_lint(select(apps, names), write_known, extra)
    elif sub == "diff":
        cmd_diff(select(apps, names), write_known, extra)
    else:
        sys.exit(f"unknown command {sub!r}; see --help")


if __name__ == "__main__":
    main(sys.argv[1:])
