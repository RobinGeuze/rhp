#!/bin/sh
# Copy the argon2 reference C (release 20190702, the final release and the
# libargon2 php links on macOS and Ubuntu) into vendor/argon2. Takes the
# release tarball, or downloads it, and checks its SHA-256 (the one Homebrew's
# argon2 formula pins). Writes VERSION and SHA256SUMS. Running it again
# changes nothing.
#
#   crates/rhp-csrc/tools/sync-argon2.sh [phc-winner-argon2-20190702.tar.gz]
set -eu

VERSION=20190702
URL=https://github.com/P-H-C/phc-winner-argon2/archive/refs/tags/$VERSION.tar.gz
TARBALL_SHA256=daf972a89577f8772602bf2eb38b6a3dd3d922bf5724d45e7f9589b5e830442c

# The library sources php's libargon2 is built from (the Makefile's SRC with
# the portable ref.c), their headers, and the licence.
FILES='
LICENSE
include/argon2.h
src/argon2.c
src/blake2/blake2-impl.h
src/blake2/blake2.h
src/blake2/blake2b.c
src/blake2/blamka-round-ref.h
src/core.c
src/core.h
src/encoding.c
src/encoding.h
src/ref.c
src/thread.c
src/thread.h
'

die() { echo "sync-argon2: $*" >&2; exit 1; }

crate=$(cd "$(dirname "$0")/.." && pwd)
dest=$crate/vendor/argon2

if command -v sha256sum >/dev/null 2>&1; then
	sha256() { sha256sum "$@"; }
else
	sha256() { shasum -a 256 "$@"; }
fi

put() {
	if ! cmp -s "$1" "$2"; then
		mkdir -p "$(dirname "$2")"
		cp "$1" "$2"
		echo "updated ${2#"$crate"/}"
	fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ $# -ge 1 ]; then
	tarball=$1
else
	tarball=$tmp/argon2.tar.gz
	curl -fsSL -o "$tarball" "$URL" || die "download of $URL failed"
fi
got=$(sha256 "$tarball" | cut -d' ' -f1)
[ "$got" = "$TARBALL_SHA256" ] || die "$tarball: sha256 $got, want $TARBALL_SHA256"

mkdir "$tmp/x"
tar -xzf "$tarball" -C "$tmp/x"
src=$tmp/x/phc-winner-argon2-$VERSION
[ -d "$src" ] || die "$tarball: no phc-winner-argon2-$VERSION/ inside"

for f in $FILES; do
	[ -f "$src/$f" ] || die "$f: missing from the tarball"
	put "$src/$f" "$dest/$f"
done

(cd "$dest" && find . -type f ! -name VERSION ! -name SHA256SUMS | sed 's|^\./||') |
	while read -r f; do
		case "
$FILES
" in
		*"
$f
"*) ;;
		*) rm "$dest/$f"; echo "removed vendor/argon2/$f" ;;
		esac
	done
find "$dest" -type d -empty -delete

printf '%s\n%s\n' "$VERSION" "$URL" >"$tmp/VERSION"
put "$tmp/VERSION" "$dest/VERSION"

# shellcheck disable=SC2086
(cd "$dest" && sha256 $FILES) >"$tmp/SHA256SUMS"
put "$tmp/SHA256SUMS" "$dest/SHA256SUMS"
