//! Одна задача рендера: ffmpeg с графом шаблона, прогресс из `-progress pipe:1`, отмена.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use sc_core::{Container, InsertPlan, OutputSettings, SubWord, Template, cover_duration};

use sc_raster::Art;

use crate::encode::{self, audio_args, muxer, video_args};
use crate::graph::{self, CoverGraph, GraphParams, Input, InsertGraph};
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

/// Рекламная вставка шортса.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertJob {
    pub path: PathBuf,
    /// Место вставки — секунды исходника (внутри `start..end` задачи).
    pub at: f64,
    /// Длительность клипа, с.
    pub duration: f64,
    /// Размер кадра клипа (после поворота).
    pub size: (u32, u32),
    pub has_audio: bool,
    pub plan: InsertPlan,
}

/// Откуда кадр обложки.
#[derive(Debug, Clone, PartialEq)]
pub enum CoverFrame {
    /// Кадр исходника задачи в момент `t`, секунды исходника.
    Source { t: f64 },
    /// Картинка вместо кадра видео; `size` — её размер.
    Image { path: PathBuf, size: (u32, u32) },
}

/// Обложка шортса: кадр по своему шаблону стоит в начале (SPEC §5).
#[derive(Debug, Clone, PartialEq)]
pub struct CoverJob {
    pub frame: CoverFrame,
    /// Шаблон обложки (с баннером и подстановками).
    pub template: Template,
    /// Длительность, кадров вывода.
    pub frames: u32,
    /// «Звук сразу»: обложка поверх первых кадров, без сдвига и тишины.
    pub sound_now: bool,
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
    /// Рекламная вставка: основное видео стоит, пока она идёт.
    pub insert: Option<InsertJob>,
    /// Обложка-кадр в начале.
    pub cover: Option<CoverJob>,
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
    /// Длительность итогового шортса (со вставкой и обложкой).
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0) + self.insert.as_ref().map_or(0.0, |i| i.duration) + self.cover_shift()
    }

    /// Частота кадров вывода.
    fn out_fps(&self) -> f64 {
        self.output.fps.unwrap_or(self.src_fps)
    }

    /// Длительность обложки, с.
    fn cover_duration(&self) -> f64 {
        self.cover.as_ref().map_or(0.0, |c| cover_duration(c.frames, self.out_fps()))
    }

    /// Насколько обложка сдвигает шортс (0 — «звук сразу» или обложки нет).
    fn cover_shift(&self) -> f64 {
        if self.cover.as_ref().is_some_and(|c| !c.sound_now) { self.cover_duration() } else { 0.0 }
    }

    /// Файл готового кадра обложки.
    fn cover_png(&self) -> PathBuf {
        self.work_dir.join("cover.png")
    }

    fn params(&self, src: (u32, u32)) -> GraphParams {
        GraphParams {
            src,
            src_fps: self.src_fps,
            resolution: self.output.resolution,
            fps: self.output.fps,
            vaapi: self.encoder.vaapi_device.is_some(),
            pix_fmt: encode::pix_fmt(self.output.codec, self.output.prores),
            audio: self.has_audio,
        }
    }

    /// Аргументы ffmpeg для кадра обложки (PNG размером с холст в [`Self::cover_png`]) и граф;
    /// `art` — растры шаблона обложки. `None` — обложки нет.
    pub fn cover_args(&self, art: &Art) -> Result<Option<(Vec<String>, graph::Graph)>, RenderError> {
        let Some(c) = &self.cover else { return Ok(None) };
        c.template.validate()?;
        let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-v", "error", "-y"].map(String::from).into();
        let src = match &c.frame {
            CoverFrame::Source { t } => {
                a.extend(["-ss".into(), secs(*t), "-i".into(), self.input.display().to_string()]);
                self.src_size
            }
            CoverFrame::Image { path, size } => {
                a.extend(["-loop".into(), "1".into(), "-i".into(), path.display().to_string()]);
                *size
            }
        };
        // Кадр — в размер холста шаблона шортса (масштаб к выводу — в основном проходе), без VAAPI.
        let params = GraphParams { resolution: None, fps: None, vaapi: false, audio: false, ..self.params(src) };
        let g = graph::build(&c.template, art, &[], None, None, &params);
        for (k, input) in g.inputs.iter().enumerate() {
            cover_input_opts(&mut a, input);
            a.extend(["-i".into(), self.input_path_in(&self.work_dir.join("cover"), k + 1, input).display().to_string()]);
        }
        a.extend(["-filter_complex".into(), g.filter.clone(), "-map".into(), "[out]".into()]);
        a.extend(["-frames:v", "1", "-update", "1", "-f", "image2"].map(String::from));
        a.push(self.cover_png().display().to_string());
        Ok(Some((a, g)))
    }

    /// Последовательности картинок субтитров по слоям шаблона: (номер слоя, последовательность).
    /// Время — от начала шортса: слова после вставки сдвинуты на её длину (во время вставки слой спрятан).
    pub fn subtitle_sequences(&self) -> Vec<(usize, SubtitleSeq)> {
        let canvas = self.template.canvas;
        let (words, end) = match &self.insert {
            Some(ins) => {
                let shift = |t: f64| if t >= ins.at { t + ins.duration } else { t };
                let words: Vec<SubWord> =
                    self.subtitles.iter().map(|w| SubWord { start: shift(w.start), end: shift(w.end), ..w.clone() }).collect();
                (words, self.end + ins.duration)
            }
            None => (self.subtitles.clone(), self.end),
        };
        (self.template.layers.iter().enumerate())
            .filter_map(|(i, l)| subtitles::sequence(l, &words, self.start, end, canvas).map(|s| (i, s)))
            .collect()
    }

    fn insert_graph(&self) -> Option<InsertGraph> {
        let ins = self.insert.as_ref()?;
        Some(InsertGraph {
            at: ins.at - self.start,
            duration: ins.duration,
            rest: (self.end - ins.at).max(0.0),
            size: ins.size,
            has_audio: ins.has_audio,
            path: ins.path.clone(),
            plan: ins.plan.clone(),
        })
    }

    /// Аргументы ffmpeg (без имени программы) и граф; его входы надо записать в `work_dir` ([`Self::run`]).
    /// `art` — растры шаблона задачи (`Art::new(&self.template)`), `subs` — [`Self::subtitle_sequences`].
    pub fn args(&self, art: &Art, subs: &[(usize, SubtitleSeq)]) -> Result<(Vec<String>, graph::Graph), RenderError> {
        self.template.validate()?;
        let out = &self.output;
        let video = out.video(encode::is_vaapi(&self.encoder.name));
        let codec_args = video_args(&self.encoder.name, &video, out.prores, self.out_fps())?;
        let muxer = muxer(out.container, out.codec, out.audio.codec)?;
        let params = self.params(self.src_size);
        let positions: Vec<(usize, i32, i32)> = subs.iter().map(|(i, s)| (*i, s.x, s.y)).collect();
        let insert = self.insert_graph();
        let cover = self.cover.as_ref().map(|c| CoverGraph { duration: self.cover_duration(), over: c.sound_now });
        let g = graph::build(&self.template, art, &positions, insert.as_ref(), cover.as_ref(), &params);

        let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-v", "error", "-y", "-progress", "pipe:1", "-nostats", "-stats_period", "0.25"]
            .map(String::from)
            .into();
        if let Some(dev) = &self.encoder.vaapi_device {
            a.extend(["-vaapi_device".into(), dev.display().to_string()]);
        }
        // Поиск до -i: быстрый и, при перекодировании, точный до кадра. Со вставкой первый вход — до её места.
        let first_end = self.insert.as_ref().map_or(self.end, |i| i.at);
        a.extend(["-ss".into(), secs(self.start), "-t".into(), secs((first_end - self.start).max(0.0))]);
        a.extend(["-i".into(), self.input.display().to_string()]);
        for (k, input) in g.inputs.iter().enumerate() {
            match input {
                Input::Mask(_) | Input::Sprite { .. } => {}
                Input::BackgroundImage { fps } => {
                    a.extend(["-loop".into(), "1".into(), "-framerate".into(), format!("{fps:.4}")]);
                }
                Input::BackgroundVideo { .. } => a.extend(["-stream_loop".into(), "-1".into()]),
                Input::Subtitles { .. } => a.extend(["-f".into(), "concat".into()]),
                Input::Clip { decoder, .. } => {
                    a.extend(["-stream_loop".into(), "-1".into()]);
                    if let Some(d) = decoder {
                        a.extend(["-c:v".into(), d.clone()]);
                    }
                }
                Input::SourceTail => {
                    let at = self.insert.as_ref().map_or(self.end, |i| i.at);
                    a.extend(["-ss".into(), secs(at), "-t".into(), secs((self.end - at).max(0.0))]);
                }
                Input::Insert { .. } => {
                    let d = self.insert.as_ref().map_or(0.0, |i| i.duration);
                    a.extend(["-t".into(), secs(d)]);
                }
                Input::Cover => {
                    let fps = self.out_fps();
                    a.extend(["-loop".into(), "1".into(), "-framerate".into(), format!("{fps:.4}")]);
                    a.extend(["-t".into(), secs(self.cover_duration())]);
                }
            }
            a.extend(["-i".into(), self.input_path(k + 1, input).display().to_string()]);
        }
        a.extend(["-filter_complex".into(), g.filter.clone(), "-map".into(), "[out]".into()]);
        a.extend(codec_args);
        if g.audio_out || self.has_audio {
            a.extend(["-map", if g.audio_out { "[aout]" } else { "0:a:0" }].map(String::from));
            a.extend(audio_args(&out.audio));
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
        self.input_path_in(&self.work_dir, n, input)
    }

    /// Файл входа `n` графа; сгенерированные — в каталоге `dir`.
    fn input_path_in(&self, dir: &Path, n: usize, input: &Input) -> PathBuf {
        match input {
            Input::Mask(_) => dir.join(format!("in{n}.pgm")),
            Input::Sprite { .. } | Input::BackgroundImage { .. } => dir.join(format!("in{n}.pam")),
            Input::BackgroundVideo { path } | Input::Clip { path, .. } | Input::Insert { path } => path.clone(),
            Input::SourceTail => self.input.clone(),
            Input::Subtitles { .. } => dir.join(format!("in{n}.ffconcat")),
            Input::Cover => self.cover_png(),
        }
    }

    /// Пишет сгенерированные входы графа `g` (маски, растры, субтитры) в каталог `dir`.
    fn write_inputs(&self, dir: &Path, g: &graph::Graph, art: &Art, subs: &[(usize, SubtitleSeq)]) -> Result<(), RenderError> {
        let io = |path: &Path| {
            let path = path.to_owned();
            move |source| RenderError::Io { path, source }
        };
        std::fs::create_dir_all(dir).map_err(io(dir))?;
        for (k, input) in g.inputs.iter().enumerate() {
            let data = match input {
                Input::Mask(m) => graph::mask_pgm(m.width, m.height, m.radius),
                Input::Sprite { layer, above } => {
                    let la = &art.layers[*layer];
                    let s = if *above { &la.above } else { &la.below };
                    s.as_ref().expect("вход графа есть только у существующего растра").bitmap.to_pam()
                }
                Input::BackgroundImage { .. } => art.background.as_ref().expect("фон-картинка растрирована").to_pam(),
                Input::BackgroundVideo { .. }
                | Input::Clip { .. }
                | Input::Insert { .. }
                | Input::SourceTail
                | Input::Cover => continue,
                Input::Subtitles { layer } => {
                    let seq = &subs.iter().find(|(i, _)| i == layer).expect("вход есть только у слоя с картинками").1;
                    let name = |f: usize| format!("in{}-{f}.png", k + 1);
                    for (f, bm) in seq.frames.iter().enumerate() {
                        let path = dir.join(name(f));
                        std::fs::write(&path, bm.to_png()).map_err(io(&path))?;
                    }
                    seq.ffconcat(name).into_bytes()
                }
            };
            let path = self.input_path_in(dir, k + 1, input);
            std::fs::write(&path, data).map_err(io(&path))?;
        }
        Ok(())
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
        if let Some(dir) = self.dest.parent() {
            std::fs::create_dir_all(dir).map_err(io(dir))?;
        }
        self.write_inputs(&self.work_dir, &g, &art, &subs)?;
        if let Some(c) = &self.cover {
            let result = self.render_cover(c, cancel);
            if result.is_err() {
                let _ = std::fs::remove_dir_all(&self.work_dir);
            }
            result?;
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

    /// Предварительный проход: кадр обложки по её шаблону → `cover.png`.
    fn render_cover(&self, c: &CoverJob, cancel: &AtomicBool) -> Result<(), RenderError> {
        let art = Art::new(&c.template)?;
        let (args, g) = self.cover_args(&art)?.expect("обложка есть");
        self.write_inputs(&self.work_dir.join("cover"), &g, &art, &[])?;
        log::info!("ffmpeg (обложка) {}", args.join(" "));
        if cancel.load(Ordering::Relaxed) {
            return Err(RenderError::Cancelled);
        }
        let out = Command::new("ffmpeg").args(&args).stdin(Stdio::null()).output().map_err(RenderError::Spawn)?;
        if !out.status.success() || !self.cover_png().exists() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            return Err(RenderError::Failed(format!("обложка: {err}")));
        }
        Ok(())
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

/// Ключи перед `-i` для входа графа обложки (там нет хвоста исходника, вставки и самой обложки).
fn cover_input_opts(a: &mut Vec<String>, input: &Input) {
    match input {
        Input::BackgroundImage { fps } => a.extend(["-loop".into(), "1".into(), "-framerate".into(), format!("{fps:.4}")]),
        Input::BackgroundVideo { .. } => a.extend(["-stream_loop".into(), "-1".into()]),
        Input::Clip { decoder, .. } => {
            a.extend(["-stream_loop".into(), "-1".into()]);
            if let Some(d) = decoder {
                a.extend(["-c:v".into(), d.clone()]);
            }
        }
        _ => {}
    }
}

fn secs(v: f64) -> String {
    format!("{v:.3}")
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
            insert: None,
            cover: None,
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
        assert!(matches!(job("av1_nvenc", false).args(&art, &[]), Err(RenderError::UnsupportedEncoder(_))));
        let mut j = job("libx264", false);
        j.output.container = Container::Webm;
        assert!(matches!(j.args(&art, &[]), Err(RenderError::UnsupportedContainer("webm"))));
    }

    #[test]
    fn insert_splits_source_and_maps_spliced_audio() {
        let mut j = job("h264_vaapi", true);
        let plan = j.template.insert_plan(sc_core::InsertMode::Slot, &[], false);
        j.insert = Some(InsertJob { path: "/lib/ad.mp4".into(), at: 71.5, duration: 15.0, size: (1920, 1080), has_audio: true, plan });
        assert_eq!(j.duration(), 45.0);
        let s = joined(&j);
        assert!(s.contains("-ss 61.500 -t 10.000 -i /in/ролик.mp4 -ss 71.500 -t 20.000 -i /in/ролик.mp4 -t 15.000 -i /lib/ad.mp4 -filter_complex"), "{s}");
        assert!(s.contains("-map [out] -c:v h264_vaapi"), "{s}");
        assert!(s.contains("-map [aout] -c:a aac -b:a 192k"), "{s}");
        // субтитры после вставки сдвинуты на её длину
        j.subtitles = vec![
            SubWord { text: "до".into(), start: 70.0, end: 70.5, keyword: false },
            SubWord { text: "после".into(), start: 72.0, end: 72.5, keyword: false },
        ];
        let mut t = Template::minimal();
        t.layers.push(
            toml::from_str(
                r##"
                type = "subtitles"
                rect = { x = 0.05, y = 0.6, w = 0.9, h = 0.1 }
                font = "Montserrat ExtraBold"
                size = 0.06
                color = "#ffffff"
                "##,
            )
            .unwrap(),
        );
        j.template = t;
        let seqs = j.subtitle_sequences();
        let tl = &seqs[0].1.timeline;
        let total: f64 = tl.iter().map(|(_, d)| d).sum();
        assert!((total - 45.0).abs() < 1e-6, "{tl:?}");
        // «после» начинается в 72 − 61.5 + 15 = 25.5 с от начала шортса
        let mut t = 0.0;
        let starts: Vec<f64> = tl.iter().map(|(_, d)| { let s = t; t += d; s }).collect();
        assert!(starts.iter().any(|s| (s - 25.5).abs() < 1e-6), "{tl:?}");
    }

    #[test]
    fn progress_lines() {
        assert_eq!(parse_out_time("out_time_us=1500000"), Some(1.5));
        assert_eq!(parse_out_time("out_time_us=N/A"), None);
        assert_eq!(parse_out_time("out_time_us=-9223372036854775807"), None);
        assert_eq!(parse_out_time("progress=continue"), None);
    }
}
