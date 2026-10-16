# Linux animation flicker test

Checks that UI animations on Linux end cleanly. With WebKitGTK 2.54, the last frame of every
fade showed the element as it was before the fade started, for one frame, and then the final
state. For example, after a slider label faded to "Reset", "Contrast" flashed back once, and
after switching from Masks to Adjustments, the Masks panel reappeared for one frame.

The test runs RapidRAW in an off-screen Wayland compositor, hovers a slider label and switches
between the Adjustments and Masks panels with a virtual pointer, and records every frame
losslessly. It fails when a frame differs from both of its neighbours while the neighbours
match each other.

## Requirements

Linux with `sway`, `wayvnc`, `wf-recorder`, `grim`, `ffmpeg`, `dbus-run-session` and Node.js.
A GPU is used if available. Nothing is shown on screen and your own RapidRAW settings and
running RapidRAW are not touched: the test uses its own D-Bus session, settings and a
generated test image.

```bash
# Ubuntu / Debian
sudo apt install sway wayvnc wf-recorder grim ffmpeg dbus
# NixOS
nix shell nixpkgs#sway nixpkgs#wayvnc nixpkgs#wf-recorder nixpkgs#grim nixpkgs#ffmpeg
```

## Usage

Build RapidRAW (`npm run tauri build`, or `cargo build --release --features tauri/custom-protocol`
after `npm run build`), then run:

```bash
node tests/linux-animation-flicker/check-flicker.mjs src-tauri/target/release/RapidRAW
```

| Option         | Description                                       | Default |
| -------------- | ------------------------------------------------- | ------- |
| `--cycles <n>` | Times each animation is played in both directions | 5       |
| `--keep`       | Keep the recordings and logs even when it passes  | -       |

Exit status: 0 when no flicker is found, 1 when it is, 2 when the test could not run or the app
did not react to the virtual pointer (for example after a layout change moved the controls; their
positions are at the top of `check-flicker.mjs`). On failure the recordings and logs are kept and
their folder is printed.
