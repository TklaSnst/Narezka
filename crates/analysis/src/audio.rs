//! Звук исходника для whisper: ffmpeg → моно 16 кГц прямо в память (без временного WAV).
//!
//! Час звука — ~230 МБ f32; whisper всё равно принимает весь буфер целиком.
//! ffmpeg отдаёт s16, а не f32: при выводе во float он сводит стерео в моно без нормировки
//! (громче в √2 раз, выбросы за ±1), и whisper на таком звуке теряет пунктуацию и работает медленнее.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::AnalysisError;

pub const SAMPLE_RATE: u32 = 16_000;

/// Извлекает первую звуковую дорожку. `duration` — длительность по ffprobe (для прогресса 0…1).
pub fn extract_pcm(
    path: &Path,
    duration: f64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<Vec<f32>, AnalysisError> {
    let mut child = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-vn", "-ac", "1", "-ar", &SAMPLE_RATE.to_string(), "-f", "s16le", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| AnalysisError::Spawn { tool: "ffmpeg", source })?;

    let mut err = child.stderr.take().expect("stderr piped");
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });

    let expected = (duration.max(0.0) * SAMPLE_RATE as f64) as usize;
    let mut pcm = Vec::with_capacity(expected + SAMPLE_RATE as usize);
    let mut out = child.stdout.take().expect("stdout piped");
    let mut buf = vec![0u8; 1 << 16];
    // Байт, не добравший до целого сэмпла на границе чтения.
    let mut tail = Vec::with_capacity(1);
    let mut last = 0.0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AnalysisError::Cancelled);
        }
        let n = out.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let mut bytes = &buf[..n];
        if let Some(lo) = tail.pop() {
            pcm.push(sample([lo, bytes[0]]));
            bytes = &bytes[1..];
        }
        let (pairs, rest) = bytes.as_chunks::<2>();
        tail.extend_from_slice(rest);
        pcm.extend(pairs.iter().map(|&c| sample(c)));
        if expected > 0 {
            let f = (pcm.len() as f32 / expected as f32).min(1.0);
            if f - last >= 0.01 {
                last = f;
                progress(f);
            }
        }
    }
    let status = child.wait()?;
    let stderr = err_t.join().unwrap_or_default();
    if !status.success() {
        return Err(AnalysisError::Failed { tool: "ffmpeg", stderr: stderr.trim().to_owned() });
    }
    if pcm.is_empty() {
        return Err(AnalysisError::NoAudio);
    }
    progress(1.0);
    Ok(pcm)
}

fn sample(b: [u8; 2]) -> f32 {
    i16::from_le_bytes(b) as f32 / 32768.0
}
