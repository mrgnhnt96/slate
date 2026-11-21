//! A tiny GPUI app wired up for Slate, used to prove it end to end:
//!
//! ```sh
//! cargo build -p slate --example hello --features render
//! cargo run -p slate-cli -- shoot --out /tmp/hello.png -- target/debug/examples/hello
//! cargo run -p slate-cli -- drive --steps crates/slate/examples/hello.steps.json --out /tmp/hello \
//!     -- target/debug/examples/hello
//! ```
//!
//! What it draws changes after the window opens (a "loaded" card appears and
//! a counter ticks five times), so a shot that shows them proves the hidden window kept
//! drawing. A fourth card is a view built during the first render after the
//! card loads, which loads 60 ms later itself, so it only shows loaded if
//! frames kept coming during the wait. The bottom half is there to be driven: a text field, a button
//! with a hover colour, and a list that takes focus and moves its selection
//! with `j` and `k`, bound only in the list's key context. `cmd-k` opens a
//! palette from anywhere. The status line says whether GPUI sees the window
//! as active and how many times the list heard it gained focus.

use std::time::Duration;

use gpui_kit::base::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

actions!(hello, [TogglePalette, Next, Previous]);

const ROWS: usize = 24;
const TICKS: u32 = 5;

struct Hello {
    loaded: bool,
    ticks: u32,
    clicks: u32,
    palette: bool,
    selected: usize,
    list_focus: FocusHandle,
    list_focus_events: u32,
    input: Entity<InputState>,
    /// Built on the first render after loading, like a screen that makes its
    /// view lazily.
    lazy: Option<Entity<Lazy>>,
    _subscriptions: Vec<Subscription>,
}

impl Hello {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let _ = this.update(cx, |this, cx| {
                this.loaded = true;
                cx.notify();
            });
            // Five ticks, done before the first step, so steps are all that
            // change the picture after that.
            for _ in 0..TICKS {
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
        let list_focus = cx.focus_handle();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type here"));
        // Focus listeners only hear about focus in a window GPUI sees as active.
        let heard = cx.on_focus(&list_focus, window, |this: &mut Self, _, cx| {
            this.list_focus_events += 1;
            cx.notify();
        });
        Self {
            loaded: false,
            ticks: 0,
            clicks: 0,
            palette: false,
            selected: 0,
            list_focus,
            list_focus_events: 0,
            input,
            lazy: None,
            _subscriptions: vec![heard],
        }
    }

    fn list(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.list_focus.is_focused(window);
        div()
            .id("list")
            .key_context("List")
            .track_focus(&self.list_focus)
            .on_action(cx.listener(|this, _: &Next, _, cx| {
                this.selected = (this.selected + 1).min(ROWS - 1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Previous, _, cx| {
                this.selected = this.selected.saturating_sub(1);
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| window.focus(&this.list_focus, cx)))
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .rounded(px(8.))
            .border_2()
            .border_color(if focused { rgb(0x1565c0) } else { rgb(0xbdb6a6) })
            .bg(rgb(0xffffff))
            .children((0..ROWS).map(|row| {
                let selected = row == self.selected;
                div()
                    .px(px(12.))
                    .py(px(4.))
                    .when(selected, |row| row.bg(rgb(0x1565c0)).text_color(rgb(0xffffff)))
                    .child(format!("Row {}", row + 1))
            }))
    }
}

fn card(color: u32, label: impl Into<SharedString>) -> Div {
    div()
        .flex_1()
        .h(px(56.))
        .rounded(px(12.))
        .bg(rgb(color))
        .p(px(12.))
        .text_color(rgb(0xffffff))
        .text_size(px(16.))
        .child(label.into())
}

/// A view built during `Hello::render` once the app has loaded, which then
/// loads 60 ms later, the way a screen builds its view lazily and then fetches
/// its data. It's built on the first frame drawn after 300 ms and only loads
/// if the hidden window keeps drawing frames after that one.
struct Lazy {
    loaded: bool,
}

impl Lazy {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(60)).await;
            let _ = this.update(cx, |this, cx| {
                this.loaded = true;
                cx.notify();
            });
        })
        .detach();
        Self { loaded: false }
    }
}

/// The lazy card's colour once it has loaded, which the tests look for.
const LAZY_LOADED: u32 = 0x00838f;

impl Render for Lazy {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if self.loaded { card(LAZY_LOADED, "Built in render") } else { card(0x9e9e9e, "Building…") }
    }
}

impl Render for Hello {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Built on the first render after loading, as a screen the app opens
        // later builds its view lazily.
        let lazy = if self.loaded {
            self.lazy.get_or_insert_with(|| cx.new(Lazy::new)).clone().into_any_element()
        } else {
            card(0x9e9e9e, "Not built yet").into_any_element()
        };
        let status = if self.loaded { card(0x2e7d32, "Loaded after open") } else { card(0x9e9e9e, "Loading…") };
        let active = if window.is_window_active() { "yes" } else { "no" };
        let title = if self.palette { "Palette open (cmd-k)" } else { "Slate hello" };
        div()
            .key_context("Hello")
            .on_action(cx.listener(|this, _: &TogglePalette, _, cx| {
                this.palette = !this.palette;
                cx.notify();
            }))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(24.))
            .pt(px(44.))
            .bg(rgb(0xf5f1e8))
            .text_color(rgb(0x1d1b16))
            .child(div().text_size(px(24.)).when(self.palette, |title| title.text_color(rgb(0x6a1b9a))).child(title))
            .child(div().text_size(px(14.)).child(format!(
                "Ticks while hidden: {} · Window active: {active} · List focus events: {}",
                self.ticks, self.list_focus_events
            )))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(card(0x1565c0, "Blue"))
                    .child(card(0xc62828, "Red"))
                    .child(status)
                    .child(lazy),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .gap(px(12.))
                    .min_h(px(0.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(
                                div()
                                    .h(px(36.))
                                    .px(px(10.))
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(rgb(0xbdb6a6))
                                    .bg(rgb(0xffffff))
                                    .child(Input::new(&self.input)),
                            )
                            .child(
                                div()
                                    .id("button")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.clicks += 1;
                                        cx.notify();
                                    }))
                                    .rounded(px(8.))
                                    .bg(rgb(0x2e7d32))
                                    .hover(|button| button.bg(rgb(0xef6c00)))
                                    .px(px(16.))
                                    .py(px(10.))
                                    .text_color(rgb(0xffffff))
                                    .child(format!("Clicked {} times", self.clicks)),
                            )
                            .child(div().text_size(px(13.)).child("Every pixel here was drawn below the desktop.")),
                    )
                    .child(self.list(window, cx)),
            )
    }
}

fn main() {
    slate::application().run(|cx| {
        gpui_kit::base::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-k", TogglePalette, None),
            KeyBinding::new("j", Next, Some("List")),
            KeyBinding::new("k", Previous, Some("List")),
        ]);
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
            cx.new(|cx| Hello::new(window, cx))
        })
        .expect("open the window");
        slate::activate(cx);
    });
}
