"""Выборка чата: одна страница каждые STEP с. Плотность = число сообщений / длительность страницы."""
import sys,time,json
from concurrent.futures import ThreadPoolExecutor
src=open("chat.py").read().split("def by_offset")[0]; exec(src)
vid,dur,step,th=sys.argv[1],int(sys.argv[2]),int(sys.argv[3]),int(sys.argv[4])
def one(off):
    c=gql({"videoID":vid,"contentOffsetSeconds":off}); ms=[node(e) for e in c["edges"]]
    ms=[m for m in ms if m[0]>=off] or ms
    span=max(1,ms[-1][0]-ms[0][0]+1) if ms else 0
    return {"t":off,"n":len(ms),"span":span,"rate":len(ms)/span if span else 0,"msgs":ms}
t=time.time()
with ThreadPoolExecutor(th) as ex: res=list(ex.map(one, range(0,dur,step)))
print(f"sample step={step}: {len(res)} requests, {time.time()-t:.1f}s")
json.dump(res,open(f"cache/{vid}.chatsample{step}.json","w"),ensure_ascii=False)
