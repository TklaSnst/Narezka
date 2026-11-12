#!/usr/bin/env python3
"""Дикторский текст сценария (script.md) → «транскрипт» с расчётными таймкодами.

Записи голоса с канала нет, поэтому берём то, что произносится в кадре (строки `> …`),
и раскладываем по времени с темпом диктора. Заголовки `##`/`###` сценария — эталон границ тем.
Опционально вставляем рекламный блок (проверка, что LLM помечает рекламу).

  make_transcript.py script.md out.json [--ad-after "## 5."]
"""
import json, re, sys, argparse

CPS = 15.0          # символов в секунду (~150 слов/мин для русского)
GAP_SENT, GAP_PARA, GAP_SECT = 0.35, 0.8, 1.5

AD = ("Кстати, пока мы говорим про шифрование. Весь этот разбор я снимал через Lumen VPN, "
      "и это спонсор сегодняшнего ролика. Lumen прячет твой трафик от провайдера, работает на ноутбуке, "
      "телефоне и роутере, и не режет скорость, проверял на тех же замерах в Wireshark. "
      "По ссылке в описании первая неделя бесплатно, а с промокодом из описания — скидка сорок процентов на год. "
      "Всё, возвращаемся к пакетам.")

def clean(s):
    s = re.sub(r"\*\*|__|`", "", s)
    s = re.sub(r"^\s*[-*]\s+", "", s)
    s = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", s)
    return s.strip()

def sentences(text):
    parts = re.split(r"(?<=[.!?…»])\s+(?=[«\"A-ZА-ЯЁ0-9—–-])", text)
    out = []
    for p in (p.strip() for p in parts):
        if out and re.fullmatch(r"\d+[.)]", out[-1]):   # «1.» из нумерованного списка — к следующему
            out[-1] += " " + p
        elif p:
            out.append(p)
    return out

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("script"); ap.add_argument("out")
    ap.add_argument("--ad-after", help="вставить рекламу перед разделом, начинающимся с этой строки")
    a = ap.parse_args()

    paras, sect, cur = [], None, []   # paras: (section_title, level, text)
    def flush():
        if cur: paras.append((sect, " ".join(cur))); cur.clear()
    level_title = {}
    for line in open(a.script, encoding="utf-8"):
        line = line.rstrip("\n")
        if line.startswith("## Дополнительные") or line.startswith("## Источники"):
            break
        m = re.match(r"^(#{2,3})\s+(.*)$", line)
        if m:
            flush()
            if a.ad_after and line.startswith(a.ad_after):
                sect = {"title": "РЕКЛАМА (вставка)", "level": 2, "ad": True}
                paras.append((sect, AD))
            sect = {"title": m.group(2), "level": len(m.group(1))}
            continue
        if line.startswith(">"):
            t = clean(line.lstrip(">").strip())
            if t: cur.append(t)
            elif cur: flush()
        else:
            flush()
    flush()

    sents, sections, t, prev_sect = [], [], 0.0, None
    for sect, text in paras:
        if sect is not prev_sect:
            if prev_sect is not None: t += GAP_SECT - GAP_PARA
            sections.append({**sect, "first": len(sents), "start": round(t, 2)})
            prev_sect = sect
        for s in sentences(text):
            d = max(len(s) / CPS, 0.6)
            sents.append({"id": len(sents), "start": round(t, 2), "end": round(t + d, 2), "text": s})
            t += d + GAP_SENT
        t += GAP_PARA - GAP_SENT
    json.dump({"source": a.script, "duration": round(t, 1), "sentences": sents, "sections": sections},
              open(a.out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print(f"{len(sents)} предложений, {len(sections)} разделов, {t/60:.1f} мин", file=sys.stderr)

if __name__ == "__main__":
    main()
