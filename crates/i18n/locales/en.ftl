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
cut-timeline-hint = Wheel — zoom, Shift+wheel — scroll, click — seek. Drag the edges of the selected short or its yellow insert marker. N — next candidate, V — insert at cursor, Esc — deselect.
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
cut-text-hint = Click a word to set the start, click again to set the end (padded into the pause); Enter — short, Z — skip zone; right-click a word of the selected short — subtitle keyword. Wheel over the text scrolls it.
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
export-encoder = Encoder: { $name }, { $quality }
export-no-encoder = No working { $codec } encoder (or audio encoder) — pick another codec or check ffmpeg in Settings
export-start = { $n ->
    [0] No shorts to render
    [one] Render { $n } short
   *[other] Render { $n } shorts
}
export-nothing = Mark shorts on the Cut tab.
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
design-pick-font = Choose a font file
design-filter-font = Fonts
design-font-failed = Could not add the font: not a font file or it cannot be read

font-loading = Loading fonts…
font-search = Search
font-bundled = Bundled
font-user = Added
font-system = System
font-missing = Font not installed: preview and render will use a fallback
font-add-file = Add font…
font-add-file-hint = The TTF/OTF file is copied to the app's font folder
font-weight-thin = Thin
font-weight-extralight = Extra light
font-weight-light = Light
font-weight-regular = Regular
font-weight-medium = Medium
font-weight-semibold = Semibold
font-weight-bold = Bold
font-weight-extrabold = Extra bold
font-weight-black = Black

layer-main-video = Video
layer-text = Text
layer-image = Image
layer-capsule = Capsule
layer-shape = Plate
layer-banner = Banner slot
layer-subtitles = Subtitles

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
prop-sub-mode = Mode
prop-sub-active = Current word
prop-sub-keyword = Keywords
prop-sub-unspoken = Unspoken
prop-sub-hide = hide
prop-sub-layout = Layout
prop-sub-words = {" "}words/line
prop-sub-lines = {" "}lines
prop-sub-uppercase = Uppercase
prop-sub-plate = Line plate
prop-sub-hint = Words come from the speech of the short selected in “Cut” (no speech — a sample). Keywords come from cutting; right-click a word in the “Cut” text to mark or unmark it.
sub-mode-static = static
sub-mode-highlight = word highlight
sub-mode-reveal = reveal
subtitles-sample = Your short subtitles look like this
sublist-title = Short subtitles
sublist-no-layer = The template has no subtitles layer — phrases are split as 3 words × 2 lines.
sublist-empty = No speech in this short.
sublist-seek = Go to phrase
sublist-reset = Restore spoken text
sublist-hint = Fix a phrase and press Enter (Esc cancels). You can add or remove words — the timing is shared among them.

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

## Templates
template-terminal = Terminal
template-blur-title = Blur + title
template-split = Two screens
template-meme = Meme caption
template-full-crop = Full crop
template-clean-title = Clean background + title
tpl-heading = Template
tpl-builtin = Built-in
tpl-user = Yours
tpl-user-empty = (none yet — “Save as…”)
tpl-pick-hint = Batch template. Picking can be undone (Ctrl+Z).
tpl-modified-hint = • — template changed and not saved to the library
tpl-save = Save
tpl-save-hint = Write changes to your template
tpl-save-as = Save as…
tpl-save-as-hint = New template of your own from the current look
tpl-duplicate = Duplicate
tpl-duplicate-hint = Copy the template to “Yours”; the batch switches to the copy, changes are kept
tpl-rename = Rename…
tpl-delete = Delete
tpl-delete-confirm = Delete template “{ $name }”? The batch keeps its current look.
tpl-name = Name
tpl-new-name = My template
tpl-copy-name = { $name } (copy)
tpl-ok = Save
tpl-cancel = Cancel
tpl-saved = Template “{ $name }” saved
tpl-err-empty = Enter a name
tpl-err-taken = Template “{ $name }” already exists
tpl-err-io = Could not write the template: { $error }
tpl-local = This short only
tpl-local-hint = Changes and template choice apply to the selected short only; the rest stay on the batch template
tpl-local-no-short = Select a short to edit it separately
tpl-local-has = ✏ The short has its own changes
tpl-local-reset = Reset
tpl-local-reset-hint = Put the short back on the batch template
prop-edge = Video edge
edge-none = None
edge-line = Line
edge-shadow = Shadow
edge-both = Line and shadow

## Library (step 5.1)
library-add = Add files…
library-add-title = Add to library
library-filter-files = Images, videos and fonts
library-drop-hint = or drop files into the window
library-drop-here = Drop to add to the library
library-importing = Importing: { $n }…
library-search-hint = Search: name or #tag
library-all = All
library-empty = The library is empty. Add banners, ad inserts, images and fonts — files are copied into the library.
library-nothing-found = Nothing found.
library-added = Added: { $name }
library-duplicate = “{ $name }” is already in the library.
library-deleted = Deleted: { $name }
library-err-unreadable = Could not add “{ $name }”: not an image, video or font.
library-err-io = Could not add “{ $name }”: { $error }
library-err-save = Could not save the library: { $error }
library-select-hint = Select an asset to see its details.
library-name = Name
library-kind = Type
library-tags = Tags
library-tags-hint = comma-separated
library-hide-banner = Hide the banner during this insert
library-info-font = Font
library-info-codec = Format
library-info-size = Size
library-info-duration = Duration
library-info-alpha = Transparency
library-info-audio = Audio
library-info-file = File
library-info-added = Added
library-info-used = Used
library-used = { $n ->
    [one] once
   *[other] { $n } times
}, last on { $date }
library-unused = not used yet
library-still = still image
library-yes = yes
library-no = no
library-missing = ⚠ The asset file is missing from the library folder.
library-delete = Delete
library-delete-confirm = Delete “{ $name }” from the library? The file will be removed.
library-cancel = Cancel
library-open-dir = Open folder
asset-kind-banner = Banner
asset-kind-insert = Insert
asset-kind-image = Image
asset-kind-background = Background
asset-kind-font = Font
orientation-vertical = vertical
orientation-horizontal = horizontal
orientation-square = square
size-kb = { $n } KB
size-mb = { $n } MB
size-gb = { $n } GB

## Batch banner
banner-title = Batch banner
banner-place-hint = Position and size: the “Banner slot” frame on the canvas; the banner is fitted into it.
banner-none = No banner
banner-missing = ⚠ The batch banner is not in the library (deleted or its file is gone) — shorts will be rendered without it.
banner-library-empty = No banners or images in the library.
banner-open-library = Open the library
banner-text-on = Text next to it
banner-text-hint = A promo code or link — moves together with the banner.
banner-text-placeholder = Promo code *SHORTS* — 20% off
banner-side = Where
banner-side-below = below
banner-side-above = above
banner-side-left = left
banner-side-right = right
banner-skip-short = No banner on this short
library-use-banner = Use as the batch banner
library-use-banner-hint = The banner goes into the template's “Banner slot” on every short.
library-is-banner = ✔ Batch banner
library-banner-set = “{ $name }” is the batch banner now.

## Same as last time
preset-title = Same as last time?
preset-when = Last batch: { $when }.
preset-template = Template
preset-banner = Banner
preset-banner-gone = (deleted from the library)
preset-output = Folder
preset-dir-default = next to the source
preset-yes = Yes, same as last time
preset-no = No, start fresh

## Insert (step 5.3)
insert-title = Ad insert
insert-none = No insert
insert-clip = Clip
insert-missing = ⚠ The batch insert is not in the library (deleted or its file is gone) — shorts will be rendered without it.
insert-library-empty = No videos in the library — add an insert…
insert-mode = Show
insert-mode-auto = Auto: { $mode }
insert-mode-hint = Auto — by the clip orientation ({ $orientation }): vertical goes full screen, the rest — into the video slot.
insert-mode-fullscreen = full screen
insert-mode-slot = in the video slot
insert-hide-banner-hint = Remembered for the clip: the next batch with it does the same.
insert-unplaced = Not placed: { $n }
insert-place-all = Place
insert-place-all-hint = Shorts without an insert point get one in a pause near the middle.
insert-select-short = Select a short to set its insert point (drag the marker on the timeline; Shift — no snapping to pauses).
insert-skip-short = No insert on this short
insert-at = Insert at { $at } from the start
insert-not-placed = Insert point not set
insert-here = At cursor
insert-here-hint = Insert point — in the pause next to the cursor (V).
insert-play = ▶ Insert
insert-play-hint = Play from the insert point: the video stops, the insert plays, then the video goes on.
insert-remove = Remove the insert point
insert-total = Total { $total } (short { $short } + insert { $insert })
insert-too-long = longer than 3 minutes — YouTube won't take it as a Short
insert-playing = Insert playing: { $t } / { $d }
insert-unplaced-export = ⚠ { $n ->
    [one] { $n } short has no insert point — it will be rendered without the insert.
   *[other] { $n } shorts have no insert point — they will be rendered without the insert.
}
export-too-long = ⚠ { $n ->
    [one] { $n } short is longer than 3 minutes (with the insert) — YouTube won't take it as a Short.
   *[other] { $n } shorts are longer than 3 minutes (with the insert) — YouTube won't take them as Shorts.
}
export-with-insert = { $total } with the insert
library-use-insert = Use as the batch insert
library-use-insert-hint = The clip goes into every short of the batch; its point is set on the Cut timeline.
library-is-insert = ✔ Batch insert
library-insert-set = “{ $name }” is the batch insert now.
preset-insert = Insert

## Cover frame (step 6.1)
cover-heading = Short cover
cover-no-short = Select a short to give it a cover
cover-enable = Cover at the start of the short
cover-enable-hint = The chosen frame with captions goes first — YouTube uses it as the short's thumbnail
cover-all = Covers for all accepted
cover-all-hint = Give every accepted short without a cover one in the current style
cover-edit = ✏ Edit cover
cover-edit-hint = Preview and editor show the cover frame: edit captions and layers with the mouse, like a template
cover-src-frame = Video frame
cover-src-image = Picture
cover-no-images = No pictures in the library — add some on the Library tab
cover-frame-hint = Anywhere in the source, not only inside the short
cover-prev-frame = Previous frame
cover-next-frame = Next frame
cover-short-start = short start
cover-take-current = Use the frame the preview is on
cover-outside = The frame is outside the short
cover-image-missing = (picture not in the library)
cover-frames = Duration:
cover-frames-unit = {" "}fr.
cover-frames-ms = ≈ { $ms } ms
cover-sound-now = Sound right away
cover-sound-now-hint = The cover hides the first frames of the short and sound starts at once. Unchecked, the cover is added before the short with silence
cover-style = Style:
cover-style-hint = Built-in styles are captions over the short's template; your own are whole saved covers
cover-style-user-empty = No styles of your own yet
cover-style-plain = No captions
cover-style-big-title = Big title
cover-style-top-title = Title on top
cover-style-save = Save style as…
cover-style-save-hint = This cover (without the frame) becomes a style for other shorts
cover-style-new-name = My cover
cover-style-delete = Delete style
cover-style-saved = Cover style “{ $name }” saved

export-format = Format and quality
export-container = Container
export-codec = Codec
export-codec-missing = No working encoder in ffmpeg (checked by a trial encode at startup, see Settings)
export-prores = Variant
export-prores-hint = ProRes quality is set by the variant: Proxy and LT are lighter, HQ and 4444 are for lossless editing.
export-encoder-kind = Encoder
encoder-auto = Auto
export-encoder-kind-missing = No such encoder for this codec (or it failed the trial encode)
export-quality = Quality
quality-high = High
quality-balanced = Balanced
quality-fast = Fast
quality-custom = Custom
export-resolution = Resolution
export-resolution-template = { $size } (template)
export-resolution-custom = Custom
export-fps = Frame rate
export-fps-source = same as source
export-audio = Audio
export-audio-missing = This audio encoder is missing from the ffmpeg build
export-advanced = Advanced settings
export-rate = Mode
export-rate-bitrate = Bitrate
export-rate-crf = Quality (CRF)
export-rate-qp = Quality (QP)
export-bitrate = Bitrate
export-crf-hint = Lower is better and larger
export-speed = Speed
export-speed-gpu = GPU encoder speed is not adjustable
speed-slow = slow
speed-medium = medium
speed-fast = fast
speed-faster = very fast
export-profile = Profile
profile-auto = auto
profile-baseline = Baseline
profile-main = Main
profile-high = High
export-gop = Keyframe every
export-gop-auto = auto
export-presets = My presets
export-preset-none = —
export-preset-save = Save as…
export-preset-delete = Delete
export-preset-name = preset name
export-preset-ok = Save
export-preset-cancel = Cancel
export-mbps = { $mbps } Mbit/s

export-parallel = At once
export-parallel-hint = How many shorts to render in parallel. The CPU filter graph is the bottleneck: more than 2–3 barely helps.
export-clear = Clear list
export-overall = Total: { $n } of { $total }
export-overall-paused = Paused: { $n } of { $total }
export-pause = ⏸ Pause
export-resume = ▶ Resume
export-cancel-all = Cancel all
export-cancel-file = Cancel this file (the partial file is deleted)
export-waiting = waiting
export-done-in = done in { $time }
export-file-cancelled = cancelled
export-log = Log
export-log-title = ffmpeg log: { $name }
export-log-copy = Copy
export-copy-path = Copy path
export-status-failed = , { $n } failed
export-status-paused = Render paused { $n }/{ $total } · { $pct }%
export-conflict = If the name is taken
export-conflict-policy-continue = Continue numbering
export-conflict-policy-ask = Ask
export-conflict-policy-overwrite = Overwrite
export-conflict-title = Files already exist
export-conflict-text = { $n ->
    [one] Numbering from 1 would overwrite { $n } file:
   *[other] Numbering from 1 would overwrite { $n } files:
}
export-conflict-continue = Continue numbering
export-conflict-overwrite = Overwrite
export-conflict-cancel = Cancel
