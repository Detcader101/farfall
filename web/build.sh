#!/bin/sh
# Build the web version into web/dist: the wasm module, its JS glue, and
# the page. Serve the directory (any static server) and open index.html.
set -eu
cd "$(dirname "$0")/.."
cargo build --profile web --target wasm32-unknown-unknown -p farfall-app
wasm-bindgen --target web --no-typescript \
  --out-dir web/dist --out-name farfall \
  target/wasm32-unknown-unknown/web/farfall_app.wasm
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
    -o web/dist/farfall_bg.wasm web/dist/farfall_bg.wasm
fi
cp web/index.html web/xr.js web/dist/
touch web/dist/.nojekyll
ls -la web/dist
