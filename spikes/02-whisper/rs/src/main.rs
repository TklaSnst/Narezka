//! Спайк 0.2: whisper.cpp через FFI — VAD + DTW, слова с таймкодами на исходной шкале.
//! Использование: whisper-spike <модель.bin> <vad.bin> <16k-mono.wav>
#![allow(non_camel_case_types, non_upper_case_globals, non_snake_case, dead_code)]
mod sys {
    include!(concat!(env!("OUT_DIR"), "/whisper.rs"));
}
use std::ffi::{CStr, CString};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (model, vad, wav) = (&args[1], &args[2], &args[3]);
    // RADV без ReBAR: «host-visible VRAM» на деле в системной RAM → encode ×5 медленнее.
    unsafe { std::env::set_var("GGML_VK_DISABLE_HOST_VISIBLE_VIDMEM", "1") };

    let pcm: Vec<f32> = hound::WavReader::open(wav).unwrap()
        .samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();

    let model_c = CString::new(model.as_str()).unwrap();
    let vad_c = CString::new(vad.as_str()).unwrap();
    let lang = CString::new("ru").unwrap();
    let t = std::time::Instant::now();
    unsafe {
        let mut cp = sys::whisper_context_default_params();
        cp.use_gpu = true;
        cp.flash_attn = false; // DTW несовместим с flash attention
        cp.dtw_token_timestamps = true;
        cp.dtw_aheads_preset = sys::whisper_alignment_heads_preset_WHISPER_AHEADS_LARGE_V3_TURBO;
        let ctx = sys::whisper_init_from_file_with_params(model_c.as_ptr(), cp);
        assert!(!ctx.is_null());

        let mut p = sys::whisper_full_default_params(sys::whisper_sampling_strategy_WHISPER_SAMPLING_GREEDY);
        p.n_threads = 4;
        p.language = lang.as_ptr();
        p.token_timestamps = true;
        p.vad = true;
        p.vad_model_path = vad_c.as_ptr();
        p.print_progress = false;
        p.print_realtime = false;
        assert_eq!(sys::whisper_full(ctx, p, pcm.as_ptr(), pcm.len() as i32), 0);
        eprintln!("распознано за {:.1} с", t.elapsed().as_secs_f32());

        let eot = sys::whisper_token_eot(ctx);
        let mut words = 0;
        for s in 0..sys::whisper_full_n_segments(ctx) {
            let (mut word, mut w0, mut w1, mut wd) = (String::new(), 0i64, 0i64, 0i64);
            let mut flush = |word: &mut String, w0, w1, wd| {
                if !word.trim().is_empty() {
                    println!("{:8.2} {:8.2} dtw {:8.2}  {}", w0 as f64 / 100.0, w1 as f64 / 100.0, wd as f64 / 100.0, word.trim());
                    words += 1;
                }
                word.clear();
            };
            for i in 0..sys::whisper_full_n_tokens(ctx, s) {
                let d = sys::whisper_full_get_token_data(ctx, s, i);
                if d.id >= eot { continue; } // служебные токены
                let text = CStr::from_ptr(sys::whisper_full_get_token_text(ctx, s, i)).to_string_lossy().into_owned();
                let t0 = sys::whisper_full_get_token_t0(ctx, s, i);
                let t1 = sys::whisper_full_get_token_t1(ctx, s, i);
                // t_dtw остаётся на шкале VAD — сдвигаем на ту же поправку, что у t0
                let dtw = d.t_dtw + (t0 - d.t0);
                if text.starts_with(' ') || word.is_empty() {
                    flush(&mut word, w0, w1, wd);
                    w0 = t0;
                    wd = dtw;
                }
                word.push_str(&text);
                w1 = t1;
            }
            flush(&mut word, w0, w1, wd);
        }
        eprintln!("слов: {words}, VAD-сегментов: {}", sys::whisper_full_n_vad_segments(ctx));
        sys::whisper_free(ctx);
    }
}
