//! Host read-only (PROTOCOL.md, "Read-only"): the engine's own switch refuses saving and puts back
//! every command's change (`cadcraft_engine::read_only`), and a backstop here: each drawing as it
//! was when read-only came on (or when it was opened since) is kept, and a pass that ends with a
//! drawing changed some other way (an undo from the toolbar, a drag) puts it back and says why on
//! the status line. Drawings are immutable behind an `Arc`, so keeping one costs nothing.

use std::collections::HashMap;
use std::sync::Arc;

use cadcraft_engine::doc::Drawing;
use cadcraft_ui_egui::CadApp;

#[derive(Default)]
pub struct ReadOnly {
    reason: Option<String>,
    /// Each open drawing (by `uid`) as it is to stay.
    kept: HashMap<u64, Arc<Drawing>>,
}

impl ReadOnly {
    /// `policy {readOnly}`: on with this reason, or off.
    pub fn set(&mut self, app: &mut CadApp, reason: Option<String>) {
        cadcraft_engine::read_only::set(reason);
        self.reason = cadcraft_engine::read_only::reason();
        self.kept.clear();
        if let Some(why) = self.reason.clone() {
            self.keep_new(app);
            app.set_status(why);
        }
    }

    /// After every pass while read-only: a change the engine did not catch is put back.
    pub fn after_pass(&mut self, app: &mut CadApp) {
        let Some(why) = self.reason.clone() else { return };
        let mut put_back = false;
        for st in &mut app.session.docs {
            match self.kept.get(&st.uid) {
                Some(kept) if !Arc::ptr_eq(kept, &st.doc) => {
                    st.doc = kept.clone();
                    st.revision += 1;
                    put_back = true;
                }
                _ => {}
            }
        }
        self.keep_new(app);
        // Closed drawings are not kept.
        let open: Vec<u64> = app.session.docs.iter().map(|d| d.uid).collect();
        self.kept.retain(|uid, _| open.contains(uid));
        if put_back {
            log::warn!("read-only: a drawing changed outside a command; put back");
            app.set_status(why);
        }
    }

    /// Drawings opened (or made) since: kept as they are now.
    fn keep_new(&mut self, app: &CadApp) {
        for st in &app.session.docs {
            self.kept.entry(st.uid).or_insert_with(|| st.doc.clone());
        }
    }
}
