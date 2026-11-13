#!/usr/bin/env python3
"""Спайк 2.2: эмбеддинги предложений (llama-server --embedding) + TextTiling, сверка с разделами сценария.

  tiling.py net.json [--port 8090] [--win 45] [--instr]
Сервер: llama-server -m Qwen3-Embedding-0.6B-Q8_0.gguf --embedding --pooling last -ub 2048 -b 2048 --port 8090
"""
import json, sys, argparse, urllib.request, math, time

def embed(texts, port):
    out = []
    for i in range(0, len(texts), 64):
        req = urllib.request.Request(f"http://127.0.0.1:{port}/v1/embeddings",
            data=json.dumps({"input": texts[i:i+64]}).encode(), headers={"Content-Type": "application/json"})
        d = json.load(urllib.request.urlopen(req))
        out += [e["embedding"] for e in sorted(d["data"], key=lambda e: e["index"])]
    return out

def norm(v):
    n = math.sqrt(sum(x*x for x in v)) or 1.0
    return [x/n for x in v]

def mean(vs):
    s = [0.0]*len(vs[0])
    for v in vs:
        for i, x in enumerate(v): s[i] += x
    return norm(s)

def cos(a, b): return sum(x*y for x, y in zip(a, b))

def tiling(sents, E, win, min_gap, k):
    n = len(sents)
    sims = [None]*n  # sims[i] — граница перед предложением i
    for i in range(1, n):
        t = sents[i]["start"]
        L = [E[j] for j in range(i) if sents[j]["end"] >= t - win] or [E[i-1]]
        R = [E[j] for j in range(i, n) if sents[j]["start"] <= t + win] or [E[i]]
        sims[i] = cos(mean(L), mean(R))
    depth = [0.0]*n
    for i in range(1, n):
        l = i
        while l > 1 and sims[l-1] >= sims[l]: l -= 1
        r = i
        while r < n-1 and sims[r+1] >= sims[r]: r += 1
        depth[i] = (sims[l]-sims[i]) + (sims[r]-sims[i])
    ds = [d for d in depth[1:]]
    mu = sum(ds)/len(ds); sd = math.sqrt(sum((d-mu)**2 for d in ds)/len(ds))
    cut = mu + k*sd
    cand = sorted((i for i in range(1, n) if depth[i] > cut), key=lambda i: -depth[i])
    picked = []
    for i in cand:
        if all(abs(sents[i]["start"] - sents[j]["start"]) >= min_gap for j in picked):
            picked.append(i)
    return sorted(picked), sims, depth

def score(picked, sents, truth, tol):
    tt = [sents[s]["start"] for s in truth]
    pt = [sents[i]["start"] for i in picked]
    hit_t = sum(any(abs(a-b) <= tol for b in pt) for a in tt)
    hit_p = sum(any(abs(a-b) <= tol for b in tt) for a in pt)
    return hit_t/len(tt), (hit_p/len(pt) if pt else 0)

ap = argparse.ArgumentParser()
ap.add_argument("files", nargs="+"); ap.add_argument("--port", default=8090)
ap.add_argument("--instr", action="store_true")
a = ap.parse_args()
for f in a.files:
    d = json.load(open(f)); sents = d["sentences"]
    truth = [s["first"] for s in d["sections"] if s["first"] > 0]
    texts = [s["text"] for s in sents]
    if a.instr:
        texts = ["Instruct: Определи тему фрагмента видео\nQuery: " + t for t in texts]
    t0 = time.time(); E = [norm(e) for e in embed(texts, a.port)]
    print(f"{f}: {len(sents)} предл., эмбеддинги {time.time()-t0:.1f} с, dim {len(E[0])}, разделов {len(truth)}")
    for win in (30, 45, 60, 90):
        for k in (-0.5, 0.0, 0.5):
            for mg in (20, 40):
                p, _, _ = tiling(sents, E, win, mg, k)
                r, pr = score(p, sents, truth, 10)
                print(f"  win {win:3} k {k:+.1f} gap {mg}: найдено {len(p):3}  полнота {r:.2f}  точность {pr:.2f}")
