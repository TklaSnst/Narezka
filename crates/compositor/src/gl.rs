//! GL-примитивы превью: текстура с FBO и двухпроходный gaussian blur.

use glow::HasContext;

/// Текстура RGBA8 с FBO, в которую можно рисовать.
pub struct Target {
    pub tex: glow::Texture,
    pub fbo: glow::Framebuffer,
    pub w: i32,
    pub h: i32,
    mipmaps: bool,
}

impl Target {
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn new(gl: &glow::Context, w: i32, h: i32, mipmaps: bool) -> Result<Self, String> {
        unsafe {
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w,
                h,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            let min = if mipmaps { glow::LINEAR_MIPMAP_LINEAR } else { glow::LINEAR };
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, min as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            if mipmaps {
                gl.generate_mipmap(glow::TEXTURE_2D);
            }
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            let t = Self { tex, fbo, w, h, mipmaps };
            if status != glow::FRAMEBUFFER_COMPLETE {
                t.delete(gl);
                return Err(format!("FBO {w}×{h} неполный: 0x{status:x}"));
            }
            Ok(t)
        }
    }

    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn update_mipmaps(&self, gl: &glow::Context) {
        if self.mipmaps {
            unsafe {
                gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
                gl.generate_mipmap(glow::TEXTURE_2D);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
        }
    }

    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_texture(self.tex);
        }
    }
}

const VS: &str = r#"
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

// Один проход separable gaussian. uv_rect — какой участок источника растянуть на выход,
// step_uv — шаг между отсчётами в uv источника, lod — уровень мипмапа (уменьшение 4K без алиасинга).
const FS: &str = r#"
uniform sampler2D tex;
uniform vec4 uv_rect;
uniform vec2 step_uv;
uniform float lod;
uniform float sigma;
uniform int radius;
uniform float dim;
in vec2 v_uv;
out vec4 o;
void main() {
    vec2 uv = uv_rect.xy + v_uv * uv_rect.zw;
    vec3 c = vec3(0.0);
    float wsum = 0.0;
    for (int i = -radius; i <= radius; i++) {
        float x = float(i);
        float w = exp(-x * x / (2.0 * sigma * sigma));
        c += w * textureLod(tex, uv + step_uv * x, lod).rgb;
        wsum += w;
    }
    o = vec4(c / wsum * dim, 1.0);
}
"#;

pub struct BlurParams {
    /// Участок источника (uv: x, y, w, h), растягиваемый на выход.
    pub uv_rect: [f32; 4],
    pub out_w: i32,
    pub out_h: i32,
    /// Сигма в пикселях выхода.
    pub sigma: f32,
    /// Множитель яркости 0..1.
    pub dim: f32,
}

pub struct Blur {
    prog: glow::Program,
    vao: glow::VertexArray,
    a: Option<Target>,
    b: Option<Target>,
}

impl Blur {
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let header =
                if gl.version().is_embedded { "#version 300 es\nprecision highp float;\n" } else { "#version 330 core\n" };
            let prog = gl.create_program()?;
            let mut shaders = vec![];
            for (ty, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
                let s = gl.create_shader(ty)?;
                gl.shader_source(s, &format!("{header}{src}"));
                gl.compile_shader(s);
                if !gl.get_shader_compile_status(s) {
                    return Err(gl.get_shader_info_log(s));
                }
                gl.attach_shader(prog, s);
                shaders.push(s);
            }
            gl.link_program(prog);
            for s in shaders {
                gl.delete_shader(s);
            }
            if !gl.get_program_link_status(prog) {
                return Err(gl.get_program_info_log(prog));
            }
            let vao = gl.create_vertex_array()?;
            Ok(Self { prog, vao, a: None, b: None })
        }
    }

    /// Размывает `src` в свою текстуру. Возвращает её и `true`, если она пересоздана
    /// (тогда её надо заново зарегистрировать в egui).
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn run(&mut self, gl: &glow::Context, src: &Target, p: &BlurParams) -> Result<(glow::Texture, bool), String> {
        unsafe {
            let mut recreated = false;
            if self.a.as_ref().is_none_or(|t| t.w != p.out_w || t.h != p.out_h) {
                for t in [self.a.take(), self.b.take()].into_iter().flatten() {
                    t.delete(gl);
                }
                self.a = Some(Target::new(gl, p.out_w, p.out_h, false)?);
                self.b = Some(Target::new(gl, p.out_w, p.out_h, false)?);
                recreated = true;
            }
            let (Some(a), Some(b)) = (self.a.as_ref(), self.b.as_ref()) else { unreachable!() };

            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            gl.use_program(Some(self.prog));
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
            let loc = |n: &str| gl.get_uniform_location(self.prog, n);
            gl.uniform_1_i32(loc("tex").as_ref(), 0);
            gl.uniform_1_f32(loc("sigma").as_ref(), p.sigma);
            gl.uniform_1_i32(loc("radius").as_ref(), (p.sigma * 3.0).ceil() as i32);

            // Проход 1: источник (кроп + уменьшение через мипмап) → a, по горизонтали.
            let src_px_x = src.w as f32 * p.uv_rect[2] / p.out_w as f32;
            let src_px_y = src.h as f32 * p.uv_rect[3] / p.out_h as f32;
            let lod = src_px_x.max(src_px_y).max(1.0).log2();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(a.fbo));
            gl.viewport(0, 0, a.w, a.h);
            gl.bind_texture(glow::TEXTURE_2D, Some(src.tex));
            gl.uniform_4_f32_slice(loc("uv_rect").as_ref(), &p.uv_rect);
            gl.uniform_2_f32(loc("step_uv").as_ref(), p.uv_rect[2] / p.out_w as f32, 0.0);
            gl.uniform_1_f32(loc("lod").as_ref(), lod);
            gl.uniform_1_f32(loc("dim").as_ref(), 1.0);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            // Проход 2: a → b, по вертикали, с затемнением.
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(b.fbo));
            gl.bind_texture(glow::TEXTURE_2D, Some(a.tex));
            gl.uniform_4_f32_slice(loc("uv_rect").as_ref(), &[0.0, 0.0, 1.0, 1.0]);
            gl.uniform_2_f32(loc("step_uv").as_ref(), 0.0, 1.0 / p.out_h as f32);
            gl.uniform_1_f32(loc("lod").as_ref(), 0.0);
            gl.uniform_1_f32(loc("dim").as_ref(), p.dim);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(None);
            gl.use_program(None);
            Ok((b.tex, recreated))
        }
    }

    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn delete(&mut self, gl: &glow::Context) {
        unsafe {
            for t in [self.a.take(), self.b.take()].into_iter().flatten() {
                t.delete(gl);
            }
            gl.delete_program(self.prog);
            gl.delete_vertex_array(self.vao);
        }
    }
}
