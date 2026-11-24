//! Pictures of the web views an app lays over its GPUI view, for the
//! `render` feature. GPUI's `render_to_image` draws only GPUI's own scene, so
//! a `wry` WKWebView would come out as the bare background under it. WebKit
//! draws its own picture in the app's process with
//! `-[WKWebView takeSnapshotWithConfiguration:completionHandler:]`, which
//! needs no Screen Recording permission, and Slate lays it over GPUI's.
//!
//! The WebKit and CoreGraphics calls here are unsafe code, like `mac.rs`'s.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_void};
use std::rc::Rc;

use block2::RcBlock;
use objc::runtime::{BOOL, Class, Object, YES};
use objc::{class, msg_send, sel, sel_impl};

use crate::mac::{NSPoint, NSRect, NSSize, View};
use crate::render::Layer;

type Id = *mut Object;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: *const c_void;
    fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut c_void;
    fn CGContextSetInterpolationQuality(context: *mut c_void, quality: i32);
    fn CGContextDrawImage(context: *mut c_void, rect: NSRect, image: *mut c_void);
    fn CGContextRelease(context: *mut c_void);
}

// `kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big`: bytes R, G, B,
// A, with the colour already multiplied by the alpha.
const RGBA_PREMULTIPLIED: u32 = 1 | (4 << 12);
const INTERPOLATION_HIGH: i32 = 3;

/// A web view the app hosts, and where it shows in the window.
pub struct WebView {
    view: Id,
    /// In points from the window's top left.
    frame: NSRect,
    /// Its own alpha times its ancestors' up to GPUI's view.
    opacity: f32,
}

impl View {
    /// The web views laid over GPUI's view that show: not hidden, not
    /// fully transparent and not empty. They come in the order AppKit draws
    /// them, back first.
    pub fn web_views(self) -> Vec<WebView> {
        let Some(class) = Class::get("WKWebView") else {
            // WebKit isn't loaded, so the app has no web view.
            return Vec::new();
        };
        let mut found = Vec::new();
        collect(self, self.id(), class, &mut found);
        found
    }
}

fn collect(gpui: View, parent: Id, class: &Class, found: &mut Vec<WebView>) {
    // SAFETY: `parent` is a live view in GPUI's view's tree, on the main
    // thread; `subviews` is an NSArray of views, read within its count.
    unsafe {
        let subviews: Id = msg_send![parent, subviews];
        let count: usize = msg_send![subviews, count];
        for index in 0..count {
            let view: Id = msg_send![subviews, objectAtIndex: index];
            let hidden: BOOL = msg_send![view, isHiddenOrHasHiddenAncestor];
            if hidden == YES {
                continue;
            }
            let is_web: BOOL = msg_send![view, isKindOfClass: class];
            if is_web != YES {
                collect(gpui, view, class, found);
                continue;
            }
            if let Some(web) = web_view(gpui, view) {
                found.push(web);
            }
        }
    }
}

fn web_view(gpui: View, view: Id) -> Option<WebView> {
    // SAFETY: `view` is a live web view inside GPUI's view; these read
    // geometry and alpha, and walk superviews up to GPUI's view.
    unsafe {
        let bounds: NSRect = msg_send![view, bounds];
        let rect: NSRect = msg_send![view, convertRect: bounds toView: gpui.id()];
        let flipped: BOOL = msg_send![gpui.id(), isFlipped];
        let gpui_bounds: NSRect = msg_send![gpui.id(), bounds];
        let top = if flipped == YES {
            rect.origin.y - gpui_bounds.origin.y
        } else {
            gpui_bounds.origin.y + gpui_bounds.size.height - (rect.origin.y + rect.size.height)
        };
        let frame = NSRect { origin: NSPoint { x: rect.origin.x - gpui_bounds.origin.x, y: top }, size: rect.size };
        let mut opacity = 1.0;
        let mut at = view;
        while !at.is_null() && at != gpui.id() {
            let alpha: f64 = msg_send![at, alphaValue];
            opacity *= alpha;
            at = msg_send![at, superview];
        }
        let shows = opacity > 0.0 && frame.size.width > 0.0 && frame.size.height > 0.0;
        shows.then_some(WebView { view, frame, opacity: opacity as f32 })
    }
}

/// A web view's picture on its way: WebKit hands it over on the main thread
/// once it has drawn it.
pub struct Snapshot {
    result: Rc<RefCell<Option<Result<Layer, String>>>>,
}

impl Snapshot {
    /// The picture, once WebKit has handed it over.
    pub fn take(&self) -> Option<Result<Layer, String>> {
        self.result.borrow_mut().take()
    }
}

impl WebView {
    /// Asks WebKit for the web view's picture, at `scale` device pixels per
    /// point, placed where the view shows in the window.
    pub fn snapshot(&self, scale: f32) -> Snapshot {
        let result = Rc::new(RefCell::new(None));
        let scale = f64::from(scale);
        let left = (self.frame.origin.x * scale).round();
        let top = (self.frame.origin.y * scale).round();
        let width = ((self.frame.origin.x + self.frame.size.width) * scale).round() - left;
        let height = ((self.frame.origin.y + self.frame.size.height) * scale).round() - top;
        let place = Place { x: left as i32, y: top as i32, width: width as u32, height: height as u32 };
        let opacity = self.opacity;
        let handed = result.clone();
        let handler = RcBlock::new(move |image: *mut c_void, error: *mut c_void| {
            let (image, error) = (image.cast::<Object>(), error.cast::<Object>());
            let layer = if image.is_null() {
                Err(format!("WebKit gave no picture: {}", describe(error)))
            } else {
                pixels(image, place).map(|pixels| Layer {
                    x: place.x,
                    y: place.y,
                    width: place.width,
                    height: place.height,
                    opacity,
                    pixels,
                })
            };
            handed.replace(Some(layer));
        });
        // SAFETY: a live WKWebView on the main thread; WebKit copies the
        // handler block and calls it once, on the main thread, with an
        // NSImage or an NSError. The configuration is retained by `new` and
        // released after the call.
        unsafe {
            let config: Id = msg_send![class!(WKSnapshotConfiguration), new];
            let bounds: NSRect = msg_send![self.view, bounds];
            let _: () = msg_send![config, setRect: bounds];
            let width: Id = msg_send![class!(NSNumber), numberWithDouble: self.frame.size.width];
            let _: () = msg_send![config, setSnapshotWidth: width];
            let _: () = msg_send![config, setAfterScreenUpdates: YES];
            let handler = RcBlock::as_ptr(&handler).cast::<c_void>();
            let _: () = msg_send![self.view, takeSnapshotWithConfiguration: config completionHandler: handler];
            let _: () = msg_send![config, release];
        }
        Snapshot { result }
    }
}

/// Where a picture goes, in device pixels from the window's top left.
#[derive(Clone, Copy)]
struct Place {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// The NSImage's pixels at the place's size, as premultiplied sRGB RGBA rows
/// from the top.
fn pixels(image: Id, place: Place) -> Result<Vec<u8>, String> {
    let (width, height) = (place.width as usize, place.height as usize);
    let mut pixels = vec![0u8; width * height * 4];
    if pixels.is_empty() {
        return Ok(pixels);
    }
    // SAFETY: `image` is the live NSImage WebKit handed over. The bitmap
    // context draws into `pixels`, which outlives it and has exactly
    // `height` rows of `width * 4` bytes; the CGImage belongs to the NSImage.
    unsafe {
        let nil: Id = std::ptr::null_mut();
        let no_rect: *const NSRect = std::ptr::null();
        let cg_image: *mut c_void = msg_send![image, CGImageForProposedRect: no_rect context: nil hints: nil];
        if cg_image.is_null() {
            return Err("WebKit's picture has no bitmap".into());
        }
        let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        let context =
            CGBitmapContextCreate(pixels.as_mut_ptr().cast(), width, height, 8, width * 4, space, RGBA_PREMULTIPLIED);
        CGColorSpaceRelease(space);
        if context.is_null() {
            return Err(format!("couldn't make a {width}x{height} bitmap for WebKit's picture"));
        }
        CGContextSetInterpolationQuality(context, INTERPOLATION_HIGH);
        let all =
            NSRect { origin: NSPoint { x: 0.0, y: 0.0 }, size: NSSize { width: width as f64, height: height as f64 } };
        CGContextDrawImage(context, all, cg_image);
        CGContextRelease(context);
    }
    Ok(pixels)
}

fn describe(error: Id) -> String {
    if error.is_null() {
        return "no error either".into();
    }
    // SAFETY: `error` is a live NSError; its description is an NSString
    // whose UTF-8 copy lives as long as it.
    unsafe {
        let text: Id = msg_send![error, localizedDescription];
        if text.is_null() {
            return "an error with no description".into();
        }
        let utf8: *const c_char = msg_send![text, UTF8String];
        if utf8.is_null() { String::new() } else { CStr::from_ptr(utf8).to_string_lossy().into_owned() }
    }
}
