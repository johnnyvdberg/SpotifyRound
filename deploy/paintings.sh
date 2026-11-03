#!/usr/bin/env bash
# The museum's paintings in assets/museum, fetched from the Rijksmuseum.
#
#   deploy/paintings.sh [FILE ...]
#       downloads each painting listed in assets/museum/paintings.tsv from
#       the Rijksmuseum's IIIF server, fitted within 512 pixels: build.rs
#       packs them, compressed, into the layers the museum streams in. Given
#       FILEs, only those are fetched again. The works are in the public
#       domain (assets/museum/CREDITS.md).
set -euo pipefail
cd "$(dirname "$0")/.."
grep -v '^#' assets/museum/paintings.tsv | while IFS=$'\t' read -r file _ _ _ _ image _; do
    if [ $# -gt 0 ] && [[ " $* " != *" $file "* ]]; then
        continue
    fi
    if [ $# -gt 0 ] || [ ! -f "assets/museum/$file" ]; then
        curl -sSfL -A "SpotifyRound-museum" -o "assets/museum/$file" "$image"
    fi
done
