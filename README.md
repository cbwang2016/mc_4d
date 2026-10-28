# mc_4d

A Minecraft-like voxel sandbox in four spatial dimensions (X, Y, Z, W), written in Rust with `wgpu` and `winit`.

You see a 3D slice of the 4D world through your body. You can move along the 4th direction or rotate the slice into it, and the blocks (tesseracts) change shape as the slice cuts them differently. See [DESIGN.md](DESIGN.md) for how it works.

<video src="docs/intro.mp4" controls width="100%"></video>

**[▶ Watch the 51-second intro video](docs/intro.mp4)** (use this link if the player above doesn't show). It shows walking in the slice, stepping and turning into W, snapping back, a hollow tesseract seen from inside the 4th dimension, ghost layers and hyperspherical creatures. The keys being pressed are shown on screen.

## Running

```sh
cargo run --release
```

The first launch starts a tutorial, which you can replay any time from the pause menu (Esc → Tutorial).

Options (`--help` lists all of them):

| Option | Meaning |
|---|---|
| `--world NAME` | Load or create a world save (default `world`) |
| `--seed N` | Seed for a new world |
| `--creative` | Start a new world in Creative mode (flying, unlimited blocks) |
| `--data-dir DIR` | Keep settings, controls and saves under `DIR` |

Settings, controls and saves live in the platform config and data directories (on Linux, `~/.config/mc4d/` and `~/.local/share/mc4d/`).

## Default controls

Every binding can be changed under **Esc → Controls**. Click a binding and press a key, mouse button or wheel direction. Right-click a binding to clear it. Conflicting bindings are shown in red. Your bindings are saved to `controls.toml`.

| Input | Action |
|---|---|
| Mouse | Look (yaw / pitch) |
| W A S D | Move within the slice |
| Q / E | Move kata / ana (along the 4th direction) |
| Space / Left Shift | Jump / sneak (fly up / down in Creative) |
| Left Ctrl | Sprint |
| Mouse wheel or `[` / `]` | Turn forward into ana / kata |
| `;` / `'` | Swing right into kata / ana |
| Hold Left Alt + mouse | Rotate the slice freely |
| R | Snap the slice back onto the axes (keeps your facing direction) |
| Tab | Flip 90° into W |
| G | Ghost layers: red = just ana of your slice, blue = just kata |
| V | Side view: the perpendicular slice (bottom right) |
| B | 4D glasses: outlines of creatures in nearby slices |
| Left / right mouse button | Break / place block |
| Middle mouse button | Pick block |
| 1–9 | Hotbar slot |
| F | Toggle flying (Creative) |
| F1 / F2 / F3 | Hide HUD / screenshot / debug overlay |
| Enter | Continue the tutorial |
| Esc | Pause menu: options, controls, tutorial, game mode, save |

## HUD

- **Top left:** your XYZW position, the slice normal ("ana"), and how far the slice is tilted from the world axes.
- **Top right:** the 4D compass. Arrows show where world X, Z and W point on screen. Bars show how much of each axis points along ana, out of your slice.
- **Bottom right:** the side view, with ana pointing to the right.

## Development

```sh
cargo test --workspace          # core math, physics, terrain, input, tutorial, WGSL validation
cargo run -- --screenshot out.png --ana 30 --ghosts   # headless render to PNG
cargo run --release -- --bench 200                    # headless frame-time benchmark
cargo run -- --smoke-test 5 --data-dir /tmp/mc4d      # open the window briefly, screenshot, exit
cargo run --release -- --video docs/intro.mp4          # re-render the intro video (needs ffmpeg)
cargo run --release -- --video-stills /tmp/stills      # one preview still per shot of the video
```

The intro video is scripted in `crates/app/src/video.rs`: a list of shots with camera paths, captions and the keys shown on screen. It renders headlessly and pipes frames to ffmpeg (`--ffmpeg PATH` if it isn't on `PATH`). The committed file was then compressed:

```sh
ffmpeg -i intro.mp4 -c:v libx264 -preset veryslow -crf 32 -pix_fmt yuv420p -movflags +faststart docs/intro.mp4
```

Headless options: `--size WxH`, `--pos X,Y,Z,W`, `--yaw/--pitch/--ana/--right-ana DEG`, `--no-hud`, `--no-inset`, `--debug`, `--tutorial N`.

Crates:

- `crates/core` (`mc4d-core`): orientation math, chunks, terrain, 4D DDA ray-cast, physics and saving. It has no GPU code.
- `crates/render` (`mc4d-render`): the `wgpu` ray-caster (`shaders/raycast.wgsl`) and GPU chunk residency.
- `crates/app` (`mc4d`): the binary, with the winit loop, rebindable input, game logic, egui HUD and menus, and the tutorial.

The bundled DejaVu fonts (`crates/app/assets/fonts`) are under their own license; see `LICENSE-DejaVu`.
