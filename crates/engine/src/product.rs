//! The product name an embedding host shows this editor under. A host that runs the editing UI
//! inside its own product (a pane of another application) sets its name once, before anything is
//! shown; every message that names CADCraft (the interface's catalogs, the command line, the
//! status bar) then names the host's product instead. Unset, nothing changes and nothing is done.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The name messages are written with.
pub const UPSTREAM: &str = "CADCraft";

/// Rewritten texts kept, at most: the catalogs and literals that name the product are a few hundred.
const MAX_KEPT: usize = 8192;

static NAME: OnceLock<&'static str> = OnceLock::new();
static KEPT: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);

/// Show this editor as `name` from now on, for the life of the process. The first call wins;
/// `false` when a name was already set.
pub fn set_name(name: &'static str) -> bool {
    NAME.set(name).is_ok()
}

/// The host product's name, if one was set.
pub fn hosted() -> Option<&'static str> {
    NAME.get().copied()
}

/// The product's name as the user reads it.
pub fn name() -> &'static str {
    hosted().unwrap_or(UPSTREAM)
}

/// `text` with the host product's name where it names CADCraft, for text that lives as long as
/// the program (catalog entries, literals). Each rewritten text is made once and kept.
pub fn rebrand(text: &str) -> &str {
    let Some(name) = hosted() else { return text };
    if !text.contains(UPSTREAM) {
        return text;
    }
    let Ok(mut kept) = KEPT.lock() else { return text };
    let kept = kept.get_or_insert_with(HashMap::new);
    if let Some(done) = kept.get(text) {
        return *done;
    }
    if kept.len() >= MAX_KEPT {
        // Text made at run time that names the product: rewritten each time, not kept.
        return text;
    }
    let done: &'static str = Box::leak(text.replace(UPSTREAM, name).into_boxed_str());
    kept.insert(text.to_string(), done);
    done
}

/// [`rebrand`] for text made at run time (messages with values in them): rewritten, not kept.
pub fn rebrand_owned(text: String) -> String {
    match named(&text) {
        Cow::Borrowed(_) => text,
        Cow::Owned(done) => done,
    }
}

/// `text` with the host product's name where it names CADCraft.
pub fn named(text: &str) -> Cow<'_, str> {
    match hosted() {
        Some(name) if text.contains(UPSTREAM) => Cow::Owned(text.replace(UPSTREAM, name)),
        _ => Cow::Borrowed(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unset (as every test process is), texts are returned as they are.
    #[test]
    fn without_a_host_nothing_changes() {
        assert_eq!(hosted(), None);
        assert_eq!(name(), "CADCraft");
        assert_eq!(rebrand("About CADCraft"), "About CADCraft");
        assert_eq!(named("CADCraft can't"), "CADCraft can't");
    }
}
