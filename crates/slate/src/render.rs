//! The app's own picture of its window (the `render` feature): GPUI's frame
//! from `Window::render_to_image`, with the web views the app lays over it
//! drawn in where they show. `render_to_image` draws only GPUI's scene, so
//! without them a web view would come out as the background under it.
//!
//! WebKit hands a web view's picture over later, on the main thread, so a
//! shot is two halves: [`start`] renders the frame and asks for the
//! pictures in the same moment, and [`finish`] waits for them, lays them over
//! the frame and writes the PNG.

use std::path::Path;

use gpui_kit::{AsyncApp, Window};

/// The frame, waiting for the web views' pictures.
#[cfg(feature = "render")]
pub(crate) struct Frame {
    image: image::RgbaImage,
    #[cfg(target_os = "macos")]
    snapshots: Vec<crate::snapshot::Snapshot>,
}

#[cfg(not(feature = "render"))]
pub(crate) enum Frame {}

/// How long WebKit gets to hand over a web view's picture. It takes a few
/// milliseconds, so running out means it never will.
#[cfg(feature = "render")]
const SNAPSHOT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How often to look for the pictures while waiting.
#[cfg(feature = "render")]
const SNAPSHOT_POLL: std::time::Duration = std::time::Duration::from_millis(5);

/// Renders the window's last frame with Metal, which needs no Screen
/// Recording permission and doesn't care whether the window can be seen, and
/// asks WebKit for each web view's picture.
#[cfg(feature = "render")]
pub(crate) fn start(window: &mut Window) -> Result<Frame, String> {
    let image = window.render_to_image().map_err(|error| format!("render_to_image: {error:#}"))?;
    #[cfg(target_os = "macos")]
    let snapshots = {
        let scale = window.scale_factor();
        let view = crate::mac::View::of(window)?;
        view.web_views().iter().map(|web| web.snapshot(scale)).collect()
    };
    Ok(Frame {
        image,
        #[cfg(target_os = "macos")]
        snapshots,
    })
}

#[cfg(not(feature = "render"))]
pub(crate) fn start(_: &mut Window) -> Result<Frame, String> {
    Err("the app was built without slate's `render` feature".into())
}

/// Waits for the web views' pictures, lays them over the frame and writes it
/// to `path` as a PNG. A picture that fails or never comes fails the whole
/// shot, so no shot is ever missing part of the window.
#[cfg(feature = "render")]
pub(crate) async fn finish(frame: Frame, path: &Path, cx: &mut AsyncApp) -> Result<(), String> {
    let Frame {
        mut image,
        #[cfg(target_os = "macos")]
        snapshots,
    } = frame;
    #[cfg(target_os = "macos")]
    for layer in pictures(snapshots, cx).await? {
        over(&mut image, &layer);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|error| format!("writing {}: {error}", path.display()))
}

#[cfg(not(feature = "render"))]
pub(crate) async fn finish(frame: Frame, _: &Path, _: &mut AsyncApp) -> Result<(), String> {
    match frame {}
}

/// Each snapshot's picture, in the order they were asked for.
#[cfg(all(feature = "render", target_os = "macos"))]
async fn pictures(snapshots: Vec<crate::snapshot::Snapshot>, cx: &mut AsyncApp) -> Result<Vec<Layer>, String> {
    let start = std::time::Instant::now();
    let mut layers: Vec<Option<Layer>> = snapshots.iter().map(|_| None).collect();
    loop {
        for (snapshot, layer) in snapshots.iter().zip(&mut layers) {
            if layer.is_none()
                && let Some(picture) = snapshot.take()
            {
                *layer = Some(picture.map_err(|problem| format!("a web view's picture: {problem}"))?);
            }
        }
        if layers.iter().all(Option::is_some) {
            return Ok(layers.into_iter().flatten().collect());
        }
        if start.elapsed() > SNAPSHOT_TIMEOUT {
            return Err(format!("WebKit didn't hand over a web view's picture within {SNAPSHOT_TIMEOUT:?}"));
        }
        cx.background_executor().timer(SNAPSHOT_POLL).await;
    }
}

/// A picture to lay over the frame: premultiplied sRGB RGBA rows from the
/// top, placed in device pixels from the window's top left. It may reach
/// past the window's edges.
#[cfg(feature = "render")]
pub(crate) struct Layer {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// How opaque the whole picture is, from 0 to 1.
    pub opacity: f32,
    pub pixels: Vec<u8>,
}

/// Lays `layer` over `image` ("source over"), keeping what shows through
/// its transparent parts and clipping it to the image.
#[cfg(feature = "render")]
pub(crate) fn over(image: &mut image::RgbaImage, layer: &Layer) {
    let (width, height) = (i64::from(image.width()), i64::from(image.height()));
    for row in 0..i64::from(layer.height) {
        let y = i64::from(layer.y) + row;
        if !(0..height).contains(&y) {
            continue;
        }
        for column in 0..i64::from(layer.width) {
            let x = i64::from(layer.x) + column;
            if !(0..width).contains(&x) {
                continue;
            }
            let at = ((row * i64::from(layer.width) + column) * 4) as usize;
            let Some(source) = layer.pixels.get(at..at + 4) else {
                return;
            };
            let pixel = image.get_pixel_mut(x as u32, y as u32);
            pixel.0 = blend(source, pixel.0, layer.opacity);
        }
    }
}

/// A premultiplied source pixel at `opacity` over a straight one.
#[cfg(feature = "render")]
fn blend(source: &[u8], under: [u8; 4], opacity: f32) -> [u8; 4] {
    let unit = |value: u8| f32::from(value) / 255.0;
    let source_alpha = unit(source[3]) * opacity;
    let under_alpha = unit(under[3]);
    let alpha = source_alpha + under_alpha * (1.0 - source_alpha);
    if alpha <= 0.0 {
        return [0; 4];
    }
    let channel = |index: usize| {
        let premultiplied = unit(source[index]) * opacity + unit(under[index]) * under_alpha * (1.0 - source_alpha);
        ((premultiplied / alpha).clamp(0.0, 1.0) * 255.0).round() as u8
    };
    [channel(0), channel(1), channel(2), (alpha * 255.0).round() as u8]
}

#[cfg(all(test, feature = "render"))]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn layer(x: i32, y: i32, width: u32, height: u32, pixel: [u8; 4]) -> Layer {
        Layer { x, y, width, height, opacity: 1.0, pixels: pixel.repeat((width * height) as usize) }
    }

    #[test]
    fn an_opaque_picture_replaces_what_is_under_it() {
        let mut image = RgbaImage::from_pixel(4, 4, Rgba([10, 20, 30, 255]));
        over(&mut image, &layer(1, 2, 2, 1, [200, 0, 100, 255]));
        assert_eq!(image.get_pixel(1, 2).0, [200, 0, 100, 255]);
        assert_eq!(image.get_pixel(2, 2).0, [200, 0, 100, 255]);
        assert_eq!(image.get_pixel(0, 2).0, [10, 20, 30, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [10, 20, 30, 255]);
        assert_eq!(image.get_pixel(3, 2).0, [10, 20, 30, 255]);
    }

    #[test]
    fn a_transparent_picture_lets_the_frame_show_through() {
        let mut image = RgbaImage::from_pixel(2, 2, Rgba([10, 20, 30, 255]));
        over(&mut image, &layer(0, 0, 2, 2, [0, 0, 0, 0]));
        assert_eq!(image.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn half_transparent_pixels_mix_with_the_frame() {
        let mut image = RgbaImage::from_pixel(1, 1, Rgba([0, 0, 200, 255]));
        // Premultiplied: red at half alpha.
        over(&mut image, &layer(0, 0, 1, 1, [128, 0, 0, 128]));
        assert_eq!(image.get_pixel(0, 0).0, [128, 0, 100, 255]);
    }

    #[test]
    fn the_views_own_alpha_fades_the_whole_picture() {
        let mut image = RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255]));
        let mut faded = layer(0, 0, 1, 1, [255, 255, 255, 255]);
        faded.opacity = 0.5;
        over(&mut image, &faded);
        assert_eq!(image.get_pixel(0, 0).0, [128, 128, 128, 255]);
    }

    #[test]
    fn a_picture_past_the_edges_is_clipped_to_the_window() {
        let mut image = RgbaImage::from_pixel(3, 3, Rgba([0, 0, 0, 255]));
        over(&mut image, &layer(-1, 2, 5, 4, [255, 255, 255, 255]));
        for x in 0..3 {
            assert_eq!(image.get_pixel(x, 2).0, [255, 255, 255, 255]);
            assert_eq!(image.get_pixel(x, 1).0, [0, 0, 0, 255]);
        }
    }
}
