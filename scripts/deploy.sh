#!/usr/bin/env bash
# Builds the web version and puts dist/ either into a folder (the argument) or, without one,
# into a commit on the local gh-pages branch. Nothing is pushed: after a gh-pages commit run
# `git push origin gh-pages` yourself.
#
#   scripts/deploy.sh /srv/www/bd    # copy into a folder
#   scripts/deploy.sh                # commit to gh-pages
set -euo pipefail
cd "$(dirname "$0")/.."
# Trunk.toml: target crates/ui/index.html, dist crates/ui/dist. A relative public URL lets
# the page live under any path, e.g. https://user.github.io/repo/.
# Stage 11b (docs/stats.md): the build's version, the short commit hash unless given, shown
# on the start screen and sent with the statistics. BD_STATS_URL, the receiver of the games,
# goes into the build as it is; BD_GOATCOUNTER (https://CODE.goatcounter.com/count) puts
# GoatCounter's script into the page. Unset, nothing is sent and nothing counted.
export BD_VERSION=${BD_VERSION:-$(git rev-parse --short HEAD)}
trunk build --release --public-url ./
dist=crates/ui/dist
touch "$dist/.nojekyll"
if [ -n "${BD_GOATCOUNTER:-}" ]; then
    gc="<script data-goatcounter=\"$BD_GOATCOUNTER\" async src=\"//gc.zgo.at/count.js\"></script>"
    sed -i "s|</head>|$gc\n</head>|" "$dist/index.html"
fi
echo "версия $BD_VERSION, статистика: ${BD_STATS_URL:-нет}, GoatCounter: ${BD_GOATCOUNTER:-нет}"

if [ $# -gt 0 ]; then
    mkdir -p "$1"
    cp -r "$dist"/. "$1"/
    echo "dist/ скопирован в $1"
    exit 0
fi

# The tree of dist/ through a throwaway index: the working tree and HEAD stay as they are.
index=$(mktemp)
trap 'rm -f "$index"' EXIT
rm -f "$index"
git_dir=$(git rev-parse --absolute-git-dir)
tree=$(cd "$dist" && export GIT_INDEX_FILE=$index &&
    git --git-dir="$git_dir" --work-tree=. add -A . && git --git-dir="$git_dir" write-tree)
parent=$(git rev-parse -q --verify refs/heads/gh-pages || true)
commit=$(git commit-tree "$tree" ${parent:+-p "$parent"} -m "deploy $(git rev-parse --short HEAD)")
git update-ref refs/heads/gh-pages "$commit"
echo "gh-pages: $commit. Опубликовать: git push origin gh-pages"
