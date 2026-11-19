"""Сигналы стрима по секундам и их проверка на клипах зрителей (временный эталон вместо таймкодов пользователя)."""
import json,sys,numpy as np,time
vid=sys.argv[1]; t0=time.time()
a=np.memmap(f"cache/v{vid}.pcm",dtype=np.int16,mode="r"); n=len(a)//16000
x=a[:n*16000].reshape(n,16000).astype(np.float32)/32768
rms=20*np.log10(np.sqrt((x*x).mean(1))+1e-6)
# всплеск громкости: превышение над скользящей медианой 5 мин, сглажено по 3 с
def rolling_med(v,w):
    pad=np.pad(v,(w//2,w//2),mode="edge"); from numpy.lib.stride_tricks import sliding_window_view as sw
    return np.median(sw(pad,w)[:len(v)],axis=1)
base=rolling_med(rms[::5],60).repeat(5)[:n]
loud=np.convolve(rms-base,np.ones(3)/3,"same")
print(f"громкость: {time.time()-t0:.1f}s, {n} c")
smp=json.load(open(f"cache/{vid}.chatsample30.json"))
chat=np.zeros(n); 
for s in smp:
    if s["t"]<n: chat[s["t"]:s["t"]+30]=s["rate"]
cb=rolling_med(chat[::30],21).repeat(30)[:n]; chat_rel=chat/(cb+0.5)
clips=json.load(open(f"cache/{vid}.clips.json"))
def peaks(sig,k,gap=60):
    order=np.argsort(-sig); out=[]
    for i in order:
        if all(abs(i-j)>gap for j in out): out.append(int(i))
        if len(out)>=k: break
    return out
def hit(pk,c): o=c["videoOffsetSeconds"]; return any(o-20<=p<=o+c["durationSeconds"]+20 for p in pk)
for name,sig in [("громкость",loud),("чат (выборка)",chat_rel),("сумма z",(loud-loud.mean())/loud.std()+(chat_rel-chat_rel.mean())/chat_rel.std())]:
    for k in (30,60,100):
        pk=peaks(sig,k); h=[c for c in clips if hit(pk,c)]
        top=[c for c in clips if c["viewCount"]>=20]
        print(f"{name:15} top{k:<3}: клипов поймано {len(h)}/{len(clips)}, из популярных (≥20 просм.) {sum(hit(pk,c) for c in top)}/{len(top)}")
# случайная база
rng=np.random.default_rng(0); r=[sum(hit(list(rng.integers(0,n,60)),c) for c in clips) for _ in range(50)]
print("случайные 60 точек: в среднем", np.mean(r))
