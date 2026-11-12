"""Спайк 0.2: ASS с двумя дорожками «текущего слова» для сравнения таймкодов на слух.
Сверху — таймкоды токенов (t0/t1), снизу — DTW. Использование: karaoke.py <слова.txt> <от_с> <до_с> <выход.ass>"""
import sys

words_txt, start, end, dst = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
words = []
for line in open(words_txt):
    t0, t1, _, dtw, *text = line.split()
    words.append((float(t0), float(t1), float(dtw), " ".join(text)))

def ts(t):
    t = max(0.0, t - start)
    return f"{int(t // 3600)}:{int(t % 3600 // 60):02}:{t % 60:05.2f}"

out = ["[Script Info]", "PlayResX: 1920", "PlayResY: 1080", "", "[V4+ Styles]",
       "Format: Name, Fontname, Fontsize, PrimaryColour, OutlineColour, BorderStyle, Outline, Alignment, MarginV",
       "Style: Tok,DejaVu Sans,72,&H0000FFFF,&H00000000,1,4,8,60",
       "Style: Dtw,DejaVu Sans,72,&H0000FF00,&H00000000,1,4,2,60",
       "", "[Events]", "Format: Layer, Start, End, Style, Text",
       f"Dialogue: 0,{ts(start)},{ts(end)},Tok,{{\\an7\\fs36}}жёлтый сверху: токены t0/t1",
       f"Dialogue: 0,{ts(start)},{ts(end)},Dtw,{{\\an1\\fs36}}зелёный снизу: DTW"]
for i, (t0, t1, dtw, text) in enumerate(words):
    if not start <= t0 < end:
        continue
    nxt = words[i + 1][2] if i + 1 < len(words) else dtw + 0.5
    out.append(f"Dialogue: 0,{ts(t0)},{ts(max(t1, t0 + 0.1))},Tok,{text}")
    # у DTW только точка начала — слово держим до начала следующего, но не дольше 1.5 с
    out.append(f"Dialogue: 0,{ts(dtw)},{ts(min(max(nxt, dtw + 0.1), dtw + 1.5))},Dtw,{text}")
open(dst, "w").write("\n".join(out) + "\n")
