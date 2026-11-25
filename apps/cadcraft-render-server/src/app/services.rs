//! The UI's file dialogs, answered by the host: every Open, Save As, export, plot to file, APPLOAD
//! and alias-file pick (`CadApp::pick_then`, `crates/ui-egui/src/lib.rs`) goes through
//! `Services::start_pick`, the hook the desktop's macOS sheets use, and becomes an `ask_open` or
//! `ask_save`; the host's `answer` is handed to the UI's callback, which acts on it in the next
//! pass (`CadApp::logic` → `finish_pick`), after which `ask_done` goes out. The blocking pickers are
//! set only to say which dialogs there are; with `start_pick` set the UI never calls them.
//!
//! A save's answer is a place of the host's: where the UI then names it (the command line, the
//! status line), the file's name alone is shown ([`Asks::hide_places`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cadcraft_ui_egui::{CadApp, Pick, Services};

use crate::domain::wire::ServerMsg;

/// What the UI does with an answer (`StartPick`'s callback).
type Done = Box<dyn FnOnce(Option<String>) + Send>;

/// Asks waiting at once, at most: the UI shows one dialog at a time; a host that never answers
/// cannot make this grow.
const MAX_WAITING: usize = 16;

/// Places remembered for the status and command lines.
const PLACES_KEPT: usize = 64;

#[derive(Default)]
struct Table {
    next: u32,
    waiting: HashMap<u32, Done>,
    /// Answers handed to the UI since the last pass: `(ask, a path was given)`.
    delivered: Vec<(u32, bool)>,
    /// The places the host gave, newest last.
    places: Vec<String>,
}

/// The asks of this process. Lives on the frame loop's thread with the UI.
#[derive(Clone, Default)]
pub struct Asks {
    table: Rc<RefCell<Table>>,
}

impl Asks {
    /// The `Services` whose dialogs ask the host through `send`.
    pub fn services(&self, send: Rc<dyn Fn(ServerMsg)>) -> Services {
        let asks = self.clone();
        Services {
            pick_open: Some(Box::new(|| -> Option<String> { None })),
            pick_save: Some(Box::new(|_: &str| -> Option<String> { None })),
            pick_lisp: Some(Box::new(|_: &str| -> Option<String> { None })),
            pick_pgp: Some(Box::new(|_: Option<&str>| -> Option<String> { None })),
            start_pick: Some(Box::new(move |pick: Pick, done: Done| asks.ask(&*send, &pick, done))),
            no_file_system: false,
        }
    }

    /// The host answered `ask` (`None`: cancelled). An answer to an ask that is not waiting
    /// (answered twice, or never asked) is ignored.
    pub fn answer(&self, ask: u32, path: Option<String>) {
        let done = {
            let mut table = self.table.borrow_mut();
            let Some(done) = table.waiting.remove(&ask) else { return };
            table.delivered.push((ask, path.is_some()));
            if let Some(p) = &path {
                table.places.push(p.clone());
                if table.places.len() > PLACES_KEPT {
                    table.places.remove(0);
                }
            }
            done
        };
        // Outside the borrow: the callback only sends on the UI's channel.
        done(path);
    }

    /// The answers the UI took since the last call, for `ask_done` after the pass that ran them.
    pub fn take_delivered(&self) -> Vec<(u32, bool)> {
        std::mem::take(&mut self.table.borrow_mut().delivered)
    }

    /// Where the UI wrote one of the host's places into the status line or the command line
    /// (lines from `log_from` on), the file's name alone. The host says where a file went.
    pub fn hide_places(&self, app: &mut CadApp, log_from: usize) {
        let table = self.table.borrow();
        if table.places.is_empty() {
            return;
        }
        if let Some((text, _)) = app.status.as_mut() {
            hide(&table.places, text);
        }
        for line in app.session.log.iter_mut().skip(log_from) {
            hide(&table.places, line);
        }
    }

    fn ask(&self, send: &dyn Fn(ServerMsg), pick: &Pick, done: Done) {
        let ask = {
            let mut table = self.table.borrow_mut();
            if table.waiting.len() >= MAX_WAITING {
                drop(table);
                done(None);
                return;
            }
            table.next = table.next.wrapping_add(1).max(1);
            let ask = table.next;
            table.waiting.insert(ask, done);
            ask
        };
        send(match pick {
            Pick::Save(name) => ServerMsg::AskSave { ask, name: file_name(name).to_string(), exts: save_exts(name) },
            // Open serves the drawing formats, templates, imports, xrefs and linetype files alike:
            // any file.
            Pick::Open => ServerMsg::AskOpen { ask, exts: Vec::new() },
            Pick::Lisp(_) => ServerMsg::AskOpen { ask, exts: vec!["lsp".into()] },
            Pick::Pgp(_) => ServerMsg::AskOpen { ask, exts: vec!["pgp".into()] },
        });
    }
}

/// The types a save may take, the suggested name's first (the desktop's Save dialog,
/// `apps/cadcraft/src/main.rs` `filters_for_save`): a drawing's Save As can change its format.
fn save_exts(name: &str) -> Vec<String> {
    const DRAWINGS: [&str; 3] = ["dxf", "dwg", "dwt"];
    let own = std::path::Path::new(name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match own {
        Some(e) if DRAWINGS.contains(&e.as_str()) => {
            let mut exts = vec![e.clone()];
            exts.extend(DRAWINGS.iter().filter(|d| **d != e).map(|d| d.to_string()));
            exts
        }
        Some(e) if !e.is_empty() => vec![e],
        _ => DRAWINGS.iter().map(|d| d.to_string()).collect(),
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// `text` with each of `places` by its file name.
fn hide(places: &[String], text: &mut String) {
    for place in places.iter().rev() {
        if !place.is_empty() && text.contains(place.as_str()) {
            *text = text.replace(place.as_str(), file_name(place));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// An ask goes to the host at once; its answer reaches the UI's callback once, and is
    /// reported as delivered once (for `ask_done`); a second answer changes nothing.
    #[test]
    fn a_save_ask_waits_for_the_hosts_answer() {
        let asks = Asks::default();
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let services = asks.services(Rc::new(move |m: ServerMsg| log.borrow_mut().push(m)));
        let (tx, rx) = mpsc::channel();
        let start = services.start_pick.as_ref().unwrap();
        start(
            Pick::Save("Drawing1.dwg".into()),
            Box::new(move |p: Option<String>| {
                let _ = tx.send(p);
            }),
        );
        let exts = vec!["dwg".to_string(), "dxf".into(), "dwt".into()];
        assert_eq!(*sent.borrow(), vec![ServerMsg::AskSave { ask: 1, name: "Drawing1.dwg".into(), exts }]);
        assert!(rx.try_recv().is_err(), "nothing until the host answers");
        asks.answer(1, Some("/staging/Drawing1.dwg".into()));
        asks.answer(1, None);
        assert_eq!(rx.try_recv(), Ok(Some("/staging/Drawing1.dwg".into())));
        assert!(rx.try_recv().is_err());
        assert_eq!(asks.take_delivered(), vec![(1, true)]);
        assert!(asks.take_delivered().is_empty());
    }

    #[test]
    fn open_asks_name_their_types() {
        let asks = Asks::default();
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let services = asks.services(Rc::new(move |m: ServerMsg| log.borrow_mut().push(m)));
        let start = services.start_pick.as_ref().unwrap();
        start(Pick::Open, Box::new(|_: Option<String>| {}));
        start(Pick::Lisp(String::new()), Box::new(|_: Option<String>| {}));
        assert_eq!(*sent.borrow(), vec![ServerMsg::AskOpen { ask: 1, exts: vec![] }, ServerMsg::AskOpen { ask: 2, exts: vec!["lsp".into()] }]);
        assert_eq!(save_exts("plan.pdf"), vec!["pdf".to_string()]);
        assert_eq!(save_exts("Drawing"), vec!["dxf".to_string(), "dwg".into(), "dwt".into()]);
    }

    #[test]
    fn a_place_of_the_host_shows_as_its_file_name() {
        let places = vec![r"C:\Users\u\AppData\staging\export-1\plan.pdf".to_string()];
        let mut text = format!("Plotted to {}", places[0]);
        hide(&places, &mut text);
        assert_eq!(text, "Plotted to plan.pdf");
        let mut other = "Saved".to_string();
        hide(&places, &mut other);
        assert_eq!(other, "Saved");
    }
}
