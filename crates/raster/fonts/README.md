# Встроенные шрифты

Вшиваются в бинарник (`include_bytes!` в `src/fonts.rs`): шаблон на них выглядит одинаково на любой системе.
Все — SIL Open Font License 1.1 (тексты лицензий рядом), файлы не изменены. Кириллица есть во всех.

| Семейство | Файлы | Роль | Источник |
|---|---|---|---|
| Press Start 2P | `PressStart2P-Regular.ttf` | пиксельный | github.com/google/fonts, `ofl/pressstart2p` |
| Montserrat | `Montserrat-{Regular,Bold,ExtraBold,Black}.ttf` | жирный гротеск, `sans-serif` | github.com/JulietaUla/Montserrat, `fonts/ttf` |
| JetBrains Mono | `JetBrainsMono-{Regular,Bold,ExtraBold}.ttf` | моноширинный, `monospace` | github.com/JetBrains/JetBrainsMono, `fonts/ttf` |

Статические начертания, а не variable: не проверено, выставляет ли cosmic-text ось `wght`.
