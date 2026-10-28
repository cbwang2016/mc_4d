# mc_4d: Design

A Minecraft-like voxel sandbox in four spatial dimensions (X, Y, Z, W), written in Rust.

## 1. Core decisions

| Topic | Decision |
|---|---|
| Visualization | Primary view is a **3D slice** of the 4D world through the player, rendered with a normal perspective camera |
| Renderer | **GPU ray-casting** with a 4D DDA (Amanatides–Woo in 4 axes). No meshing, so any slice orientation works |
| Gravity | **Locked to world Y.** Y is never rotated out of the slice |
| Slice rotation | Continuous, with a snap key (and optional auto-snap) to the 24 axis-aligned orientations |
| Stack | Raw `wgpu` + `winit`, `glam` for math. No game engine |

Pinned versions (as of 2026-10): `wgpu 30`, `winit 0.30` (0.31 is still beta), `glam 0.34`, `bytemuck`, `pollster`, `noise` (4D OpenSimplex/Perlin), `serde` + `toml` + `directories` (settings), `egui` + `egui-wgpu` (menus, pinned to the release matching wgpu 30).

## 2. Coordinates and orientation

- World axes: **X, Z, W are horizontal, Y is up.**
- Block `(x,y,z,w) ∈ ℤ⁴` occupies the unit tesseract `[x,x+1)×…×[w,w+1)`.
- Player position: `Vec4` (f32 is fine for the prototype; switch to chunk-relative coordinates if precision becomes an issue).

### Camera frame

The camera frame is four orthonormal 4D vectors: `right`, `up`, `forward`, `ana`. `ana` is the slice normal, and the visible slice is the hyperplane `{p : (p − eye)·ana = 0}`.

The orientation state has two parts:

```
heading: Quat   // a 3D rotation acting on the horizontal space (X, Z, W)
pitch:   f32    // clamped to ±89°
```

To use the quaternion, embed the horizontal space as a 3D vector `h = (X, Z, W)`. Then:

```
right_h   = heading * (1,0,0)   → 4D (h.x, 0, h.y, h.z)
forward_h = heading * (0,1,0)   → 4D
ana       = heading * (0,0,1)   → 4D
forward   =  cos(pitch)·forward_h + sin(pitch)·Y
up        = −sin(pitch)·forward_h + cos(pitch)·Y
```

Two invariants follow from this construction (and get unit tests):

1. **Pitch never changes the slice.** `forward` and `up` both lie in span(forward_h, Y), and `ana` is independent of pitch.
2. **Up stays exactly vertical when pitch = 0, and the slice always contains Y.** `ana` has no Y component.

### Rotations

Every input rotation is a rotation of `heading` about one of its *local* axes. These are 3D axes in the quaternion's space: `Vec3::X` = right, `Vec3::Y` = forward, `Vec3::Z` = ana. They are not world axes.

| Input | Plane (camera frame) | Implementation |
|---|---|---|
| Mouse X | forward ↔ right (yaw) | `heading = heading * Quat::from_axis_angle(Vec3::Z, a)` |
| Mouse Y | forward ↔ up (pitch) | `pitch += a` (view only) |
| Mouse wheel | forward ↔ ana | `heading = heading * Quat::from_axis_angle(Vec3::X, a)` |
| Alt + mouse X | right ↔ ana | `heading = heading * Quat::from_axis_angle(Vec3::Y, a)` |
| Alt + mouse Y | forward ↔ ana | same as the wheel |

Renormalize `heading` every frame. All rotations pivot around the eye.

### Snapping

**R** aligns the *slice*, not the view. It turns `ana` onto the nearest signed world axis with the shortest rotation (`Quat::from_rotation_arc(ana, axis) * heading`), so the direction you face is kept as far as possible, and slerps there over about 150 ms. The "tilt" shown in the HUD is the angle between `ana` and that axis; plain yaw never tilts the slice. The 24 rotations of the cube in (X, Z, W) are the fully aligned orientations, used by the 90° flip. Optional auto-snap: when there has been no slice-rotation input for N seconds and the slice is within ~10° of aligned, ease into alignment.

**Tab** does an animated 90° rotation in the forward↔ana plane (look into the 4th dimension and back).

## 3. Controls

All bindings can be changed by the user (see 3.1). These are the defaults:

| Default input | Action |
|---|---|
| WASD | Move along forward_h / right_h (within the slice, horizontal) |
| Q / E | Move kata / ana (along `ana`, i.e. shift the slice) |
| Space / Shift | Jump / sneak (fly up / down in creative mode) |
| Mouse | Yaw / pitch |
| Wheel | Rotate forward ↔ ana |
| Alt + mouse | Rotate the slice (X: right ↔ ana, Y: forward ↔ ana) |
| R | Snap to the nearest axis-aligned orientation |
| Tab | 90° forward ↔ ana flip |
| LMB / RMB | Break / place block |
| F1 | Toggle HUD; F3 debug overlay |
| Esc | Pause menu (Options → Controls) |

### 3.1 Key rebinding (Minecraft-style)

Game code never reads raw keys. It reads **actions**. The input layer maps physical inputs to actions through a user-editable binding table.

- **`Action` enum.** Every bindable action, e.g. `MoveForward`, `MoveAna`, `Jump`, `SnapSlice`, `FlipAna`, `Break`, `Place`, `SliceRotateModifier` (the default Alt hold), `RotateAnaPos` / `RotateAnaNeg` (default wheel up / down), `ToggleHud` and so on. Each action has a stable string id (`"move.forward"`), a display name, and a category (Movement, 4D, Gameplay, UI) for grouping on the controls screen, like Minecraft's.
- **`Input` binding.** One of:
  - `Key(winit KeyCode)`: a *physical* key, so WASD stays in the same place on AZERTY/Dvorak keyboards. The label shown is the key's logical name on the current layout.
  - `Mouse(button)`
  - `WheelUp` / `WheelDown`
  - `Unbound`
- **Primary and secondary.** Each action holds one primary and one optional secondary binding. The secondary means every 4D rotation also gets a keyboard-only alternative, e.g. rotate forward↔ana on `[` / `]`, unbound by default.
- **Holds and presses.** Each action is either a *hold* (movement, the modifier) or a *press* (snap, flip, toggle). The input layer exposes `is_down(action)`, `just_pressed(action)` and `wheel_steps(action)` per frame.
- **Mouse look is not a binding.** It stays on the mouse axes. The controls screen adds these settings next to it:
  - mouse sensitivity
  - invert Y
  - slice-rotation sensitivity (wheel and modifier+mouse)
- **Controls screen.** It's in the pause menu.
  - Bindings are listed by category.
  - Click a binding, then press any key, mouse button or wheel direction to assign it. Esc cancels.
  - Bindings that conflict are shown in red, like Minecraft does. Conflicts are allowed, but warned about.
  - "Reset" is available per action, plus "Reset all".
- **Persistence.** Bindings are saved to `controls.toml` in the platform config directory (`directories` crate, serialized with `serde`). Actions are stored by id, so new actions added in later versions fall back to their defaults and unknown ids are ignored.

## 4. World data

- **Chunk:** 16⁴ blocks = 65,536 blocks, `u8` block id each, so 64 KiB per chunk. Key: `IVec4` chunk coordinate.
- **Uniform chunks:** chunks that are entirely one block type (all air, all stone) are stored as a single id and never uploaded. Most of the 4D world is like this.
- **Storage:** `HashMap<IVec4, Chunk>` on the CPU.
- **World height:** finite, Y ∈ [0, 256), i.e. 16 chunks.
- **Load set:** chunks whose center is within `R` of the eye *and* within the chunk half-diagonal (16 blocks) of the slice hyperplane, plus a margin of one chunk so rotation doesn't pop. Recompute the load set when the slice moves or rotates past a threshold.

### Terrain generation (on worker threads)

- Surface height: `h(x,z,w)` from 3D fractal noise over the horizontal coordinates.
- Caves: 4D noise threshold.
- Layers: grass, dirt, stone.
- Trees and ores later.

## 5. Renderer

### GPU data

- **Chunk pool:** one storage buffer holding `P` fixed slots of 16⁴ `u8` (packed 4 per `u32`). Starting value `P = 1024`, which is 64 MiB.
- **Indirection window:** an `N⁴` array of `u32` centered on the player's chunk (start with `N = 17`, about 84k entries). Each entry encodes:
  - "uniform block id `b`", or
  - "pool slot `s`".

  The window is toroidally addressed (chunk coordinate mod N), so moving only rewrites the entries that change.
- **Uniforms:** eye, the four camera basis vectors, fov, time, sun direction (4D).

### Primary pass (full-screen compute or fragment shader)

1. For pixel `(u,v)`, the ray direction is `normalize(forward + u·tan(fov/2)·aspect·right + v·tan(fov/2)·up)`. It lies in the slice by construction.
2. Run a 4D DDA through the indirection window:
   - Skip uniform-air chunks a whole chunk at a time.
   - Inside a stored chunk, step voxel by voxel.
   - A later optimization: a per-chunk 4⁴ occupancy bitmask for brick skipping.
3. Process the hit:
   - The face normal is the axis of the last step (one of ±X, ±Y, ±Z, ±W).
   - The hit point lies on a 3D cell of a tesseract. Texture coordinates are the three coordinates other than the normal axis, so the texture is 3D (procedural at first).
4. Shade:
   - Block color.
   - Per-normal shading, with W faces tinted distinctly so they're recognizable.
   - Edge darkening wherever two of the in-face coordinates are near integer boundaries. This keeps the cross-section shapes readable at odd angles.
   - Sun term. The 4D sun direction is projected into the slice, so lighting and shadows match what's visible.
   - Shadow ray toward the projected sun, plus ambient occlusion from the face's in-face neighbours.
5. Output color and depth. Depth is used later for compositing entities and ghosts.

### Aids for perceiving W (later milestones)

- **Ghost layers:** extra marches with the eye offset by `±δ·ana` (δ ≈ 1 block), at half resolution. Composite them translucently: red for ana, blue for kata, only where they differ from the main hit.
- **Perpendicular inset:** a small viewport rendered with the basis `(ana, up, forward)` substituted for `(right, up, forward)`. This shows a sideways look into W. It's nearly free with a ray-caster.
- **4D compass:** a HUD widget showing world X, Z and W projected into the camera's (right, forward, ana) frame, plus numeric XYZW coordinates.
- **Entity glasses:** entities within some W distance are drawn projected into the slice as outlines.

## 6. Gameplay systems

- **Player collider:** an axis-aligned 4D box of 0.6 × 1.8 × 0.6 × 0.6. It's world-aligned and never rotates, so rotating the slice can never push the player into a block.
- **Collision:** sweep and resolve one axis at a time (X, Y, Z, W).
- **Movement:** input builds a 4D velocity from forward_h, right_h, ana and Y. Gravity acts on Y.
- **Block targeting:** the same 4D DDA on the CPU along `forward` from the eye, with max distance 6.
  - Break removes the hit cell.
  - Place sets `hit_cell + normal`, after an overlap check against the player box.
  - Note: when the slice is rotated, the normal can be ±W, so placement can go "off-slice". That's correct behavior.
- **Edits:** update the CPU chunk, re-upload that slot, and update the indirection entry when a chunk changes between uniform and stored.

## 7. Crate layout

```
mc_4d/                 (cargo workspace)
├─ crates/core/        mc4d-core: no GPU. Math (orientation, snapping), world, chunks,
│                      terrain gen, 4D DDA, physics. All unit-tested.
├─ crates/render/      mc4d-render: wgpu setup, chunk pool and indirection upload,
│                      ray-cast pass, HUD pass. WGSL in crates/render/shaders/
└─ crates/app/         mc4d (binary): winit event loop, game loop, `input` module (Action enum, binding table, controls.toml load/save), menus
```

The 4D DDA exists twice, once in Rust (core) and once in WGSL. A test renders a few rays with both and compares the hit cells, so the two can't silently diverge.

## 8. Milestones

| # | Goal | Done when |
|---|---|---|
| M0 | Window, wgpu surface, full-screen shader | A gradient renders; resizing works |
| M1 | `core::camera`: orientation, rotations, snapping | Unit tests for orthonormality, the pitch/slice invariant, and snapping |
| M2 | One hard-coded chunk set (flat ground + a few test tesseracts), GPU DDA, free-fly camera with all controls. Input goes through the action layer from day one, with defaults hard-coded | You can fly around, rotate into W, and see cubes morph into cross-sections |
| M3 | Chunk pool, indirection window, streaming, terrain generation on worker threads | Infinite horizontal 4D terrain at 60 fps |
| M4 | Physics, gravity, collision, break/place, crosshair | Walking survival-style movement |
| M5 | HUD: 4D compass, coordinates, perpendicular inset, F3 overlay, pause menu with the Controls screen (rebinding, conflicts, reset, persistence) | Every action can be rebound and survives a restart |
| M6 | Ghost layers, sun shadows / AO, snap and auto-snap polish | |
| M7 | Entities (hypersphere "glooms"), hotbar inventory, save/load | |
| M8 | Guided tutorial (auto on first launch, replay from the pause menu) | |

Implementation notes on M7 and M8:
- **Saves:** only modified chunks are saved, as run-length encoded files `chunks/x_y_z_w.bin`. Everything else regenerates from the seed. `level.toml` holds the player state.
- **Tutorial:** each step's objective completes when the player actually does it, tracked through gameplay stats (distance moved along ana, angle turned into W, blocks placed, and so on). Explanatory steps advance with Enter. Key names in the text come from the current bindings.
