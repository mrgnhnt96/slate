//! Shoots the example app (crates/slate/examples/hello.rs) hidden and checks
//! the shot shows a view the app built during a render 300 ms after opening
//! and that loaded 60 ms after that. Under the old `shoot` the hidden window got no frames
//! during the wait, so that view was built in the shot's only frame and the
//! shot showed it still loading.
//!
//! It launches a real (hidden) window, so it needs a macOS login session and
//! is ignored by default:
//!
//! ```sh
//! cargo test -p slate-cli --test shoot -- --ignored
//! ```
//!
//! `SLATE_HELLO` points it at an example already built with slate's `render`
//! feature (`scripts/prove.sh` does this). Without it the test builds one into
//! `target/slate-test`, its own target folder, since the `cargo test` running
//! it holds the lock on the usual one.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The lazily built card's colour once it has loaded (`LAZY_LOADED` in the
/// example).
const LAZY_LOADED: [u8; 3] = [0x00, 0x83, 0x8f];

#[test]
#[ignore = "opens a hidden window: needs a macOS login session"]
fn a_view_built_in_render_loads_before_the_shot() {
    let hello = hello();
    let out = tempfile::tempdir().unwrap();
    let png = out.path().join("shot.png");
    // The view is built on the first frame after 300 ms and loads 60 ms
    // later, well inside the wait if frames keep coming.
    let status = Command::new(env!("CARGO_BIN_EXE_slate"))
        .args(["shoot", "--size", "800x500", "--wait", "500", "--out"])
        .arg(&png)
        .arg("--")
        .arg(&hello)
        .status()
        .unwrap();
    assert!(status.success(), "slate shoot failed");
    let image = image::open(&png).unwrap().into_rgb8();
    let loaded = image.pixels().filter(|pixel| close(pixel.0, LAZY_LOADED)).count();
    // The card is about 160x56 points; text and rounded corners cover some.
    assert!(loaded > 2000, "the lazily built card never loaded: {loaded} pixels of its colour in {}", png.display());
}

/// Within a little of `want` in every channel, for resampling to points.
fn close(have: [u8; 3], want: [u8; 3]) -> bool {
    have.iter().zip(want).all(|(have, want)| have.abs_diff(want) <= 4)
}

fn hello() -> PathBuf {
    if let Some(path) = std::env::var_os("SLATE_HELLO") {
        return path.into();
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target/slate-test");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "--quiet", "-p", "slate", "--example", "hello", "--features", "render", "--target-dir"])
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success(), "building the example failed");
    target.join("debug/examples/hello")
}
