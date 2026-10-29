//! Drawing frames with OpenGL: each frame is one single-channel texture
//! (1 byte per pixel) shown by a tiny shader that also inverts and, when
//! zoomed out, averages the texels under each screen pixel so faint detail
//! is not skipped. All GL calls happen inside egui's paint callback, where
//! the context is current.

use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{self, Rect};
use eframe::egui_glow::{self, ShaderVersion};
use eframe::glow::{self, HasContext as _};

/// Most texture taps per axis when zoomed out (16 bilinear samples ≈ 8×8 texels).
const MAX_TAPS: f32 = 4.0;

struct Upload {
    width: usize,
    height: usize,
    gray: Vec<u8>,
}

struct Program {
    program: glow::Program,
    vbo: glow::Buffer,
    vao: Option<glow::VertexArray>,
    u_rect: Option<glow::UniformLocation>,
    u_invert: Option<glow::UniformLocation>,
    u_foot: Option<glow::UniformLocation>,
    u_taps: Option<glow::UniformLocation>,
    u_tex: Option<glow::UniformLocation>,
    /// `RED`/`R8` on GL 3+ and ES 3, `LUMINANCE` on older contexts.
    format: u32,
    internal: i32,
}

#[derive(Default)]
struct State {
    program: Option<Program>,
    program_failed: bool,
    textures: Vec<Option<glow::Texture>>,
    pending: Vec<Option<Upload>>,
    trash: Vec<glow::Texture>,
    error: Option<String>,
}

/// Where and how to draw one frame.
#[derive(Clone, Copy)]
pub struct Draw {
    pub index: usize,
    /// The image's full extent on screen, in points (may exceed the view).
    pub image: Rect,
    /// Image size in pixels.
    pub size: [usize; 2],
    pub invert: bool,
}

#[derive(Clone, Default)]
pub struct Gpu {
    state: Arc<Mutex<State>>,
}

impl Gpu {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Replaces the frames. The bytes are uploaded on the next paint and then freed.
    pub fn set_frames(&self, frames: Vec<(usize, usize, Vec<u8>)>) {
        let mut s = self.lock();
        let old: Vec<_> = s.textures.drain(..).flatten().collect();
        s.trash.extend(old);
        s.textures = frames.iter().map(|_| None).collect();
        s.pending = frames
            .into_iter()
            .map(|(width, height, gray)| {
                Some(Upload {
                    width,
                    height,
                    gray,
                })
            })
            .collect();
    }

    pub fn take_error(&self) -> Option<String> {
        self.lock().error.take()
    }

    /// A paint callback drawing frame `d.index` clipped to `view`.
    pub fn paint(&self, ctx: &egui::Context, view: Rect, d: Draw) -> egui::PaintCallback {
        let gpu = self.clone();
        let ctx = ctx.clone();
        egui::PaintCallback {
            rect: view,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                let mut s = gpu.lock();
                // SAFETY: called by egui_glow with its context current; all
                // objects used here were created on that same context.
                if let Err(e) = unsafe { s.draw(painter.gl(), &info, d) } {
                    s.error = Some(e);
                    ctx.request_repaint();
                }
            })),
        }
    }

    /// Frees all GL objects; call on exit with the context current.
    pub fn destroy(&self, gl: &glow::Context) {
        let mut s = self.lock();
        let textures: Vec<_> = s.textures.drain(..).flatten().collect();
        s.trash.extend(textures);
        s.pending.clear();
        // SAFETY: eframe calls `on_exit` with the GL context current.
        unsafe {
            s.empty_trash(gl);
            if let Some(p) = s.program.take() {
                gl.delete_program(p.program);
                gl.delete_buffer(p.vbo);
                if let Some(vao) = p.vao {
                    gl.delete_vertex_array(vao);
                }
            }
        }
    }
}

impl State {
    unsafe fn empty_trash(&mut self, gl: &glow::Context) {
        for t in self.trash.drain(..) {
            unsafe { gl.delete_texture(t) };
        }
    }

    unsafe fn draw(
        &mut self,
        gl: &glow::Context,
        info: &egui::PaintCallbackInfo,
        d: Draw,
    ) -> Result<(), String> {
        unsafe {
            self.empty_trash(gl);
            if self.program.is_none() && !self.program_failed {
                match Program::new(gl) {
                    Ok(p) => self.program = Some(p),
                    Err(e) => {
                        self.program_failed = true;
                        return Err(format!("Could not set up image display: {e}"));
                    }
                }
            }
            self.upload(gl)?;
            let (Some(p), Some(&Some(texture))) = (&self.program, self.textures.get(d.index))
            else {
                return Ok(());
            };

            let vp = info.viewport;
            let ndc_x = |x: f32| (x - vp.min.x) / vp.width() * 2.0 - 1.0;
            let ndc_y = |y: f32| 1.0 - (y - vp.min.y) / vp.height() * 2.0;
            // Physical screen pixels per image pixel.
            let k = d.image.width() / d.size[0] as f32 * info.pixels_per_point;
            let (filter, taps, foot) = if k >= 1.0 {
                (glow::NEAREST, 1.0, 0.0)
            } else {
                let foot = 1.0 / k; // texels under one screen pixel
                (glow::LINEAR, (foot / 2.0).ceil().clamp(1.0, MAX_TAPS), foot)
            };

            gl.use_program(Some(p.program));
            gl.uniform_4_f32(
                p.u_rect.as_ref(),
                ndc_x(d.image.min.x),
                ndc_y(d.image.min.y),
                ndc_x(d.image.max.x),
                ndc_y(d.image.max.y),
            );
            gl.uniform_1_f32(p.u_invert.as_ref(), if d.invert { 1.0 } else { 0.0 });
            gl.uniform_1_f32(p.u_taps.as_ref(), taps);
            gl.uniform_2_f32(
                p.u_foot.as_ref(),
                foot / d.size[0] as f32,
                foot / d.size[1] as f32,
            );
            gl.uniform_1_i32(p.u_tex.as_ref(), 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, filter as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter as i32);

            if let Some(vao) = p.vao {
                gl.bind_vertex_array(Some(vao));
            } else {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(p.vbo));
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 0, 0);
            }
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            if p.vao.is_some() {
                gl.bind_vertex_array(None);
            } else {
                gl.disable_vertex_attrib_array(0);
                gl.bind_buffer(glow::ARRAY_BUFFER, None);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            Ok(())
        }
    }

    unsafe fn upload(&mut self, gl: &glow::Context) -> Result<(), String> {
        let Some(p) = &self.program else {
            return Ok(());
        };
        let (format, internal) = (p.format, p.internal);
        let max = unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) }.max(0) as usize;
        for (i, slot) in self.pending.iter_mut().enumerate() {
            let Some(up) = slot.take() else { continue };
            if up.width > max || up.height > max {
                return Err(format!(
                    "Frame {} is {}×{} pixels; this graphics card shows at most {max}×{max}.",
                    i + 1,
                    up.width,
                    up.height
                ));
            }
            unsafe {
                let t = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_S,
                    glow::CLAMP_TO_EDGE as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_T,
                    glow::CLAMP_TO_EDGE as i32,
                );
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                // Default min filter expects mipmaps; we never make any.
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    internal,
                    up.width as i32,
                    up.height as i32,
                    0,
                    format,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&up.gray)),
                );
                self.textures[i] = Some(t);
            }
            // `up.gray` is dropped here: the GPU holds the only copy.
        }
        Ok(())
    }
}

impl Program {
    unsafe fn new(gl: &glow::Context) -> Result<Program, String> {
        let version = ShaderVersion::get(gl);
        let modern = version.is_new_shader_interface();
        let header = format!(
            "{}{}",
            version.version_declaration(),
            if version.is_embedded() {
                "precision mediump float;\n"
            } else {
                ""
            }
        );
        let (vin, vout, fin) = if modern {
            ("in", "out", "in")
        } else {
            ("attribute", "varying", "varying")
        };
        let vertex = format!(
            "{header}{vin} vec2 a_pos;\n{vout} vec2 v_uv;\nuniform vec4 u_rect;\n\
             void main() {{\n  v_uv = a_pos;\n  \
             gl_Position = vec4(mix(u_rect.xy, u_rect.zw, a_pos), 0.0, 1.0);\n}}\n"
        );
        let (out_decl, sample, out) = if modern {
            ("out vec4 out_color;\n", "texture", "out_color")
        } else {
            ("", "texture2D", "gl_FragColor")
        };
        let fragment = format!(
            "{header}{fin} vec2 v_uv;\n{out_decl}uniform sampler2D u_tex;\n\
             uniform float u_invert;\nuniform float u_taps;\nuniform vec2 u_foot;\n\
             void main() {{\n  float sum = 0.0;\n  \
             for (int i = 0; i < 4; i++) {{\n    if (float(i) >= u_taps) break;\n    \
             for (int j = 0; j < 4; j++) {{\n      if (float(j) >= u_taps) break;\n      \
             vec2 off = (vec2(float(i), float(j)) + 0.5) / u_taps - 0.5;\n      \
             sum += {sample}(u_tex, v_uv + off * u_foot).r;\n    }}\n  }}\n  \
             float g = sum / (u_taps * u_taps);\n  g = mix(g, 1.0 - g, u_invert);\n  \
             {out} = vec4(g, g, g, 1.0);\n}}\n"
        );

        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, src) in [
                (glow::VERTEX_SHADER, vertex),
                (glow::FRAGMENT_SHADER, fragment),
            ] {
                let s = gl.create_shader(kind)?;
                gl.shader_source(s, &src);
                gl.compile_shader(s);
                if !gl.get_shader_compile_status(s) {
                    let log = gl.get_shader_info_log(s);
                    gl.delete_shader(s);
                    gl.delete_program(program);
                    return Err(log);
                }
                gl.attach_shader(program, s);
                shaders.push(s);
            }
            gl.bind_attrib_location(program, 0, "a_pos");
            gl.link_program(program);
            for s in shaders {
                gl.detach_shader(program, s);
                gl.delete_shader(s);
            }
            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                gl.delete_program(program);
                return Err(log);
            }

            let corners: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            let bytes: Vec<u8> = corners.iter().flat_map(|v| v.to_ne_bytes()).collect();
            let vbo = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
            // GLSL 1.40+ / ES 3 contexts have vertex arrays (and core
            // profiles, as on macOS, require one).
            let vao = if modern {
                let vao = gl.create_vertex_array()?;
                gl.bind_vertex_array(Some(vao));
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 0, 0);
                gl.bind_vertex_array(None);
                Some(vao)
            } else {
                None
            };
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            let (format, internal) = if modern {
                (glow::RED, glow::R8 as i32)
            } else {
                (glow::LUMINANCE, glow::LUMINANCE as i32)
            };
            Ok(Program {
                u_rect: gl.get_uniform_location(program, "u_rect"),
                u_invert: gl.get_uniform_location(program, "u_invert"),
                u_foot: gl.get_uniform_location(program, "u_foot"),
                u_taps: gl.get_uniform_location(program, "u_taps"),
                u_tex: gl.get_uniform_location(program, "u_tex"),
                program,
                vbo,
                vao,
                format,
                internal,
            })
        }
    }
}
