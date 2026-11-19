//! `filter_complex` шортса по шаблону (ARCHITECTURE §6).
//!
//! Кропы считаются теми же функциями `sc-core`, что и в превью ([`background_crop`], [`main_video_crop_px`]),
//! параметры размытия — те же, что у композитора превью (¼ холста, сигма 40 px холста шириной 1080).
//! Всё, что не зависит от кадра (фон-картинка, тени, рамки, фигуры), — готовые растры `sc-raster`,
//! те же, что показывает превью; граф только накладывает их по порядку слоёв.

use std::path::PathBuf;

use sc_core::{Background, Canvas, InsertPlan, InsertZ, Layer, Template, background_crop, insert_fit, main_video_crop_px};
use sc_raster::{Art, Sprite, clip_rect_px, video_rect_px};

/// Во сколько раз фон меньше холста (как в превью, `sc_compositor::BLUR_DIVISOR`).
pub const BLUR_DIVISOR: u32 = 4;
/// Сигма размытия фона в пикселях холста шириной 1080 (как в превью, `sc_compositor::BLUR_SIGMA`).
pub const BLUR_SIGMA: f32 = 40.0;

/// Маска скругления для слоя: PGM-файл размером со слой.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskSpec {
    /// Индекс слоя в шаблоне.
    pub layer: usize,
    pub width: u32,
    pub height: u32,
    /// Радиус в пикселях холста.
    pub radius: f32,
}

/// Дополнительный вход ffmpeg (после исходника `0`), по порядку номеров `1, 2…`.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// Маска скругления (PGM).
    Mask(MaskSpec),
    /// Растр слоя `layer` (PAM): над содержимым слоя (`above`) или под ним.
    Sprite { layer: usize, above: bool },
    /// Фон-картинка (PAM) в цикле с частотой `fps`.
    BackgroundImage { fps: f64 },
    /// Фон-видео по кругу.
    BackgroundVideo { path: PathBuf },
    /// Субтитры слоя `layer`: concat-список картинок с длительностями ([`crate::subtitles::SubtitleSeq`]).
    Subtitles { layer: usize },
    /// Анимация слоя `Clip` (GIF, видео с альфой) по кругу; `decoder` — декодер, сохраняющий альфу.
    Clip { path: PathBuf, decoder: Option<String> },
    /// Исходник после места вставки (тот же файл, второй вход: `-ss` от места вставки).
    SourceTail,
    /// Клип вставки (`-t` длительность вставки).
    Insert { path: PathBuf },
    /// Готовый кадр обложки (PNG размером с холст) на длину обложки.
    Cover,
}

/// Обложка в графе: готовый кадр (размером с холст) в начале шортса.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoverGraph {
    /// Длительность, с.
    pub duration: f64,
    /// «Звук сразу»: кадр закрывает первые кадры шортса, ничего не сдвигается; иначе — перед шортсом
    /// (`concat`), на нём тишина.
    pub over: bool,
}

/// Вставка в графе: основное видео замирает на последнем кадре перед ней, вставка идёт поверх по плану,
/// звук — склейка «исходник → вставка → исходник» (ARCHITECTURE §6).
#[derive(Debug, Clone, PartialEq)]
pub struct InsertGraph {
    /// Начало вставки, секунды от начала шортса.
    pub at: f64,
    pub duration: f64,
    /// Остаток исходника после вставки, с.
    pub rest: f64,
    /// Размер кадра клипа (после поворота).
    pub size: (u32, u32),
    pub has_audio: bool,
    pub path: PathBuf,
    pub plan: InsertPlan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    /// Текст для `-filter_complex`; выход — метка `[out]`.
    pub filter: String,
    pub inputs: Vec<Input>,
    /// Звук собран графом (метка `[aout]`) — при вставке или обложке с тишиной; иначе звук берётся из исходника как есть.
    pub audio_out: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphParams {
    /// Размер кадра исходника (после поворота).
    pub src: (u32, u32),
    /// Частота кадров исходника — темп сгенерированных фонов, если `fps` не задан.
    pub src_fps: f64,
    /// Выходное разрешение; `None` — холст шаблона.
    pub resolution: Option<Canvas>,
    /// Частота кадров вывода; `None` — как у исходника.
    pub fps: Option<f64>,
    /// Хвост для VAAPI: `format=nv12,hwupload` вместо `format=<pix_fmt>`.
    pub vaapi: bool,
    /// Формат пикселей для CPU-кодировщика (`yuv420p`, у ProRes — 10 бит 4:2:2/4:4:4).
    pub pix_fmt: &'static str,
    /// Есть звук у исходника.
    pub audio: bool,
}

/// Строит граф. Шаблон должен быть проверен (`Template::validate`), `art` — растры этого шаблона,
/// `subtitles` — слои субтитров, которым есть что показать: (номер слоя, левый верхний угол картинок);
/// `insert` — рекламная вставка шортса, `cover` — обложка (время субтитров и вставки — от начала шортса без неё).
pub fn build(
    template: &Template,
    art: &Art,
    subtitles: &[(usize, i32, i32)],
    insert: Option<&InsertGraph>,
    cover: Option<&CoverGraph>,
    p: &GraphParams,
) -> Graph {
    let canvas = template.canvas;
    let (cw, ch) = (canvas.width, canvas.height);
    let out_fps = num(p.fps.unwrap_or(p.src_fps) as f32);
    let videos: Vec<usize> = template.main_videos().map(|(i, _, _)| i).collect();
    let mut inputs = Vec::new();

    let mut chains = Vec::new();
    let blur = matches!(template.background, Background::Blur { .. });

    // Со вставкой исходник — два входа: до места вставки (последний кадр держится всю вставку) и после.
    let mut head = String::from("[0:v]");
    if let Some(ins) = insert {
        let tail = add_input(&mut inputs, Input::SourceTail);
        chains.push(format!(
            "[0:v]tpad=stop_mode=clone:stop_duration={d}[pa];[pa][{tail}:v]concat=n=2:v=1:a=0[src]",
            d = secs(ins.duration)
        ));
        head = String::from("[src]");
    }
    // Вход: постоянная частота вывода (VFR-исходник иначе протекает в шортс через размытый фон,
    // который идёт по меткам исходника) и 8-битный yuv420p (для 8-битных исходников — без преобразования).
    head += &format!("fps={out_fps},");
    let branches = videos.len() + usize::from(blur);
    // Без видео и размытия (кадр не нужен) — исходник в никуда.
    head += &if branches == 0 { "nullsink".to_owned() } else { format!("format=yuv420p,split={branches}") };
    if blur {
        head += "[bg]";
    }
    for k in 0..videos.len() {
        head += &format!("[v{k}]");
    }
    chains.push(head);

    // Фон. Сгенерированные фоны идут с частотой вывода: база графа задаёт темп `overlay`.
    let dim = |darken: f32| {
        let d = num(1.0 - darken);
        // Умножение RGB на d, в YUV: яркость от «чёрного», цвет от середины шкалы.
        format!("lutyuv=y='minval+(val-minval)*{d}':u='(val-128)*{d}+128':v='(val-128)*{d}+128'")
    };
    match &template.background {
        Background::Blur { darken, scale } => {
            let (x, y, w, h) = background_crop(*scale, p.src, canvas).to_pixels_even(p.src.0, p.src.1);
            let (bw, bh) = (even(cw / BLUR_DIVISOR), even(ch / BLUR_DIVISOR));
            let sigma = BLUR_SIGMA * (cw as f32 / 1080.0) / BLUR_DIVISOR as f32;
            chains.push(format!(
                "[bg]crop={w}:{h}:{x}:{y},scale={bw}:{bh},gblur=sigma={sigma},{dim},scale={cw}:{ch}[base0]",
                sigma = num(sigma),
                dim = dim(*darken),
            ));
        }
        Background::Color { color } => {
            let [r, g, b, _] = color.0;
            chains.push(format!("color=c=0x{r:02x}{g:02x}{b:02x}:s={cw}x{ch}:r={out_fps},format=yuv420p[base0]"));
        }
        Background::Gradient { .. } | Background::Image { .. } => {
            let n = add_input(&mut inputs, Input::BackgroundImage { fps: p.fps.unwrap_or(p.src_fps) });
            chains.push(format!("[{n}:v]format=yuv420p[base0]"));
        }
        Background::Video { path, darken } => {
            let n = add_input(&mut inputs, Input::BackgroundVideo { path: path.clone() });
            // Размер видео-фона заранее неизвестен: «cover» средствами ffmpeg (scale с increase + crop).
            chains.push(format!(
                "[{n}:v]fps={out_fps},scale={cw}:{ch}:force_original_aspect_ratio=increase,crop={cw}:{ch},\
                 setsar=1,format=yuv420p,{dim}[base0]",
                dim = dim(*darken),
            ));
        }
    }
    // База — бесконечный источник (кроме размытия): граф завершает первое наложение видео.
    let mut shortest_pending = !blur;

    // Во время вставки часть слоёв не видна (план вставки).
    let hide = |i: usize| -> String {
        match insert {
            Some(ins) if ins.plan.hidden.contains(&i) => {
                format!(":enable='not(between(t,{},{}))'", secs(ins.at), secs(ins.at + ins.duration))
            }
            _ => String::new(),
        }
    };
    // Слои по порядку: [под слоем] → видео → [над слоем].
    let mut base = "base0".to_owned();
    let mut step = 0;
    let mut overlay = |chains: &mut Vec<String>, base: &mut String, top: String, x: i32, y: i32, extra: &str| {
        step += 1;
        let next = format!("base{step}");
        chains.push(format!("[{base}]{top}overlay={x}:{y}{extra}[{next}]"));
        *base = next;
    };
    let shortest = |on: bool| if on { ":shortest=1" } else { "" };
    let insert_chain = |chains: &mut Vec<String>, inputs: &mut Vec<Input>| -> Option<(String, i32, i32)> {
        let ins = insert?;
        let n = add_input(inputs, Input::Insert { path: ins.path.clone() });
        let (dx, dy, dw, dh) = ins.plan.dest;
        let fit = insert_fit(ins.size, dw, dh);
        let (cx, cy, cw, ch) = fit.crop;
        let (ox, oy, ow, oh) = fit.out;
        let mut c = format!("[{n}:v]fps={out_fps},crop={cw}:{ch}:{cx}:{cy},scale={ow}:{oh}:flags=lanczos,setsar=1,format=yuv420p");
        if (ow, oh) != (dw, dh) {
            c += &format!(",pad={dw}:{dh}:{ox}:{oy}:black");
        }
        if ins.plan.radius >= 0.5 {
            let layer = match ins.plan.z {
                InsertZ::AfterVideo(i) | InsertZ::Before(i) => i,
            };
            let m = add_input(inputs, Input::Mask(MaskSpec { layer, width: dw, height: dh, radius: ins.plan.radius }));
            c += &format!(",format=yuva420p[insa];[insa][{m}:v]alphamerge");
        }
        // Вставка начинается в `at`: до этого наложение пропускает основное видео, после конца — тоже.
        c += &format!(",setpts=PTS-STARTPTS+{}/TB[ins]", secs(ins.at));
        chains.push(c);
        Some(("[ins]".into(), dx as i32, dy as i32))
    };
    let ins_extra = ":eof_action=pass";
    let sprite = |s: &Option<Sprite>| s.as_ref().map(|s| (s.x, s.y));
    let mut k = 0;
    let z = insert.map(|i| i.plan.z);
    for (i, layer) in template.layers.iter().enumerate() {
        let la = &art.layers[i];
        if z == Some(InsertZ::Before(i))
            && let Some((top, x, y)) = insert_chain(&mut chains, &mut inputs)
        {
            overlay(&mut chains, &mut base, top, x, y, ins_extra);
        }
        let hid = hide(i);
        if let Some((x, y)) = sprite(&la.below) {
            let n = add_input(&mut inputs, Input::Sprite { layer: i, above: false });
            overlay(&mut chains, &mut base, format!("[{n}:v]"), x, y, &hid);
        }
        if let Layer::MainVideo { crop, dest, corner_radius, .. } = layer {
            let (sx, sy, sw, sh) = main_video_crop_px(*crop, p.src, *dest, canvas);
            let (dx, dy, dw, dh) = video_rect_px(*dest, canvas);
            let radius_px = (corner_radius * cw as f32).min(dw.min(dh) as f32 / 2.0);
            let mut chain = format!("[v{k}]crop={sw}:{sh}:{sx}:{sy},scale={dw}:{dh}:flags=lanczos");
            if radius_px >= 0.5 {
                let n = add_input(&mut inputs, Input::Mask(MaskSpec { layer: i, width: dw, height: dh, radius: radius_px }));
                chain += &format!(",format=yuva420p[m{k}a];[m{k}a][{n}:v]alphamerge");
            }
            chain += &format!("[m{k}]");
            chains.push(chain);
            overlay(&mut chains, &mut base, format!("[m{k}]"), dx as i32, dy as i32, shortest(shortest_pending));
            shortest_pending = false;
            k += 1;
            if z == Some(InsertZ::AfterVideo(i))
                && let Some((top, x, y)) = insert_chain(&mut chains, &mut inputs)
            {
                overlay(&mut chains, &mut base, top, x, y, ins_extra);
            }
        }
        if let Some((x, y)) = sprite(&la.above) {
            let n = add_input(&mut inputs, Input::Sprite { layer: i, above: true });
            overlay(&mut chains, &mut base, format!("[{n}:v]"), x, y, &hid);
        }
        if let Layer::Clip { path, decoder, .. } = layer
            && let Some((x, y, w, h)) = clip_rect_px(layer, canvas)
        {
            // Вход бесконечный (по кругу) — наложение заканчивается вместе с базой.
            let n = add_input(&mut inputs, Input::Clip { path: path.clone(), decoder: decoder.clone() });
            chains.push(format!("[{n}:v]fps={out_fps},scale={w}:{h}:flags=lanczos,format=yuva420p[c{i}]"));
            overlay(&mut chains, &mut base, format!("[c{i}]"), x, y, &format!(":shortest=1{hid}"));
        }
        if let Some(&(_, x, y)) = subtitles.iter().find(|s| s.0 == i) {
            // Картинка держится до следующей (repeatlast) — переменная частота входа не важна.
            let n = add_input(&mut inputs, Input::Subtitles { layer: i });
            overlay(&mut chains, &mut base, format!("[{n}:v]"), x, y, &hid);
        }
    }
    if z == Some(InsertZ::Before(template.layers.len()))
        && let Some((top, x, y)) = insert_chain(&mut chains, &mut inputs)
    {
        overlay(&mut chains, &mut base, top, x, y, ins_extra);
    }

    // Обложка — после всех слоёв (они живут во времени шортса без обложки).
    if let Some(c) = cover {
        let n = add_input(&mut inputs, Input::Cover);
        chains.push(format!("[{n}:v]fps={out_fps},format=yuv420p,setsar=1[cover]"));
        let next = "basec".to_owned();
        if c.over {
            chains.push(format!("[{base}][cover]overlay=0:0:eof_action=pass[{next}]"));
        } else {
            chains.push(format!("[{base}]setsar=1[basem];[cover][basem]concat=n=2:v=1:a=0[{next}]"));
        }
        base = next;
    }

    // Хвост: выходное разрешение и формат для кодировщика.
    let mut tail = format!("[{base}]");
    if let Some(r) = p.resolution.filter(|r| *r != canvas) {
        tail += &format!("scale={}:{}:flags=lanczos,", r.width, r.height);
    }
    tail += "setsar=1,";
    tail += &if p.vaapi { "format=nv12,hwupload".to_owned() } else { format!("format={}", p.pix_fmt) };
    tail += "[out]";
    chains.push(tail);

    // Звук со вставкой: каждый кусок — ровно своей длины (обрезан и дополнен тишиной), иначе после вставки
    // звук разъедется с картинкой.
    let mut audio_out = false;
    if let Some(ins) = insert
        && (p.audio || ins.has_audio)
    {
        const AF: &str = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";
        let piece = |input: Option<usize>, d: f64, label: &str| match input {
            Some(n) => format!("[{n}:a]{AF},atrim=duration={d},apad=whole_dur={d}[{label}]", d = secs(d)),
            None => format!("anullsrc=r=48000:cl=stereo,atrim=duration={d}[{label}]", d = secs(d)),
        };
        let tail = inputs.iter().position(|i| *i == Input::SourceTail).map(|k| k + 1);
        let ins_n = inputs.iter().position(|i| matches!(i, Input::Insert { .. })).map(|k| k + 1);
        chains.push(piece(p.audio.then_some(0), ins.at, "a0"));
        chains.push(piece(ins_n.filter(|_| ins.has_audio), ins.duration, "a1"));
        chains.push(piece(tail.filter(|_| p.audio), ins.rest, "a2"));
        chains.push("[a0][a1][a2]concat=n=3:v=0:a=1[aout]".into());
        audio_out = true;
    }
    // Обложка перед шортсом: на ней тишина.
    if let Some(c) = cover.filter(|c| !c.over)
        && (audio_out || p.audio)
    {
        const AF: &str = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";
        let main = if audio_out {
            let last = chains.last_mut().expect("звук вставки собран");
            *last = last.replace("[aout]", "[amain]");
            "[amain]".to_owned()
        } else {
            chains.push(format!("[0:a]{AF}[amain]"));
            "[amain]".to_owned()
        };
        chains.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={d}[acover]", d = secs(c.duration)));
        chains.push(format!("[acover]{main}concat=n=2:v=0:a=1[aout]"));
        audio_out = true;
    }

    Graph { filter: chains.join(";\n"), inputs, audio_out }
}

/// Секунды для фильтров: три знака, точка.
fn secs(v: f64) -> String {
    format!("{v:.3}")
}

/// Добавляет вход и возвращает его номер у ffmpeg (исходник — `0`).
fn add_input(inputs: &mut Vec<Input>, i: Input) -> usize {
    inputs.push(i);
    inputs.len()
}

fn even(v: u32) -> u32 {
    (v / 2 * 2).max(2)
}

/// Число без лишних нулей и всегда с точкой (ffmpeg не понимает запятую локали).
fn num(v: f32) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Маска скругления в формате PGM (P5): 255 внутри, 0 снаружи, сглаженный край.
pub fn mask_pgm(width: u32, height: u32, radius: f32) -> Vec<u8> {
    let mut out = format!("P5\n{width} {height}\n255\n").into_bytes();
    out.reserve((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let a = sc_core::rounded_rect_coverage(width as f32, height as f32, radius, x as f32 + 0.5, y as f32 + 0.5);
            out.push((a * 255.0).round() as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_core::{Border, Color, Paint, Rect, Shadow};

    fn params(src: (u32, u32)) -> GraphParams {
        GraphParams { src, src_fps: 60.0, resolution: None, fps: None, vaapi: false, pix_fmt: "yuv420p", audio: true }
    }

    fn graph(t: &Template, p: &GraphParams) -> Graph {
        build(t, &Art::new(t).unwrap(), &[], None, None, p)
    }

    #[test]
    fn minimal_template_graph() {
        let g = graph(&Template::minimal(), &params((1920, 1080)));
        assert!(g.inputs.is_empty());
        let f = &g.filter;
        assert!(f.starts_with("[0:v]fps=60,format=yuv420p,split=2[bg][v0]"), "{f}");
        // фон: полоса 9:16 из 16:9 → ¼ холста → сигма 10
        assert!(f.contains("[bg]crop=608:1080:656:0,scale=270:480,gblur=sigma=10,"), "{f}");
        assert!(f.contains("*0.7"), "{f}");
        // видео во всю ширину по центру, без подрезки
        assert!(f.contains("[v0]crop=1920:1080:0:0,scale=1080:608:flags=lanczos[m0]"), "{f}");
        assert!(f.contains("[base0][m0]overlay=0:656[base1]"), "{f}");
        assert!(f.ends_with("[base1]setsar=1,format=yuv420p[out]"), "{f}");
    }

    #[test]
    fn non_16x9_source_is_cropped_not_stretched() {
        // 4:3 в слот 16:9 — срезаются верх и низ
        let g = graph(&Template::minimal(), &params((1440, 1080)));
        assert!(g.filter.contains("[v0]crop=1440:810:0:136,"), "{}", g.filter);
    }

    #[test]
    fn vaapi_resolution_fps_color_and_mask() {
        let mut t = Template::minimal();
        t.background = Background::Color { color: Color::rgb(0x10, 0x18, 0x20) };
        let Layer::MainVideo { corner_radius, .. } = &mut t.layers[0] else { panic!() };
        *corner_radius = 0.05;
        let p = GraphParams {
            src: (1920, 1080),
            src_fps: 60.0,
            resolution: Some(Canvas { width: 720, height: 1280 }),
            fps: Some(30.0),
            vaapi: true,
            pix_fmt: "yuv420p",
            audio: true,
        };
        let g = graph(&t, &p);
        let f = &g.filter;
        assert!(f.starts_with("[0:v]fps=30,format=yuv420p,split=1[v0]"), "{f}");
        // темп цветного фона — частота вывода (иначе ffmpeg берёт 25 к/с источника color)
        assert!(f.contains("color=c=0x101820:s=1080x1920:r=30,format=yuv420p[base0]"), "{f}");
        assert!(f.contains("format=yuva420p[m0a];[m0a][1:v]alphamerge[m0]"), "{f}");
        assert!(f.contains("[base0][m0]overlay=0:656:shortest=1[base1]"), "{f}");
        assert!(f.ends_with("scale=720:1280:flags=lanczos,setsar=1,format=nv12,hwupload[out]"), "{f}");
        assert_eq!(g.inputs, vec![Input::Mask(MaskSpec { layer: 0, width: 1080, height: 608, radius: 54.0 })]);
    }

    #[test]
    fn layers_in_z_order_with_shadow_border_and_two_videos() {
        let mut t = Template::minimal();
        t.background = Background::Gradient { from: Color::BLACK, to: Color::WHITE, angle: 0.0 };
        t.layers = vec![
            Layer::Shape { rect: Rect::new(0.0, 0.5, 1.0, 0.5), paint: Paint::Solid { color: Color::BLACK }, corner_radius: 0.0 },
            Layer::MainVideo {
                crop: Rect::new(0.0, 0.0, 0.5, 1.0),
                dest: Rect::new(0.05, 0.1, 0.9, 0.35),
                corner_radius: 0.03,
                border: Some(Border { width: 0.004, color: Color::WHITE }),
                shadow: Some(Shadow { color: Color::BLACK, blur: 0.02, dx: 0.0, dy: 0.01 }),
            },
            Layer::MainVideo {
                crop: Rect::new(0.5, 0.0, 0.5, 1.0),
                dest: Rect::new(0.05, 0.55, 0.9, 0.35),
                corner_radius: 0.0,
                border: None,
                shadow: None,
            },
            Layer::BannerSlot { rect: Rect::new(0.0, 0.9, 1.0, 0.1) },
        ];
        t.validate().unwrap();
        let g = graph(&t, &params((1920, 1080)));
        assert_eq!(
            g.inputs,
            vec![
                Input::BackgroundImage { fps: 60.0 },
                Input::Sprite { layer: 0, above: true },
                Input::Sprite { layer: 1, above: false },
                Input::Mask(MaskSpec { layer: 1, width: 972, height: 672, radius: 0.03 * 1080.0 }),
                Input::Sprite { layer: 1, above: true },
            ]
        );
        let f = &g.filter;
        assert!(f.starts_with("[0:v]fps=60,format=yuv420p,split=2[v0][v1]"), "{f}");
        assert!(f.contains("[1:v]format=yuv420p[base0]"), "{f}");
        let order = [
            "[base0][2:v]overlay=0:960[base1]",
            "[base1][3:v]overlay=",
            "[m0a][4:v]alphamerge[m0]",
            "[base2][m0]overlay=54:192:shortest=1[base3]",
            "[base3][5:v]overlay=54:192[base4]",
            "[base4][m1]overlay=54:1056[base5]",
            "[base5]setsar=1",
        ];
        let pos: Vec<usize> = order.iter().map(|s| f.find(s).unwrap_or_else(|| panic!("нет «{s}» в {f}"))).collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "{f}");
    }

    #[test]
    fn video_background_loops_at_output_rate() {
        let mut t = Template::minimal();
        t.background = Background::Video { path: "/bg/loop.mp4".into(), darken: 0.5 };
        let g = graph(&t, &params((1920, 1080)));
        assert_eq!(g.inputs, vec![Input::BackgroundVideo { path: "/bg/loop.mp4".into() }]);
        assert!(g.filter.contains("[1:v]fps=60,scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,"), "{}", g.filter);
        assert!(g.filter.contains("*0.5"), "{}", g.filter);
        assert!(g.filter.contains("overlay=0:656:shortest=1"), "{}", g.filter);
    }

    #[test]
    fn subtitles_overlay_in_layer_order() {
        let mut t = Template::minimal();
        t.layers.push(Layer::BannerSlot { rect: Rect::new(0.0, 0.9, 1.0, 0.1) });
        t.layers.insert(1, Layer::BannerSlot { rect: Rect::new(0.0, 0.0, 1.0, 0.1) });
        let g = build(&t, &Art::new(&t).unwrap(), &[(1, 40, 1200)], None, None, &params((1920, 1080)));
        assert_eq!(g.inputs, vec![Input::Subtitles { layer: 1 }]);
        assert!(g.filter.contains("[base1][1:v]overlay=40:1200[base2]"), "{}", g.filter);
        assert!(g.filter.ends_with("[base2]setsar=1,format=yuv420p[out]"), "{}", g.filter);
    }

    #[test]
    fn animated_banner_loops_scaled_into_its_place() {
        let mut t = Template::minimal();
        t.layers.push(Layer::BannerSlot { rect: Rect::new(0.1, 0.7, 0.8, 0.1) });
        let art = sc_core::BannerArt {
            path: "/lib/b.webm".into(),
            size: (600, 200),
            animated: true,
            decoder: Some("libvpx-vp9".into()),
            text: None,
        };
        let t = t.with_banner(Some(&art));
        let g = graph(&t, &params((1920, 1080)));
        assert_eq!(g.inputs, vec![Input::Clip { path: "/lib/b.webm".into(), decoder: Some("libvpx-vp9".into()) }]);
        // рамка 864×192 → 576×192 по центру
        assert!(g.filter.contains("[1:v]fps=60,scale=576:192:flags=lanczos,format=yuva420p[c1]"), "{}", g.filter);
        assert!(g.filter.contains("[base1][c1]overlay=252:1344:shortest=1[base2]"), "{}", g.filter);
    }

    fn insert(t: &Template, mode: sc_core::InsertMode, banner: &[usize], hide: bool, audio: bool) -> InsertGraph {
        InsertGraph {
            at: 12.5,
            duration: 15.0,
            rest: 20.0,
            size: (1920, 1080),
            has_audio: audio,
            path: "/lib/ad.mp4".into(),
            plan: t.insert_plan(mode, banner, hide),
        }
    }

    #[test]
    fn slot_insert_freezes_source_and_splices_audio() {
        let mut t = Template::minimal();
        let Layer::MainVideo { corner_radius, .. } = &mut t.layers[0] else { panic!() };
        *corner_radius = 0.03;
        t.layers.push(Layer::BannerSlot { rect: Rect::new(0.1, 0.8, 0.8, 0.1) });
        let ins = insert(&t, sc_core::InsertMode::Slot, &[], false, true);
        let g = build(&t, &Art::new(&t).unwrap(), &[], Some(&ins), None, &params((1920, 1080)));
        let f = &g.filter;
        assert_eq!(
            g.inputs,
            vec![
                Input::SourceTail,
                Input::Mask(MaskSpec { layer: 0, width: 1080, height: 608, radius: 0.03 * 1080.0 }),
                Input::Insert { path: "/lib/ad.mp4".into() },
                Input::Mask(MaskSpec { layer: 0, width: 1080, height: 608, radius: 0.03 * 1080.0 }),
            ]
        );
        assert!(f.starts_with("[0:v]tpad=stop_mode=clone:stop_duration=15.000[pa];[pa][1:v]concat=n=2:v=1:a=0[src];\n[src]fps=60,format=yuv420p,split=2[bg][v0]"), "{f}");
        assert!(f.contains("[3:v]fps=60,crop=1918:1080:0:0,scale=1080:608:flags=lanczos,setsar=1,format=yuv420p,format=yuva420p[insa];[insa][4:v]alphamerge,setpts=PTS-STARTPTS+12.500/TB[ins]"), "{f}");
        assert!(f.contains("[base1][ins]overlay=0:656:eof_action=pass[base2]"), "{f}");
        assert!(f.contains("[0:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,atrim=duration=12.500,apad=whole_dur=12.500[a0]"), "{f}");
        assert!(f.contains("[3:a]aresample=48000,"), "{f}");
        assert!(f.contains("[1:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,atrim=duration=20.000,apad=whole_dur=20.000[a2]"), "{f}");
        assert!(f.ends_with("[a0][a1][a2]concat=n=3:v=0:a=1[aout]"), "{f}");
        assert!(g.audio_out);
    }

    #[test]
    fn fullscreen_insert_under_banner_hides_subtitles() {
        let mut t = Template::minimal();
        t.layers.push(Layer::BannerSlot { rect: Rect::new(0.0, 0.0, 1.0, 0.1) });
        t.layers.push(Layer::BannerSlot { rect: Rect::new(0.1, 0.8, 0.8, 0.1) });
        let art = sc_core::BannerArt { path: "/lib/b.gif".into(), size: (600, 200), animated: true, decoder: None, text: None };
        let (t, banner) = t.with_banner_layers(Some(&art));
        assert_eq!(banner, vec![1]);
        let mut ins = insert(&t, sc_core::InsertMode::Fullscreen, &banner, false, false);
        ins.size = (720, 1280);
        let g = build(&t, &Art::new(&t).unwrap(), &[(2, 0, 1500)], Some(&ins), None, &params((1920, 1080)));
        let f = &g.filter;
        // вставка — перед баннером (слой 1); субтитры (на слое 2, выше вставки) — прячутся
        let pos_ins = f.find("[ins]overlay=0:0:eof_action=pass").expect(f);
        let pos_banner = f.find("[c1]overlay=").expect(f);
        assert!(pos_ins < pos_banner, "{f}");
        assert!(f.contains("[c1]overlay=252:0:shortest=1[base"), "{f}");
        assert!(f.contains("overlay=0:1500:enable='not(between(t,12.500,27.500))'"), "{f}");
        assert!(f.contains("crop=720:1280:0:0,scale=1080:1920:flags=lanczos"), "{f}");
        // у вставки звука нет — тишина ровно на её длину
        assert!(f.contains("anullsrc=r=48000:cl=stereo,atrim=duration=15.000[a1]"), "{f}");
    }

    #[test]
    fn mask_corners_and_center() {
        let m = mask_pgm(40, 20, 8.0);
        let header = b"P5\n40 20\n255\n";
        assert_eq!(&m[..header.len()], header);
        let px = |x: usize, y: usize| m[header.len() + y * 40 + x];
        assert_eq!(px(0, 0), 0);
        assert_eq!(px(39, 19), 0);
        assert_eq!(px(20, 10), 255);
        assert_eq!(px(20, 0), 255);
        assert_eq!(px(0, 10), 255);
        assert_eq!(m.len(), header.len() + 40 * 20);
    }
}
