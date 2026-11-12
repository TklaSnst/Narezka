#!/bin/bash
# Запуск команды в headless sway на GPU (wlroots, рендер на renderD128), 1920x1080@60.
# Нужен, когда в сессии пользователя нет дисплея (экран заблокирован / DPMS off / работа по SSH).
# headless.sh <команда…>
set -e
export XDG_RUNTIME_DIR=/run/user/$(id -u)
CFG=$(mktemp)
cat > "$CFG" <<CONF
output HEADLESS-1 resolution 1920x1080@60Hz
default_border none
CONF
before=$(ls $XDG_RUNTIME_DIR | grep -E "^wayland-[0-9]+$" | sort)
WLR_BACKENDS=headless WLR_RENDERER=gles2 WLR_RENDER_DRM_DEVICE=/dev/dri/renderD128 WLR_LIBINPUT_NO_DEVICES=1 \
  WAYLAND_DISPLAY= sway -c "$CFG" > /tmp/sway-headless.log 2>&1 &
SWAY=$!
for i in $(seq 50); do
  sock=$(comm -13 <(echo "$before") <(ls $XDG_RUNTIME_DIR | grep -E "^wayland-[0-9]+$" | sort) | head -1)
  [ -n "$sock" ] && break; sleep 0.1
done
[ -z "$sock" ] && { echo "sway не стартовал"; cat /tmp/sway-headless.log; kill $SWAY; exit 1; }
export WAYLAND_DISPLAY=$sock
export SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.*.$SWAY.sock 2>/dev/null | head -1)
set +e
"$@"
rc=$?
kill $SWAY; wait $SWAY 2>/dev/null; rm -f "$CFG"
exit $rc
