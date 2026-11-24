//! Shoots and drives the web view example (crates/slate/examples/web.rs),
//! built with slate's `render` feature, and checks the web view's own pixels
//! are in the shots: `render_to_image` draws only GPUI's scene, so without
//! WebKit's picture laid over it the whole box would be GPUI's yellow.
//!
//! They launch a real (hidden) window, so they need a macOS login session
//! and are ignored by default:
//!
//! ```sh
//! cargo test -p slate-cli --test web -- --ignored
//! ```
//!
//! `SLATE_WEB` points them at an example already built with the `render`
//! feature (`scripts/prove.sh` does this). Without it they build one into
//! `target/slate-test`, as the `shoot` test does.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use image::RgbImage;

/// The example's colours: the page's block before and after the click, and
/// GPUI's box under the web view, which shows through the page's
/// transparent bottom half.
const PURPLE: [u8; 3] = [0x6a, 0x1b, 0x9a];
const TEAL: [u8; 3] = [0x00, 0x83, 0x8f];
const YELLOW: [u8; 3] = [0xfd, 0xd8, 0x35];

/// Points in the window: the page's block (top half of the web view) and
/// its transparent bottom half.
const BLOCK: (u32, u32) = (560, 160);
const SEE_THROUGH: (u32, u32) = (560, 300);

#[test]
#[ignore = "opens a hidden window: needs a macOS login session"]
fn a_shot_shows_the_web_view() {
    let web = web();
    let out = tempfile::tempdir().unwrap();
    let png = out.path().join("shot.png");
    let status = Command::new(env!("CARGO_BIN_EXE_slate"))
        .args(["shoot", "--wait", "1500", "--out"])
        .arg(&png)
        .arg("--")
        .arg(&web)
        .status()
        .unwrap();
    assert!(status.success(), "slate shoot failed");
    let image = image::open(&png).unwrap().into_rgb8();
    assert_color(&image, BLOCK, PURPLE, "the web view's block", &png);
    assert_color(&image, SEE_THROUGH, YELLOW, "GPUI's box through the page", &png);
    // A third of the box is the block: 320x160 points less its text.
    let purple = image.pixels().filter(|pixel| close(pixel.0, PURPLE, 2)).count();
    assert!(purple > 45_000, "only {purple} pixels of the web view's block in {}", png.display());
}

#[test]
#[ignore = "opens a hidden window: needs a macOS login session"]
fn a_drive_shows_the_web_view_change() {
    let web = web();
    let out = tempfile::tempdir().unwrap();
    let steps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../slate/examples/web.steps.json");
    let status = Command::new(env!("CARGO_BIN_EXE_slate"))
        .args(["drive", "--wait", "1500", "--steps"])
        .arg(&steps)
        .arg("--out")
        .arg(out.path())
        .arg("--")
        .arg(&web)
        .status()
        .unwrap();
    assert!(status.success(), "slate drive failed");
    let start = out.path().join("start.png");
    let clicked = out.path().join("clicked.png");
    let before = image::open(&start).unwrap().into_rgb8();
    let after = image::open(&clicked).unwrap().into_rgb8();
    assert_color(&before, BLOCK, PURPLE, "the web view's block", &start);
    assert_color(&after, BLOCK, TEAL, "the web view's block after the click", &clicked);
    assert_color(&after, SEE_THROUGH, YELLOW, "GPUI's box through the page", &clicked);
}

/// The app's render of its window against `screencapture -l` of the same
/// window, converted to sRGB. Needs Screen Recording permission for the
/// terminal, which the render path itself doesn't.
#[test]
#[ignore = "opens a hidden window and needs Screen Recording permission"]
fn the_render_matches_screencapture() {
    let web = web();
    let dir = tempfile::tempdir().unwrap();
    let mut app =
        Command::new(&web).env("SLATE", "1").env("SLATE_WAIT", "1500").env("SLATE_DIR", dir.path()).spawn().unwrap();
    let shot = wait_for_shot(dir.path());
    // The app stays open after its shot, so the window is still there.
    let captured = dir.path().join("screencapture.png");
    let status = Command::new("screencapture")
        .args(["-x", "-o", "-l", &shot.window_id.to_string()])
        .arg(&captured)
        .status()
        .unwrap();
    let _ = app.kill();
    let _ = app.wait();
    assert!(status.success() && captured.exists(), "screencapture -l gave no image: is Screen Recording allowed?");
    let srgb = dir.path().join("screencapture-srgb.png");
    let status = Command::new("sips")
        .args(["-m", "/System/Library/ColorSync/Profiles/sRGB Profile.icc"])
        .arg(&captured)
        .arg("--out")
        .arg(&srgb)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "sips couldn't convert the capture to sRGB");
    let rendered = image::open(shot.image.as_ref().expect("the app rendered the shot")).unwrap().into_rgb8();
    let captured = image::open(&srgb).unwrap().into_rgb8();
    assert_eq!(rendered.dimensions(), captured.dimensions());
    // The web view's box, in device pixels.
    let scale = shot.scale.round() as u32;
    let (mut total, mut worst, mut count) = (0u64, 0u8, 0u64);
    for y in 60 * scale..380 * scale {
        for x in 400 * scale..720 * scale {
            let have = rendered.get_pixel(x, y).0;
            let want = captured.get_pixel(x, y).0;
            let off = have.iter().zip(want).map(|(have, want)| have.abs_diff(want)).max().unwrap();
            total += u64::from(off);
            worst = worst.max(off);
            count += 1;
        }
    }
    let mean = total as f64 / count as f64;
    assert!(worst <= 6 && mean < 1.5, "the web view's box differs from screencapture: worst {worst}, mean {mean:.2}");
}

fn wait_for_shot(dir: &Path) -> slate::Shot {
    for _ in 0..150 {
        if let Some(shot) = slate::Shot::read(dir).unwrap() {
            return shot;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("the app wrote no shot into {}", dir.display());
}

fn assert_color(image: &RgbImage, (x, y): (u32, u32), want: [u8; 3], what: &str, path: &Path) {
    let have = image.get_pixel(x, y).0;
    assert!(close(have, want, 2), "{what} at {x},{y} is {have:?}, not {want:?}, in {}", path.display());
}

/// Within `by` of `want` in every channel.
fn close(have: [u8; 3], want: [u8; 3], by: u8) -> bool {
    have.iter().zip(want).all(|(have, want)| have.abs_diff(want) <= by)
}

fn web() -> PathBuf {
    if let Some(path) = std::env::var_os("SLATE_WEB") {
        return path.into();
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target/slate-test");
    let status = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "--quiet", "-p", "slate", "--example", "web", "--features", "render", "--target-dir"])
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success(), "building the example failed");
    target.join("debug/examples/web")
}
