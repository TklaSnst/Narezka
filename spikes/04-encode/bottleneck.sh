#!/usr/bin/env bash
# Спайк 0.4: где тратится время полного цикла (декод / граф) и сколько дают параллельные задачи.
# Использование: bottleneck.sh <исходник> <начало_с> <каталог_вывода run.py (нужны assets/)>
set -euo pipefail
S=$1; SS=$2; A=$3/assets
G=$(sed "s|ASSETS|$A|" "$(dirname "$0")/graph.txt" | tr -d "\n")
IN=(-ss "$SS" -t 60 -i "$S" -i "$A/mask-face.png" -i "$A/mask-game.png" -i "$A/title.png")
ff() { ffmpeg -nostdin -y -v error "$@"; }   # -nostdin: иначе ffmpeg съедает stdin скрипта
t() { /usr/bin/time -f "%e с, CPU %U+%S с" "$@" 2>&1 | tail -1; }

echo "декод sw:            $(t ff -ss "$SS" -t 60 -i "$S" -map 0:v -f null -)"
echo "декод vaapi:         $(t ff -hwaccel vaapi -hwaccel_output_format vaapi -ss "$SS" -t 60 -i "$S" -map 0:v -f null -)"
echo "граф, декод sw:      $(t ff "${IN[@]}" -filter_complex "$G" -map "[out]" -f null -)"
echo "граф, декод vaapi:   $(t ff -hwaccel vaapi "${IN[@]}" -filter_complex "$G" -map "[out]" -f null -)"
echo "граф без ass:        $(t ff "${IN[@]}" -filter_complex "${G/,ass=*,format=yuv420p/,format=yuv420p}" -map "[out]" -f null -)"
echo "граф без gblur:      $(t ff "${IN[@]}" -filter_complex "${G/gblur=sigma=10,/}" -map "[out]" -f null -)"
G2=$(sed -e "s/,format=yuva420p\[fc\];\[fc\]\[1:v\]alphamerge\[fa\]/[fa]/" \
         -e "s/,format=yuva420p\[gc\];\[gc\]\[2:v\]alphamerge\[ga\]/[ga]/" <<<"$G")
echo "граф без масок:      $(t ff "${IN[@]}" -filter_complex "$G2" -map "[out]" -f null -)"
echo "граф, 30 fps:        $(t ff "${IN[@]}" -filter_complex "[0:v]fps=30,${G#\[0:v\]}" -map "[out]" -f null -)"
echo "граф, 1 поток:       $(t ff "${IN[@]}" -filter_complex_threads 1 -filter_complex "$G" -map "[out]" -f null -)"

# полный рендер h264_vaapi, N задач одновременно (разные отрезки исходника)
job() {
  ff -vaapi_device /dev/dri/renderD128 -ss "$1" -t 60 -i "$S" -i "$A/mask-face.png" -i "$A/mask-game.png" \
     -i "$A/title.png" -filter_complex "${G%\[out\]},format=nv12,hwupload[out]" -map "[out]" -map 0:a:0 \
     -c:v h264_vaapi -rc_mode VBR -b:v 12M -maxrate 18M -bufsize 24M -c:a aac -b:a 192k "/tmp/sc-par-$1.mp4"
}
for n in 1 2 3 4; do
  s=$(date +%s.%N)
  for i in $(seq "$n"); do job $((SS + i * 300)) & done
  wait
  echo "$n параллельно: $(echo "$(date +%s.%N) - $s" | bc) с"
done
rm -f /tmp/sc-par-*.mp4
