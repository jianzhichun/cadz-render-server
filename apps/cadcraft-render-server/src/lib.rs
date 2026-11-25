//! What the render server shares with its tests: [`host`], the part of this process that makes
//! upstream's editing UI the host product's editor (no GPU, no I/O). The binary (`main.rs`) is
//! the rest.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod host;
