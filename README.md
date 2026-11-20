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
| `{"wait": 500}` | Waits, in milliseconds, for animations or loading. |
| `{"resize": "1024x700"}` | Resizes the window, in points. |
| `{"shot": "name"}` | Saves the window as `<out>/<name>.png`. Names are unique, with letters, digits, `-`, `_`, `.` and spaces. |

After each step Slate waits 50 ms and draws a frame, so the next click lands on what the window shows now.

For the steps to behave as they would for a person, GPUI has to treat the hidden window as the key window: focus listeners, text cursors and hover all wait for it. Slate gives that one window a subclass whose `isKeyWindow` answers yes and tells GPUI it became key. macOS itself never makes it key and never activates the app, so the app you're using keeps focus and your own keys never reach the hidden one.

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
- `SLATE_WAIT` after the window opens, Slate has GPUI draw and present a fresh frame (GPUI stops drawing an occluded window on its own), then takes the shot, or plays the steps and quits the app.

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
| `SLATE_OUT=<folder>` | command | Where the app writes `<name>.png` for each shot step (in device pixels) and `drive.json`. |

`shot.json` holds the window number (the CGWindowID), the scale factor, the size in points, and the rendered image's path or why there isn't one. `drive.json` holds how many steps played, each shot (the same fields plus its name) and the error that stopped the steps, if any. The types are in `slate::protocol` and `slate::steps`.

## Prove it

```sh
cargo test --workspace
scripts/prove.sh            # writes target/slate-proof/{render,screencapture}.png and drive*/
```

`scripts/prove.sh` shoots the example app in `crates/slate/examples/hello.rs` both ways, then drives it both ways with `crates/slate/examples/hello.steps.json`: it types into a text field, clicks a button twice, focuses a list and moves its selection with `j` and `k` (bound only in the list's key context), opens a palette with `cmd-k`, scrolls and resizes. While each run goes it asks System Events for the frontmost app every 100 ms, and fails if the example ever becomes frontmost or its window isn't below the desktop. It also fails if a shot in the drive looks just like the one before it. The example changes what it draws after it opens (a counter and a "Loaded" card), so the PNGs show the hidden window kept drawing, and its status line shows whether GPUI sees the window as active and how many focus events the list heard.
