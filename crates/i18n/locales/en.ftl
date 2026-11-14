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
cut-timeline-hint = Wheel — zoom, Shift+wheel — scroll, click — seek. Drag the edges of the selected short to adjust it. N — next candidate, Esc — deselect.
cut-shorts-empty = No shorts left. Mark one by hand: in (I), out (O), Enter.
cut-err-no-in = Mark the start first (I)
cut-err-bad-range = The short must end after it starts
cut-err-out-of-source = The short goes past the end of the video
cut-err-overlap = The short overlaps another short
cut-err-unknown = Short or video not found
cut-list = Shorts
cut-list-counts = Accepted: { $accepted } · candidates: { $candidates } · rejected: { $rejected }
cut-allow-overlap = Allow overlaps
cut-allow-overlap-hint = Accepted shorts may overlap in time
cut-accept = ✔ Accept (A)
cut-reject = ✖ Reject (R)
cut-split = Split (S)
cut-split-hint = Split the selected short at the playhead (at the nearest word boundary)
cut-merge = Merge (M)
cut-merge-hint = Merge the selected short with the next one
cut-untitled = Untitled
cut-status-accepted = Accepted — will be exported
cut-status-candidate = Candidate — suggested by analysis, awaiting your decision
cut-status-rejected = Rejected
cut-score = Model score: { $score } of 10
cut-flag-unsure = Boundary found by one method only — check the start and end
cut-flag-not-standalone = May be unclear without the rest of the video
cut-flag-too-long = Longer than 60 seconds
cut-flag-no-score = Not scored by the model — the whole topic block is used
cut-zones = Skip zones
cut-zones-empty = No skip zones.
cut-zone-intro = intro
cut-zone-outro = outro
cut-zone-ad = ad
cut-zone-manual = manual
cut-zone-add = + Zone from marks (Z)
cut-zone-remove = Remove zone
cut-zones-changed = Zones changed — cut again
cut-recut = ✂ Cut again
cut-recut-hint = Re-run the meaning-based cutting with the skip zones. Accepted shorts are kept.
cut-recut-running = Analysis: { $stage }
cut-recut-queued = Queued for analysis
cut-no-analysis = Candidates will appear after the video is analyzed. You can also mark a short by hand: in (I), out (O), Enter.
cut-text-none = The text will appear after speech recognition.
cut-text-hint = Click a word to set the start, click again to set the end (padded into the pause); Enter — short, Z — skip zone. Wheel over the text scrolls it.
cut-err-bad-split = Put the playhead inside the short, not right at its edge
cut-err-nothing-to-merge = There is no short to the right to merge with

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

## Analysis (speech recognition)
analysis-queued = Queued for analysis
analysis-start = Recognize speech
analysis-cancel = Cancel analysis
analysis-stop = Stop
analysis-cancelled = Analysis cancelled
analysis-redo = Recognize again (e.g. after editing the term list)
analysis-done = { $words ->
    [one] Recognized: { $words } word
   *[other] Recognized: { $words } words
}
analysis-stage-audio = Audio
analysis-stage-model = Loading model
analysis-stage-vad = Detecting speech
analysis-stage-speech = Speech
analysis-status-running = Analyzing “{ $name }”: { $stage }{ $queued ->
    [0] {""}
   *[other] , { $queued } more queued
}
analysis-err-no-audio = No audio — manual cutting only
analysis-err-no-model = Speech recognition model missing
analysis-err-model-load = Failed to load the speech recognition model
analysis-err-whisper = Speech recognition failed
analysis-err-ffmpeg = ffmpeg could not extract audio
analysis-err-io = File read or write error
analysis-err-no-llm = Cutting models missing
analysis-err-prompt = Error in the cutting prompt files
analysis-err-llm = LLM error while cutting
analysis-stage-structure = Text structure
analysis-stage-llm-model = Loading LLM
analysis-stage-cut = Cutting
analysis-cut-start = Cut by meaning
analysis-cut-cancelled = Cutting cancelled
analysis-recut = Cut again (e.g. after editing the prompts)
analysis-cut-done = { $n ->
    [one] { $n } candidate
   *[other] { $n } candidates
}

## Models
settings-analysis = Recognition and cutting
models-needed = Automatic cutting needs models: speech recognition, text structure and an LLM. Without them only manual cutting is available.
models-download = Download models ({ $size } GB)
models-download-cancel = Stop download
models-downloading = { $file }: { $done } of { $total } GB
models-pick = Choose model file…
models-pick-dialog = Whisper model (ggml)
models-pick-filter = Whisper model
models-reset = Default
models-reset-hint = Back to the default large-v3-turbo model
models-whisper = Whisper model
models-vad = Voice activity detector (VAD)
models-embed = Embeddings (text structure)
models-llm = Cutting LLM
settings-prompts-hint = Custom cutting prompts: pass1-boundaries.md, pass1.schema.json, pass2-score.md, pass2.schema.json in { $dir } replace the built-in ones.
models-err-network = Model download failed — check the network and retry (it resumes where it stopped)
models-err-checksum = Downloaded file is corrupted — retry the download
settings-terms = Term list
settings-terms-hint = One per line: names, games, services — helps recognition spell them right.

design-layers = Layers
design-background = Background
design-add = + Layer
design-up = Up (above others)
design-down = Down
design-duplicate = Duplicate (Ctrl+D)
design-delete = Delete layer (Delete)
design-undo = Undo (Ctrl+Z)
design-redo = Redo (Ctrl+Shift+Z)
design-snap = Snap
design-snap-hint = To canvas edges and center and to other layers; hold Ctrl while dragging to disable
design-grid = Grid
design-zones = Shorts zones
design-zones-hint = Where the YouTube Shorts interface covers the frame (approximate)
design-crop-mode = Crop
design-crop-hint = Video crop: in Crop mode (C) or holding Alt, drag the picture inside the layer; wheel zooms in/out.
design-hint = Click selects a layer, drag moves it, handles resize (Shift on a corner keeps proportions). Arrows move by 1 px (Shift — 10), Esc deselects.
design-preview-none = No short selected
design-preview-short = Short for the preview: its title and number fill {"{"}title{"}"} and {"{"}part{"}"}
design-preview-empty = Add a video on the Import tab to see the preview.
design-need-video = The template must keep at least one video layer
design-invalid = The template does not accept this value
design-unsafe-warning = The layer overlaps the area covered by the YouTube Shorts interface
design-new-text = New *text*
design-new-capsule = Part {"{"}part{"}"}
design-pick-image = Choose an image
design-pick-video = Choose a background video
design-filter-image = Images
design-filter-video = Video

layer-main-video = Video
layer-text = Text
layer-image = Image
layer-capsule = Capsule
layer-shape = Plate
layer-banner = Banner slot

prop-position = Position
prop-size = Size
prop-crop = Crop
prop-crop-reset = Reset
prop-corner-radius = Corner radius
prop-border = Border
prop-shadow = Shadow
prop-shadow-blur = Shadow blur
prop-shadow-offset = Shadow offset
prop-content = Text
prop-content-hint = *Accent* with asterisks; {"{"}title{"}"} is the short's title, {"{"}part{"}"} its part number.
prop-font = Font
prop-font-hint = System family, a style may be appended: "DejaVu Sans Bold"
prop-font-size = Size
prop-color = Color
prop-accent = Accent
prop-align = Alignment
prop-outline = Outline
prop-file = File
prop-pick = Choose…
prop-no-file = none
prop-opacity = Opacity
prop-logo = Logo
prop-logo-remove = Remove logo
prop-fill = Fill
prop-paint = Fill
prop-gradient = Gradient
prop-angle = Angle
prop-bg-type = Type
prop-darken = Darken
prop-zoom = Zoom

accent-color = by color
accent-plate = with a plate
align-left = left
align-center = center
align-right = right
valign-top = top
valign-center = middle
valign-bottom = bottom
paint-solid = solid
paint-linear = gradient
bg-blur = Blurred video
bg-color = Color
bg-gradient = Gradient
bg-image = Image
bg-video = Video
