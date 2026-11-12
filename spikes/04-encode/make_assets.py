"""Спайк 0.4: ассеты для графа шортса — маски скругления, плашка-заголовок (PNG с альфой), субтитры ASS.
Использование: make_assets.py <каталог>"""
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
FONT = str(Path.home() / ".local/share/fonts/JetBrainsMono-nerd/JetBrainsMonoNLNerdFont-ExtraBold.ttf")
SS = 4  # суперсэмплинг для ровных краёв


def rounded_mask(w, h, r, name):
    big = Image.new("L", (w * SS, h * SS), 0)
    ImageDraw.Draw(big).rounded_rectangle((0, 0, w * SS - 1, h * SS - 1), r * SS, fill=255)
    big.resize((w, h), Image.LANCZOS).save(out / name)


# размеры совпадают с панелями в run.py
rounded_mask(1000, 548, 36, "mask-face.png")
rounded_mask(1000, 1100, 36, "mask-game.png")

# плашка: тёмная подложка, зелёная рамка, «терминальный» заголовок
W, H = 1080, 170
big = Image.new("RGBA", (W * SS, H * SS), (0, 0, 0, 0))
d = ImageDraw.Draw(big)
d.rounded_rectangle((40 * SS, 20 * SS, (W - 40) * SS, (H - 20) * SS), 28 * SS,
                    fill=(12, 14, 18, 225), outline=(80, 250, 123, 255), width=4 * SS)
font = ImageFont.truetype(FONT, 54 * SS)
text = "$ ./clutch --1v3"
tw = d.textlength(text, font=font)
d.text(((W * SS - tw) / 2, (H * SS) / 2), text, font=font, fill=(80, 250, 123, 255), anchor="lm")
big.resize((W, H), Image.LANCZOS).save(out / "title.png")

# субтитры: по 3 слова, караоке-подсветка, 60 с
words = ("так ну это просто невероятно смотри как он заходит через короткий и сразу два "
         "в голову третий сидит на точке и не видит ничего вообще").split()
def ts(t):
    return f"{int(t // 3600)}:{int(t % 3600 // 60):02d}:{t % 60:05.2f}"
lines, t, i = [], 0.5, 0
while t < 59:
    chunk = [words[(i + k) % len(words)] for k in range(3)]
    i += 3
    dur = 1.8
    kar = "".join(f"{{\\k{int(dur * 100 / 3)}}}{w} " for w in chunk).rstrip()
    lines.append(f"Dialogue: 0,{ts(t)},{ts(t + dur)},Sub,,0,0,0,,{kar}")
    t += dur + 0.2
(out / "subs.ass").write_text(f"""[Script Info]
ScriptType: v4.00+
PlayResX: 1080
PlayResY: 1920
WrapStyle: 0

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Sub,JetBrainsMonoNL NF ExtraBold,78,&H007BFA50,&H00FFFFFF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,6,2,2,60,60,520,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
""" + "\n".join(lines) + "\n", encoding="utf-8")
print("ассеты:", *sorted(p.name for p in out.iterdir()))
