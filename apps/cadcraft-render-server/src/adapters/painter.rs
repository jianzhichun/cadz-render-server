//! One frame's paint into the offscreen texture: the calls egui-wgpu's own window painter makes,
//! in the same order (`egui-wgpu/src/winit.rs`, `Painter::paint_and_update_textures`), with our
//! texture where the window's surface texture was, and the same stencil buffer, cleared every
//! frame. The drawing area is a paint callback (`crates/ui-egui/src/gpu.rs`, `CanvasCallback`), so
//! the drawing is drawn by this same pass.

use egui_wgpu::wgpu;

/// The texture frames are drawn into and read back from, and the pass's stencil.
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    stencil: Option<wgpu::TextureView>,
    size: [u32; 2],
}

#[derive(Default)]
pub struct OffscreenPainter {
    target: Option<Target>,
}

impl OffscreenPainter {
    /// The texture last painted, if any.
    pub fn texture(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref().map(|t| &t.texture)
    }

    /// Paint `prims` at `size` device pixels, applying `deltas` (the textures egui set and freed
    /// since the last paint) around it. `clear`: the background in gamma space. `stencil`: the
    /// stencil format the renderer was made with.
    pub fn paint(
        &mut self,
        rs: &egui_wgpu::RenderState,
        size: [u32; 2],
        pixels_per_point: f32,
        clear: [f32; 4],
        stencil: Option<wgpu::TextureFormat>,
        prims: &[egui::ClippedPrimitive],
        deltas: &mut egui::TexturesDelta,
    ) {
        self.ensure_target(&rs.device, rs.target_format, stencil, size);
        let Some(target) = self.target.as_ref() else { return };
        let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render-server frame") });
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: target.size, pixels_per_point };
        // Textures first, then the buffers, which run the paint callbacks' prepare and hand back
        // their command buffers.
        let user_buffers = {
            let mut renderer = rs.renderer.write();
            for (id, image_deltas) in deltas.set.drain() {
                for image_delta in image_deltas {
                    renderer.update_texture(&rs.device, &rs.queue, id, &image_delta);
                }
            }
            renderer.update_buffers(&rs.device, &rs.queue, &mut encoder, prims, &screen)
        };
        {
            let renderer = rs.renderer.read();
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render-server egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0] as f64, g: clear[1] as f64, b: clear[2] as f64, a: clear[3] as f64 }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: target.stencil.as_ref().map(|view| wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Discard }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            renderer.render(&mut pass.forget_lifetime(), prims, &screen);
        }
        rs.queue.submit(user_buffers.into_iter().chain([encoder.finish()]));
        // Freed after the submit that may still use them.
        let mut renderer = rs.renderer.write();
        for id in deltas.free.drain() {
            renderer.free_texture(&id);
        }
    }

    /// A texture of `size` (re)made when the size changes. `COPY_SRC`: it is read back.
    fn ensure_target(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat, stencil: Option<wgpu::TextureFormat>, size: [u32; 2]) {
        if self.target.as_ref().is_none_or(|t| t.size != size) {
            // The old one goes before the new one is made.
            self.target = None;
            let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("render-server frame"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let stencil = stencil.map(|format| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("render-server stencil"),
                        size: extent,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[format],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            });
            self.target = Some(Target { texture, view, stencil, size });
        }
    }
}
