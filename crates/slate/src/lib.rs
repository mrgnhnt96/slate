//! Slate runs a GPUI app hidden, so an agent can screenshot it and drive it
//! with keys and clicks without a window ever showing on screen or taking
//! focus from the app in front.
//!
//! An app adds four calls around opening its main window. They do nothing
//! unless the app was launched with `SLATE=1` (the `slate` command does that):
//!
//! ```ignore
//! slate::application().run(|cx| {
//!     let options = slate::window_options(WindowOptions { ..Default::default() }, cx);
//!     cx.open_window(options, |window, cx| {
//!         slate::attach(window, cx);
//!         cx.new(|cx| MainWindow::new(window, cx))
//!     })
//!     .expect("open the main window");
//!     slate::activate(cx);
//! });
//! ```
//!
//! Under Slate the app is an accessory app (no Dock icon, never frontmost),
//! the window opens without focus at the size the command asked for, and it
//! sits one level below the desktop picture with clicks passing through it.
//! There it keeps drawing and can be captured, but is never seen.
//!
//! The capture: with the `render` feature the app renders its own window
//! with Metal (`Window::render_to_image`), which needs no Screen Recording
//! permission. Without it, the `slate` command captures the window with
//! `screencapture -l`.
//!
//! Driving: with `SLATE_STEPS` the app plays a steps file ([`steps`]) on the
//! hidden window itself, through GPUI's own keystroke and mouse dispatch,
//! saves each shot into `SLATE_OUT` and quits. So focus, text cursors and
//! hover behave, GPUI is told the window is key, though macOS never makes it
//! key.
//!
//! All the unsafe code (the AppKit calls) lives in this crate, so apps can
//! keep `unsafe_code = "forbid"`.

pub mod protocol;
pub mod steps;

pub use protocol::{Config, Shot, Size};

#[cfg(all(feature = "gpui", target_os = "macos"))]
mod mac;

#[cfg(feature = "gpui")]
mod app;

#[cfg(feature = "gpui")]
mod drive;

#[cfg(feature = "gpui")]
pub use app::{activate, application, attach, config, enabled, hidden, window_options};
