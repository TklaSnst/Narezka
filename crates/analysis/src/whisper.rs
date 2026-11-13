//! Безопасная обёртка над whisper.cpp: загрузка модели, распознавание с VAD, слова с таймкодами.
//!
//! До первого вызова whisper процесс должен выставить `GGML_VK_DISABLE_HOST_VISIBLE_VIDMEM=1`
//! (см. [`crate::prepare_ggml_env`]). Модель держит VRAM, пока жив [`Whisper`].

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sc_whisper_sys as sys;

use crate::{AnalysisError, Stage};
use crate::transcript::{FORMAT_VERSION, Segment, Span, Transcript, Word};

pub struct WhisperOptions {
    /// Код языка (`ru`) или `auto`.
    pub language: String,
    /// Словарь терминов — initial prompt.
    pub prompt: String,
    pub threads: i32,
}

pub struct Whisper {
    ctx: *mut sys::whisper_context,
    model_name: String,
    dtw: bool,
}

// Контекст whisper не привязан к потоку; одновременно им пользуется только владелец (&mut self).
unsafe impl Send for Whisper {}

impl Whisper {
    /// Загружает модель на GPU. DTW-таймкоды включаются, если по имени файла известен пресет
    /// голов внимания; с DTW flash attention выключен (иначе whisper молча отключает DTW).
    pub fn load(model: &Path) -> Result<Self, AnalysisError> {
        route_logs();
        let model_name = model.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let path = CString::new(model.as_os_str().as_encoded_bytes()).map_err(|_| AnalysisError::BadPath)?;
        let preset = dtw_preset(&model_name);
        let ctx = unsafe {
            let mut cp = sys::whisper_context_default_params();
            cp.use_gpu = true;
            if let Some(preset) = preset {
                cp.flash_attn = false;
                cp.dtw_token_timestamps = true;
                cp.dtw_aheads_preset = preset;
            }
            sys::whisper_init_from_file_with_params(path.as_ptr(), cp)
        };
        if ctx.is_null() {
            return Err(AnalysisError::ModelLoad(model.to_owned()));
        }
        Ok(Self { ctx, model_name, dtw: preset.is_some() })
    }

    /// Распознаёт весь звук (моно 16 кГц) со встроенным Silero VAD.
    /// Прогресс: сначала [`Stage::Vad`], с первого окна whisper — [`Stage::Speech`] 0…1.
    pub fn transcribe(
        &mut self,
        pcm: &[f32],
        vad_model: &Path,
        opts: &WhisperOptions,
        cancel: &AtomicBool,
        mut progress: impl FnMut(Stage, f32),
    ) -> Result<Transcript, AnalysisError> {
        let vad = CString::new(vad_model.as_os_str().as_encoded_bytes()).map_err(|_| AnalysisError::BadPath)?;
        let lang = CString::new(opts.language.as_str()).map_err(|_| AnalysisError::BadPath)?;
        let prompt = CString::new(opts.prompt.replace('\0', "")).expect("нулевые байты убраны");
        progress(Stage::Vad, 0.0);
        let mut cb = Callbacks { cancel, progress: &mut progress, speech: false };
        let user = &mut cb as *mut Callbacks as *mut c_void;

        let rc = unsafe {
            let mut p = sys::whisper_full_default_params(sys::whisper_sampling_strategy_WHISPER_SAMPLING_GREEDY);
            p.n_threads = opts.threads.max(1);
            p.language = lang.as_ptr();
            p.token_timestamps = true;
            p.vad = true;
            p.vad_model_path = vad.as_ptr();
            if !opts.prompt.trim().is_empty() {
                // Словарь — начальный контекст; дальше whisper тянет его скользящим контекстом сам.
                // carry_initial_prompt (словарь в каждом окне) хуже: на шаге 2.1 терял фразы,
                // точки в концах сегментов и растягивал сегменты до 15–20 с.
                p.initial_prompt = prompt.as_ptr();
            }
            p.print_progress = false;
            p.print_realtime = false;
            p.print_timestamps = false;
            p.progress_callback = Some(on_progress);
            p.progress_callback_user_data = user;
            p.encoder_begin_callback = Some(on_encoder_begin);
            p.encoder_begin_callback_user_data = user;
            p.abort_callback = Some(on_abort);
            p.abort_callback_user_data = user;
            sys::whisper_full(self.ctx, p, pcm.as_ptr(), pcm.len() as c_int)
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(AnalysisError::Cancelled);
        }
        if rc != 0 {
            return Err(AnalysisError::Whisper(rc));
        }
        progress(Stage::Speech, 1.0);
        Ok(self.collect(opts))
    }

    fn collect(&self, opts: &WhisperOptions) -> Transcript {
        let ctx = self.ctx;
        let mut words = Vec::new();
        let mut segments = Vec::new();
        unsafe {
            let eot = sys::whisper_token_eot(ctx);
            for s in 0..sys::whisper_full_n_segments(ctx) {
                let first = words.len();
                let mut acc = WordAcc::default();
                for i in 0..sys::whisper_full_n_tokens(ctx, s) {
                    let d = sys::whisper_full_get_token_data(ctx, s, i);
                    if d.id >= eot {
                        continue; // служебные токены и таймкоды
                    }
                    let text = CStr::from_ptr(sys::whisper_full_get_token_text(ctx, s, i)).to_bytes();
                    let t0 = sys::whisper_full_get_token_t0(ctx, s, i);
                    let t1 = sys::whisper_full_get_token_t1(ctx, s, i);
                    // t_dtw остаётся на «сжатой» шкале VAD — сдвигаем на ту же поправку, что у t0.
                    let dtw = (self.dtw && d.t_dtw >= 0).then(|| d.t_dtw + (t0 - d.t0));
                    if text.first() == Some(&b' ') {
                        acc.flush(&mut words);
                    }
                    acc.push(text, t0, t1, dtw, d.p);
                }
                acc.flush(&mut words);
                if words.len() > first {
                    let text = words[first..].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                    segments.push(Segment {
                        start: cs(sys::whisper_full_get_segment_t0(ctx, s)),
                        end: cs(sys::whisper_full_get_segment_t1(ctx, s)),
                        text,
                        words: first..words.len(),
                    });
                }
            }
            let speech = (0..sys::whisper_full_n_vad_segments(ctx))
                .map(|i| Span {
                    start: cs(sys::whisper_full_get_vad_segment_t0(ctx, i)),
                    end: cs(sys::whisper_full_get_vad_segment_t1(ctx, i)),
                })
                .collect();
            Transcript {
                version: FORMAT_VERSION,
                model: self.model_name.clone(),
                language: opts.language.clone(),
                prompt: opts.prompt.clone(),
                words,
                segments,
                speech,
            }
        }
    }
}

impl Drop for Whisper {
    fn drop(&mut self) {
        unsafe { sys::whisper_free(self.ctx) };
    }
}

/// Сантисекунды whisper → секунды.
fn cs(t: i64) -> f64 {
    t as f64 / 100.0
}

/// Собирает слово из токенов. Текст копится байтами: токен может резать многобайтовую букву UTF-8.
#[derive(Default)]
struct WordAcc {
    bytes: Vec<u8>,
    t0: i64,
    t1: i64,
    dtw: Option<i64>,
    p_sum: f32,
    n: u32,
}

impl WordAcc {
    fn push(&mut self, text: &[u8], t0: i64, t1: i64, dtw: Option<i64>, p: f32) {
        if self.n == 0 {
            self.t0 = t0;
            self.dtw = dtw;
        }
        self.bytes.extend_from_slice(text);
        self.t1 = t1;
        self.p_sum += p;
        self.n += 1;
    }

    fn flush(&mut self, words: &mut Vec<Word>) {
        let acc = std::mem::take(self);
        let text = String::from_utf8_lossy(&acc.bytes).trim().to_owned();
        if text.is_empty() {
            return;
        }
        words.push(Word {
            text,
            start: cs(acc.t0),
            end: cs(acc.t1.max(acc.t0)),
            dtw: acc.dtw.map(cs),
            p: acc.p_sum / acc.n as f32,
        });
    }
}

struct Callbacks<'a> {
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(Stage, f32),
    /// VAD закончился — whisper начал первое окно.
    speech: bool,
}

unsafe extern "C" fn on_progress(
    _: *mut sys::whisper_context,
    _: *mut sys::whisper_state,
    pct: c_int,
    user: *mut c_void,
) {
    let cb = unsafe { &mut *(user as *mut Callbacks) };
    (cb.progress)(Stage::Speech, pct.clamp(0, 100) as f32 / 100.0);
}

unsafe extern "C" fn on_encoder_begin(_: *mut sys::whisper_context, _: *mut sys::whisper_state, user: *mut c_void) -> bool {
    let cb = unsafe { &mut *(user as *mut Callbacks) };
    if !cb.speech {
        cb.speech = true;
        (cb.progress)(Stage::Speech, 0.0);
    }
    !cb.cancel.load(Ordering::Relaxed)
}

unsafe extern "C" fn on_abort(user: *mut c_void) -> bool {
    let cb = unsafe { &*(user as *const Callbacks) };
    cb.cancel.load(Ordering::Relaxed)
}

/// Пресет голов внимания для DTW по имени файла модели (`ggml-large-v3-turbo-q5_0.bin` → turbo).
fn dtw_preset(file: &str) -> Option<sys::whisper_alignment_heads_preset> {
    use sys::*;
    let name = file.to_ascii_lowercase();
    let en = name.contains(".en");
    let presets = [
        ("large-v3-turbo", whisper_alignment_heads_preset_WHISPER_AHEADS_LARGE_V3_TURBO, None),
        ("large-v3", whisper_alignment_heads_preset_WHISPER_AHEADS_LARGE_V3, None),
        ("large-v2", whisper_alignment_heads_preset_WHISPER_AHEADS_LARGE_V2, None),
        ("large-v1", whisper_alignment_heads_preset_WHISPER_AHEADS_LARGE_V1, None),
        ("medium", whisper_alignment_heads_preset_WHISPER_AHEADS_MEDIUM, Some(whisper_alignment_heads_preset_WHISPER_AHEADS_MEDIUM_EN)),
        ("small", whisper_alignment_heads_preset_WHISPER_AHEADS_SMALL, Some(whisper_alignment_heads_preset_WHISPER_AHEADS_SMALL_EN)),
        ("base", whisper_alignment_heads_preset_WHISPER_AHEADS_BASE, Some(whisper_alignment_heads_preset_WHISPER_AHEADS_BASE_EN)),
        ("tiny", whisper_alignment_heads_preset_WHISPER_AHEADS_TINY, Some(whisper_alignment_heads_preset_WHISPER_AHEADS_TINY_EN)),
    ];
    presets.into_iter().find(|(key, ..)| name.contains(key)).map(|(_, multi, en_preset)| match en_preset {
        Some(p) if en => p,
        _ => multi,
    })
}

/// Сообщения whisper/ggml — в `log` (иначе они сыплются в stderr).
fn route_logs() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe { sys::whisper_log_set(Some(on_log), std::ptr::null_mut()) });
}

unsafe extern "C" fn on_log(level: sys::ggml_log_level, text: *const c_char, _: *mut c_void) {
    if text.is_null() {
        return;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    let text = text.trim_end();
    if text.is_empty() {
        return;
    }
    match level {
        sys::ggml_log_level_GGML_LOG_LEVEL_ERROR => log::error!(target: "whisper", "{text}"),
        sys::ggml_log_level_GGML_LOG_LEVEL_WARN => log::warn!(target: "whisper", "{text}"),
        _ => log::debug!(target: "whisper", "{text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dtw_preset_by_file_name() {
        use sys::*;
        assert_eq!(dtw_preset("ggml-large-v3-turbo-q5_0.bin"), Some(whisper_alignment_heads_preset_WHISPER_AHEADS_LARGE_V3_TURBO));
        assert_eq!(dtw_preset("ggml-large-v3.bin"), Some(whisper_alignment_heads_preset_WHISPER_AHEADS_LARGE_V3));
        assert_eq!(dtw_preset("ggml-small.en.bin"), Some(whisper_alignment_heads_preset_WHISPER_AHEADS_SMALL_EN));
        assert_eq!(dtw_preset("my-finetune.bin"), None);
    }

    #[test]
    fn word_acc_joins_split_utf8() {
        // «ё» = D1 91; токенизатор может разрезать букву между токенами.
        let mut words = Vec::new();
        let mut acc = WordAcc::default();
        acc.push(b" \xD1", 100, 120, Some(105), 0.5);
        acc.push(b"\x91\xD0\xB6,", 120, 150, None, 1.0);
        acc.flush(&mut words);
        acc.flush(&mut words);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "ёж,");
        assert_eq!((words[0].start, words[0].end, words[0].dtw), (1.0, 1.5, Some(1.05)));
        assert!((words[0].p - 0.75).abs() < 1e-6);
    }
}
