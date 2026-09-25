#!/bin/sh
# Trunk post_build hook: fill sw.js with the list of built files to precache and
# a content hash, so each deploy gets a fresh offline cache.
set -eu

cd "${TRUNK_STAGING_DIR:?must run as a Trunk hook}"

files=$(find . -type f ! -name sw.js ! -name '*.map' | sed 's|^\./||' | sort)
list=$(printf '%s\n' $files | awk 'BEGIN { printf "[" } { printf "%s\"%s\"", (NR > 1 ? "," : ""), $0 } END { printf "]" }')
version=$(cat $files | cksum | cut -d' ' -f1)

sed -e "s|__PRECACHE__|$list|" -e "s|__VERSION__|$version|g" sw.js > sw.js.tmp
mv sw.js.tmp sw.js
