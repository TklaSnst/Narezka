"""Спайк 9.0: чат Twitch VOD через GQL. Режимы: cursor (последовательно) и offset (параллельно по окнам)."""
import json,sys,urllib.request,urllib.error,time
from concurrent.futures import ThreadPoolExecutor
CID="kimne78kx3ncx6brgo4mv6wki5h1ko"
UA="Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0"
H="b70a3591ff0f4e0313d126c6a1502d79a1c02baebb288227c582044aa76adf6a"
def gql(var):
    body=[{"operationName":"VideoCommentsByOffsetOrCursor","variables":var,"extensions":{"persistedQuery":{"version":1,"sha256Hash":H}}}]
    for a in range(5):
        try:
            r=urllib.request.Request("https://gql.twitch.tv/gql",data=json.dumps(body).encode(),headers={"Client-ID":CID,"User-Agent":UA})
            d=json.load(urllib.request.urlopen(r,timeout=20))[0]
            if "errors" in d: raise RuntimeError(d["errors"])
            return d["data"]["video"]["comments"]
        except Exception as e:
            err=e; time.sleep(1+a)
    raise err
def node(e):
    n=e["node"]; fr=n["message"]["fragments"]
    return (n["contentOffsetSeconds"], n["id"], "".join(f["text"] for f in fr), [f["emote"]["emoteID"] for f in fr if f.get("emote")])
def by_offset(vid, start, stop):
    """Курсор требует integrity-токен, поэтому идём только по смещению: следующий запрос с секунды последнего
    сообщения (дубли убираются по id). Если вся страница в одной секунде, сдвигаемся на +1 и теряем хвост секунды."""
    off=start; out=[]; req=0
    while off<stop:
        c=gql({"videoID":vid,"contentOffsetSeconds":off}); req+=1
        ms=[node(e) for e in c["edges"]]
        if not ms: break
        out+=ms
        last=ms[-1][0]
        if not c["pageInfo"]["hasNextPage"]: break
        off = last if last>off else off+1
    return [m for m in out if start<=m[0]<stop], req
mode,vid,dur=sys.argv[1],sys.argv[2],int(sys.argv[3])
t=time.time()
if mode=="seq":
    msgs,req=by_offset(vid,0,dur)
else:
    W=int(sys.argv[4]) if len(sys.argv)>4 else 600
    with ThreadPoolExecutor(int(sys.argv[5]) if len(sys.argv)>5 else 8) as ex:
        parts=list(ex.map(lambda s: by_offset(vid,s,s+W), range(0,dur,W)))
    msgs=[m for p in parts for m in p[0]]; req=sum(p[1] for p in parts)
seen=set(); uniq=[]
for m in sorted(msgs):
    if m[1] in seen: continue
    seen.add(m[1]); uniq.append(m)
el=time.time()-t
print(f"{mode}: {len(uniq)} msgs, {req} requests, {el:.1f}s")
json.dump(uniq,open(f"cache/{vid}.chat.json","w"),ensure_ascii=False)
