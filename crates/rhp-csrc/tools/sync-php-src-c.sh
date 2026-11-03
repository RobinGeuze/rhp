#!/bin/sh
# Copy the php-src C files rhp compiles into vendor/php-src, from a php-src
# checkout at the commit pinned in tests/phpt/php-src.ref. Writes VERSION (tag
# and commit) and SHA256SUMS. Running it again on the same checkout changes
# nothing.
#
#   crates/rhp-csrc/tools/sync-php-src-c.sh <php-src-dir>
set -eu

# Files to vendor, grouped by the cargo feature that compiles them. Paths are
# relative to the php-src root and keep their php-src layout.
FILES='
LICENSE
ext/standard/crypt_blowfish.c
ext/standard/crypt_blowfish.h
ext/standard/crypt_freesec.c
ext/standard/crypt_freesec.h
ext/standard/crypt_sha256.c
ext/standard/crypt_sha512.c
'

die() { echo "sync-php-src-c: $*" >&2; exit 1; }

[ $# -eq 1 ] || die "usage: $0 <php-src-dir>"
src=$1
[ -d "$src" ] || die "$src: not a directory"

crate=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$crate/../.." && pwd)
dest=$crate/vendor/php-src

want=$(grep -v '^#' "$root/tests/phpt/php-src.ref" | head -n 1)
have=$(git --no-optional-locks -C "$src" rev-parse HEAD) || die "$src: not a git checkout"
[ "$have" = "$want" ] ||
	die "$src is at $have; tests/phpt/php-src.ref pins $want (git -C $src checkout $want)"
tag=$(git --no-optional-locks -C "$src" describe --tags --exact-match HEAD 2>/dev/null) ||
	die "$src: HEAD $have has no tag"
# shellcheck disable=SC2086
git --no-optional-locks -C "$src" diff --quiet HEAD -- $FILES ||
	die "$src has local changes to vendored files"

if command -v sha256sum >/dev/null 2>&1; then
	sha256() { sha256sum "$@"; }
else
	sha256() { shasum -a 256 "$@"; }
fi

# Replace $2 with $1 only when the content differs, so reruns touch nothing.
put() {
	if ! cmp -s "$1" "$2"; then
		mkdir -p "$(dirname "$2")"
		cp "$1" "$2"
		echo "updated ${2#"$crate"/}"
	fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for f in $FILES; do
	[ -f "$src/$f" ] || die "$src/$f: missing"
	put "$src/$f" "$dest/$f"
done

# Drop files that are no longer on the list.
(cd "$dest" && find . -type f ! -name VERSION ! -name SHA256SUMS | sed 's|^\./||') |
	while read -r f; do
		case "
$FILES
" in
		*"
$f
"*) ;;
		*) rm "$dest/$f"; echo "removed vendor/php-src/$f" ;;
		esac
	done
find "$dest" -type d -empty -delete

printf '%s\n%s\n' "$tag" "$want" >"$tmp/VERSION"
put "$tmp/VERSION" "$dest/VERSION"

# shellcheck disable=SC2086
(cd "$dest" && sha256 $FILES) >"$tmp/SHA256SUMS"
put "$tmp/SHA256SUMS" "$dest/SHA256SUMS"
