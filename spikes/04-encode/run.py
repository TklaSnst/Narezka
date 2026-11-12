"""Спайк 0.4: кодировщики на 60-с шортсе 1080×1920.
1) эталон: граф шортса → libx264 без потерь (ref.mkv);
2) каждый кодировщик кодирует эталон — время, CPU, размер, SSIM/XPSNR против эталона;
3) полный цикл (декод + граф + кодирование) для основных вариантов.
Использование: run.py <исходник> <начало_с> <каталог_вывода> [фильтр_по_имени]"""
import json, os, re, resource, subprocess, sys, time
from pathlib import Path

src, start, out = sys.argv[1], sys.argv[2], Path(sys.argv[3])
only = sys.argv[4] if len(sys.argv) > 4 else ""
here = Path(__file__).parent
assets = out / "assets"
DUR = "60"
VA = ["-vaapi_device", "/dev/dri/renderD128"]
UP = ["-vf", "format=nv12,hwupload"]


def abr(b):  # одинаковые правила VBV для всех кодировщиков с битрейтом
    m = int(b.rstrip("M"))
    return ["-b:v", b, "-maxrate", f"{m * 3 // 2}M", "-bufsize", f"{m * 2}M"]


# имя -> (опции до входа, опции кодирования, расширение)
ENC = {}
for b in ["8M", "12M", "16M"]:
    for p in ["medium", "fast", "veryfast"]:
        ENC[f"x264-{p}-{b}"] = ([], ["-c:v", "libx264", "-preset", p, *abr(b)], "mp4")
    ENC[f"h264_vaapi-{b}"] = (VA, [*UP, "-c:v", "h264_vaapi", "-rc_mode", "VBR", *abr(b)], "mp4")
for b in ["6M", "8M", "12M"]:
    ENC[f"hevc_vaapi-{b}"] = (VA, [*UP, "-c:v", "hevc_vaapi", "-rc_mode", "VBR", *abr(b)], "mp4")
ENC["x264-medium-crf20"] = ([], ["-c:v", "libx264", "-preset", "medium", "-crf", "20"], "mp4")
ENC["h264_vaapi-cqp20"] = (VA, [*UP, "-c:v", "h264_vaapi", "-rc_mode", "CQP", "-qp", "20"], "mp4")
ENC["h264_vaapi-12M-q0"] = (VA, [*UP, "-c:v", "h264_vaapi", "-rc_mode", "VBR", "-quality", "0", *abr("12M")], "mp4")
ENC["x265-fast-8M"] = ([], ["-c:v", "libx265", "-preset", "fast", *abr("8M")], "mp4")
ENC["svtav1-p8-crf32"] = ([], ["-c:v", "libsvtav1", "-preset", "8", "-crf", "32"], "mp4")
ENC["svtav1-p10-crf32"] = ([], ["-c:v", "libsvtav1", "-preset", "10", "-crf", "32"], "mp4")
ENC["vp9-good4-crf32"] = ([], ["-c:v", "libvpx-vp9", "-deadline", "good", "-cpu-used", "4", "-row-mt", "1",
                               "-b:v", "0", "-crf", "32"], "webm")
ENC["vp9-rt8-crf32"] = ([], ["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-row-mt", "1",
                             "-b:v", "0", "-crf", "32"], "webm")
ENC["prores-hq"] = ([], ["-c:v", "prores_ks", "-profile:v", "3", "-pix_fmt", "yuv422p10le"], "mov")

FULL = ["x264-medium-12M", "x264-veryfast-12M", "h264_vaapi-12M", "hevc_vaapi-8M",
        "svtav1-p8-crf32", "vp9-rt8-crf32", "prores-hq", "null"]
AUDIO = {"mp4": ["-c:a", "aac", "-b:a", "192k"], "webm": ["-c:a", "libopus", "-b:a", "160k"],
         "mov": ["-c:a", "pcm_s16le"]}


def graph_inputs(tail=""):
    g = (here / "graph.txt").read_text().replace("ASSETS", str(assets)).replace("\n", "")
    g = g.removesuffix("[out]") + tail + "[out]"  # хвост: hwupload для VAAPI
    return ["-ss", start, "-t", DUR, "-i", src, "-i", str(assets / "mask-face.png"),
            "-i", str(assets / "mask-game.png"), "-i", str(assets / "title.png"),
            "-filter_complex", g, "-map", "[out]", "-map", "0:a:0"]


def run(cmd):
    r0, t0 = resource.getrusage(resource.RUSAGE_CHILDREN), time.monotonic()
    p = subprocess.run(["ffmpeg", "-hide_banner", "-nostats", "-y", *cmd], capture_output=True, text=True)
    t, r1 = time.monotonic() - t0, resource.getrusage(resource.RUSAGE_CHILDREN)
    if p.returncode:
        raise RuntimeError(p.stderr[-2000:])
    cpu = (r1.ru_utime - r0.ru_utime) + (r1.ru_stime - r0.ru_stime)
    return t, cpu, p.stderr


def metrics(f, ref):
    lavfi = ("[0:v]format=yuv420p,split[a0][a1];[1:v]split[b0][b1];"
             "[a0][b0]ssim[s];[a1][b1]xpsnr[x]")
    _, _, err = run(["-i", str(f), "-i", str(ref), "-filter_complex", lavfi,
                     "-map", "[s]", "-f", "null", "-", "-map", "[x]", "-f", "null", "-"])
    ssim = float(re.search(r"SSIM .*All:([\d.]+)", err).group(1))
    xp = re.search(r"XPSNR\s+y:\s*([\d.]+)", err)
    return ssim, float(xp.group(1)) if xp else None


def mbps(f):
    d = float(subprocess.check_output(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                                       "-of", "csv=p=0", str(f)]))
    return f.stat().st_size * 8 / d / 1e6


res_path = out / "results.json"
res = json.loads(res_path.read_text()) if res_path.exists() else {"enc": {}, "full": {}}
ref = out / "ref.mkv"
if not ref.exists():
    t, cpu, _ = run([*graph_inputs(), "-c:v", "libx264", "-preset", "ultrafast", "-qp", "0",
                     "-c:a", "pcm_s16le", str(ref)])
    res["ref"] = {"time": t, "cpu": cpu}
    print(f"эталон: {t:.1f} с, CPU {cpu:.0f} с", flush=True)

for name, (pre, opts, ext) in ENC.items():
    if only not in name:
        continue
    f = out / "enc" / f"{name}.{ext}"
    f.parent.mkdir(exist_ok=True)
    t, cpu, _ = run([*pre, "-i", str(ref), "-an", *opts, str(f)])
    ssim, xp = metrics(f, ref)
    res["enc"][name] = {"time": t, "cpu": cpu, "mbps": mbps(f), "ssim": ssim, "xpsnr": xp}
    print(f"{name:22} {t:6.1f} с  CPU {cpu:6.0f} с  {mbps(f):6.1f} Мбит/с  SSIM {ssim:.4f}  XPSNR {xp}", flush=True)
    res_path.write_text(json.dumps(res, indent=1, ensure_ascii=False))

for name in FULL:
    if only not in name:
        continue
    if name == "null":  # только декод + граф
        pre, opts, ext, dst, tail = [], ["-f", "null"], None, "-", ""
    else:
        pre, opts, ext = ENC[name]
        tail = ""
        if opts[:2] == UP:  # -vf нельзя вместе с filter_complex
            opts, tail = opts[2:], "," + UP[1]
        dst = str(out / "full" / f"{name}.{ext}")
        (out / "full").mkdir(exist_ok=True)
    t, cpu, _ = run([*pre, *graph_inputs(tail), *opts, *(AUDIO[ext] if ext else ["-c:a", "pcm_s16le"]), dst])
    res["full"][name] = {"time": t, "cpu": cpu}
    print(f"полный {name:22} {t:6.1f} с  CPU {cpu:6.0f} с", flush=True)
    res_path.write_text(json.dumps(res, indent=1, ensure_ascii=False))
