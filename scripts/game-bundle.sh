#!/usr/bin/env bash
# Builds the game for publishing as an Artifact: a release trunk build into its own folder,
# with relative paths, so the page and its files can live under any URL.
#
#   scripts/game-bundle.sh            # into target/game-artifact/
#   scripts/game-bundle.sh some/dir   # into some/dir/
#
# Then publish index.html with every other file of the folder next to it (docs/game-bundle.md).
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-target/game-artifact}
trunk build --release --public-url ./ --dist "$out"
(cd "$out" && find . -type f | sed 's|^\./||' | LC_ALL=C sort | xargs ls -l | awk '{print $5, $9}')
echo "всего: $(du -sb "$out" | cut -f1) байт"
