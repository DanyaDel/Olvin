#!/usr/bin/env sh
# Builds both books into <repo>/book/{ru,en} and the landing page; --serve
# serves them on http://127.0.0.1:3000/ (needs: cargo install mdbook --locked).
set -eu
cd "$(dirname "$0")/../../.."
python3 docs/wiki/tools/check_links.py
python3 docs/wiki/tools/check_lua_reference.py
mdbook build docs/wiki/ru
mdbook build docs/wiki/en
cat > book/index.html <<'HTML'
<!doctype html><meta charset="utf-8"><title>Olvin Wiki</title>
<meta http-equiv="refresh" content="0; url=ru/">
<p><a href="ru/">Русская версия</a> · <a href="en/">English version</a></p>
HTML
if [ "${1:-}" = "--serve" ]; then
  echo "http://127.0.0.1:3000/ru/  (languages: /ru/ and /en/)"
  exec python3 -m http.server 3000 --directory book --bind 127.0.0.1
fi
