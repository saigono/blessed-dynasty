#!/usr/bin/env bash
# Builds the content editor into editor/ for publishing as an Artifact: the core in wasm
# (crates/web-sim), its JS glue and a copy of every data/**/*.ron with the list of them.
# editor/index.html is the page itself and lives in git; the rest is generated.
#
#   scripts/editor-bundle.sh
#
# Then publish editor/index.html with editor/web_sim_bg.wasm, editor/web_sim.js and
# editor/data/** next to it (docs/content-editor.md).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release -p web-sim --target wasm32-unknown-unknown
# wasm-bindgen of the version in Cargo.lock: from PATH, or the one trunk keeps.
version=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"/\1/p')
bindgen=$(command -v wasm-bindgen || echo "$HOME/.cache/trunk/wasm-bindgen-$version/wasm-bindgen")
if ! "$bindgen" --version | grep -q " $version"; then
    echo "нужен wasm-bindgen $version: cargo install wasm-bindgen-cli --version $version" >&2
    exit 1
fi
# no-modules: the glue defines a global `wasm_bindgen`, so the page can run it in a Worker
# made from a blob.
"$bindgen" --target no-modules --no-typescript --out-dir editor --out-name web_sim \
    target/wasm32-unknown-unknown/release/web_sim.wasm

rm -rf editor/data
(cd data && find . -name '*.ron' | sed 's|^\./||' | LC_ALL=C sort) > /tmp/editor-files.$$
while read -r f; do
    mkdir -p "editor/data/$(dirname "$f")"
    cp "data/$f" "editor/data/$f"
done < /tmp/editor-files.$$
# The list the page fetches the data by, and the commit it was built from.
{
    printf '{"commit": "%s", "files": [' "$(git rev-parse --short HEAD)"
    sed 's/.*/"&"/' /tmp/editor-files.$$ | paste -sd, -
    printf ']}\n'
} > editor/data/files.json
rm -f /tmp/editor-files.$$

ls -l editor/index.html editor/web_sim.js editor/web_sim_bg.wasm editor/data/files.json
echo "data: $(find editor/data -name '*.ron' | wc -l) файлов, $(du -sb editor/data | cut -f1) байт"
