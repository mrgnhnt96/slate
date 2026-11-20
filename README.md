# Slate

Run a GPUI app hidden and take screenshots of it, without the window showing up or taking focus.

Two parts: the `slate` crate an app calls around opening its main window, and the `slate` command that launches the app and collects the PNG. Built for kaepora, midna and taskboard (GPUI through `gpui-kit` 0.7). macOS only.

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

## Add Slate to an app

```toml
[dependencies]
slate = { git = "https://github.com/mrgnhnt96/slate", features = ["render"] }  # see "Two ways to capture"
```

```rust
slate::application().run(|cx| {                        // was gpui_kit::application()
    let options = slate::window_options(options, cx);  // no focus, SLATE_SIZE
    cx.open_window(options, |window, cx| {
        slate::attach(window, cx);                     // hide, then shoot
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
- `SLATE_WAIT` after the window opens, Slate has GPUI draw and present a fresh frame (GPUI stops drawing an occluded window on its own), then takes the shot.

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

`shot.json` holds the window number (the CGWindowID), the scale factor, the size in points, and the rendered image's path or why there isn't one. The types are in `slate::protocol`.

## Prove it

```sh
cargo test --workspace
scripts/prove.sh            # writes target/slate-proof/{render,screencapture}.png
```

`scripts/prove.sh` shoots the example app in `crates/slate/examples/hello.rs` both ways. While each shot runs it asks System Events for the frontmost app every 100 ms, and fails if the example ever becomes frontmost or its window isn't below the desktop. The example changes what it draws after it opens (a counter and a "Loaded" card), so the PNGs show the hidden window kept drawing.
