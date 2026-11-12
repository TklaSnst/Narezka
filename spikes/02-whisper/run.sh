#!/usr/bin/env bash
# Спайк 0.2: прогон whisper-cli по модели, замер времени и пика VRAM.
# Использование: run-whisper.sh <модель.bin> <аудио.wav> <префикс-вывода> [доп. флаги whisper-cli]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
cli=${WHISPER_CLI:-$here/../out/whisper.cpp/build/bin/whisper-cli}
models=${SC_MODELS:-$HOME/.cache/shortscutter/models}
model=$1 wav=$2 out=$3; shift 3

# Без этого RADV отдаёт ggml «host-visible VRAM», которая на деле в системной RAM (без ReBAR) → encode в 5 раз медленнее.
export GGML_VK_DISABLE_HOST_VISIBLE_VIDMEM=1

vram=$(ls /sys/class/drm/card*/device/mem_info_vram_used | head -1)
base=$(cat "$vram"); peak=$base
( while sleep 0.5; do cat "$vram"; done ) > "$out.vram" & mon=$!
start=$(date +%s.%N)
"$cli" -m "$models/$model" -f "$wav" -l ru -t 4 -ojf -of "$out" "$@" > "$out.txt.log" 2> "$out.err"
end=$(date +%s.%N)
kill $mon
peak=$(sort -n "$out.vram" | tail -1)
audio=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$wav")
printf '%s\tвремя %.1f с\tx%.1f реального\tVRAM +%d МиБ\n' "$model" \
  "$(echo "$end - $start" | bc)" "$(echo "$audio / ($end - $start)" | bc -l)" $(( (peak - base) / 1048576 ))
