#!/usr/bin/env bash
# Сборка .deb: ./tools/package-deb.sh → target/debian/shortscutter_<версия>_amd64.deb
# Нужен cargo-deb (`cargo install cargo-deb --locked`). Первая сборка профиля dist — ~7 мин (whisper.cpp и llama.cpp
# заново: здесь ggml без GGML_NATIVE, под x86-64-v3, чтобы пакет шёл не только на этом CPU).
set -euo pipefail
cd "$(dirname "$0")/.."

git submodule update --init
export SC_PORTABLE=1
cargo build --profile dist -p shortscutter

# llama-server собирает build.rs sc-llama в OUT_DIR; берём самый свежий.
server=$(ls -t target/dist/build/sc-llama-*/out/build/bin/llama-server | head -1)
cp "$server" target/dist/llama-server
strip target/dist/llama-server

cargo deb -p shortscutter --profile dist --no-build "$@"
ls -lh target/debian/*.deb
