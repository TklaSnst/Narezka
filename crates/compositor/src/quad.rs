//! Шейдер слоя холста: участок текстуры в прямоугольник холста, скругление углов, затемнение.
//! Смешивание premultiplied (`ONE, ONE_MINUS_SRC_ALPHA`) — растры `sc-raster` и кадры видео.

use glow::HasContext;

use crate::gl::program;

const VS: &str = r#"
uniform vec4 dest;
uniform vec4 uv_rect;
out vec2 v_local;
out vec2 v_uv;
void main() {
    vec2 p = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    v_local = p;
    v_uv = uv_rect.xy + p * uv_rect.zw;
    // Строка 0 текстуры холста — верх кадра (как у кадров mpv и растров), поэтому без переворота.
    gl_Position = vec4((dest.xy + p * dest.zw) * 2.0 - 1.0, 0.0, 1.0);
}
"#;

// Покрытие скруглённым прямоугольником — та же формула, что `sc_core::rounded_rect_coverage`
// (в пикселях выхода, сглаживание на один пиксель).
const FS: &str = r#"
uniform sampler2D tex;
uniform vec2 size_px;
uniform float radius_px;
uniform float dim;
in vec2 v_local;
in vec2 v_uv;
out vec4 o;
void main() {
    // Уменьшение: четыре отсчёта по пикселю выхода с мипмапа на уровень детальнее — ближе к lanczos
    // рендера, чем одна трилинейная выборка (та заметно мылит текст).
    vec2 dx = dFdx(v_uv) * 0.25;
    vec2 dy = dFdy(v_uv) * 0.25;
    vec4 c = 0.25 * (texture(tex, v_uv - dx - dy, -1.0) + texture(tex, v_uv + dx - dy, -1.0)
                   + texture(tex, v_uv - dx + dy, -1.0) + texture(tex, v_uv + dx + dy, -1.0));
    float cov = 1.0;
    if (radius_px > 0.0) {
        float r = min(radius_px, min(size_px.x, size_px.y) * 0.5);
        vec2 q = abs(v_local * size_px - size_px * 0.5) - (size_px * 0.5 - r);
        float dist = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
        cov = clamp(0.5 - dist, 0.0, 1.0);
    }
    o = vec4(c.rgb * dim, c.a) * cov;
}
"#;

/// Что и куда рисовать. Прямоугольники — в долях (x, y, w, h).
pub struct QuadParams {
    pub dest: [f32; 4],
    pub uv: [f32; 4],
    /// Радиус скругления в пикселях выхода (0 — без скругления).
    pub radius_px: f32,
    /// Множитель яркости.
    pub dim: f32,
}

pub struct Quad {
    prog: glow::Program,
    vao: glow::VertexArray,
}

impl Quad {
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe { Ok(Self { prog: program(gl, VS, FS)?, vao: gl.create_vertex_array()? }) }
    }

    /// Начать серию рисования в `fbo` размером `w`×`h`.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn begin(&self, gl: &glow::Context, fbo: glow::Framebuffer, w: i32, h: i32) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.viewport(0, 0, w, h);
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.use_program(Some(self.prog));
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_1_i32(gl.get_uniform_location(self.prog, "tex").as_ref(), 0);
        }
    }

    /// # Safety
    /// GL-контекст должен быть текущим, после [`Self::begin`].
    pub unsafe fn draw(&self, gl: &glow::Context, tex: glow::Texture, out_w: i32, out_h: i32, p: &QuadParams) {
        unsafe {
            let loc = |n: &str| gl.get_uniform_location(self.prog, n);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.uniform_4_f32_slice(loc("dest").as_ref(), &p.dest);
            gl.uniform_4_f32_slice(loc("uv_rect").as_ref(), &p.uv);
            gl.uniform_2_f32(loc("size_px").as_ref(), p.dest[2] * out_w as f32, p.dest[3] * out_h as f32);
            gl.uniform_1_f32(loc("radius_px").as_ref(), p.radius_px);
            gl.uniform_1_f32(loc("dim").as_ref(), p.dim);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }

    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn end(&self, gl: &glow::Context) {
        unsafe {
            gl.disable(glow::BLEND);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }

    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.prog);
            gl.delete_vertex_array(self.vao);
        }
    }
}
