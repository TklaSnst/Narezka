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
cut-timeline-hint = Колесо — масштаб, Shift+колесо — прокрутка, клик — перемотка. Края выбранного шортса можно тянуть. Esc — снять выбор.
cut-shorts-empty = Шортсов пока нет: отметьте начало (I) и конец (O) и нажмите Enter.
cut-err-no-in = Сначала отметьте начало (I)
cut-err-bad-range = Конец шортса должен быть позже начала
cut-err-out-of-source = Шортс выходит за конец ролика
cut-err-overlap = Шортс пересекается с другим шортсом
cut-err-unknown = Шортс или ролик не найден
