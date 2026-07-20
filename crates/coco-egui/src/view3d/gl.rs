//! GL resources (live on the glow context, created inside the paint callback).

use std::path::Path;

use eframe::egui;
use eframe::glow::{self, HasContext as _};

use super::crt::{CrtUniforms, GLOW_SIZE, PHOSPHOR_SIZE};
use super::layout::{CLEAR_COLOR, glow_quad_transform};
use super::mesh::{self, MeshData, PropDef, VERTEX_STRIDE_BYTES, bytemuck_cast, screen_quad};
use super::shaders::{
    BLIT_FRAGMENT_SHADER, BLIT_VERTEX_SHADER, FRAGMENT_SHADER, MODE_CRT, MODE_GLOW, MODE_SOLID,
    VERTEX_SHADER,
};

struct GlMesh {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    index_count: i32,
}

/// A texture + framebuffer pair used as an offscreen render target.
struct RenderTarget {
    tex: glow::Texture,
    fbo: glow::Framebuffer,
}

pub(super) struct GlScene {
    program: glow::Program,
    u_view_proj: Option<glow::UniformLocation>,
    u_model: Option<glow::UniformLocation>,
    u_color: Option<glow::UniformLocation>,
    u_mode: Option<glow::UniformLocation>,
    u_crt_a: Option<glow::UniformLocation>,
    u_crt_b: Option<glow::UniformLocation>,
    u_highlight: Option<glow::UniformLocation>,
    /// One entry per prop, same order as the `PropDef` list.
    meshes: Vec<GlMesh>,
    /// Offscreen-pass program (fullscreen triangle, no vertex buffer).
    blit_program: glow::Program,
    u_pass: Option<glow::UniformLocation>,
    u_decay: Option<glow::UniformLocation>,
    /// Core profile requires *a* VAO bound even for buffer-less draws.
    empty_vao: glow::VertexArray,
    /// Phosphor ping-pong pair; `phosphor_prev` indexes last frame's surface.
    phosphor: [RenderTarget; 2],
    phosphor_prev: usize,
    glow_target: RenderTarget,
    /// The additive bezel-glow quad (same geometry as the screen quad).
    glow_mesh: GlMesh,
}

pub(super) enum GlState {
    Uninit,
    /// Shader compile/link failed — logged once, viewport stays cleared.
    Failed,
    Ready(GlScene),
}

/// One drawn prop for one frame — plain data captured by the paint callback.
pub(super) struct Instance {
    pub(super) mesh_idx: usize,
    pub(super) transform: [f32; 16],
    pub(super) color: [f32; 4],
    pub(super) is_screen: bool,
    /// 0.0 = normal, 1.0 = fully white (hover feedback, scaled by
    /// [`super::layout::HIGHLIGHT_MIX`] before it gets here).
    pub(super) highlight: f32,
}

impl GlScene {
    unsafe fn compile_shader(
        gl: &glow::Context,
        kind: u32,
        src: &str,
    ) -> Result<glow::Shader, String> {
        unsafe {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, src);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                return Err(log);
            }
            Ok(shader)
        }
    }

    unsafe fn link_program(gl: &glow::Context, vs_src: &str, fs_src: &str) -> Result<glow::Program, String> {
        unsafe {
            let vs = Self::compile_shader(gl, glow::VERTEX_SHADER, vs_src)?;
            let fs = Self::compile_shader(gl, glow::FRAGMENT_SHADER, fs_src)?;
            let program = gl.create_program()?;
            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            gl.link_program(program);
            gl.detach_shader(program, vs);
            gl.detach_shader(program, fs);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                gl.delete_program(program);
                return Err(log);
            }
            Ok(program)
        }
    }

    unsafe fn upload_mesh(
        gl: &glow::Context,
        program: glow::Program,
        mesh: &MeshData,
    ) -> Result<GlMesh, String> {
        unsafe {
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            let ebo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_cast(&mesh.verts), glow::STATIC_DRAW);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytemuck_cast(&mesh.indices),
                glow::STATIC_DRAW,
            );
            let float_size = std::mem::size_of::<f32>() as i32;
            for (name, components, offset_floats) in [
                ("a_pos", 3, 0),
                ("a_normal", 3, mesh::OFFSET_NORMAL),
                ("a_uv", 2, mesh::OFFSET_UV),
                ("a_color", 3, mesh::OFFSET_COLOR),
            ] {
                if let Some(loc) = gl.get_attrib_location(program, name) {
                    gl.enable_vertex_attrib_array(loc);
                    gl.vertex_attrib_pointer_f32(
                        loc,
                        components,
                        glow::FLOAT,
                        false,
                        VERTEX_STRIDE_BYTES,
                        offset_floats * float_size,
                    );
                }
            }
            gl.bind_vertex_array(None);
            Ok(GlMesh {
                vao,
                vbo,
                ebo,
                index_count: mesh.indices.len() as i32,
            })
        }
    }

    /// A linear-filtered, edge-clamped RGBA8 offscreen target.
    unsafe fn create_target(gl: &glow::Context, size: [i32; 2]) -> Result<RenderTarget, String> {
        unsafe {
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size[0],
                size[1],
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value);
            }
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(format!("offscreen framebuffer incomplete: 0x{status:x}"));
            }
            Ok(RenderTarget { tex, fbo })
        }
    }

    pub(super) fn new(gl: &glow::Context, props: &[PropDef]) -> Result<Self, String> {
        unsafe {
            let program = Self::link_program(gl, VERTEX_SHADER, FRAGMENT_SHADER)?;
            let blit_program = Self::link_program(gl, BLIT_VERTEX_SHADER, BLIT_FRAGMENT_SHADER)?;

            let mut meshes = Vec::with_capacity(props.len());
            for prop in props {
                meshes.push(Self::upload_mesh(gl, program, &prop.mesh)?);
            }
            let glow_mesh = Self::upload_mesh(gl, program, &screen_quad())?;

            let empty_vao = gl.create_vertex_array()?;
            let phosphor = [
                Self::create_target(gl, PHOSPHOR_SIZE)?,
                Self::create_target(gl, PHOSPHOR_SIZE)?,
            ];
            let glow_target = Self::create_target(gl, GLOW_SIZE)?;

            // Fixed sampler units: scene u_tex=0 / u_glow=1, blit u_src=0 /
            // u_prev=1. The blur's texel size never changes either.
            gl.use_program(Some(program));
            for (name, unit) in [("u_tex", 0), ("u_glow", 1)] {
                if let Some(loc) = gl.get_uniform_location(program, name) {
                    gl.uniform_1_i32(Some(&loc), unit);
                }
            }
            gl.use_program(Some(blit_program));
            for (name, unit) in [("u_src", 0), ("u_prev", 1)] {
                if let Some(loc) = gl.get_uniform_location(blit_program, name) {
                    gl.uniform_1_i32(Some(&loc), unit);
                }
            }
            gl.uniform_2_f32(
                gl.get_uniform_location(blit_program, "u_texel").as_ref(),
                1.0 / GLOW_SIZE[0] as f32,
                1.0 / GLOW_SIZE[1] as f32,
            );

            Ok(Self {
                u_view_proj: gl.get_uniform_location(program, "u_view_proj"),
                u_model: gl.get_uniform_location(program, "u_model"),
                u_color: gl.get_uniform_location(program, "u_color"),
                u_mode: gl.get_uniform_location(program, "u_mode"),
                u_crt_a: gl.get_uniform_location(program, "u_crt_a"),
                u_crt_b: gl.get_uniform_location(program, "u_crt_b"),
                u_highlight: gl.get_uniform_location(program, "u_highlight"),
                program,
                meshes,
                u_pass: gl.get_uniform_location(blit_program, "u_pass"),
                u_decay: gl.get_uniform_location(blit_program, "u_decay"),
                blit_program,
                empty_vao,
                phosphor,
                phosphor_prev: 0,
                glow_target,
                glow_mesh,
            })
        }
    }

    /// The two offscreen passes: framebuffer + decayed previous phosphor →
    /// current phosphor surface, then phosphor → small blurred glow target.
    /// Returns the phosphor texture the scene pass should display. Caller
    /// restores framebuffer/viewport/scissor.
    unsafe fn offscreen_passes(
        &mut self,
        gl: &glow::Context,
        fb_texture: glow::Texture,
        decay: f32,
    ) -> glow::Texture {
        unsafe {
            let (prev, cur) = (self.phosphor_prev, 1 - self.phosphor_prev);
            gl.use_program(Some(self.blit_program));
            gl.bind_vertex_array(Some(self.empty_vao));

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.phosphor[cur].fbo));
            gl.viewport(0, 0, PHOSPHOR_SIZE[0], PHOSPHOR_SIZE[1]);
            gl.uniform_1_i32(self.u_pass.as_ref(), 0);
            gl.uniform_1_f32(self.u_decay.as_ref(), decay);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.phosphor[prev].tex));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(fb_texture));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.glow_target.fbo));
            gl.viewport(0, 0, GLOW_SIZE[0], GLOW_SIZE[1]);
            gl.uniform_1_i32(self.u_pass.as_ref(), 1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.phosphor[cur].tex));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            self.phosphor_prev = cur;
            self.phosphor[cur].tex
        }
    }

    /// Bind the CRT-pass phosphor texture on-screen: run the offscreen
    /// passes (scissor off — they own their whole targets), then restore the
    /// on-screen state the callback got. `None` when the tube is unpowered.
    fn resolve_display_tex(
        &mut self,
        gl: &glow::Context,
        fb_texture: Option<glow::Texture>,
        decay: f32,
        restore_fbo: Option<glow::Framebuffer>,
        viewport_px: [i32; 4],
    ) -> Option<glow::Texture> {
        fb_texture.map(|fb| unsafe {
            gl.disable(glow::SCISSOR_TEST);
            let tex = self.offscreen_passes(gl, fb, decay);
            gl.bind_framebuffer(glow::FRAMEBUFFER, restore_fbo);
            gl.viewport(viewport_px[0], viewport_px[1], viewport_px[2], viewport_px[3]);
            gl.enable(glow::SCISSOR_TEST);
            tex
        })
    }

    /// Draw the prop list against the already-cleared, depth-tested viewport.
    fn draw_props(
        &self,
        gl: &glow::Context,
        instances: &[Instance],
        display_tex: Option<glow::Texture>,
    ) {
        unsafe {
            for inst in instances {
                let mesh = &self.meshes[inst.mesh_idx];
                let mode = if inst.is_screen && display_tex.is_some() {
                    gl.bind_texture(glow::TEXTURE_2D, display_tex);
                    MODE_CRT
                } else {
                    MODE_SOLID
                };
                gl.uniform_matrix_4_f32_slice(self.u_model.as_ref(), false, &inst.transform);
                gl.uniform_4_f32_slice(self.u_color.as_ref(), &inst.color);
                gl.uniform_1_i32(self.u_mode.as_ref(), mode);
                gl.uniform_1_f32(self.u_highlight.as_ref(), inst.highlight);
                gl.bind_vertex_array(Some(mesh.vao));
                gl.draw_elements(glow::TRIANGLES, mesh.index_count, glow::UNSIGNED_INT, 0);
            }
            gl.uniform_1_f32(self.u_highlight.as_ref(), 0.0);
        }
    }

    /// Bezel glow: additive, no depth write (it's light, not a surface),
    /// only when there's a powered tube and bloom is on.
    fn draw_bezel_glow(&self, gl: &glow::Context) {
        unsafe {
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.depth_mask(false);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.glow_target.tex));
            gl.uniform_matrix_4_f32_slice(
                self.u_model.as_ref(),
                false,
                &glow_quad_transform().to_cols_array(),
            );
            gl.uniform_1_i32(self.u_mode.as_ref(), MODE_GLOW);
            gl.bind_vertex_array(Some(self.glow_mesh.vao));
            gl.draw_elements(
                glow::TRIANGLES,
                self.glow_mesh.index_count,
                glow::UNSIGNED_INT,
                0,
            );
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
        }
    }

    /// Draw the frame: offscreen CRT passes, then the prop list, then the
    /// additive bezel glow. Runs inside the paint callback: viewport is
    /// already the panel rect and scissor already clips to it, so the final
    /// clear only touches our pixels; egui_glow restores its own state
    /// afterwards (the offscreen passes restore theirs via `restore_fbo` +
    /// `viewport_px` before the scene draws).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint(
        &mut self,
        gl: &glow::Context,
        instances: &[Instance],
        view_proj: &[f32; 16],
        fb_texture: Option<glow::Texture>,
        crt: CrtUniforms,
        restore_fbo: Option<glow::Framebuffer>,
        viewport_px: [i32; 4],
    ) {
        let display_tex =
            self.resolve_display_tex(gl, fb_texture, crt.decay, restore_fbo, viewport_px);

        unsafe {
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.clear_color(
                CLEAR_COLOR[0],
                CLEAR_COLOR[1],
                CLEAR_COLOR[2],
                CLEAR_COLOR[3],
            );
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.u_view_proj.as_ref(), false, view_proj);
            gl.uniform_4_f32_slice(self.u_crt_a.as_ref(), &crt.a);
            gl.uniform_4_f32_slice(self.u_crt_b.as_ref(), &crt.b);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.glow_target.tex));
            gl.active_texture(glow::TEXTURE0);
        }

        self.draw_props(gl, instances, display_tex);

        let bloom_strength = crt.a[3];
        if display_tex.is_some() && bloom_strength > 0.0 {
            self.draw_bezel_glow(gl);
        }
        unsafe {
            gl.bind_vertex_array(None);
        }
    }

    pub(super) fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_program(self.blit_program);
            gl.delete_vertex_array(self.empty_vao);
            for mesh in self.meshes.iter().chain([&self.glow_mesh]) {
                gl.delete_vertex_array(mesh.vao);
                gl.delete_buffer(mesh.vbo);
                gl.delete_buffer(mesh.ebo);
            }
            for target in self.phosphor.iter().chain([&self.glow_target]) {
                gl.delete_framebuffer(target.fbo);
                gl.delete_texture(target.tex);
            }
        }
    }
}

/// Read the just-painted viewport back from the default framebuffer and
/// write it as a PNG (the [`super::SNAPSHOT_ENV`] one-shot). GL's origin is
/// bottom-left, PNG's top-left, so rows are flipped on the way out.
pub(super) fn save_snapshot(gl: &glow::Context, info: &egui::PaintCallbackInfo, path: &Path) {
    let vp = info.viewport_in_pixels();
    let (w, h) = (vp.width_px as usize, vp.height_px as usize);
    const RGBA_BYTES: usize = 4;
    let mut pixels = vec![0u8; w * h * RGBA_BYTES];
    unsafe {
        gl.read_pixels(
            vp.left_px,
            vp.from_bottom_px,
            vp.width_px,
            vp.height_px,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
    }
    let row_bytes = w * RGBA_BYTES;
    let mut flipped = Vec::with_capacity(pixels.len());
    for row in pixels.chunks_exact(row_bytes).rev() {
        flipped.extend_from_slice(row);
    }
    match image::save_buffer(
        path,
        &flipped,
        w as u32,
        h as u32,
        image::ColorType::Rgba8,
    ) {
        Ok(()) => tracing::info!("3D desk snapshot written to {}", path.display()),
        Err(e) => tracing::error!("3D desk snapshot failed: {e}"),
    }
}
