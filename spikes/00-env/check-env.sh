#!/usr/bin/env bash
# Шаг 0.1: проверка окружения для ShortsCutter.
# Ничего не устанавливает, только печатает, что есть и чего не хватает.
#   ./check-env.sh [/dev/dri/renderD128]
set -uo pipefail
DRI=${1:-/dev/dri/renderD128}
TMP=$(mktemp -d)
trap "rm -rf $TMP" EXIT
missing=()

ok()   { printf "  \e[32mOK\e[0m   %s\n" "$*"; }
bad()  { printf "  \e[31mНЕТ\e[0m  %s\n" "$*"; }
need() { missing+=("$1"); }

echo "== Rust"
for t in rustc cargo; do
  command -v $t >/dev/null && ok "$($t --version)" || { bad $t; need rustup; }
done

echo "== Сборка"
for t in cmake ninja pkg-config gcc g++; do
  command -v $t >/dev/null && ok $t || { bad $t; need $t; }
done
ls /usr/lib/x86_64-linux-gnu/libclang*.so* >/dev/null 2>&1 && ok "libclang (bindgen)" || { bad libclang; need libclang-dev; }

echo "== ffmpeg"
if command -v ffmpeg >/dev/null; then
  ok "$(ffmpeg -hide_banner -version | head -1)"
  ENC=$(ffmpeg -hide_banner -encoders 2>/dev/null)
  for e in libx264 libx265 libsvtav1 libvpx-vp9 prores_ks aac libopus h264_vaapi hevc_vaapi av1_vaapi; do
    grep -q " $e " <<<"$ENC" && ok "энкодер $e (в сборке)" || bad "энкодер $e"
  done
  FIL=$(ffmpeg -hide_banner -filters 2>/dev/null)
  for f in ass gblur boxblur overlay scale_vaapi silencedetect loudnorm; do
    grep -q " $f " <<<"$FIL" && ok "фильтр $f" || bad "фильтр $f"
  done
  # Наличие энкодера в сборке не значит, что его умеет GPU, поэтому кодируем по-настоящему
  for e in h264_vaapi hevc_vaapi av1_vaapi; do
    ffmpeg -hide_banner -loglevel error -vaapi_device "$DRI" -f lavfi -i testsrc2=s=1080x1920:r=30:d=1 \
      -vf format=nv12,hwupload -c:v $e -y "$TMP/$e.mp4" >/dev/null 2>&1
    [[ -s $TMP/$e.mp4 ]] && ok "GPU-кодирование $e" || bad "GPU-кодирование $e"
  done
  ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=s=1920x1080:d=1 -c:v libx264 -y "$TMP/src.mp4" &&
  ffmpeg -hide_banner -loglevel error -hwaccel vaapi -hwaccel_device "$DRI" -i "$TMP/src.mp4" -f null - \
    && ok "GPU-декодирование H.264 (vaapi)" || bad "GPU-декодирование H.264 (vaapi)"
else
  bad ffmpeg; need ffmpeg
fi

echo "== Vulkan"
[[ -e /usr/share/vulkan/icd.d/radeon_icd.json ]] && ok "RADV ICD" || { bad "RADV ICD"; need mesa-vulkan-drivers; }
pkg-config --exists vulkan && ok "vulkan.pc $(pkg-config --modversion vulkan)" || { bad "заголовки Vulkan"; need libvulkan-dev; }
[[ -d /usr/include/spirv ]] && ok "spirv-headers" || { bad "spirv-headers (сборка ggml-vulkan)"; need spirv-headers; }
command -v glslc >/dev/null && ok glslc || { bad "glslc (шейдеры ggml-vulkan)"; need glslc; }
command -v vulkaninfo >/dev/null && ok "$(vulkaninfo --summary 2>/dev/null | grep -m1 deviceName | xargs)" \
  || { bad "vulkaninfo (диагностика)"; need vulkan-tools; }

echo "== VAAPI"
command -v vainfo >/dev/null && ok "$(vainfo 2>/dev/null | grep -m1 Driver)" || { bad "vainfo (диагностика)"; need vainfo; }

echo "== mpv"
pkg-config --exists mpv && ok "libmpv $(pkg-config --modversion mpv)" || { bad "libmpv-dev"; need libmpv-dev; }

echo "== GPU"
[[ -r /sys/class/drm/card1/device/mem_info_vram_total ]] &&
  ok "VRAM $(( $(cat /sys/class/drm/card1/device/mem_info_vram_total) / 1048576 )) МиБ, занято $(( $(cat /sys/class/drm/card1/device/mem_info_vram_used) / 1048576 )) МиБ"

echo
if ((${#missing[@]})); then
  echo "Не хватает пакетов:"
  echo "  sudo apt install ${missing[*]}"
else
  echo "Всё на месте."
fi
