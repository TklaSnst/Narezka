//! Одна задача рендера: ffmpeg с графом шаблона, прогресс из `-progress pipe:1`, отмена.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use sc_core::{Codec, Container, OutputSettings, SubWord, Template};

use sc_raster::Art;

use crate::graph::{self, GraphParams, Input};
use crate::naming::part_path;
use crate::subtitles::{self, SubtitleSeq};

/// Кодировщик, выбранный по `Capabilities` (рендер не проверяет ffmpeg сам).
#[derive(Debug, Clone, PartialEq)]
pub struct Encoder {
    /// Имя для `-c:v`: `h264_vaapi`, `libx264`…
    pub name: String,
    /// Устройство VAAPI — только для `*_vaapi`.
    pub vaapi_device: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderJob {
    pub input: PathBuf,
    /// Размер кадра исходника (после поворота, как в ffprobe).
    pub src_size: (u32, u32),
    /// Частота кадров исходника (темп фонов-картинок и цвета, если вывод «как у исходника»).
    pub src_fps: f64,
    pub has_audio: bool,
    /// Отрезок исходника, секунды.
    pub start: f64,
    pub end: f64,
    pub template: Template,
    /// Слова субтитров шортса (время — секунды исходника, с правками и ключевыми); пусто — субтитров нет.
    pub subtitles: Vec<SubWord>,
    pub output: OutputSettings,
    pub encoder: Encoder,
    /// Итоговый файл; пока идёт рендер, пишется `.<имя>.part` рядом.
    pub dest: PathBuf,
    /// Каталог для временных файлов задачи (маски, растры); удаляется по завершении.
    pub work_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("кодировщик {0} пока не поддерживается рендером")]
    UnsupportedEncoder(String),
    #[error("кодек не помещается в контейнер {0}")]
    UnsupportedContainer(&'static str),
    #[error("неверный шаблон: {0}")]
    Template(#[from] sc_core::TemplateError),
    #[error("{0}")]
    Raster(#[from] sc_raster::RasterError),
    #[error("не удалось запустить ffmpeg: {0}")]
    Spawn(std::io::Error),
    #[error("ffmpeg завершился с ошибкой: {0}")]
    Failed(String),
    #[error("отменено")]
    Cancelled,
    #[error("ошибка файла {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
}

impl RenderJob {
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }

    /// Последовательности картинок субтитров по слоям шаблона: (номер слоя, последовательность).
    /// Время — от начала шортса. Пока обложки и вставки нет, это просто сдвиг на `start`.
    pub fn subtitle_sequences(&self) -> Vec<(usize, SubtitleSeq)> {
        let canvas = self.template.canvas;
        (self.template.layers.iter().enumerate())
            .filter_map(|(i, l)| subtitles::sequence(l, &self.subtitles, self.start, self.end, canvas).map(|s| (i, s)))
            .collect()
    }

    /// Аргументы ffmpeg (без имени программы) и граф; его входы надо записать в `work_dir` ([`Self::run`]).
    /// `art` — растры шаблона задачи (`Art::new(&self.template)`), `subs` — [`Self::subtitle_sequences`].
    pub fn args(&self, art: &Art, subs: &[(usize, SubtitleSeq)]) -> Result<(Vec<String>, graph::Graph), RenderError> {
        self.template.validate()?;
        let codec_args = encoder_args(&self.encoder.name, self.output.video_bitrate_kbps)?;
        let codec = match self.encoder.name.as_str() {
            n if n.starts_with("h264") || n == "libx264" => Codec::H264,
            _ => Codec::Hevc,
        };
        let muxer = muxer(self.output.container, codec)?;
        let vaapi = self.encoder.vaapi_device.is_some();
        let params = GraphParams {
            src: self.src_size,
            src_fps: self.src_fps,
            resolution: self.output.resolution,
            fps: self.output.fps,
            vaapi,
        };
        let positions: Vec<(usize, i32, i32)> = subs.iter().map(|(i, s)| (*i, s.x, s.y)).collect();
        let g = graph::build(&self.template, art, &positions, &params);

        let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-v", "error", "-y", "-progress", "pipe:1", "-nostats", "-stats_period", "0.25"]
            .map(String::from)
            .into();
        if let Some(dev) = &self.encoder.vaapi_device {
            a.extend(["-vaapi_device".into(), dev.display().to_string()]);
        }
        // Поиск до -i: быстрый и, при перекодировании, точный до кадра.
        a.extend(["-ss".into(), secs(self.start), "-t".into(), secs(self.duration())]);
        a.extend(["-i".into(), self.input.display().to_string()]);
        for (k, input) in g.inputs.iter().enumerate() {
            match input {
                Input::Mask(_) | Input::Sprite { .. } => {}
                Input::BackgroundImage { fps } => {
                    a.extend(["-loop".into(), "1".into(), "-framerate".into(), format!("{fps:.4}")]);
                }
                Input::BackgroundVideo { .. } => a.extend(["-stream_loop".into(), "-1".into()]),
                Input::Subtitles { .. } => a.extend(["-f".into(), "concat".into()]),
            }
            a.extend(["-i".into(), self.input_path(k + 1, input).display().to_string()]);
        }
        a.extend(["-filter_complex".into(), g.filter.clone(), "-map".into(), "[out]".into()]);
        a.extend(codec_args);
        if self.has_audio {
            let kbps = self.output.audio_bitrate_kbps;
            a.extend(["-map", "0:a:0", "-c:a", "aac"].map(String::from));
            a.extend(["-b:a".into(), format!("{kbps}k")]);
        } else {
            a.push("-an".into());
        }
        // Главы и метаданные исходника (Twitch-VOD) шортсу не нужны: главы в MP4 — лишняя дорожка.
        a.extend(["-map_chapters", "-1", "-map_metadata", "-1"].map(String::from));
        if matches!(self.output.container, Container::Mp4 | Container::Mov) {
            a.extend(["-movflags".into(), "+faststart".into()]);
        }
        a.extend(["-f".into(), muxer.into(), part_path(&self.dest).display().to_string()]);
        Ok((a, g))
    }

    /// Файл входа `n` графа: сгенерированные — в `work_dir`, видео-фон — как есть.
    fn input_path(&self, n: usize, input: &Input) -> PathBuf {
        match input {
            Input::Mask(_) => self.work_dir.join(format!("in{n}.pgm")),
            Input::Sprite { .. } | Input::BackgroundImage { .. } => self.work_dir.join(format!("in{n}.pam")),
            Input::BackgroundVideo { path } => path.clone(),
            Input::Subtitles { .. } => self.work_dir.join(format!("in{n}.ffconcat")),
        }
    }

    /// Рендерит; `progress` получает долю 0..1. Отмена — `cancel` в `true`: процесс завершается,
    /// недописанный файл удаляется. Итоговый файл появляется только после успешного завершения.
    pub fn run(&self, cancel: &AtomicBool, mut progress: impl FnMut(f32)) -> Result<(), RenderError> {
        let art = Art::new(&self.template)?;
        let t0 = std::time::Instant::now();
        let subs = self.subtitle_sequences();
        if !subs.is_empty() {
            let n: usize = subs.iter().map(|(_, s)| s.frames.len()).sum();
            log::info!("субтитры: {n} картинок за {:.0} мс", t0.elapsed().as_secs_f64() * 1e3);
        }
        let (args, g) = self.args(&art, &subs)?;
        let io = |path: &Path| {
            let path = path.to_owned();
            move |source| RenderError::Io { path, source }
        };
        std::fs::create_dir_all(&self.work_dir).map_err(io(&self.work_dir))?;
        if let Some(dir) = self.dest.parent() {
            std::fs::create_dir_all(dir).map_err(io(dir))?;
        }
        for (k, input) in g.inputs.iter().enumerate() {
            let data = match input {
                Input::Mask(m) => graph::mask_pgm(m.width, m.height, m.radius),
                Input::Sprite { layer, above } => {
                    let la = &art.layers[*layer];
                    let s = if *above { &la.above } else { &la.below };
                    s.as_ref().expect("вход графа есть только у существующего растра").bitmap.to_pam()
                }
                Input::BackgroundImage { .. } => art.background.as_ref().expect("фон-картинка растрирована").to_pam(),
                Input::BackgroundVideo { .. } => continue,
                Input::Subtitles { layer } => {
                    let seq = &subs.iter().find(|(i, _)| i == layer).expect("вход есть только у слоя с картинками").1;
                    let name = |f: usize| format!("in{}-{f}.png", k + 1);
                    for (f, bm) in seq.frames.iter().enumerate() {
                        let path = self.work_dir.join(name(f));
                        std::fs::write(&path, bm.to_png()).map_err(io(&path))?;
                    }
                    seq.ffconcat(name).into_bytes()
                }
            };
            let path = self.input_path(k + 1, input);
            std::fs::write(&path, data).map_err(io(&path))?;
        }
        log::info!("ffmpeg {}", args.join(" "));

        let part = part_path(&self.dest);
        let result = self.spawn_and_wait(&args, cancel, &mut progress);
        let result = result.and_then(|()| std::fs::rename(&part, &self.dest).map_err(io(&self.dest)));
        if result.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        let _ = std::fs::remove_dir_all(&self.work_dir);
        if result.is_ok() {
            progress(1.0);
        }
        result
    }

    fn spawn_and_wait(&self, args: &[String], cancel: &AtomicBool, progress: &mut impl FnMut(f32)) -> Result<(), RenderError> {
        let mut child = Command::new("ffmpeg")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(RenderError::Spawn)?;
        let mut err = child.stderr.take().expect("stderr piped");
        let err_t = thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            s
        });

        // Прогресс приходит каждые 0.25 с — между строками и проверяем отмену.
        let duration = self.duration();
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
        for line in stdout.lines() {
            if cancel.load(Ordering::Relaxed) {
                return Err(kill(child));
            }
            let Ok(line) = line else { break };
            if let Some(t) = parse_out_time(&line)
                && duration > 0.0
            {
                progress((t / duration).clamp(0.0, 1.0) as f32);
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(kill(child));
        }
        let status = child.wait().map_err(RenderError::Spawn)?;
        let stderr = err_t.join().unwrap_or_default();
        if status.success() {
            Ok(())
        } else {
            let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
            let msg = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
            Err(RenderError::Failed(if msg.is_empty() { status.to_string() } else { msg }))
        }
    }
}

fn kill(mut child: Child) -> RenderError {
    let _ = child.kill();
    let _ = child.wait();
    RenderError::Cancelled
}

/// `out_time_us=12345678` → секунды (`N/A` в начале — пропуск).
fn parse_out_time(line: &str) -> Option<f64> {
    let us: i64 = line.strip_prefix("out_time_us=")?.trim().parse().ok()?;
    (us >= 0).then_some(us as f64 / 1e6)
}

fn secs(v: f64) -> String {
    format!("{v:.3}")
}

/// Параметры кодировщика (ARCHITECTURE §6): VBR, maxrate ×1.5, bufsize ×2.
/// Остальные кодеки (AV1, VP9, ProRes) — на этапе 7.
fn encoder_args(name: &str, kbps: u32) -> Result<Vec<String>, RenderError> {
    let mut a: Vec<String> = vec!["-c:v".into(), name.into()];
    match name {
        "h264_vaapi" | "hevc_vaapi" => a.extend(["-rc_mode".into(), "VBR".into()]),
        "libx264" => a.extend(["-preset".into(), "veryfast".into()]),
        "libx265" => a.extend(["-preset".into(), "fast".into()]),
        _ => return Err(RenderError::UnsupportedEncoder(name.into())),
    }
    a.extend([
        "-b:v".into(),
        format!("{kbps}k"),
        "-maxrate".into(),
        format!("{}k", kbps * 3 / 2),
        "-bufsize".into(),
        format!("{}k", kbps * 2),
    ]);
    if name == "hevc_vaapi" || name == "libx265" {
        // Без тега hvc1 QuickTime и часть плееров не открывают HEVC в MP4.
        a.extend(["-tag:v".into(), "hvc1".into()]);
    }
    Ok(a)
}

fn muxer(container: Container, codec: Codec) -> Result<&'static str, RenderError> {
    match (container, codec) {
        (Container::Mp4, Codec::H264 | Codec::Hevc) => Ok("mp4"),
        (Container::Mov, Codec::H264 | Codec::Hevc) => Ok("mov"),
        (Container::Mkv, _) => Ok("matroska"),
        (c, _) => Err(RenderError::UnsupportedContainer(c.extension())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(encoder: &str, vaapi: bool) -> RenderJob {
        RenderJob {
            input: "/in/ролик.mp4".into(),
            src_size: (1920, 1080),
            src_fps: 60.0,
            has_audio: true,
            start: 61.5,
            end: 91.5,
            template: Template::minimal(),
            subtitles: vec![],
            output: OutputSettings::default(),
            encoder: Encoder { name: encoder.into(), vaapi_device: vaapi.then(|| "/dev/dri/renderD128".into()) },
            dest: "/out/short_1.mp4".into(),
            work_dir: "/tmp/w".into(),
        }
    }

    fn joined(j: &RenderJob) -> String {
        j.args(&Art::new(&j.template).unwrap(), &j.subtitle_sequences()).unwrap().0.join(" ")
    }

    #[test]
    fn vaapi_command() {
        let s = joined(&job("h264_vaapi", true));
        assert!(s.contains("-vaapi_device /dev/dri/renderD128 -ss 61.500 -t 30.000 -i /in/ролик.mp4 -filter_complex"), "{s}");
        assert!(s.contains("format=nv12,hwupload[out] -map [out] -c:v h264_vaapi -rc_mode VBR -b:v 12000k -maxrate 18000k -bufsize 24000k"), "{s}");
        assert!(s.contains("-map 0:a:0 -c:a aac -b:a 192k"), "{s}");
        assert!(s.ends_with("-movflags +faststart -f mp4 /out/.short_1.mp4.part"), "{s}");
    }

    #[test]
    fn cpu_command_without_audio() {
        let mut j = job("libx264", false);
        j.has_audio = false;
        let s = joined(&j);
        assert!(!s.contains("vaapi"), "{s}");
        assert!(s.contains("format=yuv420p[out] -map [out] -c:v libx264 -preset veryfast"), "{s}");
        assert!(s.contains(" -an "), "{s}");
    }

    #[test]
    fn unsupported_combinations() {
        let art = Art::new(&Template::minimal()).unwrap();
        assert!(matches!(job("libsvtav1", false).args(&art, &[]), Err(RenderError::UnsupportedEncoder(_))));
        let mut j = job("libx264", false);
        j.output.container = Container::Webm;
        assert!(matches!(j.args(&art, &[]), Err(RenderError::UnsupportedContainer("webm"))));
    }

    #[test]
    fn progress_lines() {
        assert_eq!(parse_out_time("out_time_us=1500000"), Some(1.5));
        assert_eq!(parse_out_time("out_time_us=N/A"), None);
        assert_eq!(parse_out_time("out_time_us=-9223372036854775807"), None);
        assert_eq!(parse_out_time("progress=continue"), None);
    }
}
