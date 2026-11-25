//! The command line (PROTOCOL.md, "Starting (leg A)").

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::adapters::gpu::Policy;

pub const USAGE: &str = "usage:
  cadcraft-render-server --stream 127.0.0.1:PORT [--session ID] [--size WxH] [--dpr N] [--lang zh] [--theme dark|light] [--open FILE] [--automation-read-root DIR] [--automation-write-root DIR] [--adapter auto|hardware|software]
  cadcraft-render-server --snapshot DIR --frames N [--open FILE] [--script FILE] [--hold-ms N] [--gap-ms N] [--size WxH] [--dpr N] [--lang zh] [--theme dark|light] [--automation-read-root DIR] [--automation-write-root DIR] [--adapter ...]
environment: CADCRAFT_RENDER_TOKEN and CADCRAFT_CONTROL_TOKEN (64 lowercase hex each; stream mode), CADCRAFT_RENDER_LOG=error|warn|info|debug";

#[derive(Debug, PartialEq)]
pub enum Mode {
    /// Leg A to the host's listener (loopback only).
    Stream(SocketAddr),
    /// No host: `frames` PNGs into `dir`, then the control channel for `hold`.
    Snapshot { dir: PathBuf, frames: u32, script: Option<PathBuf>, hold: Duration, gap: Duration },
}

#[derive(Debug, PartialEq)]
pub struct Options {
    pub mode: Mode,
    /// For log lines only.
    pub session: Option<String>,
    /// The pane, CSS px.
    pub css: (u32, u32),
    pub dpr: f32,
    pub lang: String,
    pub dark: bool,
    /// A drawing to open before the first frame.
    pub open: Option<PathBuf>,
    /// The folders every command of the session is held to (`cadcraft_engine::file_access`).
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
    pub adapter: Policy,
}

#[derive(Debug, PartialEq)]
pub enum Parsed {
    Run(Box<Options>),
    Help,
    Version,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Parsed, String> {
    let mut args = args.into_iter();
    let (mut stream, mut snapshot, mut frames, mut script) = (None, None, None, None);
    let (mut hold, mut gap) = (Duration::ZERO, Duration::from_millis(50));
    let mut o = Options {
        mode: Mode::Stream(SocketAddr::from(([127, 0, 0, 1], 0))),
        session: None,
        css: (1280, 800),
        dpr: 1.0,
        lang: "zh".into(),
        dark: true,
        open: None,
        read_root: None,
        write_root: None,
        adapter: Policy::Auto,
    };
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--help" | "-h" => return Ok(Parsed::Help),
            "--version" | "-V" => return Ok(Parsed::Version),
            "--stream" => {
                let addr: SocketAddr = value()?.parse().map_err(|e| format!("--stream: {e}"))?;
                if !addr.ip().is_loopback() {
                    return Err(format!("--stream: {addr} is not a loopback address"));
                }
                stream = Some(addr);
            }
            "--session" => o.session = Some(value()?),
            "--size" => o.css = size(&value()?)?,
            "--dpr" => o.dpr = value()?.parse::<f32>().ok().filter(|d| d.is_finite() && *d > 0.0 && *d <= 8.0).ok_or("--dpr: a number in (0, 8]")?,
            "--lang" => o.lang = value()?,
            "--theme" => {
                o.dark = match value()?.as_str() {
                    "dark" => true,
                    "light" => false,
                    other => return Err(format!("--theme: dark or light, not {other}")),
                }
            }
            "--adapter" => o.adapter = Policy::parse(&value()?).ok_or("--adapter: auto, hardware or software")?,
            "--open" => o.open = Some(PathBuf::from(value()?)),
            "--automation-read-root" => o.read_root = Some(PathBuf::from(value()?)),
            "--automation-write-root" => o.write_root = Some(PathBuf::from(value()?)),
            "--snapshot" => snapshot = Some(PathBuf::from(value()?)),
            "--frames" => frames = Some(value()?.parse::<u32>().ok().filter(|n| *n > 0 && *n <= 100_000).ok_or("--frames: a positive count")?),
            "--script" => script = Some(PathBuf::from(value()?)),
            "--hold-ms" => hold = Duration::from_millis(value()?.parse().map_err(|_| "--hold-ms: milliseconds")?),
            "--gap-ms" => gap = Duration::from_millis(value()?.parse().map_err(|_| "--gap-ms: milliseconds")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    o.mode = match (stream, snapshot) {
        (Some(addr), None) => Mode::Stream(addr),
        (None, Some(dir)) => Mode::Snapshot { dir, frames: frames.unwrap_or(30), script, hold, gap },
        (Some(_), Some(_)) => return Err("--stream and --snapshot exclude each other".into()),
        (None, None) => return Err("--stream or --snapshot is required".into()),
    };
    Ok(Parsed::Run(Box::new(o)))
}

/// `WxH` in CSS px, each side 1..=16384.
fn size(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("--size: WxH, each 1 to 16384, not {s}");
    let (w, h) = s.split_once(['x', 'X']).ok_or_else(bad)?;
    let (w, h) = (w.trim().parse::<u32>().map_err(|_| bad())?, h.trim().parse::<u32>().map_err(|_| bad())?);
    if !(1..=16384).contains(&w) || !(1..=16384).contains(&h) {
        return Err(bad());
    }
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &str) -> Result<Options, String> {
        match parse(args.split_whitespace().map(String::from))? {
            Parsed::Run(o) => Ok(*o),
            other => Err(format!("{other:?}")),
        }
    }

    #[test]
    fn the_hosts_command_line() {
        let o = run("--stream 127.0.0.1:4100 --session s1 --size 1280x800 --dpr 2 --lang zh --theme light").unwrap();
        assert_eq!(o.mode, Mode::Stream("127.0.0.1:4100".parse().unwrap()));
        assert_eq!((o.css, o.dpr, o.dark, o.session.as_deref()), ((1280, 800), 2.0, false, Some("s1")));
        assert_eq!(run("--stream 127.0.0.1:1 --open a.dwg").unwrap().open, Some(PathBuf::from("a.dwg")));
        let o = run("--stream 127.0.0.1:1 --automation-read-root /w --automation-write-root /w/out").unwrap();
        assert_eq!((o.read_root, o.write_root), (Some(PathBuf::from("/w")), Some(PathBuf::from("/w/out"))));
    }

    #[test]
    fn snapshot_defaults_and_refusals() {
        let o = run("--snapshot out --open fan.dxf").unwrap();
        assert!(matches!(o.mode, Mode::Snapshot { frames: 30, .. }));
        assert!(run("--stream 10.0.0.2:4100").unwrap_err().contains("loopback"));
        assert!(run("--size 0x10 --snapshot out").is_err());
        assert!(run("--size 20000x10 --snapshot out").is_err());
        assert!(run("--adapter cpu --snapshot out").is_err());
        assert!(run("--session s").unwrap_err().contains("required"));
        assert!(run("--stream 127.0.0.1:1 --snapshot out").is_err());
    }
}
