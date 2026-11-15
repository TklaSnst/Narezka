app-title = ShortsCutter

tab-import = Импорт
tab-cut = Нарезка
tab-design = Оформление
tab-export = Экспорт
tab-library = Библиотека
tab-settings = Настройки

settings-language = Язык интерфейса
settings-saved = Настройки сохранены
settings-save-failed = Не удалось сохранить настройки: { $error }

lang-ru = Русский
lang-en = English

placeholder-not-ready = Этот раздел появится на следующих шагах.
batch-summary = { $sources ->
    [0] Пачка пуста
    [one] В пачке { $sources } ролик
    [few] В пачке { $sources } ролика
   *[many] В пачке { $sources } роликов
}

import-add = Добавить видео…
import-drop-hint = или перетащите файлы в окно
import-drop-here = Отпустите, чтобы добавить в пачку
import-empty = Пачка пуста — добавьте один или несколько роликов.
import-dialog-title = Выберите видео
import-dialog-filter = Видео
import-duplicate = Этот файл уже есть в пачке: { $name }
import-remove = Убрать из пачки
import-col-file = Файл
import-col-duration = Длительность
import-col-size = Кадр
import-col-fps = Кадр/с
import-col-audio = Звук
import-col-status = Состояние
import-audio-yes = есть
import-audio-no = нет

source-status-new = Чтение…
source-status-ready = Готов
source-status-analyzing = Анализ…
source-status-analyzed = Проанализирован

media-err-no-ffprobe = ffprobe не найден
media-err-timeout = ffprobe не ответил вовремя
media-err-no-video = В файле нет видео
media-err-io = Не удалось прочитать файл
media-err-unreadable = Файл не распознан как видео

settings-ffmpeg = ffmpeg
caps-checking = Проверка ffmpeg…
caps-summary = ffmpeg { $version } · GPU: { $gpu }
caps-none = нет
caps-no-ffmpeg = ffmpeg или ffprobe не найдены — импорт и рендер недоступны
caps-missing-filters = В сборке ffmpeg нет фильтров: { $filters }
caps-vaapi-device = Устройство VAAPI: { $device }
caps-kind-gpu = GPU
caps-kind-cpu = CPU

cut-sources = Ролики
cut-no-sources = Сначала добавьте ролики на вкладке «Импорт».
cut-source-not-ready = Ролик ещё читается или не открылся
cut-shorts-count = { $n ->
    [one] { $n } шортс
    [few] { $n } шортса
   *[many] { $n } шортсов
}
cut-player-failed = Превью недоступно (libmpv): { $error }
cut-open-failed = Не удалось открыть ролик: { $error }
cut-preview-empty = Выберите ролик слева
cut-preview-loading = Загрузка…
cut-play = Воспроизведение / пауза (Пробел)
cut-frame-back = Кадр назад (←), Shift+← — на 5 с
cut-frame-fwd = Кадр вперёд (→), Shift+→ — на 5 с
cut-mark-in = [ Начало (I)
cut-mark-out = Конец (O) ]
cut-add-short = + Шортс (Enter)
cut-delete-short = Удалить (Del)
cut-zoom-fit = Весь ролик
cut-timeline-hint = Колесо — масштаб, Shift+колесо — прокрутка, клик — перемотка. Края выбранного шортса можно тянуть. N — следующий кандидат, Esc — снять выбор.
cut-shorts-empty = Все кандидаты убраны. Шортс можно отметить вручную: начало (I), конец (O), Enter.
cut-err-no-in = Сначала отметьте начало (I)
cut-err-bad-range = Конец шортса должен быть позже начала
cut-err-out-of-source = Шортс выходит за конец ролика
cut-err-overlap = Шортс пересекается с другим шортсом
cut-err-unknown = Шортс или ролик не найден
cut-list = Шортсы
cut-list-counts = Принято: { $accepted } · кандидатов: { $candidates } · отклонено: { $rejected }
cut-allow-overlap = Разрешить перекрытия
cut-allow-overlap-hint = Принятые шортсы могут пересекаться по времени
cut-accept = ✔ Принять (A)
cut-reject = ✖ Отклонить (R)
cut-split = Разделить (S)
cut-split-hint = Разделить выбранный шортс у курсора (по ближайшей границе слов)
cut-merge = Объединить (M)
cut-merge-hint = Объединить выбранный шортс со следующим
cut-untitled = Без заголовка
cut-status-accepted = Принят — попадёт в экспорт
cut-status-candidate = Кандидат — предложен анализом, ждёт решения
cut-status-rejected = Отклонён
cut-score = Оценка модели: { $score } из 10
cut-flag-unsure = Граница найдена только одним способом — проверьте начало и конец
cut-flag-not-standalone = Может быть непонятно без остального видео
cut-flag-too-long = Длиннее 60 секунд
cut-flag-no-score = Модель не оценила — взят весь смысловой блок
cut-zones = Зоны пропуска
cut-zones-empty = Зон пропуска нет.
cut-zone-intro = интро
cut-zone-outro = аутро
cut-zone-ad = реклама
cut-zone-manual = вручную
cut-zone-add = + Зона из отметок (Z)
cut-zone-remove = Убрать зону
cut-zones-changed = Зоны изменились — нарежьте заново
cut-recut = ✂ Нарезать заново
cut-recut-hint = Нарезать по смыслу заново с учётом зон пропуска. Принятые шортсы останутся.
cut-recut-running = Анализ: { $stage }
cut-recut-queued = В очереди анализа
cut-no-analysis = Кандидаты появятся после анализа ролика. Шортс можно отметить и вручную: начало (I), конец (O), Enter.
cut-text-none = Текст появится после распознавания речи.
cut-text-hint = Клик по слову — начало, второй клик — конец (с отступом в паузу); Enter — шортс, Z — зона пропуска; правый клик по слову выбранного шортса — ключевое слово в субтитрах. Колесо над текстом — прокрутка.
cut-err-bad-split = Поставьте курсор внутрь шортса, не у самого края
cut-err-nothing-to-merge = Справа нет шортса для объединения

export-output = Вывод
export-dir = Папка
export-dir-next-to-source = рядом с исходником
export-dir-pick = Выбрать…
export-dir-reset = Рядом с исходником
export-dir-dialog = Папка для шортсов
export-base-name = Имя файлов
export-name-example = например, { $example }
export-encoder = Кодировщик: { $name }, { $mbps } Мбит/с
export-no-encoder = Нет рабочего кодировщика H.264 — проверьте ffmpeg на вкладке «Настройки»
export-start = { $n ->
    [0] Нет шортсов для рендера
    [one] Рендер: { $n } шортс
    [few] Рендер: { $n } шортса
   *[many] Рендер: { $n } шортсов
}
export-nothing = Отметьте шортсы на вкладке «Нарезка».
export-progress = Шортс { $n } из { $total }: { $name }
export-overall = Всего
export-cancel = Отмена
export-cancelled = Рендер отменён, недописанный файл удалён.
export-status-running = Рендер { $n }/{ $total } · { $pct }%
export-status-done = { $n ->
    [0] Рендер завершён, готовых файлов нет
    [one] Рендер завершён: { $n } файл
    [few] Рендер завершён: { $n } файла
   *[many] Рендер завершён: { $n } файлов
}
export-status-cancelled = { $n ->
    [0] Рендер отменён
    [one] Рендер отменён, готов { $n } файл
    [few] Рендер отменён, готово { $n } файла
   *[many] Рендер отменён, готово { $n } файлов
}

## Анализ (распознавание речи)
analysis-queued = В очереди на анализ
analysis-start = Распознать речь
analysis-cancel = Отменить анализ
analysis-stop = Стоп
analysis-cancelled = Анализ отменён
analysis-redo = Распознать заново (например, после правки словаря терминов)
analysis-done = { $words ->
    [one] Распознано: { $words } слово
    [few] Распознано: { $words } слова
   *[many] Распознано: { $words } слов
}
analysis-stage-audio = Звук
analysis-stage-model = Загрузка модели
analysis-stage-vad = Поиск речи
analysis-stage-speech = Речь
analysis-status-running = Анализ «{ $name }»: { $stage }{ $queued ->
    [0] {""}
   *[other] , в очереди ещё { $queued }
}
analysis-err-no-audio = Нет звука — только ручная нарезка
analysis-err-no-model = Нет модели распознавания
analysis-err-model-load = Модель распознавания не загрузилась
analysis-err-whisper = Ошибка распознавания
analysis-err-ffmpeg = ffmpeg не смог извлечь звук
analysis-err-io = Ошибка чтения или записи файла
analysis-err-no-llm = Нет моделей нарезки
analysis-err-prompt = Ошибка в файлах промптов нарезки
analysis-err-llm = Ошибка LLM при нарезке
analysis-stage-structure = Структура текста
analysis-stage-llm-model = Загрузка LLM
analysis-stage-cut = Нарезка
analysis-cut-start = Нарезать по смыслу
analysis-cut-cancelled = Нарезка отменена
analysis-recut = Нарезать заново (например, после правки промптов)
analysis-cut-done = { $n ->
    [one] { $n } кандидат
    [few] { $n } кандидата
   *[many] { $n } кандидатов
}

## Модели
settings-analysis = Распознавание и нарезка
models-needed = Для автоматической нарезки нужны модели: распознавание речи, структура текста и LLM. Без них доступна только ручная нарезка.
models-download = Скачать модели ({ $size } ГБ)
models-download-cancel = Остановить загрузку
models-downloading = { $file }: { $done } из { $total } ГБ
models-pick = Указать файл модели…
models-pick-dialog = Модель whisper (ggml)
models-pick-filter = Модель whisper
models-reset = Стандартная
models-reset-hint = Вернуть стандартную модель large-v3-turbo
models-whisper = Модель whisper
models-vad = Детектор речи (VAD)
models-embed = Эмбеддинги (структура текста)
models-llm = LLM нарезки
settings-prompts-hint = Свои промпты нарезки: файлы pass1-boundaries.md, pass1.schema.json, pass2-score.md, pass2.schema.json в { $dir } заменяют встроенные.
models-err-network = Не удалось скачать модель — проверьте сеть и повторите (загрузка продолжится с места обрыва)
models-err-checksum = Скачанный файл повреждён — повторите загрузку
settings-terms = Словарь терминов
settings-terms-hint = По одному на строку: имена, названия игр и сервисов — так распознавание пишет их правильно.

design-layers = Слои
design-background = Фон
design-add = + Слой
design-up = Выше (поверх других)
design-down = Ниже
design-duplicate = Дублировать (Ctrl+D)
design-delete = Удалить слой (Delete)
design-undo = Отменить (Ctrl+Z)
design-redo = Повторить (Ctrl+Shift+Z)
design-snap = Привязка
design-snap-hint = К краям и центру холста и к другим слоям; Ctrl при перетаскивании — без привязки
design-grid = Сетка
design-zones = Зоны Shorts
design-zones-hint = Где интерфейс YouTube Shorts закрывает кадр (приблизительно)
design-crop-mode = Кроп
design-crop-hint = Кроп видео: в режиме «Кроп» (C) или с зажатым Alt тяните картинку внутри слоя, колесо — приблизить/отдалить.
design-hint = Клик — выбрать слой, перетаскивание — двигать, ручки — размер (Shift на углу — с пропорциями). Стрелки — на 1 px (Shift — на 10), Esc — снять выделение.
design-preview-none = Шортс не выбран
design-preview-short = Шортс для превью: его заголовок и номер подставляются в {"{"}title{"}"} и {"{"}part{"}"}
design-preview-empty = Добавьте ролик на вкладке «Импорт», чтобы увидеть превью.
design-need-video = В шаблоне должен остаться хотя бы один слой видео
design-invalid = Такое значение шаблон не принимает
design-unsafe-warning = Слой заходит в зону, которую закрывает интерфейс YouTube Shorts
design-new-text = Новый *текст*
design-new-capsule = Часть {"{"}part{"}"}
design-pick-image = Выберите картинку
design-pick-video = Выберите видео для фона
design-filter-image = Картинки
design-filter-video = Видео
design-pick-font = Выберите файл шрифта
design-filter-font = Шрифты
design-font-failed = Не удалось добавить шрифт — это не файл шрифта или его нельзя прочитать

font-loading = Шрифты загружаются…
font-search = Поиск
font-bundled = Встроенные
font-user = Свои
font-system = Системные
font-missing = Шрифта нет в системе — в превью и рендере будет запасной
font-add-file = Свой шрифт…
font-add-file-hint = Файл TTF/OTF копируется в каталог шрифтов приложения
font-weight-thin = Тонкий
font-weight-extralight = Сверхсветлый
font-weight-light = Светлый
font-weight-regular = Обычный
font-weight-medium = Средний
font-weight-semibold = Полужирный
font-weight-bold = Жирный
font-weight-extrabold = Сверхжирный
font-weight-black = Чёрный

layer-main-video = Видео
layer-text = Текст
layer-image = Картинка
layer-capsule = Капсула
layer-shape = Плашка
layer-banner = Место под баннер
layer-subtitles = Субтитры

prop-position = Положение
prop-size = Размер
prop-crop = Кроп
prop-crop-reset = Сбросить
prop-corner-radius = Скругление
prop-border = Рамка
prop-shadow = Тень
prop-shadow-blur = Размытие тени
prop-shadow-offset = Сдвиг тени
prop-content = Текст
prop-content-hint = *Акцент* — звёздочками; {"{"}title{"}"} — заголовок шортса, {"{"}part{"}"} — номер части.
prop-font = Шрифт
prop-font-size = Кегль
prop-color = Цвет
prop-accent = Акцент
prop-align = Выравнивание
prop-outline = Обводка
prop-file = Файл
prop-pick = Выбрать…
prop-no-file = нет
prop-opacity = Непрозрачность
prop-logo = Логотип
prop-logo-remove = Убрать логотип
prop-fill = Заливка
prop-paint = Заливка
prop-gradient = Градиент
prop-angle = Угол
prop-bg-type = Тип
prop-darken = Затемнение
prop-zoom = Увеличение
prop-sub-mode = Режим
prop-sub-active = Текущее слово
prop-sub-keyword = Ключевые слова
prop-sub-unspoken = Несказанные
prop-sub-hide = скрыть
prop-sub-layout = Раскладка
prop-sub-words = {" "}сл. в строке
prop-sub-lines = {" "}стр.
prop-sub-uppercase = Заглавными
prop-sub-plate = Подложка строк
prop-sub-hint = Слова — из речи шортса, выбранного в «Нарезке» (без речи — образец). Ключевые — из нарезки; правый клик по слову в тексте «Нарезки» — отметить или снять.
sub-mode-static = статичный
sub-mode-highlight = подсветка слова
sub-mode-reveal = проявление
subtitles-sample = Так выглядят субтитры вашего шортса

accent-color = цветом
accent-plate = плашкой
align-left = влево
align-center = по центру
align-right = вправо
valign-top = сверху
valign-center = посередине
valign-bottom = снизу
paint-solid = сплошная
paint-linear = градиент
bg-blur = Размытое видео
bg-color = Цвет
bg-gradient = Градиент
bg-image = Картинка
bg-video = Видео

## Шаблоны
template-terminal = Terminal
template-blur-title = Размытие + заголовок
template-split = Два экрана
template-meme = Мем-плашка
template-full-crop = Полный кроп
template-clean-title = Чистый фон + заголовок
tpl-heading = Шаблон
tpl-builtin = Встроенные
tpl-user = Свои
tpl-user-empty = (пока нет — «Сохранить как…»)
tpl-pick-hint = Шаблон пачки. Выбор можно отменить (Ctrl+Z).
tpl-modified-hint = • — шаблон изменён и не сохранён в библиотеке
tpl-save = Сохранить
tpl-save-hint = Записать правки в свой шаблон
tpl-save-as = Сохранить как…
tpl-save-as-hint = Новый свой шаблон из текущего вида
tpl-duplicate = Дублировать
tpl-duplicate-hint = Копия шаблона в «Свои»; пачка переходит на копию, правки остаются
tpl-rename = Переименовать…
tpl-delete = Удалить
tpl-delete-confirm = Удалить шаблон «{ $name }»? Пачка сохранит текущий вид.
tpl-name = Название
tpl-new-name = Мой шаблон
tpl-copy-name = { $name } (копия)
tpl-ok = Сохранить
tpl-cancel = Отмена
tpl-saved = Шаблон «{ $name }» сохранён
tpl-err-empty = Введите название
tpl-err-taken = Шаблон «{ $name }» уже есть
tpl-err-io = Не удалось записать шаблон: { $error }
tpl-local = Только для этого шортса
tpl-local-hint = Правки и выбор шаблона — только для выбранного шортса, остальные остаются на шаблоне пачки
tpl-local-no-short = Выберите шортс, чтобы править его отдельно
tpl-local-has = ✏ У шортса свои правки
tpl-local-reset = Сбросить
tpl-local-reset-hint = Вернуть шортс на шаблон пачки
prop-edge = Граница видео
edge-none = Нет
edge-line = Линия
edge-shadow = Тень
edge-both = Линия и тень
