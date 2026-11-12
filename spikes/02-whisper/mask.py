"""Спайк 0.2: заглушить нулями всё, что VAD не считает речью (шкала времени сохраняется).
Использование: mask.py <вход.wav> <вывод vad-speech-segments> <выход.wav> [паддинг_мс]"""
import array, re, sys, wave

src, segs_txt, dst = sys.argv[1:4]
pad = int(sys.argv[4]) if len(sys.argv) > 4 else 200
with wave.open(src) as w:
    params, rate = w.getparams(), w.getframerate()
    pcm = array.array("h", w.readframes(w.getnframes()))

keep = bytearray(len(pcm))
for a, b in re.findall(r"Speech segment \d+: start = ([\d.]+), end = ([\d.]+)", open(segs_txt).read()):
    # vad-speech-segments печатает время в сотых долях секунды
    lo = max(0, int((float(a) / 100 - pad / 1000) * rate))
    hi = min(len(pcm), int((float(b) / 100 + pad / 1000) * rate))
    keep[lo:hi] = b"\x01" * (hi - lo)

for i, k in enumerate(keep):
    if not k:
        pcm[i] = 0
with wave.open(dst, "wb") as w:
    w.setparams(params)
    w.writeframes(pcm.tobytes())
print(f"речь: {sum(keep) / rate:.0f} с из {len(pcm) / rate:.0f} с")
