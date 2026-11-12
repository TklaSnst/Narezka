//! `filter_complex` шортса по шаблону (ARCHITECTURE §6).
//!
//! Кропы считаются теми же функциями `sc-core`, что и в превью ([`background_crop`], [`main_video_crop`]),
//! параметры размытия — те же, что у композитора превью (¼ холста, сигма 40 px холста шириной 1080).

use sc_core::{Background, Canvas, Layer, Rect, Template, background_crop, main_video_crop};

/// Во сколько раз фон меньше холста (как в превью, `sc_compositor::BLUR_DIVISOR`).
pub const BLUR_DIVISOR: u32 = 4;
/// Сигма размытия фона в пикселях холста шириной 1080 (как в превью, `sc_compositor::BLUR_SIGMA`).
pub const BLUR_SIGMA: f32 = 40.0;

/// Маска скругления для слоя: PGM-файл размером со слой, подаётся отдельным входом ffmpeg.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskSpec {
    /// Индекс слоя в шаблоне.
    pub layer: usize,
    pub width: u32,
    pub height: u32,
    /// Радиус в пикселях холста.
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    /// Текст для `-filter_complex`; выход — метка `[out]`.
    pub filter: String,
    /// Маски по порядку входов: первая — вход `1`, вторая — `2`…
    pub masks: Vec<MaskSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphParams {
    /// Размер кадра исходника (после поворота).
    pub src: (u32, u32),
    /// Выходное разрешение; `None` — холст шаблона.
    pub resolution: Option<Canvas>,
    /// Частота кадров вывода; `None` — как у исходника.
    pub fps: Option<f64>,
    /// Хвост для VAAPI: `format=nv12,hwupload` вместо `format=yuv420p`.
    pub vaapi: bool,
}

/// Строит граф. Шаблон должен быть проверен (`Template::validate`).
pub fn build(template: &Template, p: &GraphParams) -> Graph {
    let canvas = template.canvas;
    let (cw, ch) = (canvas.width, canvas.height);
    let videos: Vec<(usize, Rect, Rect, f32)> = template
        .layers
        .iter()
        .enumerate()
        .map(|(i, l)| match l {
            Layer::MainVideo { crop, dest, corner_radius } => (i, *crop, *dest, *corner_radius),
        })
        .collect();

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

    // Фон.
    match template.background {
        Background::Blur { darken, scale } => {
            let (x, y, w, h) = background_crop(scale, p.src, canvas).to_pixels_even(p.src.0, p.src.1);
            let (bw, bh) = (even(cw / BLUR_DIVISOR), even(ch / BLUR_DIVISOR));
            let sigma = BLUR_SIGMA * (cw as f32 / 1080.0) / BLUR_DIVISOR as f32;
            let d = num(1.0 - darken);
            // Затемнение — умножение RGB на d, в YUV: яркость от «чёрного», цвет от середины шкалы.
            chains.push(format!(
                "[bg]crop={w}:{h}:{x}:{y},scale={bw}:{bh},gblur=sigma={sigma},\
                 lutyuv=y='minval+(val-minval)*{d}':u='(val-128)*{d}+128':v='(val-128)*{d}+128',\
                 scale={cw}:{ch}[base0]",
                sigma = num(sigma),
            ));
        }
        Background::Color { color } => {
            let [r, g, b, _] = color.0;
            // Частота источника цвета не важна: overlay берёт кадры по видео, `shortest` завершает граф.
            chains.push(format!("color=c=0x{r:02x}{g:02x}{b:02x}:s={cw}x{ch}[base0]"));
        }
    }

    // Слои основного видео.
    let mut masks = Vec::new();
    let mut base = "base0".to_owned();
    for (k, (layer, crop, dest, radius)) in videos.iter().enumerate() {
        let (sx, sy, sw, sh) = main_video_crop(*crop, p.src, *dest, canvas).to_pixels_even(p.src.0, p.src.1);
        let (dx, dy, dw, dh) = dest.to_pixels_even(cw, ch);
        let radius_px = (radius * cw as f32).min(dw.min(dh) as f32 / 2.0);
        let mut chain = format!("[v{k}]crop={sw}:{sh}:{sx}:{sy},scale={dw}:{dh}:flags=lanczos");
        if radius_px >= 0.5 {
            masks.push(MaskSpec { layer: *layer, width: dw, height: dh, radius: radius_px });
            chain += &format!(",format=yuva420p[m{k}a];[m{k}a][{}:v]alphamerge", masks.len());
        }
        chain += &format!("[m{k}]");
        chains.push(chain);
        let shortest = if blur || k > 0 { "" } else { ":shortest=1" };
        let next = format!("base{}", k + 1);
        chains.push(format!("[{base}][m{k}]overlay={dx}:{dy}{shortest}[{next}]"));
        base = next;
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

    Graph { filter: chains.join(";\n"), masks }
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
    let (w, h) = (width as f32, height as f32);
    let r = radius.min(w / 2.0).min(h / 2.0);
    out.reserve((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            // Расстояние от центра пикселя до скруглённого прямоугольника (отрицательное — внутри).
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let qx = (px - w / 2.0).abs() - (w / 2.0 - r);
            let qy = (py - h / 2.0).abs() - (h / 2.0 - r);
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            let dist = outside + qx.max(qy).min(0.0) - r;
            let a = (0.5 - dist).clamp(0.0, 1.0);
            out.push((a * 255.0).round() as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(src: (u32, u32)) -> GraphParams {
        GraphParams { src, resolution: None, fps: None, vaapi: false }
    }

    #[test]
    fn minimal_template_graph() {
        let g = build(&Template::minimal(), &params((1920, 1080)));
        assert!(g.masks.is_empty());
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
        let g = build(&Template::minimal(), &params((1440, 1080)));
        assert!(g.filter.contains("[v0]crop=1440:810:0:136,"), "{}", g.filter);
    }

    #[test]
    fn vaapi_resolution_fps_color_and_mask() {
        let mut t = Template::minimal();
        t.background = Background::Color { color: sc_core::Color::rgb(0x10, 0x18, 0x20) };
        let Layer::MainVideo { corner_radius, .. } = &mut t.layers[0];
        *corner_radius = 0.05;
        let p = GraphParams {
            src: (1920, 1080),
            resolution: Some(Canvas { width: 720, height: 1280 }),
            fps: Some(30.0),
            vaapi: true,
        };
        let g = build(&t, &p);
        let f = &g.filter;
        assert!(f.starts_with("[0:v]fps=30,format=yuv420p,split=1[v0]"), "{f}");
        assert!(f.contains("color=c=0x101820:s=1080x1920[base0]"), "{f}");
        assert!(f.contains("format=yuva420p[m0a];[m0a][1:v]alphamerge[m0]"), "{f}");
        assert!(f.contains("overlay=0:656:shortest=1[base1]"), "{f}");
        assert!(f.ends_with("scale=720:1280:flags=lanczos,setsar=1,format=nv12,hwupload[out]"), "{f}");
        assert_eq!(g.masks, vec![MaskSpec { layer: 0, width: 1080, height: 608, radius: 54.0 }]);
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
