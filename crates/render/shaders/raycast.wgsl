// 4D voxel ray-caster. Each pixel's ray lies in the 3D slice spanned by right/up/forward,
// and walks the 4D block grid with a 4-axis DDA (mirrors mc4d-core's raycast.rs).
//
// Coordinates are relative to the origin of the center chunk (`center * 16`) so f32 stays
// precise far from the world origin.

const MAX_ENTITIES: u32 = 32u;

const FLAG_GHOSTS: u32 = 1u;
const FLAG_INSET: u32 = 2u;
const FLAG_SHADOWS: u32 = 4u;
const FLAG_GLASSES: u32 = 8u;
const FLAG_EDGES: u32 = 16u;

const AIR: u32 = 0u;
const GRASS: u32 = 3u;
const LOG: u32 = 5u;
const LEAVES: u32 = 6u;
const PLANKS: u32 = 7u;
const COAL_ORE: u32 = 9u;
const GLASS: u32 = 10u;
const BRICK: u32 = 11u;
const WATER: u32 = 13u;
const BEDROCK: u32 = 14u;
const GLOW: u32 = 15u;

// Window entries: bit 31 set = dense chunk in pool slot (low 16 bits).
// Otherwise uniform chunk with block id in low 8 bits; bit 30 = not loaded yet.
const ENTRY_DENSE: u32 = 0x80000000u;
const ENTRY_UNLOADED: u32 = 0x40000000u;
const ENTRY_OUT: u32 = 0x20000000u;
const WINDOW_RADIUS: i32 = 7;
const CHUNK_WORDS: u32 = 16384u; // 16^4 bytes / 4

struct Globals {
    eye: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    ana: vec4<f32>,
    center: vec4<i32>,
    sun: vec4<f32>,
    sky_top: vec4<f32>,
    sky_horizon: vec4<f32>,
    target_cell: vec4<i32>,
    // x: has target, y: flags, z: entity count, w: frame
    info: vec4<u32>,
    // x,y: resolution, z: tan(fov/2), w: max distance
    view: vec4<f32>,
    // inset rect in pixels: x, y, w, h
    inset: vec4<f32>,
    // x: ghost offset, y: time, z: glasses range, w: unused
    misc: vec4<f32>,
    palette: array<vec4<f32>, 16>,
    // per entity: center (xyzw), then (radius, r, g, b)
    entities: array<vec4<f32>, 64>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> window: array<u32>;
@group(0) @binding(2) var<storage, read> pool: array<u32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn chunk_entry(gc: vec4<i32>) -> u32 {
    if gc.y < 0 {
        return BEDROCK;
    }
    if gc.y >= 16 {
        return AIR;
    }
    let d = abs(gc - g.center);
    if d.x > WINDOW_RADIUS || d.z > WINDOW_RADIUS || d.w > WINDOW_RADIUS {
        return ENTRY_OUT;
    }
    let idx = (gc.x & 15) + 16 * (gc.y + 16 * ((gc.z & 15) + 16 * (gc.w & 15)));
    return window[u32(idx)];
}

fn dense_block(slot: u32, local: vec4<i32>) -> u32 {
    let li = u32(local.x | (local.y << 4u) | (local.z << 8u) | (local.w << 12u));
    let word = pool[slot * CHUNK_WORDS + (li >> 2u)];
    return (word >> ((li & 3u) * 8u)) & 255u;
}

// Block at a cell (relative to the center chunk origin). Unloaded/out-of-window = air.
fn block_at(cell: vec4<i32>) -> u32 {
    let gc = (cell >> vec4<u32>(4u)) + g.center;
    let e = chunk_entry(gc);
    if (e & ENTRY_DENSE) != 0u {
        return dense_block(e & 0xFFFFu, cell & vec4<i32>(15));
    }
    if (e & (ENTRY_UNLOADED | ENTRY_OUT)) != 0u {
        return AIR;
    }
    return e & 255u;
}

struct Hit {
    hit: bool,
    t: f32,
    cell: vec4<i32>,
    axis: i32,
    // -step along axis: +1 means the face normal points along +axis
    nsign: f32,
    block: u32,
    // distance travelled through water before the hit
    water: f32,
    // number of glass blocks passed through
    glass: f32,
    // ray left the loaded window
    out: bool,
};

fn safe_inv(v: f32) -> f32 {
    if v == 0.0 {
        return 1e30;
    }
    return 1.0 / v;
}

fn min_axis(v: vec4<f32>) -> i32 {
    var a = 0;
    var m = v.x;
    if v.y < m { a = 1; m = v.y; }
    if v.z < m { a = 2; m = v.z; }
    if v.w < m { a = 3; }
    return a;
}

fn axis_vec(a: i32) -> vec4<f32> {
    var v = vec4<f32>(0.0);
    v[a] = 1.0;
    return v;
}

// Water and glass are passed through; the hit records how much of each was crossed.
fn trace(o: vec4<f32>, d: vec4<f32>, tmax: f32, max_steps: i32) -> Hit {
    var h: Hit;
    h.hit = false;
    h.t = tmax;
    h.water = 0.0;
    h.glass = 0.0;
    h.out = false;
    h.axis = -1;

    let stepv = vec4<i32>(sign(d));
    let inv = vec4<f32>(safe_inv(d.x), safe_inv(d.y), safe_inv(d.z), safe_inv(d.w));
    let tdelta = abs(inv);
    var cell = vec4<i32>(floor(o));
    var tnext = (vec4<f32>(cell + max(stepv, vec4<i32>(0))) - o) * inv;
    tnext = select(tnext, vec4<f32>(1e30), d == vec4<f32>(0.0));
    var t = 0.0;
    var axis = -1;
    var water_start = -1.0;

    for (var i = 0; i < max_steps; i++) {
        let gc = (cell >> vec4<u32>(4u)) + g.center;
        let e = chunk_entry(gc);
        if (e & ENTRY_OUT) != 0u {
            h.out = true;
            break;
        }
        var id = AIR;
        var skip_chunk = false;
        if (e & ENTRY_DENSE) != 0u {
            id = dense_block(e & 0xFFFFu, cell & vec4<i32>(15));
        } else if (e & ENTRY_UNLOADED) != 0u {
            skip_chunk = true;
        } else {
            id = e & 255u;
            skip_chunk = id == AIR;
        }

        if id == WATER {
            if water_start < 0.0 { water_start = t; }
        } else if water_start >= 0.0 {
            h.water += t - water_start;
            water_start = -1.0;
        }

        if id == GLASS {
            h.glass += 1.0;
        } else if id != AIR && id != WATER {
            h.hit = true;
            h.t = t;
            h.cell = cell;
            h.axis = axis;
            if axis >= 0 {
                h.nsign = -f32(stepv[axis]);
            }
            h.block = id;
            return h;
        }

        if skip_chunk {
            // Jump to where the ray leaves this chunk.
            let cmin = vec4<f32>((cell >> vec4<u32>(4u)) << vec4<u32>(4u));
            let bound = select(cmin, cmin + 16.0, d > vec4<f32>(0.0));
            var texit = (bound - o) * inv;
            texit = select(texit, vec4<f32>(1e30), d == vec4<f32>(0.0));
            axis = min_axis(texit);
            t = texit[axis];
            if t > tmax { break; }
            let p = o + d * t;
            cell = vec4<i32>(floor(p));
            // Put the exit axis exactly on the next cell, other axes from the position.
            cell[axis] = i32(bound[axis]) + min(stepv[axis], 0);
            tnext = (vec4<f32>(cell + max(stepv, vec4<i32>(0))) - o) * inv;
            tnext = select(tnext, vec4<f32>(1e30), d == vec4<f32>(0.0));
            continue;
        }

        axis = min_axis(tnext);
        t = tnext[axis];
        if t > tmax { break; }
        cell[axis] += stepv[axis];
        tnext[axis] += tdelta[axis];
    }
    if water_start >= 0.0 {
        h.water += min(t, tmax) - water_start;
    }
    h.t = min(t, tmax);
    return h;
}

fn hash3(p: vec4<i32>, salt: u32) -> f32 {
    var h = (u32(p.x) * 73856093u) ^ (u32(p.y) * 19349663u) ^ (u32(p.z) * 83492791u) ^ (u32(p.w) * 2654435761u) ^ (salt * 374761393u);
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xFFFFu) / 65535.0;
}

fn block_color(id: u32, cell: vec4<i32>, f: vec4<f32>, axis: i32, nsign: f32) -> vec3<f32> {
    var base = g.palette[id].rgb;
    // 8x8x8 texel pattern on the 3D face (the coordinate along the normal is constant).
    var q = vec4<i32>(floor(f * 8.0));
    if axis >= 0 { q[axis] = 0; }
    let n = hash3(q + cell * 8, id);
    var tex = 0.86 + 0.24 * n;

    if id == GRASS {
        let top = axis == 1 && nsign > 0.0;
        let side_band = axis != 1 && f.y > 0.78 - 0.1 * hash3(q, 9u);
        if !(top || side_band) {
            base = g.palette[2].rgb;
        }
    } else if id == LOG {
        if axis == 1 {
            // Rings on the cut ends.
            let r = length(vec3<f32>(f.x, f.z, f.w) - 0.5);
            tex *= 0.8 + 0.25 * step(0.5, fract(r * 6.0));
            base = g.palette[PLANKS].rgb * 0.9;
        } else {
            tex *= 0.75 + 0.3 * hash3(vec4<i32>(q.x + q.z * 3 + q.w * 7, 0, 0, 0), 4u);
        }
    } else if id == PLANKS {
        tex *= 0.85 + 0.15 * step(0.12, fract(f.y * 4.0));
    } else if id == BRICK {
        let row = floor(f.y * 4.0);
        let off = select(0.0, 0.5, i32(row) % 2 == 1);
        let mortar = step(fract(f.y * 4.0), 0.12) + step(fract(f.x * 2.0 + f.z * 2.0 + f.w * 2.0 + off), 0.08);
        base = mix(base, vec3<f32>(0.7, 0.68, 0.62), min(mortar, 1.0));
    } else if id == COAL_ORE {
        if n > 0.72 { base = vec3<f32>(0.05); }
    } else if id == LEAVES {
        tex = 0.6 + 0.6 * n;
    } else if id == BEDROCK {
        tex = 0.4 + 1.2 * n;
    }
    return base * tex;
}

fn face_shade(axis: i32, nsign: f32) -> vec3<f32> {
    if axis == 1 {
        return vec3<f32>(select(0.55, 1.0, nsign > 0.0));
    }
    if axis == 0 { return vec3<f32>(0.86); }
    if axis == 2 { return vec3<f32>(0.76); }
    if axis == 3 { return vec3<f32>(0.80, 0.74, 0.92); } // W faces get a violet cast
    return vec3<f32>(0.3);
}

fn sky(d: vec4<f32>, sun_slice: vec4<f32>) -> vec3<f32> {
    let k = clamp(d.y * 1.4 + 0.15, 0.0, 1.0);
    var c = mix(g.sky_horizon.rgb, g.sky_top.rgb, k);
    let s = max(dot(d, sun_slice), 0.0);
    c += vec3<f32>(1.0, 0.9, 0.7) * (pow(s, 600.0) * 4.0 + pow(s, 12.0) * 0.12);
    return c;
}

// Ambient occlusion from the 3D face neighbours (6 cells around the face, in the
// three in-face directions).
fn face_ao(cell: vec4<i32>, axis: i32, nsign: f32, f: vec4<f32>) -> f32 {
    var front = cell;
    front[axis] += i32(nsign);
    var ao = 1.0;
    for (var i = 0; i < 4; i++) {
        if i == axis { continue; }
        var lo = front;
        lo[i] -= 1;
        var hi = front;
        hi[i] += 1;
        let fi = f[i];
        if block_at(lo) != AIR && block_at(lo) != WATER {
            ao -= 0.22 * (1.0 - smoothstep(0.0, 0.5, fi));
        }
        if block_at(hi) != AIR && block_at(hi) != WATER {
            ao -= 0.22 * smoothstep(0.5, 1.0, fi);
        }
    }
    return max(ao, 0.35);
}

struct Camera {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    ana: vec4<f32>,
};

fn shade(eye: vec4<f32>, d: vec4<f32>, cam: Camera, px_angle: f32, main_view: bool) -> vec3<f32> {
    let flags = g.info.y;
    let max_dist = g.view.w;
    // Light from the sun projected into the slice, so shadows match what you see.
    var sun_slice = g.sun - cam.ana * dot(g.sun, cam.ana);
    sun_slice = normalize(sun_slice + vec4<f32>(0.0, 0.05, 0.0, 0.0));

    let h = trace(eye, d, max_dist, 768);
    var col: vec3<f32>;
    var t = h.t;
    if h.hit {
        let p = eye + d * h.t;
        let f = p - vec4<f32>(h.cell);
        col = block_color(h.block, h.cell, f, h.axis, h.nsign);
        if h.axis >= 0 {
            let n = axis_vec(h.axis) * h.nsign;
            var light = 0.5 + 0.5 * max(dot(n, sun_slice), 0.0);
            if (flags & FLAG_SHADOWS) != 0u && dot(n, sun_slice) > 0.0 {
                let sh = trace(p + n * 0.002, sun_slice, 48.0, 160);
                if sh.hit { light = 0.5; }
            }
            col *= face_shade(h.axis, h.nsign) * light * face_ao(h.cell, h.axis, h.nsign, f);

            // Cross-section edges: where an in-face coordinate crosses a block boundary.
            // A coordinate's rate of change inside the slice is sqrt(1 - ana_i²), so
            // coordinates that are constant in the slice never draw an edge.
            let width = max(px_angle * h.t * 1.1, 0.008);
            var edge = 1e9;
            for (var i = 0; i < 4; i++) {
                if i == h.axis { continue; }
                let grad = sqrt(max(1.0 - cam.ana[i] * cam.ana[i], 0.0));
                if grad < 0.02 { continue; }
                let dist = min(f[i], 1.0 - f[i]) / grad;
                edge = min(edge, dist);
            }
            let is_target = g.info.x == 1u && all(h.cell == g.target_cell) && main_view;
            if (flags & FLAG_EDGES) != 0u {
                col *= mix(0.62, 1.0, smoothstep(width * 0.5, width * 1.5, edge));
            }
            if is_target {
                col = mix(vec3<f32>(0.02), col * 1.15, smoothstep(width * 1.0, width * 2.5, edge));
            }
        } else {
            col *= 0.25; // camera inside a block
        }
        if h.block == GLOW {
            col = g.palette[GLOW].rgb * (0.9 + 0.2 * hash3(vec4<i32>(floor(f * 8.0)), 3u));
        }
    } else {
        col = sky(d, sun_slice);
    }

    // Water and glass tint what is behind them.
    if h.water > 0.0 {
        let a = 1.0 - exp(-h.water * 0.18);
        col = mix(col, g.palette[WATER].rgb * 0.8, a * 0.85 + 0.1);
    }
    if h.glass > 0.0 {
        col = mix(col, g.palette[GLASS].rgb, min(0.18 * h.glass, 0.6));
    }

    // Distance fog.
    if h.hit {
        let fog = clamp((t / max_dist - 0.55) / 0.45, 0.0, 1.0);
        col = mix(col, sky(d, sun_slice), fog * fog);
    }

    // Entities: 4D hyperspheres. A ray in the slice cuts a hypersphere in a sphere whose
    // radius shrinks as the center moves off the slice.
    let ne = min(g.info.z, MAX_ENTITIES);
    for (var e = 0u; e < ne; e++) {
        let c = g.entities[e * 2u];
        let r = g.entities[e * 2u + 1u].x;
        let ecol = g.entities[e * 2u + 1u].yzw;
        let oc = eye - c;
        let b = dot(oc, d);
        let off = dot(c - eye, cam.ana);
        let cc = dot(oc, oc) - r * r;
        let disc = b * b - cc;
        if disc > 0.0 {
            let te = -b - sqrt(disc);
            if te > 0.0 && te < t {
                let p = eye + d * te;
                let n = normalize(p - c);
                let lit = 0.45 + 0.55 * max(dot(n, sun_slice), 0.0);
                col = ecol * lit;
                t = te;
            }
        } else if (flags & FLAG_GLASSES) != 0u && abs(off) < g.misc.z && main_view {
            // 4D glasses: outline of the entity's shadow in the slice.
            let cp = c - cam.ana * off;
            let ocp = eye - cp;
            let bp = dot(ocp, d);
            let dist_line = sqrt(max(dot(ocp, ocp) - bp * bp, 0.0));
            let te = -bp;
            let w = px_angle * max(te, 0.1) * 2.0;
            if te > 0.0 && te < t && abs(dist_line - r) < w {
                let tint = select(vec3<f32>(0.3, 0.5, 1.0), vec3<f32>(1.0, 0.35, 0.3), off > 0.0);
                col = mix(col, tint, 0.8);
            }
        }
    }

    // Ghost layers: things in the neighbouring slices (±offset along ana) that are closer
    // than what is visible here. Red = ana side, blue = kata side.
    if (flags & FLAG_GHOSTS) != 0u && main_view {
        let go = g.misc.x;
        let ga = trace(eye + cam.ana * go, d, min(t, 48.0), 160);
        let gk = trace(eye - cam.ana * go, d, min(t, 48.0), 160);
        if ga.hit && ga.t < t - 0.05 {
            col = mix(col, vec3<f32>(1.0, 0.25, 0.2), 0.32 * (1.0 - ga.t / 48.0));
        }
        if gk.hit && gk.t < t - 0.05 {
            col = mix(col, vec3<f32>(0.2, 0.45, 1.0), 0.32 * (1.0 - gk.t / 48.0));
        }
    }
    return col;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = g.view.xy;
    let flags = g.info.y;
    var cam: Camera;
    cam.right = g.right;
    cam.up = g.up;
    cam.forward = g.forward;
    cam.ana = g.ana;
    var px = frag.xy;
    var size = res;
    var main_view = true;

    // Inset: a sideways look into W. The view's right axis becomes ana.
    let ins = g.inset;
    if (flags & FLAG_INSET) != 0u && px.x >= ins.x && px.y >= ins.y && px.x < ins.x + ins.z && px.y < ins.y + ins.w {
        let border = 2.0;
        if px.x < ins.x + border || px.y < ins.y + border || px.x >= ins.x + ins.z - border || px.y >= ins.y + ins.w - border {
            return vec4<f32>(0.9, 0.9, 0.95, 1.0);
        }
        cam.right = g.ana;
        cam.ana = -g.right;
        px = px - ins.xy;
        size = ins.zw;
        main_view = false;
    }

    let ndc = vec2<f32>((px.x + 0.5) / size.x * 2.0 - 1.0, 1.0 - (px.y + 0.5) / size.y * 2.0);
    let th = g.view.z;
    let aspect = size.x / size.y;
    let d = normalize(cam.forward + cam.right * (ndc.x * th * aspect) + cam.up * (ndc.y * th));
    let px_angle = 2.0 * th / size.y;
    let col = shade(g.eye, d, cam, px_angle, main_view);
    return vec4<f32>(col, 1.0);
}
