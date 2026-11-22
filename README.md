# Slate

Run a GPUI app hidden and take screenshots of it or drive it with keys and clicks, without the window showing up or taking focus.

Two parts: the `slate` crate an app calls around opening its main window, and the `slate` command that launches the app and collects the PNGs. Built for kaepora, midna and taskboard (GPUI through `gpui-kit` 0.7). macOS only.

## Take a screenshot

```sh
cargo install --path crates/slate-cli    # installs the `slate` command
slate shoot --out shot.png --size 1280x820 --wait 1500 -- path/to/app --its --args
```

| Option | Default | What it does |
| --- | --- | --- |
| `--out <png>` | required | Where the PNG goes, in points: a 1280x820 window gives a 1280x820 image. |
| `--size <WxH>` | the app's own size | The window size in points. |
| `--wait <ms>` | `1000` | Time from opening the window to the shot, for the app to load. |
| `--timeout <ms>` | `30000` | How long after the wait to give up on a shot. |

The command launches the app with `SLATE=1`, waits for the shot, scales it from device pixels back to points, writes the PNG and quits the app (SIGTERM, then a kill after 2 s). It prints the path, the size and how the window was captured.

## Drive it with keys and clicks

```sh
slate drive --steps steps.json --out shots --size 1280x820 -- path/to/app --its --args
```

A hidden window gets no key presses or clicks from macOS, so the app plays them itself from a steps file, saves a PNG for each `shot` step and quits. The command takes the same `--size`, `--wait` (time before the first step) and `--timeout` as `shoot`, checks the steps file before launching, writes each shot as `<out>/<name>.png` in points and fails with the step that went wrong.

```json
[
  { "click": { "x": 200, "y": 216 } },
  { "type": "Hello from Slate" },
  { "key": "cmd-k" },
  { "shot": "palette" }
]
```

| Step | What it does |
| --- | --- |
| `{"key": "cmd-k"}` | Presses keys, GPUI style (`cmd-shift-p`, `escape`, `j`). Several keys separated by spaces are pressed in turn: `"j j k"`. Runs the keymap in the focused element's key contexts, then types into the focused text field. |
| `{"type": "text"}` | Types the text one character at a time. |
| `{"click": {"x": 10, "y": 20}}` | Clicks at a point in the window, in points from its top left. Optional: `"button"` (`left`, `right`, `middle`), `"count"` (1 to 3, for double and triple clicks), `"modifiers"` (`"cmd-shift"`). The pointer stays there, so whatever is under it shows its hover. |
| `{"move": {"x": 10, "y": 20}}` | Moves the pointer, for hover. |
| `{"scroll": {"x": 10, "y": 20, "dy": 200}}` | Scrolls whatever is under the point. Positive `dy` scrolls down the page, positive `dx` right. |
| `{"drag": {"from": {"x": 10, "y": 20}, "to": {"x": 200, "y": 20}}}` | Presses the left button at `from`, moves the pointer to `to` with it held and releases it there. Optional: `"moves"` (1 to 100, default 10), how many pointer moves it takes, each drawn before the next so drag handlers follow it; `"modifiers"` held throughout. |
| `{"menu": "View > Hide Sidebar"}` | Chooses an item in the app's menu bar by the menu titles and its own, `>` between them (`"File > Open Recent > notes.md"`). Fails when the item is disabled, isn't there (the message lists what is) or is one of macOS's own items. |
| `{"menus": "name"}` | Saves the menu bar as it would show now as `<out>/<name>.json`: each menu's items with their titles, shortcuts as macOS writes them (`⇧⌘A`), `enabled`, `checked`, separators and submenus. |
| `{"wait": 500}` | Waits, in milliseconds, for animations or loading. |
| `{"resize": "1024x700"}` | Resizes the window, in points. It stops at the app's own minimum size, as a person dragging its corner would. |
| `{"shot": "name"}` | Saves the window as `<out>/<name>.png`. Names are unique, with letters, digits, `-`, `_`, `.` and spaces. |

After each step Slate waits 50 ms and draws a frame, so the next click lands on what the window shows now.

### Menus

An app under Slate never gets the menu bar on screen, since macOS gives it only to the app in front. Its menus are still built, though. Slate makes the hidden window the app's main window, which needs no activation, so GPUI sends menu choices to it and checks which items are enabled against it. A `menus` step updates each menu the way opening it does and reads the result. A `menu` step chooses an item the way a click in the menu bar does. Neither one ever opens a menu on screen.

### Web views

When an app lays a native view over its GPUI view, such as a `wry` WKWebView for HTML mail, GPUI never sees the clicks and keys that land on it. Slate looks up which view is under the point, as the window does. When that's the app's own native view, a `click`, `move` or `drag` goes to it as AppKit mouse events, and a click gives it the keyboard. While a native view has the keyboard, `key` and `type` go to it as AppKit key events for a US keyboard. A key with `cmd` or `ctrl` is tried as a key equivalent first, on the window's views and then on the menu bar. A click back on GPUI's view gives the keyboard back to GPUI. Only the left button clicks in a web view, since its context menu would stay open. `scroll` always goes to GPUI.

For the steps to behave as they would for a person, GPUI has to treat the hidden window as the key window: focus listeners, text cursors and hover all wait for it. Slate gives that one window a subclass whose `isKeyWindow` answers yes and tells GPUI it became key. macOS itself never makes it key and never activates the app, so the app you're using keeps focus and your own keys never reach the hidden one.

The same subclass says the window can be seen: its `occlusionState` answers visible. A web view stops drawing in a window macOS reports as covered, as one below the desktop is, so without this every shot after the first shows the web view blank.

## Add Slate to an app

```toml
[dependencies]
slate = { git = "https://github.com/mrgnhnt96/slate", features = ["render"] }  # see "Two ways to capture"
```

```rust
slate::application().run(|cx| {                        // was gpui_kit::application()
    let options = slate::window_options(options, cx);  // no focus, SLATE_SIZE
    cx.open_window(options, |window, cx| {
        slate::attach(window, cx);                     // hide, then shoot or drive
        cx.new(|cx| MainWindow::new(window, cx))
    })?;
    slate::activate(cx);                               // was cx.activate(true)
});
```

All four calls do nothing unless the app was launched with `SLATE=1`, so the app behaves as before when you run it yourself. `slate::enabled()` tells the app whether it's hidden. All the unsafe code (the AppKit calls) lives in this crate, so an app can keep `unsafe_code = "forbid"`.

Under Slate:

- The app starts as an accessory app: no Dock icon, no menu bar, never the frontmost app. `slate::hidden(app)` does the same for an `Application` built some other way.
- The main window opens without focus, at `SLATE_SIZE` when the command passes `--size`.
- The window sits one level below the desktop picture (`kCGDesktopWindowLevel - 1`, -2147483624), lets clicks pass through, is on every Space, and stays out of window cycling and the Window menu. Moving it off screen isn't enough: macOS clamps a titled window back until a strip of it shows.
- GPUI stops drawing an occluded window on its own, so Slate has it draw and present a frame as soon as the window opens and every 50 ms after that until `SLATE_WAIT` is up. A view the app builds while rendering, and whatever it loads after that, shows in the shot. Then Slate presents one more fresh frame and takes the shot from it, or plays the steps and quits the app.

## Two ways to capture

| | With the `render` feature | Without it |
| --- | --- | --- |
| Who captures | The app, with GPUI's `Window::render_to_image` (Metal) | The `slate` command, with `screencapture -x -o -l <window>` |
| Permission | None | Screen Recording for the terminal running `slate` |
| Window chrome | GPUI's pixels only: no traffic lights, square corners | Traffic lights and rounded corners included |
| Colours | The exact sRGB values the app draws | Converted to the display's colour profile |

`render` turns on GPUI's `test-support` feature in the app (Cargo features unify), which `render_to_image` needs. When the app can't render, the command falls back to `screencapture` and says why.

## What the command and the app say to each other

| Variable | Set by | Meaning |
| --- | --- | --- |
| `SLATE=1` | command | Run hidden. Anything other than `1` is off. |
| `SLATE_SIZE=WxH` | command | Window size in points. |
| `SLATE_WAIT=<ms>` | command | Time from opening the window to the shot. |
| `SLATE_DIR=<folder>` | command | Where the app writes `shot.json` (and `shot.png` when it renders). Without it the app runs hidden and takes no shot. |
| `SLATE_STEPS=<file>` | command | The steps file to play. Needs `SLATE_OUT`; takes the place of `SLATE_DIR`. |
| `SLATE_OUT=<folder>` | command | Where the app writes `<name>.png` for each shot step (in device pixels), `<name>.json` for each menus step and `drive.json`. |

`shot.json` holds the window number (the CGWindowID), the scale factor, the size in points, and the rendered image's path or why there isn't one. `drive.json` holds how many steps played, each shot (the same fields plus its name), the names of the menu bar records saved and the error that stopped the steps, if any. The types are in `slate::protocol` and `slate::steps`.

## Prove it

```sh
cargo test --workspace
scripts/prove.sh            # writes target/slate-proof/{render,screencapture}.png and drive*/
cargo test -p slate-cli --test shoot -- --ignored   # opens a hidden window
```

`scripts/prove.sh` shoots the example app in `crates/slate/examples/hello.rs` both ways, then drives it both ways with `crates/slate/examples/hello.steps.json`: it types into a text field, clicks a button twice, focuses a list and moves its selection with `j` and `k` (bound only in the list's key context), opens a palette with `cmd-k`, scrolls and resizes. While each run goes it asks System Events for the frontmost app every 100 ms, and fails if the example ever becomes frontmost or its window isn't below the desktop. It also fails if a shot in the drive looks just like the one before it, and runs the ignored `shoot` test on the rendering build, which fails unless the card built during render shows loaded. The example changes what it draws after it opens (a counter, a "Loaded" card, and a card whose view it builds while rendering once that card has loaded, which then loads 60 ms later), so the PNGs show the hidden window kept drawing, and its status line shows whether GPUI sees the window as active and how many focus events the list heard.
