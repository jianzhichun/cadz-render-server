//! The control channel on this process's session: the desktop app's own server
//! (`apps/cadcraft/src/control_server.rs`, compiled in here unchanged but for its `serve_bound`
//! entry), so the MCP bridge (`cadcraft-cli mcp --connect 127.0.0.1:PORT`) sees exactly the app it
//! knows. It listens on a free loopback port; the token is the host's, written where the bridge
//! reads it (`cadcraft_mcp::control_token::token_path(port)`, owner-only) and removed at exit.
//! Requests run on the UI's thread (`CadApp::logic` → `drain_control`), on the session the user sees.

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use cadcraft_ui_egui::ControlRequest;

// `start` (the desktop's: a port given on its command line and a fresh token) is not used here.
#[allow(dead_code)]
#[path = "../../../cadcraft/src/control_server.rs"]
mod control_server;

pub struct ControlChannel {
    pub port: u16,
}

/// Where the token comes from.
pub enum Token {
    /// The host's (`CADCRAFT_CONTROL_TOKEN`).
    Given(String),
    /// None given (snapshot mode on its own): a fresh one, as the desktop app makes.
    Fresh,
}

impl ControlChannel {
    /// Listen on `127.0.0.1:0` behind `token`; each request wakes the UI through `ctx`. Returns
    /// the channel, the token in effect and the requests for `CadApp::with_control`.
    pub fn open(ctx: egui::Context, token: Token) -> std::io::Result<(Self, String, Receiver<ControlRequest>)> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let token = match token {
            Token::Given(t) => {
                cadcraft_mcp::control_token::issue_given(port, &t)?;
                t
            }
            Token::Fresh => cadcraft_mcp::control_token::issue(port)?,
        };
        let rx = control_server::serve_bound(listener, ctx, Arc::new(token.clone()));
        Ok((ControlChannel { port }, token, rx))
    }
}

impl Drop for ControlChannel {
    /// The token file goes with the process (the port is closed by the process's exit).
    fn drop(&mut self) {
        cadcraft_mcp::control_token::revoke(self.port);
    }
}

/// The host's control token (`CADCRAFT_CONTROL_TOKEN`): 64 lowercase hex, else `None`.
pub fn token_from_env() -> Option<String> {
    std::env::var("CADCRAFT_CONTROL_TOKEN").ok().filter(|t| is_token(t))
}

/// 64 lowercase hex characters (32 random bytes), the form the desktop app issues.
pub fn is_token(t: &str) -> bool {
    t.len() == 64 && t.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokens_are_64_lowercase_hex() {
        assert!(super::is_token(&"0a".repeat(32)));
        assert!(!super::is_token(&"0A".repeat(32)));
        assert!(!super::is_token("abc"));
    }
}
