# ShortsCutter

Десктопное приложение для Linux, которое нарезает длинное видео в вертикальные шортсы 9:16: смысловая нарезка локальными Whisper и LLM, оформление по шаблонам, баннеры и пакетный рендер через ffmpeg.

## Требования

- Linux x86-64, CPU с AVX2 (Intel Haswell / AMD Excavator и новее), Wayland или X11.
- GPU с Vulkan 1.2+ для распознавания и нарезки (AMD — RADV из mesa, NVIDIA — проприетарный драйвер, Intel — ANV).
  Видеопамяти — от 8 ГБ: LLM нарезки занимает ~6 ГБ. Без GPU приложение запустится, но анализ пойдёт на CPU — очень медленно.
- ffmpeg и libmpv — системные. Возможности ffmpeg проверяются при запуске: аппаратное кодирование
  (VAAPI H.264/HEVC) появится в «Экспорте», если его видит ffmpeg (`mesa-va-drivers` на Ubuntu); иначе — кодеры CPU.
- xdg-desktop-portal с бэкендом (gtk, kde, hyprland…) — для диалогов выбора файлов.
- Модели (~8.2 ГБ: Whisper large-v3-turbo, Qwen3.5-9B, эмбеддинги) приложение скачает при первом запуске в
  `~/.cache/shortscutter/models` — или укажите готовые файлы на вкладке «Импорт».

## Установка .deb (Ubuntu 26.04 / Debian)

```sh
sudo apt install ./shortscutter_0.1.0-1_amd64.deb
```

Зависимости (ffmpeg, libmpv2, libvulkan1, xdg-desktop-portal) подтянутся сами, mesa-драйверы — рекомендуемые.
Приложение появится в меню как «ShortsCutter»; из терминала — `shortscutter [видео…]`.

Сборка пакета: `cargo install cargo-deb --locked`, затем `./tools/package-deb.sh` → `target/debian/shortscutter_*_amd64.deb`
(профиль `dist`; whisper.cpp и llama.cpp собираются под x86-64-v3, а не под CPU машины сборки).

## Сборка из исходников

Нужны Rust 1.85+ (edition 2024; проще всего через rustup), cmake, компилятор C/C++, libclang (для bindgen),
заголовки Vulkan и SPIR-V, компилятор шейдеров `glslc`, заголовки libmpv, pkg-config, git. whisper.cpp и llama.cpp —
подмодули в `third_party/`, собираются статически (`llama-server` — отдельный бинарник рядом).

Ubuntu 26.04 / Debian:

```sh
sudo apt install build-essential cmake pkg-config git libclang-dev libvulkan-dev glslc spirv-headers libmpv-dev ffmpeg \
    mesa-vulkan-drivers mesa-va-drivers xdg-desktop-portal
```

Arch Linux (проверено 2026-11-19):

```sh
sudo pacman -S --needed base-devel git cmake clang rustup vulkan-headers vulkan-icd-loader shaderc spirv-headers mpv ffmpeg \
    vulkan-radeon xdg-desktop-portal        # vulkan-radeon — для AMD; Intel — vulkan-intel, NVIDIA — nvidia-utils
rustup default stable
```

Fedora (не проверялось; ffmpeg с x264 и mpv — из RPM Fusion):

```sh
sudo dnf install gcc-c++ cmake clang-devel git vulkan-loader-devel vulkan-headers spirv-headers-devel glslc mpv-libs-devel ffmpeg \
    mesa-vulkan-drivers xdg-desktop-portal
```

Сборка и запуск:

```sh
git clone https://github.com/TklaSnst/Narezka.git shortscutter && cd shortscutter
git submodule update --init
cargo build --release -p shortscutter      # первая сборка — минут 6–10 (whisper.cpp и llama.cpp с шейдерами Vulkan)
./target/release/shortscutter
```

Такой бинарник собран под CPU этой машины (`GGML_NATIVE`); для переносимой сборки — `SC_PORTABLE=1 cargo build --profile dist`.
`llama-server` бинарник ищет рядом с собой, в `../lib/shortscutter/` и в каталоге сборки — при запуске из `target/` ничего
копировать не нужно. Тесты: `cargo test --workspace` (часть тестов вызывает настоящий ffmpeg).

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
