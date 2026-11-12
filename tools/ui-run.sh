#!/bin/bash
# Сценарий UI в headless sway (из корня репо): spikes/05-preview/headless.sh tools/ui-run.sh <видео> "<команды>"
# Виртуальная клавиатура живёт всё время (иначе у seat нет keyboard и приложение не подписывается на клавиши).
# В сценарии: key <аргументы wtype>, shot <имя> → /tmp/sc13/<имя>.png. Сборка: cargo build --release.
cd "$(dirname "$0")/.."
OUT=/tmp/sc13; mkdir -p $OUT; rm -f $OUT/*
wtype -s 300000 "" & KB=$!
sleep 0.5
RUST_LOG=info ./target/release/shortscutter "$1" > $OUT/app.log 2>&1 &
APP=$!
sleep 4
key() { wtype "$@"; sleep 0.3; }
shot() { grim $OUT/$1.png; }
key -M ctrl -k 2 -m ctrl
sleep 2
shot 2-cut
eval "$2"
kill $APP $KB; wait $APP 2>/dev/null
