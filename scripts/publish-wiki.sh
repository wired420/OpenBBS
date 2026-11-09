#!/bin/sh
# Publish README.md, docs/SETUP.md and docs/DOORS.md to the GitHub wiki.
#
# Run from anywhere inside the repository, with a key that can push to it:
#   ./scripts/publish-wiki.sh
#
# The repository files stay the source of truth; this copies them to the
# wiki and fixes up links. GitHub only creates the wiki repository after its
# first page exists, so before the very first run, open the Wiki tab on
# GitHub and save any page.
set -eu
repo=$(git rev-parse --show-toplevel)
wiki_url=${WIKI_URL:-git@github.com:wired420/OpenBBS.wiki.git}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

git clone --quiet "$wiki_url" "$tmp/wiki"
python3 - "$repo" "$tmp/wiki" <<'PY'
import re, sys
repo, wiki = sys.argv[1], sys.argv[2]
blob = "https://github.com/wired420/OpenBBS/blob/main"

def fix(text):
    text = text.replace("[docs/SETUP.md](docs/SETUP.md)", "[[Setup]]")
    text = text.replace("[docs/DOORS.md](docs/DOORS.md)", "[[Doors]]")
    text = text.replace("`docs/SETUP.md`", "[[Setup]]")
    text = text.replace("`docs/DOORS.md`", "[[Doors]]")
    text = text.replace("](LICENSE)", f"]({blob}/LICENSE)")
    return re.sub(r"`(dist/[\w./-]+)`",
                  lambda m: f"[`{m.group(1)}`]({blob}/{m.group(1)})", text)

def page(src, dst, drop_title):
    text = open(f"{repo}/{src}", encoding="utf-8").read()
    if drop_title:  # the wiki shows the page name as the title
        text = text.split("\n", 1)[1].lstrip()
    open(f"{wiki}/{dst}", "w", encoding="utf-8").write(fix(text))

page("README.md", "Home.md", False)
page("docs/SETUP.md", "Setup.md", True)
page("docs/DOORS.md", "Doors.md", True)
open(f"{wiki}/_Sidebar.md", "w").write(
    "**OpenBBS**\n\n- [[Home]]\n- [[Setup]]\n- [[Doors]]\n"
    f"- [Changelog]({blob}/CHANGELOG.md)\n"
    "- [Source code](https://github.com/wired420/OpenBBS)\n")
open(f"{wiki}/_Footer.md", "w").write(
    "These pages are published from the README and `docs/` folder in the "
    "repository by `scripts/publish-wiki.sh`. Edit them there.\n")
PY

cd "$tmp/wiki"
git add -A
if git diff --cached --quiet; then
    echo "The wiki is already up to date."
    exit 0
fi
git commit --quiet -m "Update from README and docs"
git push --quiet
echo "Published: https://github.com/wired420/OpenBBS/wiki"
