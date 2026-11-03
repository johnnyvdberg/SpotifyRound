#!/usr/bin/env bash
# The imported models in assets/models, rebuilt from their originals.
#
#   deploy/models.sh [FILE ...]
#       fetches each original listed in assets/models/models.tsv into
#       target/models/ (once) and reduces it to the listed triangle count
#       with tools/decimate. For a higher quality level later, raise the
#       count in models.tsv and run this again: the originals keep every
#       detail. A source is a URL, itch:CREATOR/GAME/UPLOAD:PATH for a
#       file inside a free itch.io download, or polyhaven:ASSET for a Poly
#       Haven model's 1k glTF with its buffer and textures (api.polyhaven.com);
#       polyhaven:ASSET#PICK keeps only the nodes PICK names (tools/decimate).
#       Given FILEs, only those are rebuilt.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/models
cargo build --release -q --manifest-path tools/decimate/Cargo.toml
tool=tools/decimate/target/release/decimate

json_url() { python3 -c 'import json,sys; print(json.load(sys.stdin)["url"])'; }

fetch() {
    local source=$1
    case $source in
    itch:*)
        local spec=${source#itch:} page upload path dir
        page=${spec%%:*} path=${spec#*:} upload=${page##*/} page=${page%/*}
        dir=target/models/itch-$upload
        if [ ! -d "$dir" ]; then
            local game=https://${page%%/*}.itch.io/${page#*/} download zip
            download=$(curl -sS -X POST "$game/download_url" | json_url)
            zip=$(curl -sS -X POST -H "Referer: $download" \
                "$game/file/$upload?source=game_download" | json_url)
            curl -sSL -o "$dir.zip" "$zip"
            mkdir -p "$dir" && unzip -q -o "$dir.zip" -d "$dir"
        fi
        original=$dir/$path
        ;;
    polyhaven:*)
        local asset=${source#polyhaven:} pick='' dir
        if [[ $asset == *'#'* ]]; then
            pick=#${asset#*#}
            asset=${asset%%#*}
        fi
        dir=target/models/polyhaven-$asset
        if [ ! -d "$dir" ]; then
            mkdir -p "$dir.part"
            curl -sS -A "SpotifyRound-models" "https://api.polyhaven.com/files/$asset" |
                python3 -c 'import json,sys
g = json.load(sys.stdin)["gltf"]["1k"]["gltf"]
print(g["url"], "model.gltf")
for path, f in g["include"].items():
    print(f["url"], path)' |
                while read -r url path; do
                    mkdir -p "$dir.part/$(dirname "$path")"
                    curl -sSL -A "SpotifyRound-models" -o "$dir.part/$path" "$url"
                done
            mv "$dir.part" "$dir"
        fi
        original=$dir/model.gltf$pick
        ;;
    *)
        original=target/models/${source##*/}
        # The SMK's server drops long transfers part way; resume until whole.
        for _ in $(seq 30); do
            curl -sSL -C - -o "$original" "$source" 2>/dev/null && break
        done
        ;;
    esac
}

grep -v '^#' assets/models/models.tsv | while IFS=$'\t' read -r file triangles up fit turn source; do
    if [ $# -gt 0 ] && [[ " $* " != *" $file "* ]]; then
        continue
    fi
    fetch "$source"
    "$tool" "$original" "assets/models/$file" "$triangles" "$up" "$fit" "$turn"
done
