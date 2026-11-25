#!/usr/bin/env bash
# Prints the updater manifest (latest.json) for one desktop build: desktop-latest.sh <version> <sig-file> [arch].
# `url` is the release asset of tag v<version> (or $ASSET_URL, for the local update test); `signature` is the .sig file content.
set -euo pipefail
version=${1:?usage: desktop-latest.sh <version> <sig-file> [arch]}
sig=$(tr -d '\n' <"${2:?usage: desktop-latest.sh <version> <sig-file> [arch]}")
arch=${3:-$(uname -m)}
case "$arch" in arm64 | aarch64) arch=aarch64 ;; x86_64) ;; *) echo "unknown arch $arch" >&2; exit 1 ;; esac
[ -n "$sig" ] || { echo "empty signature file" >&2; exit 1; }
printf '{"version":"%s","notes":"Recapz %s","pub_date":"%s","platforms":{"darwin-%s":{"signature":"%s","url":"%s"}}}\n' \
  "$version" "$version" "${PUB_DATE:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}" "$arch" "$sig" \
  "${ASSET_URL:-https://github.com/tungmai-agilityio/recapz/releases/download/v$version/Recapz.app.tar.gz}"
