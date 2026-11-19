#!/usr/bin/env python3
"""Спайк 9.0: LLM ищет моменты по всему тексту стрима (окна с перехлёстом), сверка с клипами зрителей.

  moments.py <vid> [--win 480] [--overlap 60] [--hints] [--limit N]
--hints — добавлять в окно поминутную плотность чата и громкость (проверить, помогают ли сигналы LLM).
"""
import argparse, json, os, subprocess, sys, time
from pathlib import Path
import urllib.request

ap = argparse.ArgumentParser(); ap.add_argument("vid"); ap.add_argument("--win", type=float, default=480)
ap.add_argument("--overlap", type=float, default=60); ap.add_argument("--hints", action="store_true")
ap.add_argument("--limit", type=int, default=0); ap.add_argument("--tag", default="")
a = ap.parse_args()
ROOT = Path(__file__).resolve().parent; C = ROOT / "cache"
SRV = ROOT.parent.parent / "target/dist/llama-server"
MODEL = Path.home() / ".cache/shortscutter/models/Qwen3.5-9B-UD-Q4_K_XL.gguf"
PORT = 8098

tr = json.load(open(C / f"v{a.vid}.turbo.json"))["transcription"]
segs = [(s["offsets"]["from"] / 1000, s["offsets"]["to"] / 1000, s["text"].strip()) for s in tr if s["text"].strip()]
dur = segs[-1][1]

SYSTEM = """You help cut a long livestream (Twitch VOD) into vertical shorts (15–60 s) that are genuinely entertaining for a viewer who never watched the stream.
Input: a window of the auto-transcript of the streamer's speech, lines `[id] hh:mm:ss text`. Transcript has no speaker labels and may contain errors; game/video audio may be mixed in.
Find MOMENTS: a funny line or exchange, a fail, rage/outburst, a win/clutch, a shocking or unexpected turn, a hot take or story with a punchline.
Most of a stream is filler — return only strong moments, usually 0–3 per window. Return an empty list if nothing stands out.
For each moment:
- `start`, `end` — ids of the first and last line. Include the setup (what led to it) and the payoff; 15–60 s total.
- `kind` — funny | fail | rage | win | unexpected | story.
- `hook` — short title in the language of the speech (English here), ≤ 60 chars, like a shorts caption.
- `reason` — 1 sentence IN RUSSIAN for the editor: what happens and why it is interesting.
- `score` — 1–10, how likely a stranger watches it to the end. 8–10 rare and only for clear standalone moments."""
SCHEMA = {"type": "object", "properties": {"moments": {"type": "array", "items": {"type": "object", "properties": {
    "start": {"type": "integer"}, "end": {"type": "integer"},
    "kind": {"type": "string", "enum": ["funny", "fail", "rage", "win", "unexpected", "story"]},
    "hook": {"type": "string"}, "reason": {"type": "string"}, "score": {"type": "integer"}},
    "required": ["start", "end", "kind", "hook", "reason", "score"]}}}, "required": ["moments"]}

def hms(t): t = int(t); return f"{t//3600:d}:{t%3600//60:02d}:{t%60:02d}"
hints = None
if a.hints:
    import numpy as np
    pcm = np.memmap(C / f"v{a.vid}.pcm", dtype=np.int16, mode="r"); n = len(pcm) // 16000
    x = pcm[:n * 16000].reshape(n, 16000).astype(np.float32) / 32768
    rms = 20 * np.log10(np.sqrt((x * x).mean(1)) + 1e-6)
    smp = json.load(open(C / f"{a.vid}.chatsample30.json"))
    hints = (rms, {s["t"] // 60: s["rate"] for s in smp})

def window_text(w0, w1):
    idx = [i for i, s in enumerate(segs) if w0 <= s[0] < w1]
    lines = [f"[{i}] {hms(segs[i][0])} {segs[i][2]}" for i in idx]
    if hints:
        rms, chat = hints; mm = range(int(w0) // 60, int(w1) // 60 + 1)
        med = sorted(chat.values())[len(chat) // 2]
        lines.append("\nChat activity per minute (×median): " + ", ".join(f"{hms(m*60)[:-3]} ×{chat.get(m,0)/med:.1f}" for m in mm))
        lines.append("Peak loudness per minute (dB): " + ", ".join(f"{hms(m*60)[:-3]} {rms[m*60:m*60+60].max():.0f}" for m in mm if m*60 < len(rms)))
    return idx, "\n".join(lines)

def start_server():
    env = dict(os.environ, GGML_VK_DISABLE_HOST_VISIBLE_VIDMEM="1")
    log = open(C / "llm.server.log", "w")
    p = subprocess.Popen([str(SRV), "-m", str(MODEL), "-c", "16384", "-ngl", "99", "-fa", "on", "-t", "4", "--jinja",
                          "-np", "1", "--reasoning-budget", "0", "--host", "127.0.0.1", "--port", str(PORT)],
                         env=env, stdout=log, stderr=subprocess.STDOUT)
    for _ in range(300):
        time.sleep(1)
        if p.poll() is not None: sys.exit("сервер упал, см. llm.server.log")
        try:
            if urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=2).status == 200: return p
        except Exception: pass
    p.kill(); sys.exit("сервер не поднялся")

def ask(user):
    body = {"messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user}],
            "temperature": 0.3, "seed": 1, "max_tokens": 1500,
            "response_format": {"type": "json_schema", "json_schema": {"name": "out", "schema": SCHEMA}},
            "chat_template_kwargs": {"enable_thinking": False}}
    r = json.load(urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{PORT}/v1/chat/completions",
            data=json.dumps(body).encode(), headers={"Content-Type": "application/json"}), timeout=900))
    return json.loads(r["choices"][0]["message"]["content"])["moments"], r.get("timings", {})

t0 = time.time(); srv = start_server(); t_load = time.time() - t0
found = []; pp = gn = 0; w0 = 0.0; nwin = 0
try:
    while w0 < dur:
        w1 = min(w0 + a.win, dur); idx, text = window_text(w0, w1)
        if idx:
            ms, tm = ask(text); nwin += 1; pp += tm.get("prompt_n", 0); gn += tm.get("predicted_n", 0)
            for m in ms:
                if m["start"] in idx and m["end"] >= m["start"]:
                    s, e = segs[m["start"]][0], segs[min(m["end"], len(segs) - 1)][1]
                    found.append(dict(m, t0=s, t1=e))
            print(f"\r{hms(w1)} / {hms(dur)}  моментов {len(found)}", end="", flush=True)
        if a.limit and nwin >= a.limit: break
        w0 += a.win - a.overlap
finally:
    srv.kill()
el = time.time() - t0
# слияние дублей из перехлёста
found.sort(key=lambda m: m["t0"]); merged = []
for m in found:
    if merged and m["t0"] < merged[-1]["t1"] - 5:
        if m["score"] > merged[-1]["score"]: merged[-1] = m
    else: merged.append(m)
merged.sort(key=lambda m: -m["score"])
print(f"\nокон {nwin}, загрузка {t_load:.0f} с, всего {el:.0f} с; prompt {pp} ток, генерация {gn} ток; моментов {len(merged)}")
clips = json.load(open(C / f"{a.vid}.clips.json"))
def hit(m, c): o = c["videoOffsetSeconds"]; return m["t0"] - 30 <= o <= m["t1"] + 15 or o <= m["t0"] <= o + c["durationSeconds"] + 15
for k in (30, 60, 100):
    top = merged[:k]; h = [c for c in clips if any(hit(m, c) for m in top)]
    pop = [c for c in clips if c["viewCount"] >= 20]
    print(f"top{k}: клипов поймано {len(h)}/{len(clips)}, популярных {sum(any(hit(m,c) for m in top) for c in pop)}/{len(pop)}")
json.dump(merged, open(C / f"v{a.vid}.moments{a.tag}.json", "w"), ensure_ascii=False, indent=1)
for m in merged[:25]:
    print(f'{hms(m["t0"])} {m["t1"]-m["t0"]:3.0f}s {m["score"]:2d} {m["kind"]:10} {m["hook"][:50]:50} | {m["reason"][:90]}')
