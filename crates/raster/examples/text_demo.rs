//! Пробная картинка текстовых слоёв шаблона: `cargo run -p sc-raster --example text_demo -- шаблон.toml out.png`.
//! Растры слоёв накладываются на серый холст (видео и фон-размытие не рисуются).

use sc_core::{Template, TextVars};
use sc_raster::{Art, Bitmap};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let t = Template::from_toml(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    let t = t.with_vars(&TextVars { part: 3, title: "Почему *bash* до сих пор жив".into() });
    let started = std::time::Instant::now();
    sc_raster::warm_up();
    eprintln!("шрифты: {:?}", started.elapsed());
    let started = std::time::Instant::now();
    let art = Art::new(&t).unwrap();
    eprintln!("растры: {:?}", started.elapsed());
    let (w, h) = (t.canvas.width, t.canvas.height);
    let mut canvas = art.background.clone().unwrap_or_else(|| {
        let mut b = Bitmap::new(w, h);
        b.data.chunks_mut(4).for_each(|p| p.copy_from_slice(&[60, 60, 60, 255]));
        b
    });
    for s in art.layers.iter().flat_map(|l| [&l.below, &l.above]).flatten() {
        for y in 0..s.bitmap.height {
            for x in 0..s.bitmap.width {
                let (cx, cy) = (s.x + x as i32, s.y + y as i32);
                if cx < 0 || cy < 0 || cx >= w as i32 || cy >= h as i32 {
                    continue;
                }
                let src = s.bitmap.pixel(x, y);
                let i = ((cy as u32 * w + cx as u32) * 4) as usize;
                let k = 255 - src[3] as u32;
                for (d, s) in canvas.data[i..i + 4].iter_mut().zip(src) {
                    *d = (s as u32 + (*d as u32 * k + 127) / 255).min(255) as u8;
                }
            }
        }
    }
    image::RgbaImage::from_raw(w, h, canvas.data).unwrap().save(&args[2]).unwrap();
}
