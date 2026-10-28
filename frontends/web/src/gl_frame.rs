// SPDX-License-Identifier: GPL-2.0-or-later
//! Direct WebGL presentation for the RGSS framebuffer.
//!
//! The regular egui texture path requires a `Vec<Color32>` per frame.  RGSS
//! already owns tightly packed bytes, so the web renderer keeps those bytes in
//! the shared frame and lets a small WebGL shader swizzle BGRA and premultiply
//! alpha while sampling the texture.

use super::PresentedFrame;
use eframe::egui_glow::{glow, CallbackFn};
use eframe::glow::HasContext as _;
use std::sync::{Arc, Mutex};

struct Resources {
    program: glow::Program,
    vertex_buffer: glow::Buffer,
    vertex_array: Option<glow::VertexArray>,
    texture: glow::Texture,
    position: u32,
    bgra: glow::UniformLocation,
    width: u32,
    height: u32,
    uploaded_revision: u64,
    uploaded_bgra: Option<bool>,
}

pub fn callback(
    frame: Arc<Mutex<PresentedFrame>>,
    gl: Arc<glow::Context>,
) -> Option<egui::PaintCallback> {
    let resources = Arc::new(Mutex::new(create_resources(&gl)?));
    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
        "FANPLAY direct WebGL frame presentation enabled (raw RGBA/BGRA texture + shader)",
    ));
    let callback = CallbackFn::new(move |_info, painter| {
        paint(&frame, &resources, painter);
    });
    Some(egui::PaintCallback {
        rect: egui::Rect::ZERO,
        callback: Arc::new(callback),
    })
}

fn paint(
    frame: &Arc<Mutex<PresentedFrame>>,
    resources: &Arc<Mutex<Resources>>,
    painter: &eframe::egui_glow::Painter,
) {
    let mut resources = resources.lock().unwrap_or_else(|error| error.into_inner());

    let frame = frame.lock().unwrap_or_else(|error| error.into_inner());
    if frame.revision == 0 || frame.width == 0 || frame.height == 0 {
        return;
    }
    let Some(byte_count) = (frame.width as usize)
        .checked_mul(frame.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
    else {
        return;
    };
    if frame.pixels.len() < byte_count {
        return;
    }

    let gl = painter.gl();
    // SAFETY: eframe invokes glow callbacks while this context is current;
    // every object used below was created by that same context and remains
    // alive through the callback's Arc-owned resources.
    unsafe {
        gl.use_program(Some(resources.program));
        if let Some(vertex_array) = resources.vertex_array {
            gl.bind_vertex_array(Some(vertex_array));
        }
        gl.bind_texture(glow::TEXTURE_2D, Some(resources.texture));

        if resources.width != frame.width || resources.height != frame.height {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                frame.width as i32,
                frame.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&frame.pixels[..byte_count])),
            );
            resources.width = frame.width;
            resources.height = frame.height;
            resources.uploaded_revision = frame.revision;
        } else if resources.uploaded_revision != frame.revision {
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                frame.width as i32,
                frame.height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&frame.pixels[..byte_count])),
            );
            resources.uploaded_revision = frame.revision;
        }

        let is_bgra = frame.format == fanplay_core::PixelFormat::Bgra8;
        if resources.uploaded_bgra != Some(is_bgra) {
            gl.uniform_1_f32(Some(&resources.bgra), if is_bgra { 1.0 } else { 0.0 });
            resources.uploaded_bgra = Some(is_bgra);
        }
        if resources.vertex_array.is_none() {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(resources.vertex_buffer));
            gl.enable_vertex_attrib_array(resources.position);
            gl.vertex_attrib_pointer_f32(resources.position, 2, glow::FLOAT, false, 0, 0);
        }
        gl.draw_arrays(glow::TRIANGLES, 0, 6);
        if resources.vertex_array.is_none() {
            gl.disable_vertex_attrib_array(resources.position);
        }
        gl.bind_texture(glow::TEXTURE_2D, None);
        if resources.vertex_array.is_some() {
            gl.bind_vertex_array(None);
        } else {
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        gl.use_program(None);
    }
}

fn create_resources(gl: &glow::Context) -> Option<Resources> {
    let vertex_shader = compile_shader(
        gl,
        glow::VERTEX_SHADER,
        r#"
            attribute vec2 a_position;
            varying vec2 v_uv;
            void main() {
                v_uv = vec2((a_position.x + 1.0) * 0.5,
                             (1.0 - a_position.y) * 0.5);
                gl_Position = vec4(a_position, 0.0, 1.0);
            }
        "#,
    )?;
    let fragment_shader = compile_shader(
        gl,
        glow::FRAGMENT_SHADER,
        r#"
            precision mediump float;
            uniform sampler2D u_frame;
            uniform float u_bgra;
            varying vec2 v_uv;
            void main() {
                vec4 color = texture2D(u_frame, v_uv);
                if (u_bgra > 0.5) {
                    color = color.bgra;
                }
                // egui's glow renderer uses premultiplied-alpha blending.
                gl_FragColor = vec4(color.rgb * color.a, color.a);
            }
        "#,
    )?;
    // SAFETY: shader handles and the context are valid for the duration of
    // resource creation, and the context is the one supplied by eframe.
    let program = unsafe { gl.create_program().ok()? };
    // SAFETY: the program and both shader handles were created from `gl` and
    // are only used with that context before being released.
    unsafe {
        gl.attach_shader(program, vertex_shader);
        gl.attach_shader(program, fragment_shader);
        gl.bind_attrib_location(program, 0, "a_position");
        gl.link_program(program);
        gl.delete_shader(vertex_shader);
        gl.delete_shader(fragment_shader);
    }
    if unsafe { !gl.get_program_link_status(program) } {
        web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&unsafe {
            gl.get_program_info_log(program)
        }));
        // SAFETY: `program` was just created from this context and has not
        // been deleted on the failed-link path.
        unsafe { gl.delete_program(program) };
        return None;
    }

    // SAFETY: object creation is performed on the eframe-owned current
    // context; the returned handles are checked before use.
    let vertex_buffer = unsafe { gl.create_buffer().ok()? };
    let vertex_array = if supports_vertex_arrays(gl) {
        // SAFETY: the context supports VAOs (WebGL2 or the WebGL1 extension),
        // so creating one cannot hit glow's unsupported-VAO panic path.
        unsafe { gl.create_vertex_array().ok() }
    } else {
        None
    };
    let texture = unsafe { gl.create_texture().ok()? };
    // SAFETY: `program` is a successfully linked program owned by `gl`.
    let position = unsafe { gl.get_attrib_location(program, "a_position")? };
    let sampler = unsafe { gl.get_uniform_location(program, "u_frame")? };
    let bgra = unsafe { gl.get_uniform_location(program, "u_bgra")? };
    let vertices: [f32; 12] = [
        -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0,
    ];
    // SAFETY: the buffer and texture handles belong to `gl`; the vertex data
    // is a fixed-size POD slice valid for the duration of `buffer_data`.
    unsafe {
        if let Some(vertex_array) = vertex_array {
            gl.bind_vertex_array(Some(vertex_array));
        }
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertex_buffer));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(&vertices),
            glow::STATIC_DRAW,
        );
        if vertex_array.is_some() {
            gl.enable_vertex_attrib_array(position);
            gl.vertex_attrib_pointer_f32(position, 2, glow::FLOAT, false, 0, 0);
        }
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
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
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        if vertex_array.is_some() {
            gl.bind_vertex_array(None);
        }
    }
    // The sampler never changes; set it once instead of issuing the uniform
    // call for every frame.
    unsafe {
        gl.use_program(Some(program));
        gl.uniform_1_i32(Some(&sampler), 0);
        gl.use_program(None);
    }
    Some(Resources {
        program,
        vertex_buffer,
        vertex_array,
        texture,
        position,
        bgra,
        width: 0,
        height: 0,
        uploaded_revision: 0,
        uploaded_bgra: None,
    })
}

fn supports_vertex_arrays(gl: &glow::Context) -> bool {
    let version = gl.version();
    version.major >= 3
        || gl
            .supported_extensions()
            .contains("OES_vertex_array_object")
        || gl
            .supported_extensions()
            .contains("GL_OES_vertex_array_object")
}

fn compile_shader(gl: &glow::Context, kind: u32, source: &str) -> Option<glow::Shader> {
    // SAFETY: shader creation and compilation are confined to the eframe
    // context supplied by the caller.
    let shader = unsafe { gl.create_shader(kind).ok()? };
    unsafe {
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
    }
    if unsafe { !gl.get_shader_compile_status(shader) } {
        web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&unsafe {
            gl.get_shader_info_log(shader)
        }));
        // SAFETY: `shader` was created by this context and is no longer used
        // after the failed compile.
        unsafe { gl.delete_shader(shader) };
        None
    } else {
        Some(shader)
    }
}
