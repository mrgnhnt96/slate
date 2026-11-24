//! A GPUI window hosting a `wry` web view, the way kaepora-mac lays its
//! message pane over GPUI, used to prove the `render` feature draws web views
//! into the shot:
//!
//! ```sh
//! cargo build -p slate --example web --features render
//! cargo run -p slate-cli -- shoot --out /tmp/web.png -- target/debug/examples/web
//! cargo run -p slate-cli -- drive --steps crates/slate/examples/web.steps.json --out /tmp/web \
//!     -- target/debug/examples/web
//! ```
//!
//! The web view sits over a yellow GPUI box at [`WEB`]. Its page is
//! transparent, like kaepora's (`with_transparent`), with a purple block over
//! the top half, so a shot shows purple there and GPUI's yellow through the
//! bottom half. Clicking the block turns it teal. `render_to_image` alone
//! draws only GPUI's scene, so without the web view's picture the whole box
//! would be yellow.

use gpui_kit::*;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebViewBuilder};

/// Where the web view sits, in points from the window's top left.
const WEB: (f32, f32, f32, f32) = (400., 60., 320., 320.);

const PAGE: &str = r#"<!doctype html>
<html><body style="margin:0; background:transparent">
<div id="block" style="height:160px; background:#6a1b9a; color:#fff; font:20px -apple-system; padding:12px;
  box-sizing:border-box" onclick="this.style.background = '#00838f'; this.textContent = 'Clicked'">
  Drawn by WebKit</div>
</body></html>"#;

struct Web {
    _view: wry::WebView,
}

impl Render for Web {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (x, y, width, height) = WEB;
        div()
            .size_full()
            .relative()
            .p(px(24.))
            .pt(px(44.))
            .bg(rgb(0xf5f1e8))
            .text_color(rgb(0x1d1b16))
            .child(div().text_size(px(24.)).child("Slate web view"))
            .child(div().text_size(px(14.)).child("The box on the right is a WKWebView over GPUI."))
            .child(div().absolute().left(px(x)).top(px(y)).w(px(width)).h(px(height)).bg(rgb(0xfdd835)))
    }
}

fn main() {
    slate::application().run(|cx| {
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(760.), px(440.)), cx))),
            titlebar: Some(TitlebarOptions {
                title: Some("Slate web view".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.), px(16.))),
            }),
            ..Default::default()
        };
        let options = slate::window_options(options, cx);
        cx.open_window(options, |window, cx| {
            slate::attach(window, cx);
            let (x, y, width, height) = WEB;
            let view = WebViewBuilder::new()
                .with_bounds(Rect {
                    position: LogicalPosition::new(x, y).into(),
                    size: LogicalSize::new(width, height).into(),
                })
                .with_transparent(true)
                .with_focused(false)
                .with_accept_first_mouse(true)
                .with_html(PAGE)
                .build_as_child(window)
                .expect("build the web view");
            cx.new(|_| Web { _view: view })
        })
        .expect("open the window");
        slate::activate(cx);
    });
}
