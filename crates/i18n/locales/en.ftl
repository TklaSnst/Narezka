app-title = ShortsCutter

tab-import = Import
tab-cut = Cut
tab-design = Design
tab-export = Export
tab-library = Library
tab-settings = Settings

settings-language = Interface language
settings-saved = Settings saved
settings-save-failed = Failed to save settings: { $error }

lang-ru = Русский
lang-en = English

placeholder-not-ready = This section will appear in later steps.
batch-summary = { $sources ->
    [0] The batch is empty
    [one] { $sources } video in the batch
   *[other] { $sources } videos in the batch
}

import-add = Add videos…
import-drop-hint = or drop files onto the window
import-drop-here = Release to add to the batch
import-empty = The batch is empty — add one or more videos.
import-dialog-title = Choose videos
import-dialog-filter = Video
import-duplicate = This file is already in the batch: { $name }
import-remove = Remove from batch
import-col-file = File
import-col-duration = Duration
import-col-size = Frame
import-col-fps = FPS
import-col-audio = Audio
import-col-status = Status
import-audio-yes = yes
import-audio-no = no

source-status-new = Reading…
source-status-ready = Ready
source-status-analyzing = Analyzing…
source-status-analyzed = Analyzed

media-err-no-ffprobe = ffprobe not found
media-err-timeout = ffprobe timed out
media-err-no-video = The file has no video
media-err-io = Could not read the file
media-err-unreadable = Not recognized as a video

settings-ffmpeg = ffmpeg
caps-checking = Checking ffmpeg…
caps-summary = ffmpeg { $version } · GPU: { $gpu }
caps-none = none
caps-no-ffmpeg = ffmpeg or ffprobe not found — import and rendering are unavailable
caps-missing-filters = ffmpeg build lacks filters: { $filters }
caps-vaapi-device = VAAPI device: { $device }
caps-kind-gpu = GPU
caps-kind-cpu = CPU

cut-sources = Videos
cut-no-sources = Add videos on the Import tab first.
cut-source-not-ready = The video is still being read or failed to open
cut-shorts-count = { $n ->
    [one] { $n } short
   *[other] { $n } shorts
}
cut-player-failed = Preview unavailable (libmpv): { $error }
cut-open-failed = Could not open the video: { $error }
cut-preview-empty = Pick a video on the left
cut-preview-loading = Loading…
cut-play = Play / pause (Space)
cut-frame-back = Previous frame (←), Shift+← — 5 s back
cut-frame-fwd = Next frame (→), Shift+→ — 5 s forward
cut-mark-in = [ In (I)
cut-mark-out = Out (O) ]
cut-add-short = + Short (Enter)
cut-delete-short = Delete (Del)
cut-zoom-fit = Whole video
cut-timeline-hint = Wheel — zoom, Shift+wheel — scroll, click — seek. Drag the edges of the selected short to adjust it. Esc — deselect.
cut-shorts-empty = No shorts yet: mark the start (I) and end (O), then press Enter.
cut-err-no-in = Mark the start first (I)
cut-err-bad-range = The short must end after it starts
cut-err-out-of-source = The short goes past the end of the video
cut-err-overlap = The short overlaps another short
cut-err-unknown = Short or video not found

export-output = Output
export-dir = Folder
export-dir-next-to-source = next to the source
export-dir-pick = Choose…
export-dir-reset = Next to the source
export-dir-dialog = Folder for shorts
export-base-name = File name
export-name-example = e.g. { $example }
export-encoder = Encoder: { $name }, { $mbps } Mbit/s
export-no-encoder = No working H.264 encoder — check ffmpeg on the Settings tab
export-start = { $n ->
    [0] No shorts to render
    [one] Render { $n } short
   *[other] Render { $n } shorts
}
export-nothing = Mark shorts on the Cut tab.
export-progress = Short { $n } of { $total }: { $name }
export-overall = Total
export-cancel = Cancel
export-cancelled = Render cancelled, the partial file was removed.
export-status-running = Rendering { $n }/{ $total } · { $pct }%
export-status-done = { $n ->
    [0] Render finished, no files produced
    [one] Render finished: { $n } file
   *[other] Render finished: { $n } files
}
export-status-cancelled = { $n ->
    [0] Render cancelled
    [one] Render cancelled, { $n } file finished
   *[other] Render cancelled, { $n } files finished
}
