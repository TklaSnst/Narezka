"""Клипы зрителей канала, привязанные к VOD (videoOffsetSeconds). Обычный (не persisted) GQL-запрос, постранично."""
import json,sys,urllib.request,time
CID="kd1unb4b3q4t58fwlpcbzcbnm76a8fp"
def gql(q):
    r=urllib.request.Request("https://gql.twitch.tv/gql",data=json.dumps({"query":q}).encode(),headers={"Client-ID":CID})
    return json.load(urllib.request.urlopen(r,timeout=20))
login,vid,period=sys.argv[1],sys.argv[2],sys.argv[3]
cursor=None; out=[]; req=0; t=time.time()
while True:
    after=f',after:"{cursor}"' if cursor else ""
    d=gql('query{user(login:"%s"){clips(first:100%s,criteria:{period:%s,sort:VIEWS_DESC}){edges{cursor node{slug title viewCount durationSeconds createdAt videoOffsetSeconds video{id}}} pageInfo{hasNextPage}}}}'%(login,after,period))
    req+=1
    if "errors" in d: print(d["errors"]); break
    c=d["data"]["user"]["clips"]; out+= [e["node"] for e in c["edges"]]
    if not c["pageInfo"]["hasNextPage"] or not c["edges"] or req>=30: break
    cursor=c["edges"][-1]["cursor"]
mine=[c for c in out if c.get("video") and c["video"]["id"]==vid]
print(f"{req} req {time.time()-t:.1f}s, всего клипов {len(out)}, у VOD {vid}: {len(mine)}")
for c in sorted(mine,key=lambda c:-c["viewCount"])[:15]:
    print(f'{c["videoOffsetSeconds"]:>6} {c["durationSeconds"]:>3}s {c["viewCount"]:>7}  {c["title"][:60]}')
json.dump(mine,open(f"cache/{vid}.clips.json","w"),ensure_ascii=False)
