//! The editing UI with no window: upstream's `CadApp`, assembled as the desktop app assembles it
//! (`apps/cadcraft/src/main.rs`, `main`) minus everything window-, settings- or update-related,
//! driven one pass at a time by synthesized `RawInput` and painted offscreen.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use cadcraft_engine::Session;
use cadcraft_engine::file_access::{self, AutomationRoots};
use cadcraft_ui_egui::{CadApp, ControlRequest, Services};

use crate::adapters::gpu::Gpu;
use crate::adapters::painter::OffscreenPainter;
use crate::adapters::readback::Readback;
use crate::domain::geometry::FrameSize;

/// What the host chose for the UI (flags).
#[derive(Clone, Debug)]
pub struct UiOptions {
    /// An interface language code (`zh`, `en`, `de`, … `crates/ui-egui/src/i18n.rs`).
    pub lang: String,
    pub dark: bool,
    /// The folders every command of the session is held to; `None`: nothing is confined.
    pub roots: Option<Arc<AutomationRoots>>,
}

/// One pass's results the loop acts on.
pub struct Pass {
    pub shapes: Vec<egui::epaint::ClippedShape>,
    pub pixels_per_point: f32,
    pub platform: egui::PlatformOutput,
    /// When the UI wants the next pass (`Duration::MAX`: not until something happens).
    pub repaint_delay: Duration,
    /// `ui.screenshot`: the frame painted after this pass, read back.
    pub screenshots: Vec<egui::ScreenshotCallback>,
    /// The pass put a new message on the status line (a file dialog's answer that failed).
    pub status_changed: bool,
    pub ms: f32,
}

pub struct UiHost {
    pub ctx: egui::Context,
    pub app: CadApp,
    started: Instant,
    size: FrameSize,
    focused: bool,
    theme: egui::Theme,
    max_texture_side: usize,
    stencil: Option<egui_wgpu::wgpu::TextureFormat>,
    roots: Option<Arc<AutomationRoots>>,
    /// Texture changes since the last paint: passes that are not painted (pane hidden) keep
    /// theirs for the next one, as eframe does.
    deltas: egui::TexturesDelta,
    painter: OffscreenPainter,
    shots: Readback,
}

/// File format hooks for the engine (it stays I/O-agnostic), as the desktop app installs them
/// (`apps/cadcraft/src/main.rs`, `install_io`).
pub fn install_io() {
    cadcraft_engine::cmd::file::set_io(cadcraft_engine::cmd::file::IoHooks {
        read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
        write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|e| e.to_string())),
    });
    cadcraft_engine::cmd::file::set_image_decoder(|b| cadcraft_io::raster::decode(b).map_err(|e| e.to_string()));
    cadcraft_engine::cmd::file::set_write_as(|d, name, o| {
        cadcraft_io::write_with(d, name, &cadcraft_io::SaveOptions { version: o.version, binary: o.binary }).map_err(|e| e.to_string())
    });
    cadcraft_engine::cmd::exchange::set_pdf_import(|bytes, options| cadcraft_io::pdf_read::read_json(bytes, options).map_err(|e| e.to_string()));
    // AutoLISP's entget, entmake and entmod see entities as DXF groups.
    cadcraft_engine::cmd::file::set_entity_codec(cadcraft_engine::cmd::file::EntityCodec {
        groups: cadcraft_io::entity_groups,
        entity: cadcraft_io::entity_from_groups,
    });
    // ... and non-graphic objects (tables, dictionaries, layouts) as DXF records.
    cadcraft_engine::cmd::file::set_object_records(cadcraft_io::object_records);
    // What a DXF before 2007 leaves out for its proxies' string streams (#3229).
    cadcraft_engine::cmd::file::set_proxies_lost(cadcraft_io::proxies_lost);
    cadcraft_engine::cmd::file::set_dwg_compat_lost(cadcraft_io::dwg_compat_lost);
}

impl UiHost {
    /// The UI on `gpu` (none: the drawing area on the CPU), its dialogs from `services`, its
    /// control channel's requests on `control`.
    pub fn new(ctx: egui::Context, gpu: Option<&Gpu>, opts: &UiOptions, size: FrameSize, services: Services, control: Receiver<ControlRequest>) -> Self {
        // The craft-ui runtime has no native window here, as in CADCraft's browser/test hosts.
        cadcraft_ui_egui::framework::install_headless(&ctx);
        // Detached panels stay inside the pane: there is no OS to give them windows.
        ctx.set_embed_viewports(true);
        let mut app = CadApp::new(Session::empty(), services);
        cadcraft_render_server::host::editor(&mut app, &opts.lang, opts.dark);
        if let Some(gpu) = gpu {
            app.set_wgpu(&gpu.rs, gpu.stencil);
        }
        let mut app = app.with_control(control);
        if app.session.docs.is_empty() {
            // In the units the profile asks for (MEASUREINIT), as the desktop starts.
            let metric = app.session.settings.measureinit;
            app.session.new_drawing(metric);
        }
        UiHost {
            max_texture_side: gpu.map_or(8192, |g| g.rs.device.limits().max_texture_dimension_2d as usize),
            stencil: gpu.and_then(|g| g.stencil),
            roots: opts.roots.clone(),
            ctx,
            app,
            started: Instant::now(),
            size,
            focused: true,
            theme: if opts.dark { egui::Theme::Dark } else { egui::Theme::Light },
            deltas: egui::TexturesDelta::default(),
            painter: OffscreenPainter::default(),
            shots: Readback::default(),
        }
    }

    /// Open the drawing at `path`, read in the background as the desktop does. The host gave it:
    /// it is granted under the automation roots ([`Self::grant`]), so it can be saved back.
    pub fn open(&mut self, path: &Path) {
        self.grant(path);
        self.app.open_path(path);
    }

    /// A file the host gave (a dialog's answer, `--open`): the session may read and write it
    /// wherever it is, while everything else stays held to the roots.
    pub fn grant(&self, path: &Path) {
        if let Some(roots) = &self.roots
            && let Err(e) = roots.grant(path)
        {
            log::warn!("{e}");
        }
    }

    pub fn size(&self) -> FrameSize {
        self.size
    }

    pub fn set_size(&mut self, size: FrameSize) {
        self.size = size;
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// CSS px per UI point: the UI's own zoom (its interface scale).
    pub fn zoom(&self) -> f32 {
        self.ctx.zoom_factor()
    }

    /// One UI pass over `events`, as eframe runs one: the raw-input hook, then `logic` and `ui`
    /// under the app's panic guard (the desktop's `App`, `apps/cadcraft/src/main.rs`).
    pub fn pass(&mut self, events: Vec<egui::Event>) -> Pass {
        let t0 = Instant::now();
        let zoom = self.zoom();
        let points = egui::vec2(self.size.css_w as f32 / zoom, self.size.css_h as f32 / zoom);
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
        // Never minimized or occluded, also while the pane is hidden: the model's screenshots
        // must still be answered.
        let info = egui::ViewportInfo {
            native_pixels_per_point: Some(self.size.dpr),
            monitor_size: Some(points),
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            minimized: Some(false),
            maximized: Some(false),
            fullscreen: Some(false),
            focused: Some(self.focused),
            occluded: Some(false),
            ..Default::default()
        };
        let mut raw = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            viewports: std::iter::once((egui::ViewportId::ROOT, info)).collect(),
            screen_rect: Some(rect),
            max_texture_side: Some(self.max_texture_side),
            time: Some(self.started.elapsed().as_secs_f64()),
            events,
            focused: self.focused,
            system_theme: Some(self.theme),
            ..Default::default()
        };
        let status_before = self.app.status.as_ref().map(|s| s.1);
        let (app, ctx) = (&mut self.app, &self.ctx);
        // Every command the pass runs (the control channel's, typed, scripts, LISP) is held to the
        // automation roots; files the host gave are granted.
        let out = file_access::confine(self.roots.as_ref(), || {
            app.raw_input_hook(ctx, &mut raw);
            app.before_input(ctx);
            ctx.run_ui(raw, |ui| {
                app.guarded_logic(ui.ctx());
                app.close_path(ui.ctx());
                app.guarded_ui(ui);
            })
        });
        let status_changed = self.app.status.as_ref().is_some_and(|(_, at)| Some(*at) != status_before);
        // The host decides when this process ends (`stop`); Quit from the UI or `app.quit` is not it.
        if std::mem::take(&mut self.app.quit_requested) {
            log::info!("quit asked for; the host ends this process");
        }
        let egui::FullOutput { platform_output, textures_delta, shapes, pixels_per_point, mut viewport_output, .. } = out;
        self.deltas.append(textures_delta);
        let root = viewport_output.remove(&egui::ViewportId::ROOT);
        let repaint_delay = root.as_ref().map_or(Duration::MAX, |v| v.repaint_delay);
        let mut screenshots = Vec::new();
        for command in root.into_iter().flat_map(|v| v.commands) {
            match command {
                egui::ViewportCommand::Screenshot(callback) => screenshots.push(callback),
                // The pane is not a window: close, title, size, focus, cursor warp; every pass is
                // painted when the pane is shown, so "paint while hidden" asks for nothing more.
                other => log::debug!("viewport command ignored: {other:?}"),
            }
        }
        Pass {
            shapes,
            pixels_per_point,
            platform: platform_output,
            repaint_delay,
            screenshots,
            status_changed,
            ms: t0.elapsed().as_secs_f32() * 1000.0,
        }
    }

    /// The graphics device was lost (the app's own fault handler saw it): nothing more can be drawn.
    pub fn device_lost(&self) -> Option<String> {
        self.app.gpu_notice.as_ref().filter(|n| n.fault.lost).map(|n| n.fault.message.clone())
    }

    /// Tessellate and paint a pass into the offscreen texture; the paint's milliseconds.
    pub fn paint(&mut self, gpu: &Gpu, shapes: Vec<egui::epaint::ClippedShape>, pixels_per_point: f32) -> f32 {
        let t0 = Instant::now();
        let prims = self.ctx.tessellate(shapes, pixels_per_point);
        // The theme's panel colour, opaque: the UI covers the pane, and the page draws frames
        // over each other.
        let mut clear = self.ctx.global_style().visuals.panel_fill.to_normalized_gamma_f32();
        clear[3] = 1.0;
        self.painter.paint(&gpu.rs, self.size.px(), pixels_per_point, clear, self.stencil, &prims, &mut self.deltas);
        t0.elapsed().as_secs_f32() * 1000.0
    }

    pub fn texture(&self) -> Option<&egui_wgpu::wgpu::Texture> {
        self.painter.texture()
    }

    /// The texture last painted, as tight opaque RGBA rows (snapshots, screenshots). Its own
    /// buffer: the frame stream's comparison is untouched.
    pub fn read_frame(&mut self, gpu: &Gpu, out: &mut Vec<u8>) -> Result<(), String> {
        let texture = self.painter.texture().ok_or("nothing painted yet")?;
        self.shots.read(&gpu.rs, texture, self.size.px(), out)?;
        for px in out.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        Ok(())
    }

    /// Answer `ui.screenshot` with the frame just painted (egui's callback, as eframe hands it the
    /// window's capture). A read that fails drops them: their request times out and says so.
    pub fn answer_screenshots(&mut self, gpu: &Gpu, callbacks: Vec<egui::ScreenshotCallback>) {
        if callbacks.is_empty() {
            return;
        }
        let mut rgba = Vec::new();
        let read = self.read_frame(gpu, &mut rgba);
        // Screenshots are rare in a stream: their read-back buffer is not kept between them.
        self.shots = Readback::default();
        match read {
            Ok(()) => {
                let image = Arc::new(egui::ColorImage::from_rgba_unmultiplied([self.size.px_w as usize, self.size.px_h as usize], &rgba));
                for callback in callbacks {
                    callback.complete(image.clone());
                }
            }
            Err(e) => log::warn!("screenshot: {e}"),
        }
    }
}
