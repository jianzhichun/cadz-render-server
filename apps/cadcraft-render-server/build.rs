#![forbid(unsafe_code)]

// Windows gives a main thread 1 MiB of stack unless the binary asks for more; the editing UI with a
// software adapter (WARP) needs more than that on its first frame.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    if windows && std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    }
}
