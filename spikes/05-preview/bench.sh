#!/bin/bash
# Серия замеров спайка 0.5 в headless sway. Результаты — spikes/out/05/bench/*.json
cd "$(dirname "$0")"
OUT=../out/05/bench; mkdir -p $OUT
V1080=~/work/twitch/cache/2847668151/seg00_Counter-Strike.mp4
V4H=../out/05/src-4k-hevc.mp4
V4A=../out/05/src-4k-av1.mp4
run() { name=$1; shift; echo "== $name"; timeout 120 ./target/release/preview-spike "$@" --bench 10 --out $OUT/$name.json 2>/dev/null | grep -E "ui_fps|video_fps|drops|cpu|gpu|seek|segment|hwdec"; }
run 1080-h264-vaapi        $V1080
run 1080-h264-sw           $V1080 --hwdec no
run 1080-h264-vaapi-div1   $V1080 --blur-div 1
run 4k-hevc-vaapi          $V4H
run 4k-hevc-vaapi-fbo1080  $V4H --fbo-max-h 1080
run 4k-hevc-vaapi-div1     $V4H --blur-div 1
run 4k-hevc-sw             $V4H --hwdec no
run 4k-av1-vaapi           $V4A
run 4k-av1-sw              $V4A --hwdec no
run proxy720-g120-vaapi    ../out/05/proxy720-g120.mp4
run proxy720-g15-vaapi     ../out/05/proxy720-g15.mp4
