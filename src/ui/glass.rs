//! Frosted glass under a vibrant header: what egui has already drawn beneath
//! a rectangle is blurred in place, as macOS blurs content scrolling under a
//! toolbar. egui has no blur of its own, so this is an OpenGL paint callback
//! that runs just before the header is drawn over it.
//!
//! The region is copied twice: sharp at full size, and at half of it,
//! where separable Gaussian passes blur it lightly, as macOS does: the
//! letters under a toolbar stay faintly legible. The last pass
//! writes it back, blurred at the top and easing to sharp over the bottom
//! `fade`, so the messages do not stop at a line. Callers keep it within
//! the header, as macOS does.

use std::sync::{Arc, Mutex};

use eframe::egui_glow;
use eframe::glow::{self, HasContext};

/// How much smaller the blurred copy is than the region.
const DOWNSCALE: i32 = 2;
/// Horizontal and vertical passes over the small copy. Each widens the blur.
const PASSES: usize = 1;
/// How much more saturated the blur is than what it covers, as glass that
/// gathers the colours behind it.
const SATURATION: f32 = 1.3;

/// Diffuse lavender, teal and rose light behind Aurora's glass panes. One
/// small mesh, with no textures or repaint timer, also supplies the fallback.
pub fn ambient(ui: &egui::Ui, palette: &crate::theme::Palette) {
    if !palette.ambient {
        return;
    }
    let rect = ui.max_rect();
    let base = if crate::backdrop::active() {
        palette.window.gamma_multiply(0.55)
    } else {
        palette.window
    };
    use crate::theme::AuroraVariant;
    let glows: [(egui::Color32, (f32, f32), f32); 3] = match palette.aurora_variant {
        AuroraVariant::Default => [
            (palette.accent, (0.12, 0.05), 0.42),
            (egui::Color32::from_rgb(83, 179, 186), (0.9, 0.4), 0.32),
            (egui::Color32::from_rgb(167, 118, 178), (0.4, 1.0), 0.22),
        ],
        AuroraVariant::Green => [
            (palette.accent, (0.08, 0.04), 0.46),
            (egui::Color32::from_rgb(84, 212, 154), (0.92, 0.38), 0.34),
            (egui::Color32::from_rgb(184, 233, 134), (0.42, 1.0), 0.2),
        ],
        AuroraVariant::Alternative => [
            (palette.accent, (0.16, 0.06), 0.42),
            (egui::Color32::from_rgb(91, 200, 232), (0.88, 0.46), 0.34),
            (egui::Color32::from_rgb(140, 155, 232), (0.35, 0.96), 0.24),
        ],
    };
    const STEPS: u32 = 16;
    let mut mesh = egui::Mesh::default();
    for y in 0..=STEPS {
        let down = y as f32 / STEPS as f32;
        for x in 0..=STEPS {
            let across = x as f32 / STEPS as f32;
            let glow = |cx: f32, cy: f32, strength: f32| {
                (-6.0 * ((across - cx).powi(2) + (down - cy).powi(2))).exp() * strength
            };
            let mut color = base;
            for (glow_color, (cx, cy), strength) in glows {
                color = color.lerp_to_gamma(glow_color, glow(cx, cy, strength));
            }
            mesh.colored_vertex(
                rect.min + egui::vec2(across * rect.width(), down * rect.height()),
                color,
            );
        }
    }
    for y in 0..STEPS {
        for x in 0..STEPS {
            let i = y * (STEPS + 1) + x;
            mesh.add_triangle(i, i + 1, i + STEPS + 2);
            mesh.add_triangle(i, i + STEPS + 2, i + STEPS + 1);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// A paint callback that frosts `rect`, the bottom `fade` points easing from
/// blurred to sharp.
pub fn shape(rect: egui::Rect, fade: f32) -> egui::Shape {
    egui::Shape::Callback(egui::PaintCallback {
        rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
            let fade = (fade * info.pixels_per_point).round() as i32;
            let gl = painter.gl();
            let mut glass = GLASS.lock().unwrap_or_else(|p| p.into_inner());
            // A window made anew has a new context; the old one took its
            // objects with it. A context that cannot frost is asked once.
            if is_stale(glass.as_ref().map(|(made_for, _)| made_for), gl) {
                // SAFETY: called on the painting thread with its context current.
                *glass = Some((Arc::clone(gl), unsafe { Glass::new(gl) }));
            }
            if let Some((_, Some(glass))) = glass.as_mut() {
                // SAFETY: as above.
                unsafe { glass.frost(gl, &info, fade) };
            }
        })),
    })
}

/// Whether objects made for `made_for` cannot be used with `context`. The
/// `Arc` is compared, not its address: holding it keeps a closed window's
/// context from being replaced at the same address, which reused its dead
/// framebuffers on a window reopened from the tray (#638).
fn is_stale<T>(made_for: Option<&Arc<T>>, context: &Arc<T>) -> bool {
    made_for.is_none_or(|made_for| !Arc::ptr_eq(made_for, context))
}

/// The objects for the current context, kept with it, or nothing where it
/// cannot run them.
static GLASS: Mutex<Option<(Arc<glow::Context>, Option<Glass>)>> = Mutex::new(None);

/// One full-size and two small render targets, and the two programs.
struct Glass {
    blur: glow::Program,
    compose: glow::Program,
    vao: glow::VertexArray,
    sharp: Target,
    small: [Target; 2],
}

struct Target {
    texture: glow::Texture,
    framebuffer: glow::Framebuffer,
    size: (i32, i32),
}

const VERTEX: &str = r"
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

/// Nine taps folded into five linear samples.
const BLUR: &str = r"
uniform sampler2D u_texture;
uniform vec2 u_step;
in vec2 v_uv;
out vec4 o_color;
void main() {
    vec2 near = u_step * 1.3846153846;
    vec2 far = u_step * 3.2307692308;
    o_color = texture(u_texture, v_uv) * 0.2270270270
        + (texture(u_texture, v_uv + near) + texture(u_texture, v_uv - near)) * 0.3162162162
        + (texture(u_texture, v_uv + far) + texture(u_texture, v_uv - far)) * 0.0702702703;
}
";

const COMPOSE: &str = r"
uniform sampler2D u_sharp;
uniform sampler2D u_blur;
uniform float u_fade;
uniform float u_saturation;
in vec2 v_uv;
out vec4 o_color;
void main() {
    vec4 blur = texture(u_blur, v_uv);
    float luma = dot(blur.rgb, vec3(0.2126, 0.7152, 0.0722));
    // Colours are premultiplied, so they stay within their alpha.
    blur.rgb = clamp(mix(vec3(luma), blur.rgb, u_saturation), 0.0, blur.a);
    float frost = u_fade > 0.0 ? smoothstep(0.0, u_fade, v_uv.y) : 1.0;
    o_color = mix(texture(u_sharp, v_uv), blur, frost);
}
";

impl Glass {
    /// The programs and an empty vertex array, or nothing where the context
    /// cannot run them; the header then goes without the blur.
    unsafe fn new(gl: &glow::Context) -> Option<Self> {
        let version = egui_glow::ShaderVersion::get(gl);
        if !version.is_new_shader_interface() {
            log::info!("frosted header unavailable: {version:?} shaders");
            return None;
        }
        let header = version.version_declaration();
        unsafe {
            let blur = program(gl, header, BLUR)?;
            let compose = program(gl, header, COMPOSE)?;
            let vao = gl.create_vertex_array().ok()?;
            Some(Self {
                blur,
                compose,
                vao,
                sharp: Target::new(gl)?,
                small: [Target::new(gl)?, Target::new(gl)?],
            })
        }
    }

    /// Blurs the region of the bound framebuffer under `info`'s rectangle.
    unsafe fn frost(&mut self, gl: &glow::Context, info: &egui::PaintCallbackInfo, fade: i32) {
        let region = info.viewport_in_pixels();
        let clip = info.clip_rect_in_pixels();
        let (x, y, w, h) = (
            region.left_px,
            region.from_bottom_px,
            region.width_px,
            region.height_px,
        );
        if w <= 0 || h <= 0 {
            return;
        }
        let small = ((w / DOWNSCALE).max(1), (h / DOWNSCALE).max(1));
        unsafe {
            let screen = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
            self.sharp.resize(gl, (w, h));
            for target in &mut self.small {
                target.resize(gl, small);
            }
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            // The sharp copy, and the small one to blur.
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, screen);
            for (target, filter) in [(&self.sharp, glow::NEAREST), (&self.small[0], glow::LINEAR)] {
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(target.framebuffer));
                gl.blit_framebuffer(
                    x,
                    y,
                    x + w,
                    y + h,
                    0,
                    0,
                    target.size.0,
                    target.size.1,
                    glow::COLOR_BUFFER_BIT,
                    filter,
                );
            }
            gl.bind_vertex_array(Some(self.vao));
            gl.use_program(Some(self.blur));
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_1_i32(gl.get_uniform_location(self.blur, "u_texture").as_ref(), 0);
            let step = gl.get_uniform_location(self.blur, "u_step");
            gl.viewport(0, 0, small.0, small.1);
            for _ in 0..PASSES {
                for (from, to, direction) in [(0, 1, (1.0, 0.0)), (1, 0, (0.0, 1.0))] {
                    gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.small[to].framebuffer));
                    gl.bind_texture(glow::TEXTURE_2D, Some(self.small[from].texture));
                    gl.uniform_2_f32(
                        step.as_ref(),
                        direction.0 / small.0 as f32,
                        direction.1 / small.1 as f32,
                    );
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
            }
            // Back onto the screen, within the clip.
            gl.bind_framebuffer(glow::FRAMEBUFFER, screen);
            gl.viewport(x, y, w, h);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(
                clip.left_px,
                clip.from_bottom_px,
                clip.width_px,
                clip.height_px,
            );
            gl.use_program(Some(self.compose));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.sharp.texture));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.small[0].texture));
            let uniform = |name| gl.get_uniform_location(self.compose, name);
            gl.uniform_1_i32(uniform("u_sharp").as_ref(), 0);
            gl.uniform_1_i32(uniform("u_blur").as_ref(), 1);
            gl.uniform_1_f32(uniform("u_fade").as_ref(), fade as f32 / h as f32);
            gl.uniform_1_f32(uniform("u_saturation").as_ref(), SATURATION);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, None);
            // egui restores the rest of its state after a callback.
        }
    }
}

impl Target {
    unsafe fn new(gl: &glow::Context) -> Option<Self> {
        unsafe {
            let texture = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for (parameter, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, value as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            let framebuffer = gl.create_framebuffer().ok()?;
            Some(Self {
                texture,
                framebuffer,
                size: (0, 0),
            })
        }
    }

    /// Gives the texture `size` pixels, when it has another size.
    unsafe fn resize(&mut self, gl: &glow::Context, size: (i32, i32)) {
        if self.size == size {
            return;
        }
        self.size = size;
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size.0,
                size.1,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.framebuffer));
            gl.framebuffer_texture_2d(
                glow::DRAW_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(self.texture),
                0,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }
}

/// Compiles and links the shared vertex shader with `fragment`.
unsafe fn program(gl: &glow::Context, header: &str, fragment: &str) -> Option<glow::Program> {
    unsafe {
        let program = gl.create_program().ok()?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = gl.create_shader(kind).ok()?;
            gl.shader_source(shader, &format!("{header}{source}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                log::warn!("frosted header shader: {}", gl.get_shader_info_log(shader));
                return None;
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            log::warn!(
                "frosted header program: {}",
                gl.get_program_info_log(program)
            );
            return None;
        }
        Some(program)
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;

    #[test]
    fn ambient_lighting_is_bounded_and_only_paints_for_aurora() {
        for palette in [
            crate::theme::Palette::messages(),
            crate::theme::Palette::light(),
            crate::theme::Palette::aurora(true),
            crate::theme::Palette::aurora(false),
            crate::theme::Palette::aurora_green(true),
            crate::theme::Palette::aurora_green(false),
            crate::theme::Palette::aurora_alternative(true),
            crate::theme::Palette::aurora_alternative(false),
        ] {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| ambient(ui, &palette));
            output.textures_delta.clear();
            let meshes: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) => Some(mesh),
                    _ => None,
                })
                .collect();
            assert_eq!(meshes.len(), usize::from(palette.ambient));
            if let Some(mesh) = meshes.first() {
                assert_eq!(mesh.vertices.len(), 17 * 17);
                assert_eq!(mesh.indices.len(), 16 * 16 * 6);
                assert!(mesh.vertices.iter().all(|v| v.pos.is_finite()));
                assert!(
                    mesh.indices
                        .iter()
                        .all(|i| (*i as usize) < mesh.vertices.len())
                );
                assert_ne!(mesh.vertices[0].color, mesh.vertices[17 * 17 - 1].color);
            }
        }
    }

    #[test]
    fn objects_stay_with_the_context_they_were_made_for() {
        let first = Arc::new(1);
        assert!(is_stale(None, &first));
        assert!(!is_stale(Some(&first), &first));
        assert!(!is_stale(Some(&first), &Arc::clone(&first)));
        // A new context, equal or not, is another one.
        assert!(is_stale(Some(&first), &Arc::new(1)));
    }
}
