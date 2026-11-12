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
