//! cadcraft-render-server: CADCraft's editing UI with no window. Each frame is drawn offscreen
//! and streamed to the embedding host that started this process; the host sends the user's
//! pointer, keyboard and IME input back; the model drives the same session over the control
//! channel. Protocol: PROTOCOL.md beside this crate.
//!
//! Layers: `domain` (rules, no I/O), `adapters` (GPU, sockets, encoders, clipboard), `app` (the
//! UI host and the two modes), and this composition root; the host's branding is the library's
//! `host` (shared with the brand test).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod adapters;
mod app;
mod cli;
mod domain;

use app::snapshot::{self, SnapshotArgs};
use app::stream::{self, StreamArgs};
use app::ui_host::UiOptions;
use cli::{Mode, Parsed};
use domain::geometry::FrameSize;

fn main() {
    // A copy of this program started to read or write a DWG file does only that (the desktop
    // app's worker, `cadcraft_io::dwg_worker`).
    if let Some(code) = cadcraft_io::dwg_worker::serve_if_asked() {
        std::process::exit(code);
    }
    cadcraft_io::dwg_worker::use_own_worker();
    // Before any message is made: the host product's name.
    cadcraft_render_server::host::name_product();
    adapters::logging::install();
    let options = match cli::parse(std::env::args().skip(1)) {
        Ok(Parsed::Run(o)) => *o,
        Ok(Parsed::Help) => {
            eprintln!("{}", cli::USAGE);
            return;
        }
        Ok(Parsed::Version) => {
            eprintln!("cadcraft-render-server {} (protocol {})", env!("CARGO_PKG_VERSION"), domain::wire::PROTOCOL_VERSION);
            return;
        }
        Err(e) => {
            eprintln!("cadcraft-render-server: {e}\n{}", cli::USAGE);
            std::process::exit(stream::EXIT_SETUP);
        }
    };
    if let Some(session) = &options.session {
        log::info!("session {session}");
    }
    app::ui_host::install_io();
    // No settings folder: nothing of the user's own CADCraft (profile, aliases, autosave, LISP
    // support files) is read or written by this process.
    cadcraft_engine::settings_dir::set(None);
    let ui = UiOptions { lang: options.lang, dark: options.dark };
    let size = FrameSize::new(options.css.0, options.css.1, options.dpr);
    let code = match options.mode {
        Mode::Stream(addr) => stream::run(StreamArgs { addr, ui, size, adapter: options.adapter, open: options.open }),
        Mode::Snapshot { dir, frames, script, hold, gap } => {
            snapshot::run(SnapshotArgs { dir, frames, open: options.open, script, hold, gap, ui, size, adapter: options.adapter })
        }
    };
    std::process::exit(code);
}
