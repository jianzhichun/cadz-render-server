//! The system clipboard, written by the render server itself (arboard, the crate the desktop app's
//! window toolkit uses): no browser clipboard permission, and the same in WKWebView and WebView2.
//! Kept open for the process's life: on X11 the owner serves the text. Text only: the drawing
//! clipboard travels as DXF text (`crates/ui-egui/src/clipboard.rs`).

#[derive(Default)]
pub struct Clipboard {
    inner: Option<arboard::Clipboard>,
}

impl Clipboard {
    /// Put `text` on the clipboard; false where there is none (a Linux host with no display).
    pub fn set_text(&mut self, text: &str) -> bool {
        self.with(|c| c.set_text(text.to_string()))
    }

    fn with(&mut self, f: impl FnOnce(&mut arboard::Clipboard) -> Result<(), arboard::Error>) -> bool {
        if self.inner.is_none() {
            match arboard::Clipboard::new() {
                Ok(c) => self.inner = Some(c),
                Err(e) => {
                    log::warn!("clipboard: none on this system ({e})");
                    return false;
                }
            }
        }
        let Some(c) = self.inner.as_mut() else { return false };
        match f(c) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("clipboard: {e}");
                false
            }
        }
    }
}
