#!/usr/bin/env bash
# .github/scripts/linux-ci.sh: run rhp's checks in a Linux container close to CI's
# runner (.github/scripts/linux-ci.Dockerfile: Ubuntu 26.04, php 8.5 + php-cgi from the
# PPA setup-php uses, the same extensions and `-n` shims, the native libraries).
# Development happens on macOS; tests can behave differently on Linux, so check
# here before adding tests to tests/phpt/known-passing.txt.
#
#   .github/scripts/linux-ci.sh phpt [rhp-difftest phpt args...]
#       release-build rhp and rhp-difftest in the container, then
#       `rhp-difftest phpt --php-src $PHP_SRC --php-cgi php-cgi --timeout 30
#        --check-known tests/phpt/known-passing.txt <args>`;
#       e.g. `--filter ext/standard/tests/file --jobs 4`.
#   .github/scripts/linux-ci.sh shell        an interactive shell in the container
#   .github/scripts/linux-ci.sh <cmd...>     any command, e.g. `cargo clippy
#                                      --workspace --all-targets -- -D warnings`
#
# PHP_SRC: a php-src checkout at tests/phpt/php-src.ref (default ../php-src),
# mounted read-only at /php-src. The cargo target directory and registry live
# in docker volumes (rhp-linux-ci-target, rhp-linux-ci-cargo), never in the
# checkout; the binaries are /target/release/rhp{,-difftest} in the container.
# The container gets the docker VM's CPUs; JOBS (default 2) caps cargo's.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
php_src="${PHP_SRC:-$repo/../php-src}"
php_src="$(cd "$php_src" && pwd)"
image=rhp-linux-ci

docker build -q -t "$image" -f "$repo/.github/scripts/linux-ci.Dockerfile" "$repo/.github/actions" >/dev/null

tty=()
if [ -t 0 ] && [ -t 1 ]; then tty=(-it); fi
run() {
  # --init reaps the orphans tests leave behind; seccomp off and a larger
  # /dev/shm match a VM runner more closely than docker's defaults.
  docker run --rm ${tty[@]+"${tty[@]}"} --init --security-opt seccomp=unconfined --shm-size=1g \
    -v "$repo:/src" -v "$php_src:/php-src:ro" \
    -v rhp-linux-ci-target:/target -v rhp-linux-ci-cargo:/home/runner/.cargo/registry \
    -e CARGO_TARGET_DIR=/target -e CARGO_BUILD_JOBS="${JOBS:-2}" -e CARGO_TERM_COLOR \
    "$image" bash -c 'export PKG_CONFIG_PATH="$(paste -sd: /opt/pkgconfig-path)"; sudo chown runner /target /home/runner/.cargo/registry; exec "$@"' bash "$@"
}

case "${1:-shell}" in
  phpt)
    shift
    run bash -c 'cargo build -q --release -p rhp-cli -p rhp-difftest &&
      exec /target/release/rhp-difftest phpt --php-src /php-src --rhp /target/release/rhp \
        --php-cgi "$(command -v php-cgi)" --timeout 30 \
        --check-known tests/phpt/known-passing.txt "$@"' bash "$@"
    ;;
  shell) run bash ;;
  *) run "$@" ;;
esac
