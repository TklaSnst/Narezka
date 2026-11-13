//! `filter_complex` шортса по шаблону (ARCHITECTURE §6).
//!
//! Кропы считаются теми же функциями `sc-core`, что и в превью ([`background_crop`], [`main_video_crop_px`]),
//! параметры размытия — те же, что у композитора превью (¼ холста, сигма 40 px холста шириной 1080).
//! Всё, что не зависит от кадра (фон-картинка, тени, рамки, фигуры), — готовые растры `sc-raster`,
//! те же, что показывает превью; граф только накладывает их по порядку слоёв.

use std::path::PathBuf;

use sc_core::{Background, Canvas, Layer, Template, background_crop, main_video_crop_px};
use sc_raster::{Art, Sprite, video_rect_px};

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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    /// Текст для `-filter_complex`; выход — метка `[out]`.
    pub filter: String,
    pub inputs: Vec<Input>,
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
    /// Хвост для VAAPI: `format=nv12,hwupload` вместо `format=yuv420p`.
    pub vaapi: bool,
}

/// Строит граф. Шаблон должен быть проверен (`Template::validate`), `art` — растры этого шаблона.
pub fn build(template: &Template, art: &Art, p: &GraphParams) -> Graph {
    let canvas = template.canvas;
    let (cw, ch) = (canvas.width, canvas.height);
    let out_fps = num(p.fps.unwrap_or(p.src_fps) as f32);
    let videos: Vec<usize> = template.main_videos().map(|(i, _, _)| i).collect();
    let mut inputs = Vec::new();

    let mut chains = Vec::new();
    let blur = matches!(template.background, Background::Blur { .. });

    // Вход: общая частота кадров и 8-битный yuv420p (для 8-битных исходников — без преобразования).
    let mut head = String::from("[0:v]");
    if let Some(fps) = p.fps {
        head += &format!("fps={},", num(fps as f32));
    }
    let branches = videos.len() + usize::from(blur);
    head += &format!("format=yuv420p,split={branches}");
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

    // Слои по порядку: [под слоем] → видео → [над слоем].
    let mut base = "base0".to_owned();
    let mut step = 0;
    let mut overlay = |chains: &mut Vec<String>, base: &mut String, top: String, x: i32, y: i32, shortest: bool| {
        step += 1;
        let next = format!("base{step}");
        let sh = if shortest { ":shortest=1" } else { "" };
        chains.push(format!("[{base}]{top}overlay={x}:{y}{sh}[{next}]"));
        *base = next;
    };
    let sprite = |s: &Option<Sprite>| s.as_ref().map(|s| (s.x, s.y));
    let mut k = 0;
    for (i, layer) in template.layers.iter().enumerate() {
        let la = &art.layers[i];
        if let Some((x, y)) = sprite(&la.below) {
            let n = add_input(&mut inputs, Input::Sprite { layer: i, above: false });
            overlay(&mut chains, &mut base, format!("[{n}:v]"), x, y, false);
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
            overlay(&mut chains, &mut base, format!("[m{k}]"), dx as i32, dy as i32, shortest_pending);
            shortest_pending = false;
            k += 1;
        }
        if let Some((x, y)) = sprite(&la.above) {
            let n = add_input(&mut inputs, Input::Sprite { layer: i, above: true });
            overlay(&mut chains, &mut base, format!("[{n}:v]"), x, y, false);
        }
    }

    // Хвост: выходное разрешение и формат для кодировщика.
    let mut tail = format!("[{base}]");
    if let Some(r) = p.resolution.filter(|r| *r != canvas) {
        tail += &format!("scale={}:{}:flags=lanczos,", r.width, r.height);
    }
    tail += "setsar=1,";
    tail += if p.vaapi { "format=nv12,hwupload" } else { "format=yuv420p" };
    tail += "[out]";
    chains.push(tail);

    Graph { filter: chains.join(";\n"), inputs }
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
        GraphParams { src, src_fps: 60.0, resolution: None, fps: None, vaapi: false }
    }

    fn graph(t: &Template, p: &GraphParams) -> Graph {
        build(t, &Art::new(t).unwrap(), p)
    }

    #[test]
    fn minimal_template_graph() {
        let g = graph(&Template::minimal(), &params((1920, 1080)));
        assert!(g.inputs.is_empty());
        let f = &g.filter;
        assert!(f.starts_with("[0:v]format=yuv420p,split=2[bg][v0]"), "{f}");
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
        assert!(f.starts_with("[0:v]format=yuv420p,split=2[v0][v1]"), "{f}");
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
