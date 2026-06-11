#!/bin/sh
# Populate testdata/originals/ with the original M.A.X. maps (*.WRL).
#
# The 24 original maps are copyrighted game data and are NOT in this
# repository. The equivalence and shore ground-truth tests use them as
# reference fixtures; without them those tests skip.
#
# Usage:  tools/fetch-testdata.sh [MAX_DIR]
#         MAX_DIR defaults to the $MAX_DIR environment variable.
#         Point it at any M.A.X. installation (the directory with the WRLs).

set -eu

src="${1:-${MAX_DIR:-}}"
if [ -z "$src" ]; then
	echo "usage: tools/fetch-testdata.sh MAX_DIR   (or set the MAX_DIR env var)" >&2
	exit 2
fi
if [ ! -d "$src" ]; then
	echo "error: '$src' is not a directory" >&2
	exit 1
fi

dest="$(dirname "$0")/../testdata/originals"
mkdir -p "$dest"

count=0
for wrl in "$src"/*.WRL "$src"/*.wrl; do
	[ -e "$wrl" ] || continue
	cp -p "$wrl" "$dest/"
	count=$((count + 1))
done

if [ "$count" -eq 0 ]; then
	echo "error: no .WRL files found in '$src'" >&2
	exit 1
fi
echo "copied $count map(s) to $dest"
