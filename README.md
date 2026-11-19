# ShortsCutter

Десктопное приложение для Linux, которое нарезает длинное видео в вертикальные шортсы 9:16: смысловая нарезка локальными Whisper и LLM, оформление по шаблонам, баннеры и пакетный рендер через ffmpeg.

Сборка и запуск: `git submodule update --init`, затем `cargo run -p shortscutter`.
Нужны системные ffmpeg и libmpv, для сборки — cmake, компилятор C++, заголовки Vulkan и `glslc` (whisper.cpp собирается
из `third_party/` статически; первая сборка — несколько минут). Модели распознавания приложение предложит скачать само.

## Пакет .deb

Сборка: `cargo install cargo-deb --locked`, затем `./tools/package-deb.sh` → `target/debian/shortscutter_*_amd64.deb`.
Установка: `sudo apt install ./target/debian/shortscutter_*_amd64.deb` (зависимости — ffmpeg, libmpv2, libvulkan1,
xdg-desktop-portal — подтянутся сами). Нужен CPU x86-64 с AVX2 и GPU с Vulkan; модели (~8 ГБ) приложение скачает
при первом запуске в `~/.cache/shortscutter/models`.

## Hyprland

Главное окно: класс (`app_id`) `shortscutter`, заголовок `ShortsCutter`. Диалоги выбора файлов открывает
xdg-desktop-portal, поэтому класс у них — портала (`xdg-desktop-portal-gtk`, `xdg-desktop-portal-kde`…), а заголовок
всегда начинается с `ShortsCutter — ` («ShortsCutter — Выберите видео», «ShortsCutter — Папка для шортсов» и т. п.).
Окна внутри приложения («Лог», «Сохранить шаблон как…», подтверждения) рисуются в главном окне и правил не требуют.

Пример правил (синтаксис Hyprland 0.53+, `~/.config/hypr/hyprland.conf`):

```ini
# Диалоги ShortsCutter — плавающими по центру
windowrule {
    name = shortscutter-dialogs
    match:title = ^ShortsCutter — .*$
    float = on
    center = on
    size = 1000 650
}

# Главное окно — на отдельный рабочий стол (по желанию)
windowrule {
    name = shortscutter-main
    match:class = ^shortscutter$
    workspace = 4
}
```

Для Hyprland до 0.53 то же самое — через `windowrulev2`:

```ini
windowrulev2 = float, title:^ShortsCutter — .*$
windowrulev2 = center, title:^ShortsCutter — .*$
windowrulev2 = size 1000 650, title:^ShortsCutter — .*$
windowrulev2 = workspace 4, class:^shortscutter$
```

В редакторе «Оформления» Alt + перетаскивание двигает кроп видео; если Hyprland забирает Alt + мышь
(`bindm = ALT, mouse:272, movewindow`), пользуйтесь кнопкой «Кроп» или переназначьте модификатор (`$mainMod`).
