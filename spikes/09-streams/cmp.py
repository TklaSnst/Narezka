import json,sys,statistics as st
vid=sys.argv[1]; full=json.load(open(f"cache/{vid}.chat.json")); smp=json.load(open(f"cache/{vid}.chatsample30.json"))
W=30; cnt={}
for m in full: cnt[m[0]//W]=cnt.get(m[0]//W,0)+1
xs=[];ys=[]
for s in smp:
    if s["t"]>=3600: break
    xs.append(cnt.get(s["t"]//W,0)/W); ys.append(s["rate"])
print("pearson", round(st.correlation(xs,ys),3), "n",len(xs))
rk=lambda v: {i:r for r,i in enumerate(sorted(range(len(v)),key=lambda i:-v[i]))}
a,b=rk(xs),rk(ys); top=set(i for i in a if a[i]<10); print("top10 full ∩ top20 sample:", sum(1 for i in top if b[i]<20))
