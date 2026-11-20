#!/usr/bin/env bash
# Proves Slate end to end on the example app, both ways it can capture:
#   1. the app renders its own window (slate's `render` feature)
#   2. `screencapture -l` (the app built without it)
# and then drives it with crates/slate/examples/hello.steps.json (typing,
# clicks, keys bound in a key context, cmd-k, scroll, resize), checking each
# step changed what the window shows. While each run goes, it asks System Events for the frontmost app every
# 100 ms and fails if the example ever becomes frontmost (it reports any other
# change, which comes from some other app), and it checks with CGWindowList
# that the window sits below the desktop.
#
#   scripts/prove.sh [out-dir]     (default: target/slate-proof)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/target/slate-proof}"
mkdir -p "$out"
cd "$root"

frontmost() {
  osascript -e 'tell application "System Events" to tell (first application process whose frontmost is true) to return (name as text) & " " & (unix id as text)'
}

# The example's window layer and whether it's on screen, from CGWindowList.
window_layer() {
  osascript -l JavaScript -e '
    ObjC.import("CoreGraphics");
    const list = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionAll, 0)));
    const own = list.filter(w => w.kCGWindowOwnerName === "hello" && w.kCGWindowBounds.Height > 100);
    own.map(w => `layer=${w.kCGWindowLayer} onscreen=${w.kCGWindowIsOnscreen ? 1 : 0}`).join("\n")'
}

# Runs a slate command (after the name) and checks the example never came to
# the front and its window sat below the desktop.
watched() {
  local name="$1"
  shift
  local log="$out/$name.frontmost.log"
  frontmost > "$log"
  local before
  before="$(head -1 "$log")"
  (
    while true; do
      frontmost >> "$log" 2>/dev/null || true
      sleep 0.1
    done
  ) &
  local watcher=$!
  (
    for _ in $(seq 1 25); do
      window_layer > "$out/$name.layer.log" 2>&1 || true
      grep -q layer= "$out/$name.layer.log" && break
      sleep 0.2
    done
  ) &
  local layer=$!
  "$@"
  wait "$layer"
  kill "$watcher" 2>/dev/null || true
  wait "$watcher" 2>/dev/null || true
  frontmost >> "$log"

  echo "  frontmost before: $before"
  echo "  frontmost seen:   $(sort "$log" | uniq -c | sed 's/^ *//' | paste -sd ',' - | sed 's/,/, /g')"
  echo "  window:           $(tr '\n' ' ' < "$out/$name.layer.log")"
  if grep -q '^hello ' "$log"; then
    echo "FAIL: the example became the frontmost app" >&2
    return 1
  fi
  if [ "$(sort -u "$log" | wc -l | tr -d ' ')" != 1 ]; then
    # Not the example (checked above): another app took focus on its own,
    # such as an app another agent launched without Slate.
    echo "  note: the frontmost app changed during the shot, but never to the example"
  fi
  if ! grep -q 'layer=-2147483624' "$out/$name.layer.log"; then
    echo "FAIL: the window wasn't below the desktop" >&2
    return 1
  fi
}

echo "building"
cargo build --quiet --release -p slate-cli
cargo build --quiet -p slate --example hello
cp target/debug/examples/hello "$out/hello-screencapture"
mkdir -p "$out/render"
cargo build --quiet -p slate --example hello --features render
cp target/debug/examples/hello "$out/render/hello"
mkdir -p "$out/screencapture"
mv "$out/hello-screencapture" "$out/screencapture/hello"

shoot() {
  watched "$1" target/release/slate shoot --out "$out/$1.png" --size 800x500 --wait 1500 -- "$2"
}

echo "render (the app renders its own window):"
shoot render "$out/render/hello"
echo "screencapture (built without the render feature):"
shoot screencapture "$out/screencapture/hello"

drive() {
  local name="$1" app="$2"
  watched "$name" target/release/slate drive --steps crates/slate/examples/hello.steps.json --out "$out/$name" \
    --size 800x500 --wait 1000 -- "$app"
  # The example stops changing on its own before the first step, so each
  # shot differs from the one before only by what the steps between did.
  local previous=start shot
  for shot in typed clicked list palette-and-scroll resized; do
    if cmp -s "$out/$name/$previous.png" "$out/$name/$shot.png"; then
      echo "FAIL: $name/$shot.png looks just like $previous.png, so the steps between them did nothing" >&2
      return 1
    fi
    previous=$shot
  done
}

echo "drive, rendered by the app (typing, clicks, keys and focus in the hidden window):"
drive drive "$out/render/hello"
echo "drive, with screencapture:"
drive drive-screencapture "$out/screencapture/hello"
echo "ok: $out/render.png, $out/screencapture.png and the shots in $out/drive and $out/drive-screencapture"
