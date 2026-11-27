//! Host read-only: an embedding host can make every open drawing read-only for a reason it gives
//! (a licence that ran out, a drawing another session holds). While it is on, a command that
//! changes a drawing is put back and refused with that reason, and saving the drawing's own file
//! is refused; opening, looking, selecting, measuring and exporting go on. Process-wide, because
//! the interface, the control channel and their worker threads all reach the same drawings.
//! Off unless a host turns it on.

use std::sync::Mutex;

static REASON: Mutex<Option<String>> = Mutex::new(None);

/// Commands that write the drawing's own file (`qsave` is also SAVE).
const WRITES_DRAWING: [&str; 2] = ["qsave", "saveas"];

/// Read-only on with `reason`, or off with `None` (an empty reason is off).
pub fn set(reason: Option<String>) {
    let reason = normal(reason);
    match REASON.lock() {
        Ok(mut r) => *r = reason,
        Err(poisoned) => *poisoned.into_inner() = reason,
    }
}

/// Why the drawings are read-only, while they are.
pub fn reason() -> Option<String> {
    match REASON.lock() {
        Ok(r) => r.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Why the command `id` is refused before it runs, if it is (read-only on and it writes the
/// drawing's file).
pub fn refuses(id: &str) -> Option<String> {
    refusal(reason(), id)
}

fn normal(reason: Option<String>) -> Option<String> {
    reason.map(|r| r.trim().to_string()).filter(|r| !r.is_empty())
}

fn refusal(reason: Option<String>, id: &str) -> Option<String> {
    reason.filter(|_| WRITES_DRAWING.contains(&id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure parts only: the switch itself is process-wide and other tests run beside this one.
    #[test]
    fn saving_is_refused_only_while_read_only() {
        let on = normal(Some("  licence ended ".into()));
        assert_eq!(on.as_deref(), Some("licence ended"));
        assert_eq!(refusal(on.clone(), "qsave").as_deref(), Some("licence ended"));
        assert_eq!(refusal(on, "line"), None, "a change is put back after it ran, not refused before");
        assert_eq!(normal(Some("   ".into())), None, "an empty reason is off");
        assert_eq!(refusal(None, "saveas"), None);
    }
}
