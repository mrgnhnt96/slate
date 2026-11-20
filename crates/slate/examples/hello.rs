//! A tiny GPUI app wired up for Slate, used to prove it end to end:
//!
//! ```sh
//! cargo build -p slate --example hello --features render
//! cargo run -p slate-cli -- shoot --out /tmp/hello.png -- target/debug/examples/hello
//! ```
//!
//! What it draws changes after the window opens (a "loaded" card appears and
//! a counter ticks), so a shot that shows them proves the hidden window kept
//! drawing.

use std::time::Duration;

use gpui_kit::*;

struct Hello {
    loaded: bool,
    ticks: u32,
}

impl Hello {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let _ = this.update(cx, |this, cx| {
                this.loaded = true;
                cx.notify();
            });
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let ticked = this.update(cx, |this, cx| {
                    this.ticks += 1;
                    cx.notify();
                });
                if ticked.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self { loaded: false, ticks: 0 }
    }
}

fn card(color: u32, label: impl Into<SharedString>) -> Div {
    div()
        .flex_1()
        .h(px(120.))
        .rounded(px(12.))
        .bg(rgb(color))
        .p(px(16.))
        .text_color(rgb(0xffffff))
        .text_size(px(18.))
        .child(label.into())
}

impl Render for Hello {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let status = if self.loaded { card(0x2e7d32, "Loaded after open") } else { card(0x9e9e9e, "Loading…") };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(24.))
            .pt(px(48.))
            .bg(rgb(0xf5f1e8))
            .text_color(rgb(0x1d1b16))
            .child(div().text_size(px(28.)).child("Slate hello"))
            .child(div().text_size(px(16.)).child(format!("Ticks while hidden: {}", self.ticks)))
            .child(div().flex().gap(px(16.)).child(card(0x1565c0, "Blue")).child(card(0xc62828, "Red")).child(status))
            .child(
                div()
                    .flex_1()
                    .rounded(px(12.))
                    .border_2()
                    .border_color(rgb(0x1d1b16))
                    .p(px(16.))
                    .child("Every pixel here was drawn while the window sat below the desktop."),
            )
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
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(720.), px(480.)), cx))),
            titlebar: Some(TitlebarOptions {
                title: Some("Slate hello".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.), px(16.))),
            }),
            ..Default::default()
        };
        let options = slate::window_options(options, cx);
        cx.open_window(options, |window, cx| {
            slate::attach(window, cx);
            cx.new(Hello::new)
        })
        .expect("open the window");
        slate::activate(cx);
    });
}
