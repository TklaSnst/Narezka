#!/usr/bin/env python3
"""Спайк 0.3: два прохода LLM по транскрипту через llama-server (Vulkan) с JSON-схемой.

  run.py <ключ_модели> <transcript.json>... [--out DIR] [--think]

Проход 1 — смысловые блоки по окнам с перехлёстом; проход 2 — оценка и подотрезок ≤60 с для каждого блока.
Замеряет время, ток/с, пик VRAM; сравнивает границы с разделами сценария (эталон).
"""
import argparse, json, os, re, subprocess, sys, threading, time, statistics
from pathlib import Path
import requests

HERE = Path(__file__).resolve().parent
PROMPTS = HERE.parent / "prompts"
LM = Path.home() / "work/local-models"
BIN = LM / "bin/vulkan/llama-b11384"
MODELS = {
    "qwen35-9b": "Qwen3.5-9B-UD-Q4_K_XL.gguf",
    "ornith-9b": "Ornith-1.0-9B-UD-Q4_K_XL.gguf",
    "gemma4-12b": "gemma-4-12B-it-qat-UD-Q4_K_XL.gguf",
    "lfm-8b": "LFM2.5-8B-A1B-UD-Q4_K_XL.gguf",
}
PORT = 8099
WIN, OVERLAP = 360.0, 60.0          # окно прохода 1 и перехлёст, секунды
MATCH_TOL = 10.0                    # граница совпала с эталоном, если ближе 10 с
VRAM = Path("/sys/class/drm/card1/device/mem_info_vram_used")

def mmss(t): return f"{int(t)//60:02d}:{int(t)%60:02d}"
def fmt(sents): return "\n".join(f"[{s['id']}] {mmss(s['start'])} {s['text']}" for s in sents)

class VramPeak(threading.Thread):
    def __init__(self):
        super().__init__(daemon=True); self.peak = 0; self.stop = False
    def run(self):
        while not self.stop:
            self.peak = max(self.peak, int(VRAM.read_text())); time.sleep(0.2)

def start_server(model, ctx, think):
    env = dict(os.environ, GGML_VK_DISABLE_HOST_VISIBLE_VIDMEM="1", LD_LIBRARY_PATH=str(BIN))
    cmd = [str(BIN / "llama-server"), "-m", str(LM / "models" / MODELS[model]), "-c", str(ctx),
           "-ngl", "99", "-fa", "on", "-t", "4", "--jinja", "-np", "1",
           "--host", "127.0.0.1", "--port", str(PORT)]
    if not think: cmd += ["--reasoning-budget", "0"]
    log = open(OUT / f"{model}{'-think' if think else ''}.server.log", "w")
    p = subprocess.Popen(cmd, env=env, stdout=log, stderr=subprocess.STDOUT)
    for _ in range(300):
        time.sleep(1)
        if p.poll() is not None: sys.exit(f"сервер упал, см. {log.name}")
        try:
            if requests.get(f"http://127.0.0.1:{PORT}/health", timeout=2).status_code == 200: return p
        except requests.RequestException: pass
    p.kill(); sys.exit("сервер не поднялся")

def ask(system, user, schema, think, stats):
    body = {"messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
            "temperature": 0.3, "seed": 1, "max_tokens": 8192 if think else 3072,
            "response_format": {"type": "json_schema", "json_schema": {"name": "out", "schema": schema}},
            "chat_template_kwargs": {"enable_thinking": think}}
    t0 = time.time()
    r = requests.post(f"http://127.0.0.1:{PORT}/v1/chat/completions", json=body, timeout=900).json()
    dt = time.time() - t0
    tm = r.get("timings", {})
    stats.append({"wall": dt, "prompt_n": tm.get("prompt_n"), "pp": tm.get("prompt_per_second"),
                  "gen_n": tm.get("predicted_n"), "tg": tm.get("predicted_per_second"),
                  "finish": r["choices"][0]["finish_reason"]})
    text = r["choices"][0]["message"]["content"]
    try:
        return json.loads(text), None
    except json.JSONDecodeError as e:
        return None, f"{e}: {text[:200]!r}"

def pass1(tr, think, stats, errors):
    sents, dur = tr["sentences"], tr["duration"]
    system = (PROMPTS / "pass1-boundaries.md").read_text()
    schema = json.loads((PROMPTS / "pass1.schema.json").read_text())
    bounds = {}                       # id первого предложения блока → (kind, topic)
    w0 = 0.0
    while True:
        w1 = min(w0 + WIN, dur)
        win = [s for s in sents if w0 <= s["start"] < w1]
        core0 = 0 if w0 == 0 else w0 + OVERLAP / 2
        core1 = dur if w1 >= dur else w1 - OVERLAP / 2
        out, err = ask(system, fmt(win), schema, think, stats)
        if err: errors.append(f"pass1 {mmss(w0)}: {err}")
        ids = {s["id"] for s in win}
        for b in (out or {}).get("blocks", []):
            if b["start"] not in ids:
                errors.append(f"pass1 {mmss(w0)}: номер вне окна {b['start']}"); continue
            if core0 <= sents[b["start"]]["start"] < core1:
                bounds[b["start"]] = (b["kind"], b["topic"])
        if w1 >= dur: break
        w0 = w1 - OVERLAP
    bounds.setdefault(0, ("content", "?"))
    starts = sorted(bounds)
    blocks = []
    for i, st in enumerate(starts):
        en = (starts[i + 1] if i + 1 < len(starts) else len(sents)) - 1
        blocks.append({"first": st, "last": en, "kind": bounds[st][0], "topic": bounds[st][1],
                       "t0": sents[st]["start"], "t1": sents[en]["end"]})
    return blocks

def pass2(tr, title, blocks, think, stats, errors):
    sents = tr["sentences"]
    system = (PROMPTS / "pass2-score.md").read_text()
    schema = json.loads((PROMPTS / "pass2.schema.json").read_text())
    for b in blocks:
        if b["kind"] != "content": continue
        part = sents[b["first"]:b["last"] + 1]
        user = f"Тема видео: {title}\nДлительность блока: {b['t1'] - b['t0']:.0f} с\n\n{fmt(part)}"
        out, err = ask(system, user, schema, think, stats)
        if err: errors.append(f"pass2 [{b['first']}]: {err}"); continue
        if not (b["first"] <= out["start"] <= out["end"] <= b["last"]):
            errors.append(f"pass2 [{b['first']}]: подотрезок {out['start']}–{out['end']} вне блока")
            out["start"], out["end"] = b["first"], b["last"]
        out["dur"] = sents[out["end"]]["end"] - sents[out["start"]]["start"]
        b["short"] = out

def evaluate(tr, blocks):
    gt = [s["start"] for s in tr["sections"] if s["start"] > 0]
    ad = next((s for s in tr["sections"] if s.get("ad")), None)
    if ad:  # конец рекламы — тоже граница
        nxt = tr["sections"][tr["sections"].index(ad) + 1]; gt.append(nxt["start"])
    pred = [b["t0"] for b in blocks if b["t0"] > 0]
    hit = [g for g in gt if any(abs(g - p) <= MATCH_TOL for p in pred)]
    durs = [b["t1"] - b["t0"] for b in blocks]
    res = {"gt": len(gt), "recall": round(len(hit) / len(gt), 2), "blocks": len(blocks),
           "dur_median": round(statistics.median(durs)), "dur_max": round(max(durs)),
           "missed": [mmss(g) for g in gt if g not in hit]}
    if ad:
        ad1 = nxt["start"]
        ov = sum(max(0, min(b["t1"], ad1) - max(b["t0"], ad["start"])) for b in blocks if b["kind"] == "ad")
        res["ad_found"] = round(ov / (ad1 - ad["start"]), 2)
    shorts = [b["short"] for b in blocks if "short" in b]
    if shorts:
        res["shorts"] = len(shorts)
        res["shorts_le60"] = sum(s["dur"] <= 62 for s in shorts)
        res["score_mean"] = round(statistics.mean(s["score"] for s in shorts), 1)
    return res

def report(model, name, title, tr, blocks, res):
    L = [f"## {name} — {model}", "", f"`{json.dumps(res, ensure_ascii=False)}`", "",
         "| # | время | дл. | вид | тема (проход 1) | оценка | шортс | заголовок / причина |", "|---|---|---|---|---|---|---|---|"]
    for i, b in enumerate(blocks):
        s = b.get("short")
        sh = f"{mmss(tr['sentences'][s['start']]['start'])}–{mmss(tr['sentences'][s['end']]['end'])} ({s['dur']:.0f} с)" if s else ""
        L.append(f"| {i} | {mmss(b['t0'])} | {b['t1']-b['t0']:.0f} | {b['kind']} | {b['topic']} | "
                 f"{s['score'] if s else ''}{'' if not s or s['standalone'] else ' ⚠'} | {sh} | "
                 f"{('**' + s['title'] + '** — ' + s['reason']) if s else ''} |")
    return "\n".join(L) + "\n"

def main():
    global OUT
    ap = argparse.ArgumentParser()
    ap.add_argument("model", choices=MODELS); ap.add_argument("transcripts", nargs="+")
    ap.add_argument("--out", default=str(HERE.parent / "out/03")); ap.add_argument("--think", action="store_true")
    ap.add_argument("--ctx", type=int, default=12288)
    a = ap.parse_args()
    OUT = Path(a.out); OUT.mkdir(parents=True, exist_ok=True)
    tag = a.model + ("-think" if a.think else "")
    base = int(VRAM.read_text())
    vp = VramPeak(); vp.start()
    t0 = time.time(); srv = start_server(a.model, a.ctx, a.think); load = time.time() - t0
    summary = {"model": tag, "load_s": round(load, 1), "runs": []}
    md = ""
    try:
        for path in a.transcripts:
            tr = json.load(open(path)); name = Path(path).stem
            title = open(tr["source"], encoding="utf-8").readline().lstrip("# ").strip()
            s1, s2, errors = [], [], []
            t = time.time(); blocks = pass1(tr, a.think, s1, errors); t1 = time.time() - t
            t = time.time(); pass2(tr, title, blocks, a.think, s2, errors); t2 = time.time() - t
            res = evaluate(tr, blocks)
            run = {"video": name, "minutes": round(tr["duration"] / 60, 1), "pass1_s": round(t1), "pass2_s": round(t2),
                   "requests": len(s1) + len(s2), "errors": errors,
                   "pp_tok_s": round(statistics.mean(x["pp"] for x in s1 + s2 if x["pp"])),
                   "tg_tok_s": round(statistics.mean(x["tg"] for x in s1 + s2 if x["tg"]), 1),
                   "gen_tokens": sum(x["gen_n"] or 0 for x in s1 + s2),
                   "truncated": sum(x["finish"] == "length" for x in s1 + s2), **res}
            summary["runs"].append(run)
            json.dump(blocks, open(OUT / f"{name}.{tag}.blocks.json", "w"), ensure_ascii=False, indent=1)
            md += report(tag, name, title, tr, blocks, res)
            print(json.dumps(run, ensure_ascii=False), flush=True)
    finally:
        srv.terminate(); srv.wait(); vp.stop = True
    summary["vram_peak_mib"] = (vp.peak - base) // 2**20
    json.dump(summary, open(OUT / f"{tag}.summary.json", "w"), ensure_ascii=False, indent=1)
    (OUT / f"{tag}.report.md").write_text(md)
    print(f"VRAM +{summary['vram_peak_mib']} МиБ, загрузка {load:.0f} с")

if __name__ == "__main__":
    main()
