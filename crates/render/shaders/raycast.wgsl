// 4D voxel ray-caster. Each pixel's ray lies in the 3D slice spanned by right/up/forward,
// and walks the 4D block grid with a 4-axis DDA (mirrors mc4d-core's raycast.rs).
//
// Coordinates are relative to the origin of the center chunk (`center * 16`) so f32 stays
// precise far from the world origin.

const MAX_ENTITIES: u32 = 128u;

const FLAG_GHOSTS: u32 = 1u;
const FLAG_INSET: u32 = 2u;
const FLAG_SHADOWS: u32 = 4u;
const FLAG_GLASSES: u32 = 8u;
const FLAG_BEAM: u32 = 0x08000000u;
const FLAG_EDGES: u32 = 16u;
const FLAG_SMOOTH_LIGHTING: u32 = 32u;
const FLAG_FANCY: u32 = 64u;
const FLAG_ANAGLYPH: u32 = 128u;

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
const LAVA: u32 = 16u;
const CHEST: u32 = 19u;
const TNT: u32 = 62u;
const REDSTONE_WIRE: u32 = 69u;

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
    // atmosphere: daylight, precipitation, snow fraction, lightning flash
    inset: vec4<f32>,
    // x: ghost offset, y: time, z: glasses range, w: view count
    misc: vec4<f32>,
    palette: array<vec4<f32>, 256>,
    // per entity: center (xyzw), then (radius, r, g, b)
    entities: array<vec4<f32>, 768>,
    // Per view: right, up, forward, normal, pixel rectangle.
    cameras: array<vec4<f32>, 45>,
    weather_origin: vec4<i32>,
    gameplay: vec4<f32>,
    climate_origin: vec4<i32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> window: array<u32>;
@group(0) @binding(2) var<storage, read> pool: array<u32>;
@group(0) @binding(3) var<storage, read> occupancy: array<u32>;
@group(0) @binding(6) var<storage, read> state_metadata: array<u32>;
@group(0) @binding(10) var<storage, read> voxel_light: array<u32>;
@group(0) @binding(7) var precipitation_columns: texture_3d<i32>;
@group(0) @binding(15) var rain_brightness: texture_2d_array<f32>;
@group(0) @binding(16) var native_stars: texture_cube<f32>;
@group(0) @binding(8) var<storage, read> block_tiles: array<i32>;
struct BlockShapeBox { lo: vec4<f32>, hi: vec4<f32>, requirements: vec4<u32> };
struct BlockShapeData { headers: array<vec4<u32>, 8192>, boxes: array<BlockShapeBox> };
@group(0) @binding(9) var<storage, read> block_shapes: BlockShapeData;
@group(0) @binding(4) var classic_atlas: texture_2d_array<f32>;
@group(0) @binding(5) var pixel_sampler: sampler;
// One bit per 2x2x2x2 voxel group of each dense slot (any non-air block).
@group(0) @binding(17) var<storage, read> fine_occupancy: array<u32>;
// Per-tile safe ray start distances from the beam prepass (cs_beam).
@group(0) @binding(18) var<storage, read> ray_starts: array<f32>;
@group(0) @binding(19) var<storage, read_write> ray_starts_out: array<f32>;
const BEAM_TILE: u32 = 16u;
var<private> ray_start: f32;
// Ghost rays start from the eye offset along ana: a wider beam, earlier start.
var<private> ghost_start: f32;
@group(0) @binding(11) var sign_atlas: texture_2d_array<f32>;
@group(0) @binding(12) var biome_climate: texture_3d<f32>;
@group(0) @binding(13) var climate_sampler: sampler;

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

// Screen rectangles (NDC x0,y0,x1,y1) that may show an entity: the full shader,
// including the entity models, runs only there; elsewhere a variant without
// the entity code (half the registers) draws the frame.
@vertex
fn vs_rects(@builtin(vertex_index) i: u32, @location(0) ndc: vec4<f32>) -> VsOut {
    let corner = vec2<f32>(select(0.0, 1.0, i == 1u || i == 2u || i == 4u), select(0.0, 1.0, i == 2u || i == 4u || i == 5u));
    var o: VsOut;
    o.pos = vec4<f32>(mix(ndc.xy, ndc.zw, corner), 0.0, 1.0);
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

// The first 65536 words are a chunk-window indirection header. Slot 0 means the
// entire chunk has metadata zero; nonzero slots address optional packed planes.
fn block_metadata_at(cell: vec4<i32>) -> u32 {
    let gc = (cell >> vec4<u32>(4u)) + g.center;
    if gc.y < 0 || gc.y >= 16 { return 0u; }
    let entry = chunk_entry(gc);
    if (entry & (ENTRY_UNLOADED | ENTRY_OUT)) != 0u { return 0u; }
    let index = u32((gc.x & 15) + 16 * (gc.y + 16 * ((gc.z & 15) + 16 * (gc.w & 15))));
    let slot = state_metadata[index];
    if slot == 0u { return 0u; }
    let local = cell & vec4<i32>(15);
    let li = u32(local.x | (local.y << 4u) | (local.z << 8u) | (local.w << 12u));
    let word = state_metadata[65536u + (slot - 1u) * CHUNK_WORDS + (li >> 2u)];
    return (word >> ((li & 3u) * 8u)) & 255u;
}

fn voxel_light_at(cell: vec4<i32>) -> vec2<f32> {
    let gc = (cell >> vec4<u32>(4u)) + g.center;
    if gc.y < 0 { return vec2<f32>(0.0); }
    if gc.y >= 16 { return vec2<f32>(0.0, 15.0); }
    let geometry = chunk_entry(gc);
    if (geometry & (ENTRY_UNLOADED | ENTRY_OUT)) != 0u { return vec2<f32>(0.0); }
    let index = u32((gc.x & 15) + 16 * (gc.y + 16 * ((gc.z & 15) + 16 * (gc.w & 15))));
    let entry = voxel_light[index];
    var packed = entry & 255u;
    if (entry & 0x80000000u) != 0u {
        let local = cell & vec4<i32>(15);
        let li = u32(local.x | (local.y << 4u) | (local.z << 8u) | (local.w << 12u));
        let word = voxel_light[65536u + (entry & 0x3fffffffu) * CHUNK_WORDS + (li >> 2u)];
        packed = (word >> ((li & 3u) * 8u)) & 255u;
    }
    return vec2<f32>(f32(packed & 15u), f32(packed >> 4u));
}

// World.a(x,y,z,true) uses adjacent brightness for single slabs and farmland.
// Keep propagation data unchanged; this is the native render-time query only.
fn block_brightness_at(cell:vec4<i32>,id:u32)->vec2<f32>{
    var light=voxel_light_at(cell);
    if id==61u || id==24u {
        light=max(light,voxel_light_at(cell+vec4<i32>(0,1,0,0)));
        for(var axis=0;axis<4;axis++){
            if axis==1 {continue;}
            var offset=vec4<i32>(0);offset[axis]=1;
            light=max(light,voxel_light_at(cell+offset));
            light=max(light,voxel_light_at(cell-offset));
        }
    }
    return light;
}

// Beta1.5 WorldProvider.b brightness table. Day/night reduces sky light only;
// torch light remains independent of time, and lightning cannot light a cave
// whose propagated sky component is zero.
fn sky_dimming() -> f32 {
    let weather = 1.0 - clamp(g.inset.y, 0.0, 1.0) * 5.0 / 16.0;
    let daylight = clamp(g.inset.x, 0.0, 1.0) * weather * select(1.0, weather, (g.info.y & 256u) != 0u);
    return floor(11.0 * (1.0 - daylight) * (1.0 - clamp(g.inset.w, 0.0, 1.0)));
}

fn classic_brightness(levels: vec2<f32>) -> f32 {
    let level = clamp(max(levels.x, levels.y - sky_dimming()), 0.0, 15.0) / 15.0;
    return level / ((1.0 - level) * 3.0 + 1.0) * 0.95 + 0.05;
}

// Original fixed-function lighting multiplied encoded texture RGB. Apply that
// curve before converting back to the linear output expected by our sRGB target.
fn encoded_color(color: vec3<f32>) -> vec3<f32> {
    let c = max(color, vec3<f32>(0.0));
    return select(c * 12.92, 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c > vec3<f32>(0.0031308));
}
fn apply_classic_light(color: vec3<f32>, factor: vec3<f32>) -> vec3<f32> {
    let encoded = encoded_color(color);
    let lit = max(encoded * factor, vec3<f32>(0.0));
    return select(lit / 12.92, pow((lit + 0.055) / 1.055, vec3<f32>(2.4)), lit > vec3<f32>(0.04045));
}

fn macro_occupied(slot: u32, local: vec4<i32>, opaque_only: bool) -> bool {
    // The user can disable macrocell culling; differential tests independently
    // select the same reference traversal without changing other render options.
    if (g.info.y & 0xc0000000u) != 0u { return true; }
    let m = vec4<u32>(local) >> vec4<u32>(2u);
    let index = m.x | (m.y << 2u) | (m.z << 4u) | (m.w << 6u);
    let word = occupancy[slot * 16u + select(0u, 8u, opaque_only) + (index >> 5u)];
    return (word & (1u << (index & 31u))) != 0u;
}

fn fine_occupied(slot: u32, local: vec4<i32>) -> bool {
    if (g.info.y & 0xc0000000u) != 0u { return true; }
    let s = vec4<u32>(local) >> vec4<u32>(1u);
    let index = s.x | (s.y << 3u) | (s.z << 6u) | (s.w << 9u);
    return (fine_occupancy[slot * 128u + (index >> 5u)] & (1u << (index & 31u))) != 0u;
}

// Flow levels occupy separate storage IDs but use their source material palette.
fn material_id(id: u32) -> u32 {
    if id >= 32u && id <= 39u { return WATER; }
    if id >= 40u && id <= 47u { return LAVA; }
    return min(id, 255u);
}

fn fluid_height(raw: u32) -> f32 {
    if raw == WATER || raw == LAVA { return 8.0 / 9.0; }
    if raw >= 32u && raw <= 38u { return f32(39u - raw) / 9.0; }
    if raw >= 40u && raw <= 46u { return f32(47u - raw) / 9.0; }
    return 1.0;
}

// Block at a cell (relative to the center chunk origin). Unloaded/out-of-window = air.
fn block_at(cell: vec4<i32>) -> u32 {
    let gc = (cell >> vec4<u32>(4u)) + g.center;
    let e = chunk_entry(gc);
    if (e & ENTRY_DENSE) != 0u {
        return material_id(dense_block(e & 0xFFFFu, cell & vec4<i32>(15)));
    }
    if (e & (ENTRY_UNLOADED | ENTRY_OUT)) != 0u {
        return AIR;
    }
    return material_id(e & 255u);
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
    // First visible water/air interface; water remains transparent to terrain.
    water_t: f32,
    water_cell: vec4<i32>,
    water_axis: i32,
    water_sign: f32,
    water_raw: u32,
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

// Shared CPU/GPU boxes: ordinary cubes retain a cheap fast path; partial
// blocks use exact slab intersections in XYZW and metadata-dependent geometry.
fn render_metadata_at(id: u32, cell: vec4<i32>) -> u32 {
    var metadata = block_metadata_at(cell);
    if (u32(g.palette[id].w) & 16u) != 0u && (metadata & 8u) != 0u {
        // Upper door textures and geometry derive open/facing bits from the
        // lower half, as the original world-context texture method does.
        metadata = 8u | (block_metadata_at(cell - vec4<i32>(0, 1, 0, 0)) & 23u);
    }
    return metadata;
}

fn rail_code(id:u32,m:u32)->u32 {if id==76u {return m;}if (m&16u)!=0u {return m&127u;}return m&7u;}
fn rail_sample(id:u32,m:u32,tile:i32,point:vec4<f32>,normal:vec4<f32>)->vec4<f32> {
    let code=rail_code(id,m);var f=point;var n=normal;var curve=code;
    if code>=16u && code<=22u {f=point.xywz;n=normal.xywz;if code>=19u {curve=select(select(8u,7u,code==21u),select(9u,6u,code==19u),code<=20u);}}
    else if code>=23u {f=point.zywx;n=normal.zywx;curve=select(select(8u,7u,code==25u),select(9u,6u,code==23u),code<=24u);}
    if code>=1u && code<=3u {f=vec4<f32>(point.z,point.y,1.0-point.x,point.w);n=vec4<f32>(normal.z,normal.y,-normal.x,normal.w);}
    if curve>=6u && curve<=9u {for(var turn=6u;turn<curve;turn++){f=vec4<f32>(f.z,f.y,1.0-f.x,f.w);n=vec4<f32>(n.z,n.y,-n.x,n.w);}}
    return projected_tile(tile,f,1,n);
}
fn accepts_block_surface(id: u32, metadata: u32, cell: vec4<i32>, point: vec4<f32>, axis: i32, nsign: f32, slice_normal: vec4<f32>) -> bool {
    let flags = u32(g.palette[id].w);
    let cutout = (flags & 1u) != 0u || ((flags & 2u) != 0u && (g.info.y & FLAG_FANCY) != 0u);
    if !cutout || axis < 0 { return true; }
    if (flags&1024u)!=0u {return rail_sample(id,metadata,terrain_tile(id,metadata,1,1.0),point-vec4<f32>(cell),slice_normal).a>=0.5;}
    return projected_tile(terrain_tile(id, metadata, axis, nsign), point - vec4<f32>(cell), axis, slice_normal).a >= 0.5;
}

fn block_shape_hit(cell: vec4<i32>, id: u32, o: vec4<f32>, d: vec4<f32>, enter: f32, end: f32, enter_axis: i32, enter_sign: f32, slice_normal: vec4<f32>) -> vec3<f32> {
    let flags = u32(g.palette[id].w);
    if (flags&1024u)!=0u {
        let m=render_metadata_at(id,cell);let code=rail_code(id,m);var slope=0.0;var axis=0;var base=0.0625;
        if code==2u||code==3u {axis=0;slope=select(1.0,-1.0,code==3u);}
        if code==4u||code==5u {axis=2;slope=select(1.0,-1.0,code==4u);}
        if code==17u||code==18u {axis=3;slope=select(1.0,-1.0,code==18u);}
        if slope<0.0 {base+=1.0;}
        let denominator=d.y-slope*d[axis];if abs(denominator)<0.000001 {return vec3<f32>(1e30,0.0,0.0);}
        let f=o-vec4<f32>(cell);let t=(base-f.y+slope*f[axis])/denominator;
        if t>=max(0.0,enter-0.00001)&&t<=end+0.00001 && accepts_block_surface(id,m,cell,o+d*t,1,-sign(denominator),slice_normal){return vec3<f32>(t,1.0,-sign(denominator));}
        return vec3<f32>(1e30,0.0,0.0);
    }
    if (flags & 8u) != 0u {
        var metadata = 0u;
        if (flags & 3u) != 0u { metadata = render_metadata_at(id, cell); }
        if accepts_block_surface(id, metadata, cell, o + d * enter, enter_axis, enter_sign, slice_normal) {
            return vec3<f32>(enter, f32(enter_axis), enter_sign);
        }
        return vec3<f32>(1e30, 0.0, 0.0);
    }
    let metadata = render_metadata_at(id, cell);
    let header = block_shapes.headers[id * 32u + (metadata & 31u)];
    var connections = 0u;
    if (flags & 4u) != 0u {
        for (var bit = 0u; bit < 6u; bit++) {
            let axis = select(select(3, 2, bit < 4u), 0, bit < 2u);
            var neighbor = cell; neighbor[axis] += select(-1, 1, bit % 2u == 0u);
            if (u32(g.palette[block_at(neighbor)].w) & 4u) != 0u { connections |= 1u << bit; }
        }
    }
    let local_eye = o - vec4<f32>(cell);
    let inv = vec4<f32>(safe_inv(d.x), safe_inv(d.y), safe_inv(d.z), safe_inv(d.w));
    var best = vec3<f32>(1e30, 0.0, 0.0);
    for (var i = 0u; i < header.y; i++) {
        let box = block_shapes.boxes[header.x + i];
        if box.requirements.x != 0u && (connections & box.requirements.x) == 0u { continue; }
        if any((d == vec4<f32>(0.0)) & ((local_eye < box.lo) | (local_eye > box.hi))) { continue; }
        let a = (box.lo - local_eye) * inv; let b = (box.hi - local_eye) * inv;
        let near = select(min(a, b), vec4<f32>(-1e30), d == vec4<f32>(0.0));
        let far = select(max(a, b), vec4<f32>(1e30), d == vec4<f32>(0.0));
        var axis = min_axis(-near); var distance = near[axis]; var nsign = -sign(d[axis]);
        let exit = far[min_axis(far)];
        if exit < max(distance, enter) || distance > end { continue; }
        if distance < 0.00001 { axis = min_axis(far); distance = exit; nsign = sign(d[axis]); }
        if distance < enter - 0.00001 || distance > end || distance >= best.x { continue; }
        if accepts_block_surface(id, metadata, cell, o + d * distance, axis, nsign, slice_normal) {
            best = vec3<f32>(distance, f32(axis), nsign);
        }
    }
    return best;
}

// Nearby signs use their native oriented model and cached text. Skip the
// fallback voxel board only when that very sign survived entity visibility
// filtering; uncached/far signs never disappear merely because the cache is full.
fn has_sign_model(cell: vec4<i32>) -> bool {
    for (var e = 0u; e < min(g.info.z, MAX_ENTITIES); e++) {
        let kind = u32(g.entities[e * 6u + 5u].x);
        if (kind == 15u || kind == 16u) && all(vec4<i32>(floor(g.entities[e * 6u])) == cell) { return true; }
    }
    return false;
}

// Water and glass are passed through; the hit records how much of each was crossed.
// A macro exit may cross several integer planes at the same distance. Every
// negative-going tied axis belongs to the lower cell; adjusting only one axis
// lets subsequent jumps reset the other and ping-pong forever at the corner.
fn macro_exit_cell(p:vec4<f32>,d:vec4<f32>)->vec4<i32>{
    let integer=floor(p);var cell=vec4<i32>(integer);
    for(var axis=0;axis<4;axis++){
        if d[axis]<0.0 && p[axis]==integer[axis] {cell[axis]-=1;}
    }
    return cell;
}

fn trace(o: vec4<f32>, d: vec4<f32>, tmax: f32, max_steps: i32, slice_normal: vec4<f32>) -> Hit {
    var h: Hit;
    h.hit = false;
    h.t = tmax;
    h.water = 0.0;
    h.water_t = 1e30;
    h.water_axis = -1;
    h.water_sign = 1.0;
    h.water_cell = vec4<i32>(0);
    h.water_raw = WATER;
    h.glass = 0.0;
    h.out = false;
    h.axis = -1;

    let stepv = vec4<i32>(sign(d));
    let inv = vec4<f32>(safe_inv(d.x), safe_inv(d.y), safe_inv(d.z), safe_inv(d.w));
    let tdelta = abs(inv);
    // Start where the beam prepass proved the space empty (0 without it).
    var cell = vec4<i32>(floor(o + d * ray_start));
    var tnext = (vec4<f32>(cell + max(stepv, vec4<i32>(0))) - o) * inv;
    tnext = select(tnext, vec4<f32>(1e30), d == vec4<f32>(0.0));
    var t = ray_start;
    var axis = -1;
    var previous_water = false;
    var previous_water_cell = cell;
    var previous_water_raw = WATER;

    // Per-step lookups dominate traversal cost: reuse the chunk indirection
    // while inside one chunk, and the occupancy bit inside one macrocell.
    var cached_chunk = vec4<i32>(2147483647);
    var e = ENTRY_OUT;
    var cached_macro = vec4<i32>(2147483647);
    var macro_full = false;
    var cached_fine = vec4<i32>(2147483647);
    var fine_full = false;
    for (var i = 0; i < max_steps; i++) {
        let chunk_cell = cell >> vec4<u32>(4u);
        if any(chunk_cell != cached_chunk) {
            cached_chunk = chunk_cell;
            e = chunk_entry(chunk_cell + g.center);
            cached_macro = vec4<i32>(2147483647);
            cached_fine = vec4<i32>(2147483647);
        }
        if (e & ENTRY_OUT) != 0u {
            h.out = true;
            break;
        }
        var id = AIR;
        var skip_shift = 0u;
        if (e & ENTRY_DENSE) != 0u {
            let slot = e & 0xFFFFu;
            let local = cell & vec4<i32>(15);
            let macro_cell = local >> vec4<u32>(2u);
            if any(macro_cell != cached_macro) {
                cached_macro = macro_cell;
                macro_full = macro_occupied(slot, local, false);
            }
            let fine_cell = local >> vec4<u32>(1u);
            if macro_full && any(fine_cell != cached_fine) {
                cached_fine = fine_cell;
                fine_full = fine_occupied(slot, local);
            }
            if !macro_full {
                skip_shift = 2u;
            } else if !fine_full {
                // Empty 2^4 group inside a partly filled macrocell.
                skip_shift = 1u;
            } else {
                id = dense_block(slot, local);
            }
        } else if (e & ENTRY_UNLOADED) != 0u {
            skip_shift = 4u;
        } else {
            id = e & 255u;
            if id == AIR { skip_shift = 4u; }
        }

        if id != AIR && (u32(g.palette[material_id(id)].w) & 128u) != 0u && has_sign_model(cell) { id = AIR; }
        let raw_id = id;
        id = material_id(id);
        var fluid_enter = t;
        var fluid_exit = t;
        if id == WATER || id == LAVA {
            fluid_exit = min(tnext[min_axis(tnext)], tmax);
            let height = select(fluid_height(raw_id), 1.0, block_at(cell + vec4<i32>(0, 1, 0, 0)) == id);
            let surface_y = f32(cell.y) + height;
            if d.y > 0.0 {
                fluid_exit = min(fluid_exit, (surface_y - o.y) * inv.y);
            } else if d.y < 0.0 {
                fluid_enter = max(fluid_enter, (surface_y - o.y) * inv.y);
            } else if o.y >= surface_y {
                fluid_exit = fluid_enter;
            }
            if fluid_exit <= fluid_enter { id = AIR; }
        }
        var opaque_hit = vec3<f32>(t, f32(axis), -f32(stepv[max(axis, 0)]));
        if id != AIR && id != WATER && id != LAVA {
            opaque_hit = block_shape_hit(cell, id, o, d, t, min(tnext[min_axis(tnext)], tmax), axis, opaque_hit.z, slice_normal);
            if opaque_hit.x > 1e29 { id = AIR; }
        }
        if id == WATER {
            h.water += max(0.0, fluid_exit - fluid_enter);
            if h.water_t > 1e29 {
                if fluid_enter > 0.0001 && (fluid_enter > t + 0.00001 || !previous_water) {
                    h.water_t = fluid_enter; h.water_cell = cell; h.water_raw = raw_id;
                    h.water_axis = axis; h.water_sign = -f32(stepv[max(axis, 0)]);
                    if fluid_enter > t + 0.00001 { h.water_axis = 1; h.water_sign = 1.0; }
                } else if fluid_exit < min(tnext[min_axis(tnext)], tmax) - 0.00001 && fluid_exit > 0.0001 {
                    h.water_t = fluid_exit; h.water_cell = cell; h.water_raw = raw_id;
                    h.water_axis = 1; h.water_sign = 1.0;
                }
            }
            previous_water = fluid_exit >= tnext[min_axis(tnext)] - 0.00001;
            previous_water_cell = cell; previous_water_raw = raw_id;
        } else {
            if previous_water && h.water_t > 1e29 && t > 0.0001 && axis >= 0 {
                h.water_t = t; h.water_cell = previous_water_cell; h.water_raw = previous_water_raw;
                h.water_axis = axis; h.water_sign = f32(stepv[axis]);
            }
            previous_water = false;
        }

        if id != AIR && id != WATER {
            h.hit = true;
            h.t = opaque_hit.x;
            h.cell = cell;
            h.axis = i32(opaque_hit.y);
            h.nsign = opaque_hit.z;
            h.block = id;
            if id == LAVA && fluid_enter > t {
                h.t = fluid_enter;
                h.axis = 1;
                h.nsign = 1.0;
            }
            return h;
        }

        if skip_shift != 0u {
            // Jump out of an empty macrocell (4) or uniform chunk (16).
            let shift = vec4<u32>(skip_shift);
            let cmin = vec4<f32>((cell >> shift) << shift);
            let bound = select(cmin, cmin + f32(1u << skip_shift), d > vec4<f32>(0.0));
            var texit = (bound - o) * inv;
            texit = select(texit, vec4<f32>(1e30), d == vec4<f32>(0.0));
            axis = min_axis(texit);
            t = texit[axis];
            if t > tmax { break; }
            let p = o + d * t;
            cell = macro_exit_cell(p,d);
            // Keep the selected exit exact; all simultaneous negative exits were handled above.
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
    h.t = min(t, tmax);
    return h;
}

// Ghost overlays only need the first opaque hit distance, not a surface normal,
// material, or accumulated transparent medium. Cache the indirection entry while
// walking within a chunk and skip uniform transparent chunks in one step.
fn ghost_distance(o: vec4<f32>, d: vec4<f32>, tmax: f32, slice_normal: vec4<f32>, start: f32) -> f32 {
    let stepv = vec4<i32>(sign(d));
    let inv = vec4<f32>(safe_inv(d.x), safe_inv(d.y), safe_inv(d.z), safe_inv(d.w));
    let tdelta = abs(inv);
    var cell = vec4<i32>(floor(o + d * start));
    var tnext = (vec4<f32>(cell + max(stepv, vec4<i32>(0))) - o) * inv;
    tnext = select(tnext, vec4<f32>(1e30), d == vec4<f32>(0.0));
    var t = start;
    var crossing_axis = -1;
    var cached_chunk = vec4<i32>(2147483647);
    var entry = ENTRY_OUT;
    for (var i = 0; i < 160; i++) {
        let chunk = cell >> vec4<u32>(4u);
        if any(chunk != cached_chunk) {
            cached_chunk = chunk;
            entry = chunk_entry(chunk + g.center);
        }
        if (entry & ENTRY_OUT) != 0u { break; }
        var skip_shift = 0u;
        var raw_id = AIR;
        if (entry & ENTRY_DENSE) != 0u {
            let slot = entry & 0xFFFFu;
            let local = cell & vec4<i32>(15);
            if macro_occupied(slot, local, true) {
                raw_id = dense_block(slot, local);
            } else {
                skip_shift = 2u;
            }
        } else if (entry & ENTRY_UNLOADED) != 0u {
            skip_shift = 4u;
        } else {
            raw_id = entry & 255u;
            let id = material_id(raw_id);
            if id == AIR || id == WATER || id == GLASS { skip_shift = 4u; }
        }
        let id = material_id(raw_id);
        if id == LAVA {
            // Flowing lava occupies only the lower portion of its cell, exactly
            // as in the primary trace. Water is transparent to ghost occlusion.
            var enter = t;
            var exit = min(tnext[min_axis(tnext)], tmax);
            let height = select(fluid_height(raw_id), 1.0, block_at(cell + vec4<i32>(0, 1, 0, 0)) == LAVA);
            let surface_y = f32(cell.y) + height;
            if d.y > 0.0 {
                exit = min(exit, (surface_y - o.y) * inv.y);
            } else if d.y < 0.0 {
                enter = max(enter, (surface_y - o.y) * inv.y);
            } else if o.y >= surface_y {
                exit = enter;
            }
            if exit > enter { return enter; }
        } else if id != AIR && id != WATER && id != GLASS {
            let hit = block_shape_hit(cell, id, o, d, t, min(tnext[min_axis(tnext)], tmax), crossing_axis,
                -f32(stepv[max(crossing_axis, 0)]), slice_normal);
            if hit.x < 1e29 { return hit.x; }
        }
        if skip_shift != 0u {
            let shift = vec4<u32>(skip_shift);
            let cmin = vec4<f32>((cell >> shift) << shift);
            let bound = select(cmin, cmin + f32(1u << skip_shift), d > vec4<f32>(0.0));
            var texit = (bound - o) * inv;
            texit = select(texit, vec4<f32>(1e30), d == vec4<f32>(0.0));
            let axis = min_axis(texit);
            crossing_axis = axis;
            t = texit[axis];
            if t > tmax { break; }
            cell = macro_exit_cell(o+d*t,d);
            cell[axis] = i32(bound[axis]) + min(stepv[axis], 0);
            tnext = (vec4<f32>(cell + max(stepv, vec4<i32>(0))) - o) * inv;
            tnext = select(tnext, vec4<f32>(1e30), d == vec4<f32>(0.0));
        } else {
            let axis = min_axis(tnext);
            crossing_axis = axis;
            t = tnext[axis];
            if t > tmax { break; }
            cell[axis] += stepv[axis];
            tnext[axis] += tdelta[axis];
        }
    }
    return 1e30;
}

fn hash3(p: vec4<i32>, salt: u32) -> f32 {
    var h = (u32(p.x) * 73856093u) ^ (u32(p.y) * 19349663u) ^ (u32(p.z) * 83492791u) ^ (u32(p.w) * 2654435761u) ^ (salt * 374761393u);
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xFFFFu) / 65535.0;
}

// Exact original face/metadata tiles, including signed mirrored-U entries.
// W faces reuse north/south skins while keeping the continuous 4D projection.
fn terrain_tile(id: u32, raw_variant: u32, axis: i32, nsign: f32) -> i32 {
    var variant=raw_variant;
    if (id==51u||id==52u||id==76u)&&raw_variant>=16u {let code=rail_code(id,raw_variant);variant=select(select(0u,5u,code==17u),4u,code==18u);if code>=19u {variant=6u;}if id!=76u&&(raw_variant&128u)!=0u {variant|=8u;}}

    var face = 2u;
    if axis == 1 { face = select(0u, 1u, nsign > 0.0); }
    else if axis == 0 { face = select(4u, 5u, nsign > 0.0); }
    else if axis == 2 || axis == 3 { face = select(2u, 3u, nsign > 0.0); }
    let tile = block_tiles[min(id, 255u) * 96u + (variant & 15u) * 6u + face];
    // be.a(boolean): Fast selects the adjacent original opaque leaf tile.
    if id == LEAVES && (g.info.y & FLAG_FANCY) == 0u { return tile + 1; }
    return tile;
}

fn tile_color(tile: u32, uv: vec2<f32>, layer: i32) -> vec4<f32> {
    let texel = vec2<f32>(f32(tile % 16u), f32(tile / 16u)) * 16.0 + clamp(uv, vec2<f32>(0.0), vec2<f32>(0.999)) * 16.0;
    return textureSampleLevel(classic_atlas, pixel_sampler, (floor(texel) + 0.5) / 256.0, layer, 0.0);
}

// Rescale before taking fourth powers so near-face-parallel projections never
// underflow to black. Weights are continuous, nonnegative and sum to one.
fn texture_projection_weights(normal_components: vec3<f32>) -> vec3<f32> {
    let v = abs(normal_components);
    let scale = max(v.x, max(v.y, v.z));
    if scale == 0.0 { return vec3<f32>(1.0, 0.0, 0.0); }
    let scaled = v / scale;
    let square = scaled * scaled;
    let fourth = square * square;
    return fourth / (fourth.x + fourth.y + fourth.z);
}

fn projected_tile(tile: i32, f: vec4<f32>, axis: i32, slice_normal: vec4<f32>) -> vec4<f32> {
    var uv0 = vec2<f32>(f.x, 1.0 - f.y);
    var uv1 = vec2<f32>(f.w, 1.0 - f.y);
    var uv2 = uv0;
    var components = vec3<f32>(1.0, 0.0, 0.0);
    if axis == 1 {
        uv0 = f.xz; uv1 = f.xw; uv2 = f.zw;
        components = vec3<f32>(slice_normal.w, slice_normal.z, slice_normal.x);
    } else if axis == 0 {
        uv0 = vec2<f32>(f.z, 1.0 - f.y);
        components = vec3<f32>(slice_normal.w, slice_normal.z, 0.0);
    } else if axis == 2 {
        components = vec3<f32>(slice_normal.w, slice_normal.x, 0.0);
    } else if axis == 3 {
        uv1 = vec2<f32>(f.z, 1.0 - f.y);
        components = vec3<f32>(slice_normal.z, slice_normal.x, 0.0);
    }
    if tile < 0 { uv0.x = 1.0 - uv0.x; uv1.x = 1.0 - uv1.x; uv2.x = 1.0 - uv2.x; }
    let tile_index = u32(abs(tile));
    let weights = texture_projection_weights(components);
    var sampled = vec4<f32>(0.0);
    if weights.x > 0.0 { sampled += tile_color(tile_index, uv0, 0) * weights.x; }
    if weights.y > 0.0 { sampled += tile_color(tile_index, uv1, 0) * weights.y; }
    if weights.z > 0.0 { sampled += tile_color(tile_index, uv2, 0) * weights.z; }
    return sampled;
}

fn climate_tint(cell: vec4<i32>, foliage: bool) -> vec3<f32> {
    var climate = vec2<f32>(0.5);
    if g.climate_origin.w > 0 {
        let local = vec3<f32>(cell.xzw + g.center.xzw * 16 - g.climate_origin.xyz);
        let uv = (local / f32(g.climate_origin.w) + 0.5) / 33.0;
        climate = textureSampleLevel(biome_climate, climate_sampler, uv, 0.0).rg;
    }
    // Original ColorizerGrass/ColorizerFoliage: rainfall is multiplied by temperature.
    let pixel = vec2<i32>((vec2<f32>(1.0) - vec2<f32>(climate.x, climate.x * climate.y)) * 255.0);
    return encoded_color(textureLoad(classic_atlas, pixel, select(19, 20, foliage), 0).rgb);
}

// Beta e.a(wf,...): chest fronts are selected by neighboring opaque blocks,
// not placement metadata. Only the additional W pairing/front is a 4D extension.
fn chest_opaque(cell: vec4<i32>) -> bool {
    return (u32(g.palette[block_at(cell)].w) & 512u) != 0u;
}
fn chest_neighbor_count(cell: vec4<i32>) -> i32 {
    var count = 0;
    for(var axis=0;axis<4;axis++) {
        if axis==1 {continue;}
        for(var side=0;side<2;side++) {
            var offset=vec4<i32>(0);offset[axis]=select(-1,1,side==1);
            if block_at(cell+offset)==CHEST {count+=1;}
        }
    }
    return count;
}
fn chest_texture(cell: vec4<i32>, face_axis: i32, face_sign: f32) -> i32 {
    if face_axis==1 {return 25;}
    var pair=vec4<i32>(0);var pair_axis=-1;
    if chest_neighbor_count(cell)==1 {
        for(var axis=0;axis<4;axis++) {
            if axis==1 {continue;}
            for(var side=0;side<2;side++) {
                var offset=vec4<i32>(0);offset[axis]=select(-1,1,side==1);
                if block_at(cell+offset)==CHEST && chest_neighbor_count(cell+offset)==1 {pair=offset;pair_axis=axis;}
            }
        }
    }
    let side=select(-1,1,face_sign>0.0);
    if pair_axis>=0 {
        let front_axis=select(2,0,pair_axis==2);
        if face_axis!=front_axis {return 26;}
        var positive=vec4<i32>(0);positive[front_axis]=1;
        let negative_blocked=chest_opaque(cell-positive)||chest_opaque(cell+pair-positive);
        let positive_blocked=chest_opaque(cell+positive)||chest_opaque(cell+pair+positive);
        let front_sign=select(1,-1,positive_blocked && !negative_blocked);
        var half=select(0,-1,pair[pair_axis]<0);
        if (pair_axis==2 && side<0) || (pair_axis!=2 && side>0) {half=-1-half;}
        let tile=select(58,42,side==front_sign)+half;
        // Native opposite faces reverse U; retain the latch at the shared seam.
        return select(tile,-tile,(front_axis==0 && side>0)||(front_axis!=0 && side<0));
    }
    var front_axis=2;var front_sign=1;
    // Match the original Z-then-X priority; W is its direct additional axis.
    for(var n=0;n<3;n++) {
        let axis=select(select(3,0,n==1),2,n==0);
        var offset=vec4<i32>(0);offset[axis]=1;
        let negative=chest_opaque(cell-offset);let positive=chest_opaque(cell+offset);
        if negative && !positive {front_axis=axis;front_sign=1;}
        if positive && !negative {front_axis=axis;front_sign=-1;}
    }
    return select(26,27,face_axis==front_axis && side==front_sign);
}

fn block_color(id: u32, cell: vec4<i32>, f: vec4<f32>, axis: i32, nsign: f32, slice_normal: vec4<f32>) -> vec3<f32> {
    let metadata = render_metadata_at(id, cell);
    var tile = terrain_tile(id, metadata, axis, nsign);
    if id == CHEST {tile=chest_texture(cell,axis,nsign);}
    if id == GRASS && axis != 1 && (u32(g.palette[block_at(cell + vec4<i32>(0, 1, 0, 0))].w) & 32u) != 0u { tile = 68; }
    var sampled = projected_tile(tile, f, axis, slice_normal);
    if (u32(g.palette[id].w)&1024u)!=0u {sampled=rail_sample(id,metadata,tile,f,slice_normal);}
    var tint = vec3<f32>(1.0);
    if id == GRASS && axis == 1 && nsign > 0.0 { tint = climate_tint(cell, false); }
    if id == LEAVES {
        tint = climate_tint(cell, true);
        // Original species constants, io.a()/io.b(), not biome-colored spruce/birch.
        if (metadata & 1u) != 0u { tint = vec3<f32>(97.0, 153.0, 97.0) / 255.0; }
        else if (metadata & 2u) != 0u { tint = vec3<f32>(128.0, 167.0, 85.0) / 255.0; }
    }
    if id == REDSTONE_WIRE {
        let power = f32(metadata & 15u) / 15.0;
        // cn.e wire-surface tint; zero-power .3 differs from ri's particle color.
        tint = vec3<f32>(select(0.3, power*0.6+0.4, power>0.0),
            max(power*power*0.7-0.5,0.0),max(power*power*0.6-0.7,0.0));
    }
    return apply_classic_light(sampled.rgb, tint);
}

fn face_shade(axis: i32, nsign: f32) -> vec3<f32> {
    if axis == 1 {
        return vec3<f32>(select(0.5, 1.0, nsign > 0.0));
    }
    if axis == 0 { return vec3<f32>(0.6); }
    if axis == 2 || axis == 3 { return vec3<f32>(0.8); }
    return vec3<f32>(0.3);
}

// Fixed horizontal coordinates within each slice; default W-normal is native XYZ.
fn sky_direction(d:vec4<f32>,ana:vec4<f32>)->vec3<f32>{
    let n=normalize(ana.xzw);var r=vec3<f32>(1.0,0.0,0.0)-n*n.x;
    if dot(r,r)<0.0001 {r=vec3<f32>(0.0,1.0,0.0)-n*n.y;}
    r=normalize(r);let f=normalize(cross(n,r));return vec3<f32>(dot(d.xzw,r),d.y,dot(d.xzw,f));
}
fn celestial_sprite(d:vec3<f32>,normal:vec3<f32>,layer:i32,size:f32)->vec3<f32>{
    let depth=dot(d,normal);if depth<=0.0{return vec3<f32>(0.0);}
    var r=vec3<f32>(1.0,0.0,0.0)-normal*normal.x;
    if dot(r,r)<0.0001{r=vec3<f32>(0.0,0.0,1.0)-normal*normal.z;}
    r=normalize(r);let up=normalize(cross(normal,r));
    var uv=vec2<f32>(dot(d,r),-dot(d,up))/(depth*size*2.0)+vec2<f32>(0.5);
    if layer==25 {uv=vec2<f32>(1.0)-uv;}
    if any(uv<vec2<f32>(0.0))||any(uv>=vec2<f32>(1.0)){return vec3<f32>(0.0);}
    let tex=textureSampleLevel(classic_atlas,pixel_sampler,uv*0.125,layer,0.0);
    return encoded_color(tex.rgb)*tex.a;
}
// Analytic intersection of the original sixteen-triangle sunrise/sunset fan.
// Its ring is(radius120, vertical40*alpha), centered100 units toward the sun.
fn sunset_fan(d:vec3<f32>,sun:vec3<f32>)->vec4<f32>{
    if abs(g.sun.y)>0.4 || d.y<0.0 || dot(sun.xz,sun.xz)<0.00001{return vec4<f32>(0.0);}
    let f=g.sun.y/0.4*0.5+0.5;let a=pow(1.0-(1.0-sin(f*3.1415927))*0.99,2.0);
    let horizon=normalize(sun.xz);let side=dot(d.xz,vec2<f32>(horizon.y,-horizon.x));let front=dot(d.xz,horizon);
    let vertical=d.y*3.0/max(a,0.00001);
    if side*side+vertical*vertical<0.00000001 {return vec4<f32>(f*0.3+0.7,f*f*0.7+0.2,0.2,select(0.0,a,front>0.0));}
    let theta=atan2(side,vertical);
    let step=6.2831853/16.0;let sector=floor(theta/step)+0.5;
    let radius=120.0*cos(step*0.5)/cos(theta-sector*step);
    let toward=front-vertical;let denominator=100.0*length(vec2<f32>(side,vertical))+radius*toward;
    if denominator<=0.0{return vec4<f32>(0.0);}
    let center_weight=radius*toward/denominator;
    if center_weight<0.0||center_weight>1.0{return vec4<f32>(0.0);}
    return vec4<f32>(f*0.3+0.7,f*f*0.7+0.2,0.2,a*center_weight);
}
fn sky(d: vec4<f32>, sun_slice: vec4<f32>,ana:vec4<f32>) -> vec3<f32> {
    let k=clamp(d.y*1.4+0.15,0.0,1.0);var c=mix(g.sky_horizon.rgb,g.sky_top.rgb,k);
    let direction=sky_direction(d,ana);let sun=normalize(sky_direction(sun_slice,ana));
    let clear=1.0-g.inset.y;
    let celestial_offset=dot(g.sun,ana);
    let celestial_visibility=sqrt(max(0.0,1.0-celestial_offset*celestial_offset));
    var additive=celestial_sprite(direction,sun,24,0.3)*clear*celestial_visibility;
    additive+=celestial_sprite(direction,-sun,25,0.2)*clear*celestial_visibility;
    let native=vec3<f32>(direction.x,g.sun.y*direction.y+g.sun.z*direction.z,-g.sun.z*direction.y+g.sun.y*direction.z);
    let star_level=pow(clamp(0.25-g.sun.y*2.0,0.0,1.0),2.0)*0.5*clear;
    // Original blend is SRC_ALPHA,ONE and star RGBA is(level,level,level,level).
    if star_level>0.0 {additive+=vec3<f32>(textureSampleLevel(native_stars,pixel_sampler,native,0.0).r*star_level*star_level);}
    let sunset=sunset_fan(direction,sun);
    let encoded=mix(encoded_color(c),sunset.rgb,sunset.a)+additive;
    c=select(encoded/12.92,pow((encoded+0.055)/1.055,vec3<f32>(2.4)),encoded>vec3<f32>(0.04045));
    return c;
}

// Native cloud texture: Fast is the120.33 plane, Fancy has four-block
// thickness at108.33 and twelve-block texels. W translates the original sheet.
fn cloud_layer(eye:vec4<f32>,d:vec4<f32>,limit:f32)->vec4<f32>{
    let fancy=(g.info.y & FLAG_FANCY)!=0u;
    // Both native cloud repeats divide6144; wrap integer chunk origins before
    // f32 conversion to retain pixel motion far from the world origin.
    let world=eye+vec4<f32>(vec4<i32>(g.center.x%384,g.center.y,g.center.z%384,g.center.w%768))*16.0;
    let o=vec3<f32>(world.x+world.w*0.5+g.misc.y*0.6,world.y,world.z+world.w*0.5);
    let ray=vec3<f32>(d.x+d.w*0.5,d.y,d.z+d.w*0.5);
    var hit=1e30;var axis=1;var face_sign=select(-1.0,1.0,d.y<0.0);var uv=vec2<f32>(0.0);
    if !fancy {
        if abs(ray.y)<0.000001{return vec4<f32>(0.0);}
        hit=(120.33-o.y)/ray.y;if hit<=0.0||hit>=min(limit,512.0){return vec4<f32>(0.0);}
        uv=fract((o.xz+ray.xz*hit)/2048.0);
    } else {
        var enter=0.0;var exit=min(limit,512.0);
        if abs(ray.y)>0.000001 {
            let a=(108.33-o.y)/ray.y;let b=(112.33-o.y)/ray.y;
            enter=max(0.0,min(a,b));exit=min(exit,max(a,b));
        } else if o.y<108.33||o.y>112.33 {return vec4<f32>(0.0);}
        if enter>=exit{return vec4<f32>(0.0);}
        let p=o+ray*(enter+0.0001);var cell=vec2<i32>(floor((p.xz+vec2<f32>(0.0,3.96))/12.0));
        let direction=vec2<i32>(sign(ray.xz));let inv=vec2<f32>(safe_inv(ray.x),safe_inv(ray.z));
        var next=(vec2<f32>(cell+max(direction,vec2<i32>(0)))*12.0-vec2<f32>(0.0,3.96)-o.xz)*inv;
        next=select(next,vec2<f32>(1e30),abs(ray.xz)<vec2<f32>(0.000001));
        var t=enter;
        for(var i=0;i<96;i++){
            if t>=exit{break;}
            let texel=vec2<i32>((cell%vec2<i32>(256)+vec2<i32>(256))%vec2<i32>(256));
            if textureLoad(classic_atlas,texel,26,0).a>0.1 {hit=t;uv=(vec2<f32>(texel)+0.5)/256.0;break;}
            let side=select(0,1,next.y<next.x);t=next[side];cell[side]+=direction[side];next[side]+=abs(inv[side])*12.0;axis=select(0,2,side==1);face_sign=-f32(direction[side]);
        }
        if hit>1e29{return vec4<f32>(0.0);}
    }
    let tex=textureSampleLevel(classic_atlas,pixel_sampler,uv,26,0.0);
    var tint=vec3<f32>(0.1+0.9*g.inset.x,0.1+0.9*g.inset.x,0.15+0.85*g.inset.x);
    tint=mix(tint,vec3<f32>(dot(tint,vec3<f32>(0.3)))*0.6,g.inset.y);
    if fancy {tint*=select(select(0.9,0.8,axis==2),select(0.7,1.0,face_sign>0.0),axis==1);}
    let alpha=tex.a*0.8*(1.0-smoothstep(384.0,512.0,hit));
    return vec4<f32>(apply_classic_light(tex.rgb,tint),alpha);
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

// Minecraft-like skin boxes extended into W. Each part is a genuine 4D box;
// rotated slices intersect the complete multipart creature, not a billboard.
struct ModelPart {
    lo: vec4<f32>,
    hi: vec4<f32>,
    // Original skin pixel coordinates: u, v, width, height. Depth follows z.
    skin: vec4<f32>,
    depth: f32,
    rotated: bool,
    native: bool,
    pivot: vec3<f32>,
    angles: vec3<f32>,
    root_yaw: f32,
};

fn model_box(center: vec3<f32>, size: vec3<f32>, skin: vec2<f32>) -> ModelPart {
    let half = size / 32.0;
    let c = center / 16.0;
    let w = min(half.x, half.z);
    return ModelPart(vec4<f32>(c - half, -w), vec4<f32>(c + half, w), vec4<f32>(skin, size.xy), size.z, false, false, vec3<f32>(0.0), vec3<f32>(0.0), 0.0);
}

// Original ModelRenderer Euler order is Z * Y * X; rays move into each
// part's coordinate system, so rotated bodies/limbs retain exact silhouettes.
fn rotate_model(v: vec3<f32>, a: vec3<f32>) -> vec3<f32> {
    let x = vec3<f32>(v.x, cos(a.x)*v.y-sin(a.x)*v.z, sin(a.x)*v.y+cos(a.x)*v.z);
    let y = vec3<f32>(cos(a.y)*x.x+sin(a.y)*x.z, x.y, -sin(a.y)*x.x+cos(a.y)*x.z);
    return vec3<f32>(cos(a.z)*y.x-sin(a.z)*y.y, sin(a.z)*y.x+cos(a.z)*y.y, y.z);
}
fn unrotate_model(v: vec3<f32>, a: vec3<f32>) -> vec3<f32> {
    let z = vec3<f32>(cos(a.z)*v.x+sin(a.z)*v.y, -sin(a.z)*v.x+cos(a.z)*v.y, v.z);
    let y = vec3<f32>(cos(a.y)*z.x-sin(a.y)*z.z, z.y, sin(a.y)*z.x+cos(a.y)*z.z);
    return vec3<f32>(y.x, cos(a.x)*y.y+sin(a.x)*y.z, -sin(a.x)*y.y+cos(a.x)*y.z);
}
fn native_model_box(lo: vec3<f32>, hi: vec3<f32>, pivot: vec3<f32>, angles: vec3<f32>, uv: vec2<f32>, texture_size: vec3<f32>) -> ModelPart {
    let size = hi - lo;
    let w = min(size.x, size.z) / 32.0;
    return ModelPart(vec4<f32>(lo / 16.0, -w), vec4<f32>(hi / 16.0, w),
        vec4<f32>(uv, texture_size.xy), texture_size.z, false, true, pivot / 16.0, angles, 0.0);
}
fn vehicle_model_box(lo:vec3<f32>,hi:vec3<f32>,pivot:vec3<f32>,angles:vec3<f32>,uv:vec2<f32>,texture_size:vec3<f32>,w:f32)->ModelPart {
    var part=native_model_box(lo,hi,pivot,angles,uv,texture_size);
    part.root_yaw=1.5707964;part.lo.w=-w;part.hi.w=w;return part;
}
fn part_point(part: ModelPart, point: vec4<f32>) -> vec4<f32> {
    if !part.native { return point; }
    return vec4<f32>(unrotate_model(unrotate_model(vec3<f32>(point.x, 1.5-point.y, -point.z),vec3<f32>(0.0,part.root_yaw,0.0))-part.pivot, part.angles), point.w);
}
fn part_vector(part: ModelPart, vector: vec4<f32>) -> vec4<f32> {
    if !part.native { return vector; }
    return vec4<f32>(unrotate_model(unrotate_model(vector.xyz * vec3<f32>(1.0,-1.0,-1.0),vec3<f32>(0.0,part.root_yaw,0.0)), part.angles), vector.w);
}
fn part_normal(part: ModelPart, normal: vec4<f32>) -> vec4<f32> {
    if !part.native { return normal; }
    return vec4<f32>(rotate_model(rotate_model(normal.xyz, part.angles),vec3<f32>(0.0,part.root_yaw,0.0))*vec3<f32>(1.0,-1.0,-1.0), normal.w);
}

// Exact dd.a sheep dye colors. Apply only to the separate wool mesh.
fn sheep_wool_tint(color: u32) -> vec3<f32> {
    switch color & 15u {
        case 1u: { return vec3<f32>(0.95,0.7,0.2); }
        case 2u: { return vec3<f32>(0.9,0.5,0.85); }
        case 3u: { return vec3<f32>(0.6,0.7,0.95); }
        case 4u: { return vec3<f32>(0.9,0.9,0.2); }
        case 5u: { return vec3<f32>(0.5,0.8,0.1); }
        case 6u: { return vec3<f32>(0.95,0.7,0.8); }
        case 7u: { return vec3<f32>(0.3); }
        case 8u: { return vec3<f32>(0.6); }
        case 9u: { return vec3<f32>(0.3,0.6,0.7); }
        case 10u: { return vec3<f32>(0.7,0.4,0.9); }
        case 11u: { return vec3<f32>(0.2,0.4,0.8); }
        case 12u: { return vec3<f32>(0.5,0.4,0.3); }
        case 13u: { return vec3<f32>(0.4,0.5,0.2); }
        case 14u: { return vec3<f32>(0.8,0.3,0.3); }
        case 15u: { return vec3<f32>(0.1); }
        default: { return vec3<f32>(1.0); }
    }
}

// Generated from verified Beta1.5 constructor vertices, pivots, skin offsets and neutral poses.
fn classic_model_count(kind: u32) -> u32 {
    switch kind {
        case 1u: { return 6u; }
        case 2u: { return 9u; }
        case 3u: { return 12u; }
        case 4u: { return 8u; }
        case 5u: { return 9u; }
        case 6u: { return 6u; }
        case 7u: { return 6u; }
        case 8u: { return 11u; }
        case 9u: { return 6u; }
        case 10u: { return 1u; }
        case 11u: { return 11u; }
        case 18u: { return 5u; }
        case 19u: { return 6u; }
        default: { return 0u; }
    }
}
fn classic_model_part(kind: u32, part: u32, phase: f32, state: u32) -> ModelPart {
    switch kind {
        case 1u: {
            switch part {
                case 0u: { return native_model_box(vec3<f32>(-4.0, -4.0, -8.0), vec3<f32>(4.0, 4.0, 0.0), vec3<f32>(0.0, 12.0, -6.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(8.0, 8.0, 8.0)); }
                case 1u: { return native_model_box(vec3<f32>(-5.0, -10.0, -7.0), vec3<f32>(5.0, 6.0, 1.0), vec3<f32>(0.0, 11.0, 2.0), vec3<f32>(1.5707964, 0.0, 0.0), vec2<f32>(28, 8), vec3<f32>(10.0, 16.0, 8.0)); }
                case 2u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 6.0, 2.0), vec3<f32>(-3.0, 18.0, 7.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                case 3u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 6.0, 2.0), vec3<f32>(3.0, 18.0, 7.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                case 4u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 6.0, 2.0), vec3<f32>(-3.0, 18.0, -5.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                case 5u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 6.0, 2.0), vec3<f32>(3.0, 18.0, -5.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                default: {}
            }
        }
        case 2u: {
            switch part {
                case 0u: { return native_model_box(vec3<f32>(-2.0, -3.0, 0.0), vec3<f32>(2.0, 3.0, 2.0), vec3<f32>(0.0, 14.0, 6.0), vec3<f32>(1.5707964, 0.0, 0.0), vec2<f32>(52, 0), vec3<f32>(4.0, 6.0, 2.0)); }
                case 1u: { return native_model_box(vec3<f32>(-4.0, -5.0, -4.0), vec3<f32>(-3.0, -2.0, -3.0), vec3<f32>(0.0, 3.0, -7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(22, 0), vec3<f32>(1.0, 3.0, 1.0)); }
                case 2u: { return native_model_box(vec3<f32>(3.0, -5.0, -4.0), vec3<f32>(4.0, -2.0, -3.0), vec3<f32>(0.0, 3.0, -7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(22, 0), vec3<f32>(1.0, 3.0, 1.0)); }
                case 3u: { return native_model_box(vec3<f32>(-4.0, -4.0, -6.0), vec3<f32>(4.0, 4.0, 0.0), vec3<f32>(0.0, 4.0, -8.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(8.0, 8.0, 6.0)); }
                case 4u: { return native_model_box(vec3<f32>(-6.0, -10.0, -7.0), vec3<f32>(6.0, 8.0, 3.0), vec3<f32>(0.0, 5.0, 2.0), vec3<f32>(1.5707964, 0.0, 0.0), vec2<f32>(18, 4), vec3<f32>(12.0, 18.0, 10.0)); }
                case 5u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 12.0, 2.0), vec3<f32>(-4.0, 12.0, 7.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                case 6u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 12.0, 2.0), vec3<f32>(4.0, 12.0, 7.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                case 7u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 12.0, 2.0), vec3<f32>(-4.0, 12.0, -6.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                case 8u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 12.0, 2.0), vec3<f32>(4.0, 12.0, -6.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                default: {}
            }
        }
        case 3u: {
            switch part {
                case 0u: { return native_model_box(vec3<f32>(-3.0, -4.0, -6.0), vec3<f32>(3.0, 2.0, 2.0), vec3<f32>(0.0, 6.0, -8.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(6.0, 6.0, 8.0)); }
                case 6u: { return native_model_box(vec3<f32>(-3.5999999046325684, -4.599999904632568, -4.599999904632568), vec3<f32>(3.5999999046325684, 2.5999999046325684, 2.5999999046325684), vec3<f32>(0.0, 6.0, -8.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(6.0, 6.0, 6.0)); }
                case 1u: { return native_model_box(vec3<f32>(-4.0, -10.0, -7.0), vec3<f32>(4.0, 6.0, -1.0), vec3<f32>(0.0, 5.0, 2.0), vec3<f32>(1.5707964, 0.0, 0.0), vec2<f32>(28, 8), vec3<f32>(8.0, 16.0, 6.0)); }
                case 7u: { return native_model_box(vec3<f32>(-5.75, -11.75, -8.75), vec3<f32>(5.75, 7.75, 0.75), vec3<f32>(0.0, 5.0, 2.0), vec3<f32>(1.5707964, 0.0, 0.0), vec2<f32>(28, 8), vec3<f32>(8.0, 16.0, 6.0)); }
                case 2u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 12.0, 2.0), vec3<f32>(-3.0, 12.0, 7.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                case 8u: { return native_model_box(vec3<f32>(-2.5, -0.5, -2.5), vec3<f32>(2.5, 6.5, 2.5), vec3<f32>(-3.0, 12.0, 7.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                case 3u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 12.0, 2.0), vec3<f32>(3.0, 12.0, 7.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                case 9u: { return native_model_box(vec3<f32>(-2.5, -0.5, -2.5), vec3<f32>(2.5, 6.5, 2.5), vec3<f32>(3.0, 12.0, 7.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                case 4u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 12.0, 2.0), vec3<f32>(-3.0, 12.0, -5.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                case 10u: { return native_model_box(vec3<f32>(-2.5, -0.5, -2.5), vec3<f32>(2.5, 6.5, 2.5), vec3<f32>(-3.0, 12.0, -5.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                case 5u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 12.0, 2.0), vec3<f32>(3.0, 12.0, -5.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                case 11u: { return native_model_box(vec3<f32>(-2.5, -0.5, -2.5), vec3<f32>(2.5, 6.5, 2.5), vec3<f32>(3.0, 12.0, -5.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                default: {}
            }
        }
        case 4u: {
            switch part {
                case 0u: { return native_model_box(vec3<f32>(-2.0, -6.0, -2.0), vec3<f32>(2.0, 0.0, 1.0), vec3<f32>(0.0, 15.0, -4.0), vec3<f32>(-0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(4.0, 6.0, 3.0)); }
                case 1u: { return native_model_box(vec3<f32>(-3.0, -4.0, -3.0), vec3<f32>(3.0, 4.0, 3.0), vec3<f32>(0.0, 16.0, 0.0), vec3<f32>(1.5707964, 0.0, 0.0), vec2<f32>(0, 9), vec3<f32>(6.0, 8.0, 6.0)); }
                case 2u: { return native_model_box(vec3<f32>(-1.0, 0.0, -3.0), vec3<f32>(2.0, 5.0, 0.0), vec3<f32>(-2.0, 19.0, 1.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(26, 0), vec3<f32>(3.0, 5.0, 3.0)); }
                case 3u: { return native_model_box(vec3<f32>(-1.0, 0.0, -3.0), vec3<f32>(2.0, 5.0, 0.0), vec3<f32>(1.0, 19.0, 1.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(26, 0), vec3<f32>(3.0, 5.0, 3.0)); }
                case 4u: { return native_model_box(vec3<f32>(0.0, 0.0, -3.0), vec3<f32>(1.0, 4.0, 3.0), vec3<f32>(-4.0, 13.0, 0.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(24, 13), vec3<f32>(1.0, 4.0, 6.0)); }
                case 5u: { return native_model_box(vec3<f32>(-1.0, 0.0, -3.0), vec3<f32>(0.0, 4.0, 3.0), vec3<f32>(4.0, 13.0, 0.0), vec3<f32>(0.0, 0.0, -0.0), vec2<f32>(24, 13), vec3<f32>(1.0, 4.0, 6.0)); }
                case 6u: { return native_model_box(vec3<f32>(-2.0, -4.0, -4.0), vec3<f32>(2.0, -2.0, -2.0), vec3<f32>(0.0, 15.0, -4.0), vec3<f32>(-0.0, 0.0, 0.0), vec2<f32>(14, 0), vec3<f32>(4.0, 2.0, 2.0)); }
                case 7u: { return native_model_box(vec3<f32>(-1.0, -2.0, -3.0), vec3<f32>(1.0, 0.0, -1.0), vec3<f32>(0.0, 15.0, -4.0), vec3<f32>(-0.0, 0.0, 0.0), vec2<f32>(14, 4), vec3<f32>(2.0, 2.0, 2.0)); }
                default: {}
            }
        }
        case 5u: {
            switch part {
                case 0u: { return native_model_box(vec3<f32>(-6.0, -8.0, -6.0), vec3<f32>(6.0, 8.0, 6.0), vec3<f32>(0.0, 8.0, 0.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(12.0, 16.0, 12.0)); }
                case 1u: { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 18.0, 1.0), vec3<f32>(5.0, 15.0, 0.0), vec3<f32>(0.0, 1.5707964, 0.0), vec2<f32>(48, 0), vec3<f32>(2.0, 18.0, 2.0)); }
                case 2u: { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 18.0, 1.0), vec3<f32>(3.535534, 15.0, 3.535534), vec3<f32>(0.0, 0.7853982, 0.0), vec2<f32>(48, 0), vec3<f32>(2.0, 18.0, 2.0)); }
                case 3u: { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 18.0, 1.0), vec3<f32>(3.061617e-16, 15.0, 5.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(48, 0), vec3<f32>(2.0, 18.0, 2.0)); }
                case 4u: { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 18.0, 1.0), vec3<f32>(-3.535534, 15.0, 3.535534), vec3<f32>(0.0, -0.7853982, 0.0), vec2<f32>(48, 0), vec3<f32>(2.0, 18.0, 2.0)); }
                case 5u: { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 18.0, 1.0), vec3<f32>(-5.0, 15.0, 6.123234e-16), vec3<f32>(0.0, -1.5707964, 0.0), vec2<f32>(48, 0), vec3<f32>(2.0, 18.0, 2.0)); }
                case 6u: { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 18.0, 1.0), vec3<f32>(-3.535534, 15.0, -3.535534), vec3<f32>(0.0, -2.3561945, 0.0), vec2<f32>(48, 0), vec3<f32>(2.0, 18.0, 2.0)); }
                case 7u: { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 18.0, 1.0), vec3<f32>(-9.18485e-16, 15.0, -5.0), vec3<f32>(0.0, -3.1415927, 0.0), vec2<f32>(48, 0), vec3<f32>(2.0, 18.0, 2.0)); }
                case 8u: { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 18.0, 1.0), vec3<f32>(3.535534, 15.0, -3.535534), vec3<f32>(0.0, -3.9269907, 0.0), vec2<f32>(48, 0), vec3<f32>(2.0, 18.0, 2.0)); }
                default: {}
            }
        }
        case 6u: {
            switch part {
                case 0u: { return native_model_box(vec3<f32>(-4.0, -8.0, -4.0), vec3<f32>(4.0, 0.0, 4.0), vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(8.0, 8.0, 8.0)); }
                case 1u: { return native_model_box(vec3<f32>(-4.0, 0.0, -2.0), vec3<f32>(4.0, 12.0, 2.0), vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(16, 16), vec3<f32>(8.0, 12.0, 4.0)); }
                case 2u: { return native_model_box(vec3<f32>(-3.0, -2.0, -2.0), vec3<f32>(1.0, 10.0, 2.0), vec3<f32>(-5.0, 2.0, 0.0), vec3<f32>(-1.5707964, -0.1, 0.1), vec2<f32>(40, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                case 3u: { return native_model_box(vec3<f32>(-1.0, -2.0, -2.0), vec3<f32>(3.0, 10.0, 2.0), vec3<f32>(5.0, 2.0, -0.0), vec3<f32>(-1.5707964, 0.1, -0.1), vec2<f32>(40, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                case 4u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 12.0, 2.0), vec3<f32>(-2.0, 12.0, 0.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                case 5u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 12.0, 2.0), vec3<f32>(2.0, 12.0, 0.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 12.0, 4.0)); }
                default: {}
            }
        }
        case 7u: {
            switch part {
                case 0u: { return native_model_box(vec3<f32>(-4.0, -8.0, -4.0), vec3<f32>(4.0, 0.0, 4.0), vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(8.0, 8.0, 8.0)); }
                case 1u: { return native_model_box(vec3<f32>(-4.0, 0.0, -2.0), vec3<f32>(4.0, 12.0, 2.0), vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(16, 16), vec3<f32>(8.0, 12.0, 4.0)); }
                case 2u: { return native_model_box(vec3<f32>(-1.0, -2.0, -1.0), vec3<f32>(1.0, 10.0, 1.0), vec3<f32>(-5.0, 2.0, 0.0), vec3<f32>(-1.5707964, -0.1, 0.1), vec2<f32>(40, 16), vec3<f32>(2.0, 12.0, 2.0)); }
                case 3u: { return native_model_box(vec3<f32>(-1.0, -2.0, -1.0), vec3<f32>(1.0, 10.0, 1.0), vec3<f32>(5.0, 2.0, -0.0), vec3<f32>(-1.5707964, 0.1, -0.1), vec2<f32>(40, 16), vec3<f32>(2.0, 12.0, 2.0)); }
                case 4u: { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 12.0, 1.0), vec3<f32>(-2.0, 12.0, 0.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(2.0, 12.0, 2.0)); }
                case 5u: { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 12.0, 1.0), vec3<f32>(2.0, 12.0, 0.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(2.0, 12.0, 2.0)); }
                default: {}
            }
        }
        case 8u: {
            switch part {
                case 0u: { return native_model_box(vec3<f32>(-4.0, -4.0, -8.0), vec3<f32>(4.0, 4.0, 0.0), vec3<f32>(0.0, 15.0, -3.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(32, 4), vec3<f32>(8.0, 8.0, 8.0)); }
                case 1u: { return native_model_box(vec3<f32>(-3.0, -3.0, -3.0), vec3<f32>(3.0, 3.0, 3.0), vec3<f32>(0.0, 15.0, 0.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(6.0, 6.0, 6.0)); }
                case 2u: { return native_model_box(vec3<f32>(-5.0, -4.0, -6.0), vec3<f32>(5.0, 4.0, 6.0), vec3<f32>(0.0, 15.0, 9.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 12), vec3<f32>(10.0, 8.0, 12.0)); }
                case 3u: { return native_model_box(vec3<f32>(-15.0, -1.0, -1.0), vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(-4.0, 15.0, 2.0), vec3<f32>(0.0, 0.7853982, -0.7853982), vec2<f32>(18, 0), vec3<f32>(16.0, 2.0, 2.0)); }
                case 4u: { return native_model_box(vec3<f32>(-1.0, -1.0, -1.0), vec3<f32>(15.0, 1.0, 1.0), vec3<f32>(4.0, 15.0, 2.0), vec3<f32>(0.0, -0.7853982, 0.7853982), vec2<f32>(18, 0), vec3<f32>(16.0, 2.0, 2.0)); }
                case 5u: { return native_model_box(vec3<f32>(-15.0, -1.0, -1.0), vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(-4.0, 15.0, 1.0), vec3<f32>(0.0, 0.3926991, -0.58119464), vec2<f32>(18, 0), vec3<f32>(16.0, 2.0, 2.0)); }
                case 6u: { return native_model_box(vec3<f32>(-1.0, -1.0, -1.0), vec3<f32>(15.0, 1.0, 1.0), vec3<f32>(4.0, 15.0, 1.0), vec3<f32>(0.0, -0.3926991, 0.58119464), vec2<f32>(18, 0), vec3<f32>(16.0, 2.0, 2.0)); }
                case 7u: { return native_model_box(vec3<f32>(-15.0, -1.0, -1.0), vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(-4.0, 15.0, 0.0), vec3<f32>(0.0, -0.3926991, -0.58119464), vec2<f32>(18, 0), vec3<f32>(16.0, 2.0, 2.0)); }
                case 8u: { return native_model_box(vec3<f32>(-1.0, -1.0, -1.0), vec3<f32>(15.0, 1.0, 1.0), vec3<f32>(4.0, 15.0, 0.0), vec3<f32>(0.0, 0.3926991, 0.58119464), vec2<f32>(18, 0), vec3<f32>(16.0, 2.0, 2.0)); }
                case 9u: { return native_model_box(vec3<f32>(-15.0, -1.0, -1.0), vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(-4.0, 15.0, -1.0), vec3<f32>(0.0, -0.7853982, -0.7853982), vec2<f32>(18, 0), vec3<f32>(16.0, 2.0, 2.0)); }
                case 10u: { return native_model_box(vec3<f32>(-1.0, -1.0, -1.0), vec3<f32>(15.0, 1.0, 1.0), vec3<f32>(4.0, 15.0, -1.0), vec3<f32>(0.0, 0.7853982, 0.7853982), vec2<f32>(18, 0), vec3<f32>(16.0, 2.0, 2.0)); }
                default: {}
            }
        }
        case 9u: {
            switch part {
                case 0u: { return native_model_box(vec3<f32>(-4.0, -8.0, -4.0), vec3<f32>(4.0, 0.0, 4.0), vec3<f32>(0.0, 4.0, 0.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(8.0, 8.0, 8.0)); }
                case 1u: { return native_model_box(vec3<f32>(-4.0, 0.0, -2.0), vec3<f32>(4.0, 12.0, 2.0), vec3<f32>(0.0, 4.0, 0.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(16, 16), vec3<f32>(8.0, 12.0, 4.0)); }
                case 2u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 6.0, 2.0), vec3<f32>(-2.0, 16.0, 4.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                case 3u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 6.0, 2.0), vec3<f32>(2.0, 16.0, 4.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                case 4u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 6.0, 2.0), vec3<f32>(-2.0, 16.0, -4.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                case 5u: { return native_model_box(vec3<f32>(-2.0, 0.0, -2.0), vec3<f32>(2.0, 6.0, 2.0), vec3<f32>(2.0, 16.0, -4.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 16), vec3<f32>(4.0, 6.0, 4.0)); }
                default: {}
            }
        }
        case 10u: {
            switch part {
                case 0u: { return native_model_box(vec3<f32>(-4.0, 16.0, -4.0), vec3<f32>(4.0, 24.0, 4.0), vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(8.0, 8.0, 8.0)); }
                default: {}
            }
        }
        case 11u: {
            switch part {
                case 0u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(-3.0, -3.0, -2.0), vec3<f32>(3.0, 3.0, 2.0), vec3<f32>(-1.0, 13.5, -7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(6.0, 6.0, 4.0)); } return native_model_box(vec3<f32>(-3.0, -3.0, -2.0), vec3<f32>(3.0, 3.0, 2.0), vec3<f32>(-1.0, 13.5, -7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(6.0, 6.0, 4.0)); }
                case 1u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(-4.0, -2.0, -3.0), vec3<f32>(2.0, 7.0, 3.0), vec3<f32>(0.0, 18.0, 0.0), vec3<f32>(0.7853982, 0.0, 0.0), vec2<f32>(18, 14), vec3<f32>(6.0, 9.0, 6.0)); } return native_model_box(vec3<f32>(-4.0, -2.0, -3.0), vec3<f32>(2.0, 7.0, 3.0), vec3<f32>(0.0, 14.0, 2.0), vec3<f32>(1.5707964, 0.0, 0.0), vec2<f32>(18, 14), vec3<f32>(6.0, 9.0, 6.0)); }
                case 2u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 8.0, 1.0), vec3<f32>(-2.5, 22.0, 2.0), vec3<f32>(4.712389, 0.0, 0.0), vec2<f32>(0, 18), vec3<f32>(2.0, 8.0, 2.0)); } return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 8.0, 1.0), vec3<f32>(-2.5, 16.0, 7.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 18), vec3<f32>(2.0, 8.0, 2.0)); }
                case 3u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 8.0, 1.0), vec3<f32>(0.5, 22.0, 2.0), vec3<f32>(4.712389, 0.0, 0.0), vec2<f32>(0, 18), vec3<f32>(2.0, 8.0, 2.0)); } return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 8.0, 1.0), vec3<f32>(0.5, 16.0, 7.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 18), vec3<f32>(2.0, 8.0, 2.0)); }
                case 4u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 8.0, 1.0), vec3<f32>(-2.49, 17.0, -4.0), vec3<f32>(5.811947, 0.0, 0.0), vec2<f32>(0, 18), vec3<f32>(2.0, 8.0, 2.0)); } return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 8.0, 1.0), vec3<f32>(-2.5, 16.0, -4.0), vec3<f32>(-0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 3.1415927) * 0.45, 0.0, 0.0), vec2<f32>(0, 18), vec3<f32>(2.0, 8.0, 2.0)); }
                case 5u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 8.0, 1.0), vec3<f32>(0.51, 17.0, -4.0), vec3<f32>(5.811947, 0.0, 0.0), vec2<f32>(0, 18), vec3<f32>(2.0, 8.0, 2.0)); } return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 8.0, 1.0), vec3<f32>(0.5, 16.0, -4.0), vec3<f32>(0.0, 0.0, 0.0) + vec3<f32>(sin(phase + 0.0) * 0.45, 0.0, 0.0), vec2<f32>(0, 18), vec3<f32>(2.0, 8.0, 2.0)); }
                case 6u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(-3.0, -5.0, 0.0), vec3<f32>(-1.0, -3.0, 1.0), vec3<f32>(-1.0, 13.5, -7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(16, 14), vec3<f32>(2.0, 2.0, 1.0)); } return native_model_box(vec3<f32>(-3.0, -5.0, 0.0), vec3<f32>(-1.0, -3.0, 1.0), vec3<f32>(-1.0, 13.5, -7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(16, 14), vec3<f32>(2.0, 2.0, 1.0)); }
                case 7u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(1.0, -5.0, 0.0), vec3<f32>(3.0, -3.0, 1.0), vec3<f32>(-1.0, 13.5, -7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(16, 14), vec3<f32>(2.0, 2.0, 1.0)); } return native_model_box(vec3<f32>(1.0, -5.0, 0.0), vec3<f32>(3.0, -3.0, 1.0), vec3<f32>(-1.0, 13.5, -7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(16, 14), vec3<f32>(2.0, 2.0, 1.0)); }
                case 8u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(-2.0, 0.0, -5.0), vec3<f32>(1.0, 3.0, -1.0), vec3<f32>(-0.5, 13.5, -7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 10), vec3<f32>(3.0, 3.0, 4.0)); } return native_model_box(vec3<f32>(-2.0, 0.0, -5.0), vec3<f32>(1.0, 3.0, -1.0), vec3<f32>(-0.5, 13.5, -7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 10), vec3<f32>(3.0, 3.0, 4.0)); }
                case 9u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 8.0, 1.0), vec3<f32>(-1.0, 21.0, 6.0), vec3<f32>(0.8, 0.0, 0.0), vec2<f32>(9, 18), vec3<f32>(2.0, 8.0, 2.0)); } return native_model_box(vec3<f32>(-1.0, 0.0, -1.0), vec3<f32>(1.0, 8.0, 1.0), vec3<f32>(-1.0, 12.0, 8.0), vec3<f32>(0.8, 0.0, 0.0), vec2<f32>(9, 18), vec3<f32>(2.0, 8.0, 2.0)); }
                case 10u: { if (state & 4u) != 0u { return native_model_box(vec3<f32>(-4.0, -3.0, -3.0), vec3<f32>(4.0, 3.0, 4.0), vec3<f32>(-1.0, 16.0, -3.0), vec3<f32>(1.2566371, 0.0, 0.0), vec2<f32>(21, 0), vec3<f32>(8.0, 6.0, 7.0)); } return native_model_box(vec3<f32>(-4.0, -3.0, -3.0), vec3<f32>(4.0, 3.0, 4.0), vec3<f32>(-1.0, 14.0, -3.0), vec3<f32>(1.5707964, 0.0, 0.0), vec2<f32>(21, 0), vec3<f32>(8.0, 6.0, 7.0)); }
                default: {}
            }
        }
        case 18u: {
            switch part {
                case 0u: { return vehicle_model_box(vec3<f32>(-12.0, -8.0, -3.0), vec3<f32>(12.0, 8.0, 1.0), vec3<f32>(0.0, 21.0, 0.0), vec3<f32>(1.5707964, 0.0, 0.0), vec2<f32>(0, 8), vec3<f32>(24.0, 16.0, 4.0), 0.75); }
                case 1u: { return vehicle_model_box(vec3<f32>(-10.0, -7.0, -1.0), vec3<f32>(10.0, -1.0, 1.0), vec3<f32>(-11.0, 21.0, 0.0), vec3<f32>(0.0, 4.712389, 0.0), vec2<f32>(0, 0), vec3<f32>(20.0, 6.0, 2.0), 0.75); }
                case 2u: { return vehicle_model_box(vec3<f32>(-10.0, -7.0, -1.0), vec3<f32>(10.0, -1.0, 1.0), vec3<f32>(11.0, 21.0, 0.0), vec3<f32>(0.0, 1.5707964, 0.0), vec2<f32>(0, 0), vec3<f32>(20.0, 6.0, 2.0), 0.75); }
                case 3u: { return vehicle_model_box(vec3<f32>(-10.0, -7.0, -1.0), vec3<f32>(10.0, -1.0, 1.0), vec3<f32>(0.0, 21.0, -9.0), vec3<f32>(0.0, 3.1415927, 0.0), vec2<f32>(0, 0), vec3<f32>(20.0, 6.0, 2.0), 0.75); }
                case 4u: { return vehicle_model_box(vec3<f32>(-10.0, -7.0, -1.0), vec3<f32>(10.0, -1.0, 1.0), vec3<f32>(0.0, 21.0, 9.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(20.0, 6.0, 2.0), 0.75); }
                default: {}
            }
        }
        case 19u: {
            switch part {
                case 0u: { return vehicle_model_box(vec3<f32>(-10.0, -8.0, -1.0), vec3<f32>(10.0, 8.0, 1.0), vec3<f32>(0.0, 23.0, 0.0), vec3<f32>(1.5707964, 0.0, 0.0), vec2<f32>(0, 10), vec3<f32>(20.0, 16.0, 2.0), 0.49); }
                case 1u: { return vehicle_model_box(vec3<f32>(-8.0, -9.0, -1.0), vec3<f32>(8.0, -1.0, 1.0), vec3<f32>(-9.0, 23.0, 0.0), vec3<f32>(0.0, 4.712389, 0.0), vec2<f32>(0, 0), vec3<f32>(16.0, 8.0, 2.0), 0.49); }
                case 2u: { return vehicle_model_box(vec3<f32>(-8.0, -9.0, -1.0), vec3<f32>(8.0, -1.0, 1.0), vec3<f32>(9.0, 23.0, 0.0), vec3<f32>(0.0, 1.5707964, 0.0), vec2<f32>(0, 0), vec3<f32>(16.0, 8.0, 2.0), 0.49); }
                case 3u: { return vehicle_model_box(vec3<f32>(-8.0, -9.0, -1.0), vec3<f32>(8.0, -1.0, 1.0), vec3<f32>(0.0, 23.0, -7.0), vec3<f32>(0.0, 3.1415927, 0.0), vec2<f32>(0, 0), vec3<f32>(16.0, 8.0, 2.0), 0.49); }
                case 4u: { return vehicle_model_box(vec3<f32>(-8.0, -9.0, -1.0), vec3<f32>(8.0, -1.0, 1.0), vec3<f32>(0.0, 23.0, 7.0), vec3<f32>(0.0, 0.0, 0.0), vec2<f32>(0, 0), vec3<f32>(16.0, 8.0, 2.0), 0.49); }
                case 5u: { return vehicle_model_box(vec3<f32>(-9.0, -7.0, -1.0), vec3<f32>(9.0, 7.0, 0.0), vec3<f32>(0.0, 23.0, 0.0), vec3<f32>(-1.5707964, 0.0, 0.0), vec2<f32>(44, 10), vec3<f32>(18.0, 14.0, 1.0), 0.49); }
                default: {}
            }
        }
        default: {}
    }
    return model_box(vec3<f32>(0.0), vec3<f32>(1.0), vec2<f32>(0.0));
}
fn painting_art(index:u32)->vec4<f32>{
 switch (index & 31u) {
case 0u: { return vec4<f32>(0.0,0.0,16.0,16.0); }
case 1u: { return vec4<f32>(16.0,0.0,16.0,16.0); }
case 2u: { return vec4<f32>(32.0,0.0,16.0,16.0); }
case 3u: { return vec4<f32>(48.0,0.0,16.0,16.0); }
case 4u: { return vec4<f32>(64.0,0.0,16.0,16.0); }
case 5u: { return vec4<f32>(80.0,0.0,16.0,16.0); }
case 6u: { return vec4<f32>(96.0,0.0,16.0,16.0); }
case 7u: { return vec4<f32>(0.0,32.0,32.0,16.0); }
case 8u: { return vec4<f32>(32.0,32.0,32.0,16.0); }
case 9u: { return vec4<f32>(64.0,32.0,32.0,16.0); }
case 10u: { return vec4<f32>(96.0,32.0,32.0,16.0); }
case 11u: { return vec4<f32>(128.0,32.0,32.0,16.0); }
case 12u: { return vec4<f32>(0.0,64.0,16.0,32.0); }
case 13u: { return vec4<f32>(16.0,64.0,16.0,32.0); }
case 14u: { return vec4<f32>(0.0,128.0,32.0,32.0); }
case 15u: { return vec4<f32>(32.0,128.0,32.0,32.0); }
case 16u: { return vec4<f32>(64.0,128.0,32.0,32.0); }
case 17u: { return vec4<f32>(96.0,128.0,32.0,32.0); }
case 18u: { return vec4<f32>(128.0,128.0,32.0,32.0); }
case 19u: { return vec4<f32>(0.0,96.0,64.0,32.0); }
case 20u: { return vec4<f32>(0.0,192.0,64.0,64.0); }
case 21u: { return vec4<f32>(64.0,192.0,64.0,64.0); }
case 22u: { return vec4<f32>(128.0,192.0,64.0,64.0); }
case 23u: { return vec4<f32>(192.0,64.0,64.0,48.0); }
case 24u: { return vec4<f32>(192.0,112.0,64.0,48.0); }
default: {return vec4<f32>(0.0,0.0,16.0,16.0);}
 }
}

// Beta ModelBiped armour: inflated copies of the player boxes with the same UVs.
// Parts 7..15: helmet, chest body/arms, leggings body/legs, boots. Returns
// (base part, material 1..5 or 0, inflation in pixels, second texture).
fn player_armor(part:u32,state:u32)->vec4<u32> {
    let a=state>>16u;
    switch part {
        case 7u: {return vec4<u32>(0u,a&7u,2u,0u);}
        case 8u,9u,10u: {return vec4<u32>(part-7u,(a>>3u)&7u,2u,0u);}
        case 11u: {return vec4<u32>(1u,(a>>6u)&7u,1u,1u);}
        case 12u,13u: {return vec4<u32>(part-8u,(a>>6u)&7u,1u,1u);}
        case 14u,15u: {return vec4<u32>(part-10u,(a>>9u)&7u,2u,0u);}
        default: {return vec4<u32>(0u);}
    }
}
fn player_part(part:u32,phase:f32,state:u32)->ModelPart {
    let encoded_pitch=(state>>8u)&255u;
    let pitch=select(0.0,(f32(encoded_pitch)-128.0)/127.0*1.5707964,encoded_pitch!=0u);
    if part==6u {return native_model_box(vec3<f32>(-4.5,-8.5,-4.5),vec3<f32>(4.5,0.5,4.5),vec3<f32>(0.0),vec3<f32>(-pitch,0.0,0.0),vec2<f32>(32.0,0.0),vec3<f32>(8.0));}
    let walk=select(0.0,phase,(state & 64u)!=0u);
    var p=classic_model_part(6u,part,walk,state);
    if part==0u {p.angles.x=-pitch;}
    if part==2u || part==3u {p.angles=vec3<f32>(sin(walk+select(3.1415927,0.0,part==2u))*0.45,0.0,0.0);}
    return p;
}

// Part 16: the held item in the right hand. Items are flat 6-pixel sprites
// (Beta scales icons by 0.375), terrain blocks small cubes.
fn player_held(phase:f32,state:u32,held:u32)->ModelPart {
    // Child of the right arm: pivot at the hand (arm-local (-1, 10, 0) px).
    let arm=player_part(2u,phase,state);
    let hand=arm.pivot*16.0+rotate_model(vec3<f32>(-1.0,10.0,0.0),arm.angles);
    let angles=vec3<f32>(arm.angles.x,0.0,0.0);
    // Beta lays held items almost flat (-100 deg about X, 45 about Y): the icon's
    // handle sits in the hand and its diagonal points forward and outward.
    if held>=256u {return native_model_box(vec3<f32>(-9.0,-0.25,-9.0),vec3<f32>(0.0,0.25,0.0),hand,angles,vec2<f32>(0.0),vec3<f32>(1.0));}
    return native_model_box(vec3<f32>(-2.0,-2.0,-4.0),vec3<f32>(2.0,2.0,0.0),hand,angles,vec2<f32>(0.0),vec3<f32>(1.0));
}
fn held_skin(part:ModelPart,point:vec4<f32>,axis:i32,held:u32)->vec4<f32> {
    let f=(point.xyz-part.lo.xyz)/(part.hi.xyz-part.lo.xyz);
    var uv=vec2<f32>(1.0-f.z,f.x);
    if held<256u && axis==2 {uv=vec2<f32>(f.x,f.y);}
    if held<256u && axis==1 {uv=vec2<f32>(f.x,f.z);}
    return tile_color(held%256u,uv,i32(held/256u));
}

fn model_count(kind: u32, state: u32) -> u32 {
    if kind==22u {return 17u;}
    if kind==19u && (state >> 8u)!=0u {return 7u;}
    if kind == 3u && (state & 1u) != 0u { return 6u; }
    if (kind >= 1u && kind <= 11u) || kind == 18u || kind == 19u { return classic_model_count(kind); }
    return select(1u, 2u, kind == 15u);
}

var<private> held_icon: u32;

fn model_part(kind: u32, part: u32, phase: f32, state: u32) -> ModelPart {
    if kind==21u {let art=painting_art(state);let half=vec4<f32>(art.z/32.0,art.w/32.0,0.03125,0.5);return ModelPart(-half,half,art,1.0,false,false,vec3<f32>(0.0),vec3<f32>(0.0),0.0);}
    if kind==22u {
        if part<7u {return player_part(part,phase,state);}
        if part==16u {return player_held(phase,state,held_icon);}
        let armor=player_armor(part,state);
        var p=player_part(armor.x,phase,state);
        let grow=f32(armor.z)*0.5/16.0;
        p.lo=vec4<f32>(p.lo.xyz-grow,p.lo.w);p.hi=vec4<f32>(p.hi.xyz+grow,p.hi.w);
        return p;
    }
    if kind==19u && part==6u {return model_box(vec3<f32>(0.0,10.0,0.0),vec3<f32>(12.0),vec2<f32>(0.0));}
    if (kind >= 1u && kind <= 11u) || kind == 18u || kind == 19u { return classic_model_part(kind, part, phase, state); }
    if kind == 15u || kind == 16u {
        if part == 0u {
            if kind == 15u {
                return ModelPart(vec4<f32>(-1.0, 1.0/6.0, -1.0/12.0, -1.0),
                    vec4<f32>(1.0, 7.0/6.0, 1.0/12.0, 1.0), vec4<f32>(0.0, 0.0, 24.0, 12.0), 2.0, false, false, vec3<f32>(0.0), vec3<f32>(0.0), 0.0);
            }
            return ModelPart(vec4<f32>(-1.0, -11.0/24.0, -23.0/24.0, -1.0),
                vec4<f32>(1.0, 13.0/24.0, -19.0/24.0, 1.0), vec4<f32>(0.0, 0.0, 24.0, 12.0), 2.0, false, false, vec3<f32>(0.0), vec3<f32>(0.0), 0.0);
        }
        return ModelPart(vec4<f32>(-1.0/12.0, -1.0, -1.0/12.0, -1.0/12.0),
            vec4<f32>(1.0/12.0, 1.0/6.0, 1.0/12.0, 1.0/12.0), vec4<f32>(0.0, 14.0, 2.0, 14.0), 2.0, false, false, vec3<f32>(0.0), vec3<f32>(0.0), 0.0);
    }
    if kind == 12u { return model_box(vec3<f32>(0.0), vec3<f32>(2.0, 2.0, 64.0), vec2<f32>(0.0)); }
    if kind == 13u { return model_box(vec3<f32>(0.0), vec3<f32>(28.0), vec2<f32>(0.0)); }
    if kind == 14u || kind == 17u { return model_box(vec3<f32>(0.0), vec3<f32>(32.0), vec2<f32>(0.0)); }
    return model_box(vec3<f32>(0.0), vec3<f32>(1.0), vec2<f32>(0.0));
}

// Return distance, entering normal axis and sign. Slab intersections work for
// arbitrary slice rotations because rays retain all four spatial components.
fn model_intersection(o: vec4<f32>, d: vec4<f32>, part: ModelPart) -> vec3<f32> {
    if any((d == vec4<f32>(0.0)) & ((o < part.lo) | (o > part.hi))) { return vec3<f32>(1e30, 0.0, 0.0); }
    let inv = vec4<f32>(safe_inv(d.x), safe_inv(d.y), safe_inv(d.z), safe_inv(d.w));
    let a = (part.lo - o) * inv;
    let b = (part.hi - o) * inv;
    let near = select(min(a, b), vec4<f32>(-1e30), d == vec4<f32>(0.0));
    let far = select(max(a, b), vec4<f32>(1e30), d == vec4<f32>(0.0));
    let axis = min_axis(-near);
    let enter = near[axis];
    let exit = far[min_axis(far)];
    if exit < max(enter, 0.001) { return vec3<f32>(1e30, 0.0, 0.0); }
    if enter < 0.001 {
        let out_axis = min_axis(far);
        return vec3<f32>(exit, f32(out_axis), sign(d[out_axis]));
    }
    return vec3<f32>(enter, f32(axis), -sign(d[axis]));
}

fn model_skin(part: ModelPart, p: vec4<f32>, axis: i32, nsign: f32, layer: i32) -> vec4<f32> {
    var f = clamp((p - part.lo) / (part.hi - part.lo), vec4<f32>(0.0), vec4<f32>(0.999));
    var face = axis;
    var sign_face = nsign;
    if part.rotated {
        f = vec4<f32>(f.x, 1.0 - f.z, f.y, f.w);
        if face == 1 { face = 2; } else if face == 2 { face = 1; sign_face = -nsign; }
    }
    if part.native {
        // Native model Y points down, and its front is -Z.
        f.y = 1.0 - f.y;
        if face == 1 || face == 2 { sign_face = -sign_face; }
    }
    let size = part.skin.zw;
    let depth = part.depth;
    var origin = part.skin.xy + vec2<f32>(depth, depth);
    var extent = size;
    var uv = vec2<f32>(f.x, 1.0 - f.y);
    if face == 1 {
        origin = part.skin.xy + vec2<f32>(depth + select(size.x, 0.0, sign_face > 0.0), 0.0);
        extent = vec2<f32>(size.x, depth); uv = f.xz;
    } else if face == 0 {
        origin = part.skin.xy + vec2<f32>(select(0.0, depth + size.x, sign_face > 0.0), depth);
        extent = vec2<f32>(depth, size.y); uv = vec2<f32>(f.z, 1.0 - f.y);
    } else if face == 2 && sign_face < 0.0 {
        origin.x += size.x + depth; uv.x = 1.0 - f.x;
    }
    let pixel = floor(origin + clamp(uv, vec2<f32>(0.0), vec2<f32>(0.999)) * extent) + 0.5;
    return textureSampleLevel(classic_atlas, pixel_sampler, pixel / 256.0, layer, 0.0);
}

fn world_snow(eye: vec4<f32>, d: vec4<f32>, limit: f32) -> vec4<f32> {
    if g.inset.y <= 0.001 || (g.weather_origin.w & 65536) == 0 { return vec4<f32>(0.0); }
    let o=eye.xzw;
    let ray=d.xzw;
    let a=dot(ray,ray);
    if a<0.000001 { return vec4<f32>(0.0); }
    let draw_radius=select(5,10,(g.info.y & FLAG_FANCY)!=0u);
    let trace_limit=min(limit,(f32(draw_radius)+1.0)/max(abs(d.y),0.00001));
    let start_cell=vec3<i32>(floor(o));
    var cell=start_cell;
    let stepv=vec3<i32>(sign(ray));
    let inv=vec3<f32>(safe_inv(ray.x),safe_inv(ray.y),safe_inv(ray.z));
    var next=(vec3<f32>(cell+max(stepv,vec3<i32>(0)))-o)*inv;
    next=select(next,vec3<f32>(1e30),ray==vec3<f32>(0.0));
    var t=0.0;
    var color=vec3<f32>(0.0);
    var opacity=0.0;
    for(var i=0;i<72;i++) {
        if t>=trace_limit || any(abs(cell-start_cell)>vec3<i32>(draw_radius)) || opacity>0.95 {break;}
        let world_cell=cell+g.center.xzw*16;
        let local=world_cell-g.weather_origin.xyz;
        if all(local>=vec3<i32>(0)) && all(local<vec3<i32>(25)) {
            let packed=textureLoad(precipitation_columns,local,0);
            let column=packed.x;
            let end=min(min(next.x,min(next.y,next.z)),trace_limit);
            let highest_y=eye.y+max(d.y*max(0.0,t-0.001),d.y*(end+0.001))+f32(g.center.y)*16.0;
            if (column & 131072)==0 && highest_y>=f32(column & 65535) {
                let snow=true;
                if (column & 65536)!=0 {
                let randoms=bitcast<vec4<u32>>(packed);
                let offsets=vec3<f32>(f32(randoms.y & 65535u),f32(randoms.y >> 16u),f32(randoms.z & 65535u))/65535.0;
                let center=vec3<f32>(cell)+vec3<f32>(0.2)+0.6*offsets;
                let oc=o-center;
                let b=dot(oc,ray);
                let radius=select(0.075,0.12,snow);
                let disc=b*b-a*(dot(oc,oc)-radius*radius);
                if disc>0.0 {
                    let hit=(-b-sqrt(disc))/a;
                    if hit>max(0.0,t-0.001) && hit<trace_limit {
                        let y=eye.y+d.y*hit+f32(g.center.y)*16.0;
                        if y>=f32(column & 65535) {
                            let phase=fract((y+g.misc.y*select(12.0,1.4,snow))/6.0+f32(randoms.z >> 16u)/65535.0);
                            let streak_length=select(0.38,0.10,snow);
                            if phase<streak_length {
                                let closest=o+ray*(-b/a)-center;
                                let u=clamp(0.5+length(closest)/(2.0*radius),0.0,0.999);
                                let uv=vec2<f32>(u*0.25,phase/streak_length);
                                let particle=textureSampleLevel(classic_atlas,pixel_sampler,uv,select(16,17,snow),0.0);
                                let border=min(min(local.x,24-local.x),min(min(local.y,24-local.y),min(local.z,24-local.z)));
                                let alpha=particle.a*g.inset.y*(1.0-smoothstep(f32(draw_radius)*0.65,f32(draw_radius),hit))*smoothstep(0.0,2.0,f32(border))*0.8;
                                let lit=particle.rgb*(0.25+0.75*g.inset.x)+g.inset.w*0.5;
                                color+=lit*alpha*(1.0-opacity);
                                opacity+=alpha*(1.0-opacity);
                            }
                        }
                    }
                }
                }
            }
        }
        var axis=0;
        if next.y<next.x {axis=1;}
        if next.z<next[axis] {axis=2;}
        t=next[axis];cell[axis]+=stepv[axis];next[axis]+=abs(inv[axis]);
    }
    return vec4<f32>(color,opacity);
}

// Visit only actual rain planes. The previous voxel DDA tested all three
// planes in every cell, including planes the ray could never intersect there.
fn world_precipitation(eye: vec4<f32>, d: vec4<f32>, limit: f32) -> vec4<f32> {
    if g.inset.y<=0.001 || g.weather_origin.w==0 {return vec4<f32>(0.0);}
    var color=vec3<f32>(0.0);var opacity=0.0;
    if (g.weather_origin.w & 131072)!=0 {
        let o=eye.xzw;let ray=d.xzw;
        let inv=vec3<f32>(safe_inv(ray.x),safe_inv(ray.y),safe_inv(ray.z));
        let delta=abs(inv);
        let start=vec3<i32>(floor(o));
        var next=(vec3<f32>(start)+vec3<f32>(0.5)-o)*inv;
        next=select(next,next+delta,next<vec3<f32>(0.1));
        next=select(next,vec3<f32>(1e30),abs(ray)<vec3<f32>(0.000001));
        let radius=select(5,10,(g.info.y & FLAG_FANCY)!=0u);
        let trace_limit=min(limit,(f32(radius)+1.0)/max(abs(d.y),0.00001));
        for(var step=0;step<72;step++) {
            var axis=0;if next.y<next.x {axis=1;}if next.z<next[axis] {axis=2;}
            let hit=next[axis];next[axis]+=delta[axis];
            if hit>=trace_limit || opacity>0.95 {break;}
            let p=o+ray*hit;let cell=vec3<i32>(floor(p));
            if any(abs(cell-start)>vec3<i32>(radius)) {break;}
            let local=cell+g.center.xzw*16-g.weather_origin.xyz;
            if any(local<vec3<i32>(0)) || any(local>=vec3<i32>(25)) {continue;}
            let packed=textureLoad(precipitation_columns,local,0);
            if (packed.x & 196608)!=0 {continue;}
            let y=eye.y+d.y*hit+f32(g.center.y)*16.0;
            if y<f32(packed.x & 65535) {continue;}
            let f=p-vec3<f32>(cell);
            let u=(f.x+f.y+f.z-f[axis])*2.0;
            let v=y*0.5+g.misc.y*1.25+f32(packed.w)/16.0;
            let particle=textureLoad(rain_brightness,vec2<i32>(vec2<f32>(fract(u)*64.0,fract(v)*256.0)),i32(clamp(15.0-sky_dimming(),0.0,15.0)),0);
            if particle.a<=0.01 {continue;}
            let offset=vec3<f32>(cell)+vec3<f32>(0.5)-o;
            let fade=clamp(1.0-dot(offset,offset)/(2.0*f32(radius*radius)),0.0,1.0);
            let alpha=particle.a*g.inset.y*fade;
            let lit=particle.rgb;
            color+=lit*alpha*(1.0-opacity);opacity+=alpha*(1.0-opacity);
        }
    }
    if (g.weather_origin.w & 65536)!=0 {
        let snow=world_snow(eye,d,limit);color=color*(1.0-snow.a)+snow.rgb;opacity=opacity+(1.0-opacity)*snow.a;
    }
    return vec4<f32>(color,opacity);
}

struct ShadeResult { color: vec3<f32>, distance: f32 };
// Neighbouring-slice hints fade to nothing at this distance; each pixel traces
// two extra rays, so their length is the main cost of ghost layers.
const GHOST_RANGE: f32 = 16.0;

// Ghost-layer first-hit distances (ana, kata) or 1e30. `limit_t` may be the
// pre-entity trace distance: shade_hit compares against the final distance.
fn ghost_pair(eye: vec4<f32>, d: vec4<f32>, cam: Camera, main_view: bool, limit_t: f32) -> vec2<f32> {
    if (g.info.y & FLAG_GHOSTS) == 0u || !main_view || limit_t <= 0.05 { return vec2<f32>(1e30); }
    let go = g.misc.x;
    let limit = min(limit_t - 0.05, GHOST_RANGE);
    // The beam prepass covers both ghost origins: nothing lies before ray_start.
    if ghost_start >= limit { return vec2<f32>(1e30); }
    return vec2<f32>(ghost_distance(eye + cam.ana * go, d, limit, cam.ana, ghost_start), ghost_distance(eye - cam.ana * go, d, limit, cam.ana, ghost_start));
}

fn shade(eye: vec4<f32>, d: vec4<f32>, cam: Camera, px_angle: f32, main_view: bool) -> ShadeResult {
    let h = trace(eye, d, g.view.w, 768, cam.ana);
    return shade_hit(eye, d, cam, px_angle, main_view, h, ghost_pair(eye, d, cam, main_view, h.t));
}

fn shade_hit(eye: vec4<f32>, d: vec4<f32>, cam: Camera, px_angle: f32, main_view: bool, h: Hit, ghosts: vec2<f32>) -> ShadeResult {
    let flags = g.info.y;
    let max_dist = g.view.w;
    // Light from the sun projected into the slice, so shadows match what you see.
    var sun_slice = g.sun - cam.ana * dot(g.sun, cam.ana);
    if dot(sun_slice,sun_slice)>0.000001 {sun_slice=normalize(sun_slice);} else {sun_slice=vec4<f32>(0.0,select(1.0,-1.0,g.sun.y<0.0),0.0,0.0);}

    var col: vec3<f32>;
    var t = h.t;
    if h.hit {
        let p = eye + d * h.t;
        let f = p - vec4<f32>(h.cell);
        col = block_color(h.block, h.cell, f, h.axis, h.nsign, cam.ana);
        if h.axis >= 0 {
            let n = axis_vec(h.axis) * h.nsign;
            let neighbor=h.cell+vec4<i32>(n);
            let outside=block_brightness_at(neighbor,block_at(neighbor));
            let own=block_brightness_at(h.cell,h.block);
            // Native bounded faces sample their containing cell when inset.
            // Cactus is exceptional: cn.c offsets full-bound faces geometrically.
            let inset_face=f[h.axis]>0.0001 && f[h.axis]<0.9999 && h.block!=90u;
            let surface=select(outside,own,inset_face);
            let levels=vec2<f32>(max(surface.x,own.x),surface.y);
            var light = classic_brightness(levels);
            if (flags & FLAG_SHADOWS) != 0u && levels.y - sky_dimming() > levels.x && dot(n, sun_slice) > 0.0 {
                let sh = ghost_distance(p + n * 0.002, sun_slice, 48.0, cam.ana, 0.0);
                if sh < 1e29 { light *= 0.55; }
            }
            let sprite_lighting = (u32(g.palette[h.block].a) & 64u) != 0u;
            var ao=1.0;
            if (flags & FLAG_SMOOTH_LIGHTING)!=0u && !sprite_lighting && (u32(g.palette[h.block].a)&16u)==0u && h.block!=90u {ao=face_ao(h.cell,h.axis,h.nsign,f);}
            let face = select(face_shade(h.axis,h.nsign), vec3<f32>(1.0), sprite_lighting);
            col = apply_classic_light(col, face * light * ao);

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
            col = apply_classic_light(col, vec3<f32>(classic_brightness(voxel_light_at(h.cell)) * 0.25));
        }
        if h.block == LAVA {
            // The original lava texture animates its own heat field; no added
            // world-space brightness pulse. Emission keeps the surface bright.
            col = apply_classic_light(block_color(LAVA, h.cell, f, h.axis, h.nsign, cam.ana), face_shade(h.axis, h.nsign));
        }
        if main_view && g.info.x==1u && all(h.cell==g.target_cell) && g.gameplay.x>0.0 {
            let stage=min(9u,u32(g.gameplay.x*10.0));
            let cracks=projected_tile(i32(240u+stage),f,h.axis,cam.ana);
            col=mix(col,col*cracks.rgb,cracks.a);
        }
    } else {
        col = sky(d, sun_slice,cam.ana);
    }

    // Glass tint; water is composited after entities so objects behind its
    // surface receive the same transparent skin as the terrain behind it.
    if h.glass > 0.0 {
        col = mix(col, g.palette[GLASS].rgb, min(0.18 * h.glass, 0.6));
    }

    // Distance fog.
    if h.hit {
        let fog = clamp((t / max_dist - 0.55) / 0.45, 0.0, 1.0);
        col = mix(col, mix(g.sky_horizon.rgb,g.sky_top.rgb,clamp(d.y*1.4+0.15,0.0,1.0)), fog * fog);
    }

    // BEGIN ENTITY PASS
    // Broad-phase sphere rejection keeps rays that miss a creature from testing
    // its individual skinned boxes. Only Glooms retain hypersphere geometry.
    let ne = min(g.info.z, MAX_ENTITIES);
    for (var e = 0u; e < ne; e++) {
        let c = g.entities[e * 6u];
        let r = g.entities[e * 6u + 1u].x;
        let ecol = g.entities[e * 6u + 1u].yzw;
        let entity_info = g.entities[e * 6u + 5u];
        let kind = u32(entity_info.x);
        let state = u32(entity_info.w) | select(0u, u32(entity_info.y) << 16u, kind==22u);
        held_icon = select(0u, u32(entity_info.z) - 1u, kind==22u && entity_info.z > 0.5);
        let bound = r * select(select(2.5,3.0,kind==21u), 1.0, kind == 0u);
        let oc = eye - c;
        let b = dot(oc, d);
        let off = dot(c - eye, cam.ana);
        let disc = b * b - (dot(oc, oc) - bound * bound);
        if disc > 0.0 && -b + sqrt(disc) > 0.0 && -b - sqrt(disc) < t {
            if kind==24u {
                // Source lightning uses additive nested square tubes. A shear
                // follows each jagged segment and extends its cross-section to W.
                let span=g.entities[e*6u+2u];
                if abs(span.y)>0.0001 {
                    let slope=vec4<f32>(span.x,0.0,span.z,span.w)/span.y;
                    let local_eye=oc-slope*oc.y;let local_ray=d-slope*d.y;
                    for(var shell=0; shell<4; shell++) {
                        let width=(0.1+f32(shell)*0.2)*select(0.95+f32(state & 7u)*0.1,1.0,(state & 16u)!=0u);
                        var tube=model_box(vec3<f32>(0.0),vec3<f32>(1.0),vec2<f32>(0.0));
                        tube.lo=vec4<f32>(-width,-abs(span.y)*0.5,-width,-width);tube.hi=-tube.lo;
                        let hit=model_intersection(local_eye,local_ray,tube);
                        if hit.x>0.0&&hit.x<t {col+=ecol*0.3;}
                    }
                }
                continue;
            }
            if kind == 0u {
                let te = -b - sqrt(disc);
                if te > 0.0 && te < t {
                    let n = normalize(eye + d * te - c);
                    let lit = classic_brightness(voxel_light_at(vec4<i32>(floor(c)))) * (0.65 + 0.35 * max(dot(n, sun_slice), 0.0));
                    col = apply_classic_light(ecol, vec3<f32>(lit)); t = te;
                }
            } else {
                let forward = g.entities[e * 6u + 2u];
                let right = g.entities[e * 6u + 3u];
                let ana = g.entities[e * 6u + 4u];
                let local_eye = vec4<f32>(dot(oc, right), oc.y, dot(oc, forward), dot(oc, ana)) / r;
                let local_ray = vec4<f32>(dot(d, right), d.y, dot(d, forward), dot(d, ana)) / r;
                var best = t;
                var best_part = 0u;
                var best_hit = vec3<f32>(1e30, 0.0, 0.0);
                let phase = g.misc.y * 5.0 + dot(c, vec4<f32>(0.5));
                let count = model_count(kind, state);
                for (var part = 0u; part < count; part++) {
                    if kind==22u && part>=7u && part<16u && player_armor(part,state).y==0u {continue;}
                    if kind==22u && part==16u && entity_info.z < 0.5 {continue;}
                    let box = model_part(kind, part, phase, state);
                    let part_eye=part_point(box,local_eye);
                    // Native player polygons are back-face culled. A boom pressed
                    // against a wall must not turn the inside of the head opaque.
                    if kind==22u && all(part_eye>box.lo) && all(part_eye<box.hi) {continue;}
                    let hit = model_intersection(part_point(box, local_eye), part_vector(box, local_ray), box);
                    if hit.x < best {
                        // Wool is a separate inflated model. Transparent face
                        // texels reveal the native head/body underneath it.
                        if kind==22u && part==6u {
                            let point=part_point(box,local_eye+local_ray*hit.x);
                            if model_skin(box,point,i32(hit.y),hit.z,21).a<0.05 {continue;}
                        }
                        // Armour texels are mostly transparent (boots cover only the
                        // ankle): reveal leggings and the body beneath, as in Beta.
                        if kind==22u && part==16u {
                            if held_skin(box,part_point(box,local_eye+local_ray*hit.x),i32(hit.y),held_icon).a<0.05 {continue;}
                        }
                        if kind==22u && part>=7u && part<16u {
                            let armor=player_armor(part,state);var point=part_point(box,local_eye+local_ray*hit.x);var sign=hit.z;
                            if part==10u||part==13u||part==15u {point.x=box.lo.x+box.hi.x-point.x;if i32(hit.y)==0 {sign=-sign;}}
                            if model_skin(box,point,i32(hit.y),sign,29+i32(armor.y-1u)*2+i32(armor.w)).a<0.05 {continue;}
                        }
                        if kind == 3u && part >= 6u {
                            let point = part_point(box, local_eye + local_ray * hit.x);
                            if model_skin(box, point, i32(hit.y), hit.z, 15).a < 0.05 { continue; }
                        }
                        best = hit.x; best_part = part; best_hit = hit;
                    }
                }
                if best < t {
                    let part = model_part(kind, best_part, phase, state);
                    let point = part_point(part, local_eye + local_ray * best);
                    let axis = i32(best_hit.y);
                    var skin = vec4<f32>(ecol, 1.0);
                    if kind==21u {
                        let art=painting_art(state);
                        var pixel=vec2<f32>(192.0,0.0)+fract(point.xy)*16.0;
                        if axis==2 && best_hit.z>0.0 {
                            pixel=clamp(art.xy+vec2<f32>(0.5-point.x/(art.z/16.0),0.5-point.y/(art.w/16.0))*art.zw,art.xy+vec2<f32>(0.5),art.xy+art.zw-vec2<f32>(0.5));
                        }
                        skin=textureSampleLevel(classic_atlas,pixel_sampler,(floor(pixel)+vec2<f32>(0.5))/256.0,27,0.0);
                    } else if kind == 13u || kind == 14u || kind == 17u {
                        let f = (point - part.lo) / (part.hi - part.lo);
                        var uv = vec2<f32>(f.x, 1.0 - f.y);
                        if axis == 0 { uv.x = f.z; } else if axis == 1 { uv = f.xz; }
                        var tile = u32(entity_info.z);
                        if kind == 17u { tile = u32(terrain_tile(TNT, 0u, axis, best_hit.z)); }
                        skin = tile_color(tile, uv, i32(entity_info.y));
                    } else if kind == 15u || kind == 16u {
                        skin = model_skin(part, point, axis, best_hit.z, 18);
                        let text_layer = i32(state >> 8u) - 1;
                        if best_part == 0u && axis == 2 && best_hit.z > 0.0 && text_layer >= 0 {
                            // A sign's 4D face has two horizontal tangents. Project
                            // screen-right onto that face, retaining native upright
                            // text in XZ and readable text in W-facing views.
                            var tangent = vec2<f32>(dot(cam.right, right), dot(cam.right, ana));
                            if dot(tangent, tangent) < 0.00001 { tangent = vec2<f32>(-1.0, 0.0); }
                            tangent = normalize(tangent);
                            let support = abs(tangent.x) + abs(tangent.y);
                            let u = 0.5 + dot(point.xw, tangent) / (2.0 * support);
                            let v = 1.0 - (point.y - part.lo.y) / (part.hi.y - part.lo.y);
                            skin = textureSampleLevel(sign_atlas, pixel_sampler,
                                clamp(vec2<f32>(u, v), vec2<f32>(0.5/192.0, 0.5/96.0), vec2<f32>(191.5/192.0, 95.5/96.0)), text_layer, 0.0);
                        }
                    } else if kind != 12u {
                        var layer = i32(kind + 1u);
                        if kind==18u {layer=22;}if kind==19u {layer=23;}if kind==22u {layer=21;}
                        if kind==22u && best_part>=7u && best_part<16u {let armor=player_armor(best_part,state);layer=29+i32(armor.y-1u)*2+i32(armor.w);}
                        if kind == 11u { layer = select(12, 13, (state & 2u) != 0u); if (state & 16u) != 0u { layer = 14; } }
                        var skin_point=point;var skin_sign=best_hit.z;
                        if kind==22u && (best_part==3u||best_part==5u||best_part==10u||best_part==13u||best_part==15u) {
                            skin_point.x=part.lo.x+part.hi.x-skin_point.x;
                            if axis==0 {skin_sign=-skin_sign;}
                        }
                        skin = model_skin(part, skin_point, axis, skin_sign, layer);
                        if kind==22u && best_part==16u {skin=held_skin(part,point,axis,held_icon);}
                        if kind == 3u && best_part >= 6u {
                            skin = model_skin(part, point, axis, best_hit.z, 15);
                            skin = vec4<f32>(apply_classic_light(skin.rgb, sheep_wool_tint(state >> 8u)), skin.a);
                        }
                    }
                    if kind==19u && best_part==6u {
                        let f=(point-part.lo)/(part.hi-part.lo);var uv=vec2<f32>(f.x,1.0-f.y);
                        if axis==0 {uv.x=f.z;}else if axis==1 {uv=f.xz;}
                        let cargo=select(CHEST,20u,(state >> 8u)==2u);
                        skin=tile_color(u32(terrain_tile(cargo,0u,axis,best_hit.z)),uv,0);
                    }
                    if kind == 17u && (state & 64u) != 0u { skin = vec4<f32>(mix(skin.rgb, vec3<f32>(1.0), 0.7), skin.a); }
                    if skin.a > 0.05 {
                        let normal = part_normal(part, axis_vec(axis) * best_hit.z);
                        let world_normal = right * normal.x + vec4<f32>(0.0, normal.y, 0.0, 0.0) + forward * normal.z + ana * normal.w;
                        let lit = classic_brightness(voxel_light_at(vec4<i32>(floor(c)))) * (0.65 + 0.35 * max(dot(world_normal, sun_slice), 0.0));
                        var rgb = skin.rgb;
                        if kind!=21u && (state & 32u) != 0u { rgb = mix(rgb, vec3<f32>(1.0, 0.12, 0.1), 0.55); }
                        if kind == 9u && (state & 8u) != 0u { rgb = mix(rgb, vec3<f32>(1.0), step(0.0, sin(g.misc.y * 25.0)) * 0.65); }
                        col = apply_classic_light(rgb, vec3<f32>(lit)); t = best;
                    }
                }
            }
        } else if kind < 12u && (flags & FLAG_GLASSES) != 0u && abs(off) < g.misc.z && main_view {
            let cp = c - cam.ana * off;
            let ocp = eye - cp;
            let bp = dot(ocp, d);
            let dist_line = sqrt(max(dot(ocp, ocp) - bp * bp, 0.0));
            let te = -bp;
            let width = px_angle * max(te, 0.1) * 2.0;
            if te > 0.0 && te < t && abs(dist_line - r) < width {
                let tint = select(vec3<f32>(0.3, 0.5, 1.0), vec3<f32>(1.0, 0.35, 0.3), off > 0.0);
                col = mix(col, tint, 0.8);
            }
        }
    }
    // END ENTITY PASS

    let cloud=cloud_layer(eye,d,t);col=mix(col,cloud.rgb,cloud.a);

    // Visible animated water skin, rather than only a blue volume tint.
    // The classic 16x16 still/flowing effects supply their original alpha ramp.
    if h.water_t < t && h.water_axis >= 0 {
        let point = eye + d * h.water_t;
        let f = point - vec4<f32>(h.water_cell);
        let still_top = h.water_raw == WATER && h.water_axis == 1;
        let tile = select(206, 205, still_top);
        let skin = projected_tile(tile, f, h.water_axis, cam.ana);
        // Liquid brightness in Beta qo.d is max(cell, cell above).
        let light = max(classic_brightness(voxel_light_at(h.water_cell)),
            classic_brightness(voxel_light_at(h.water_cell + vec4<i32>(0, 1, 0, 0))));
        // Beta's fixed-function renderer blended water in display/gamma space.
        // Preserve that deeper blue instead of modern linear-blend pastel water.
        let background = max(col, vec3<f32>(0.0));
        let background_display = select(background * 12.92, 1.055 * pow(background, vec3<f32>(1.0 / 2.4)) - 0.055, background > vec3<f32>(0.0031308));
        let water_display = select(skin.rgb * 12.92, 1.055 * pow(skin.rgb, vec3<f32>(1.0 / 2.4)) - 0.055, skin.rgb > vec3<f32>(0.0031308))
            * light * face_shade(h.water_axis, h.water_sign);
        let blended = mix(background_display, water_display, skin.a);
        col = select(blended / 12.92, pow((blended + 0.055) / 1.055, vec3<f32>(2.4)), blended > vec3<f32>(0.04045));
    }
    if h.water > 0.0 && (h.water_t > 1e29 || h.water_t < t) {
        let depth = min(h.water, max(t - min(h.water_t, t), 0.0) + select(0.0, h.water, h.water_t > 1e29));
        col = mix(col, vec3<f32>(0.01, 0.035, 0.35), 1.0 - exp(-depth * 0.055));
    }

    // Ghost layers: things in the neighbouring slices (±offset along ana) that are closer
    // than what is visible here. Red = ana side, blue = kata side.
    if (flags & FLAG_GHOSTS) != 0u && main_view && t > 0.05 {
        let ga = ghosts.x;
        let gk = ghosts.y;
        if ga < t - 0.05 {
            col = mix(col, vec3<f32>(1.0, 0.25, 0.2), 0.32 * (1.0 - ga / GHOST_RANGE));
        }
        if gk < t - 0.05 {
            col = mix(col, vec3<f32>(0.2, 0.45, 1.0), 0.32 * (1.0 - gk / GHOST_RANGE));
        }
    }
    // BEGIN FUSED WEATHER REFERENCE
    let precipitation=world_precipitation(eye,d,t);
    col=col*(1.0-precipitation.a)+precipitation.rgb;
    // END FUSED WEATHER REFERENCE
    return ShadeResult(col,t);
}

// BEGIN SHARED PIXEL VIEW
struct PixelView { camera: Camera, ray: vec4<f32>, pixel_angle: f32, main_view: bool, border: bool };
fn pixel_view(frag:vec2<f32>)->PixelView {
    let res = g.view.xy;
    var cam: Camera;
    cam.right = g.right;
    cam.up = g.up;
    cam.forward = g.forward;
    cam.ana = g.ana;
    var px = frag;
    var size = res;
    var main_view = true;

    // One ray per output pixel regardless of view count. The last containing view
    // wins, allowing the two-view layout to keep its small overlay inset.
    var selected = 0u;
    var rect = vec4<f32>(0.0, 0.0, res);
    let count = min(u32(g.misc.w), 9u);
    for (var i = 0u; i < count; i++) {
        let r = g.cameras[i * 5u + 4u];
        if all(px >= r.xy) && all(px < r.xy + r.zw) {
            selected = i;
            rect = r;
        }
    }
    if count > 0u {
        cam.right = g.cameras[selected * 5u];
        cam.up = g.cameras[selected * 5u + 1u];
        cam.forward = g.cameras[selected * 5u + 2u];
        cam.ana = g.cameras[selected * 5u + 3u];
    }
    px -= rect.xy;
    size = rect.zw;
    main_view = selected == 0u;
    let border=count>1u && (count>2u || selected>0u) && (any(px<vec2<f32>(1.0)) || any(px>=size-vec2<f32>(1.0)));

    let ndc = vec2<f32>((px.x + 0.5) / size.x * 2.0 - 1.0, 1.0 - (px.y + 0.5) / size.y * 2.0);
    let th = g.view.z;
    let aspect = size.x / size.y;
    let d = normalize(cam.forward + cam.right * (ndc.x * th * aspect) + cam.up * (ndc.y * th));
    let px_angle = 2.0 * th / size.y;
    return PixelView(cam,d,px_angle,main_view,border);
}
// END SHARED PIXEL VIEW
struct FragmentResult {
    @location(0) color:vec4<f32>,
    @location(1) distances:vec2<f32>,
};
fn render_pixel(frag:vec4<f32>)->FragmentResult {
    let pixel=pixel_view(frag.xy);
    if (g.info.y & FLAG_BEAM) != 0u {
        let tiles_x=(u32(g.view.x)+BEAM_TILE-1u)/BEAM_TILE;
        let tile=(u32(frag.y)/BEAM_TILE)*tiles_x+u32(frag.x)/BEAM_TILE;
        ray_start=ray_starts[tile*2u];
        ghost_start=ray_starts[tile*2u+1u];
    } else { ray_start=0.0; ghost_start=0.0; }
    if pixel.border {return FragmentResult(vec4<f32>(0.65,0.68,0.75,1.0),vec2<f32>(1e30));}
    let cam=pixel.camera;let d=pixel.ray;
    if (g.info.y & FLAG_ANAGLYPH)!=0u {
        let left=shade(g.eye-cam.right*0.035,d,cam,pixel.pixel_angle,pixel.main_view);
        let right=shade(g.eye+cam.right*0.035,d,cam,pixel.pixel_angle,pixel.main_view);
        let color=vec3<f32>(dot(left.color,vec3<f32>(0.299,0.587,0.114)),right.color.g,right.color.b);
        return FragmentResult(vec4<f32>(color,1.0),vec2<f32>(left.distance,right.distance));
    }
    let result=shade(g.eye,d,cam,pixel.pixel_angle,pixel.main_view);
    return FragmentResult(vec4<f32>(result.color,1.0),vec2<f32>(result.distance));
}

@fragment
fn fs_main(@builtin(position) frag:vec4<f32>)->FragmentResult {return render_pixel(frag);}
@fragment
fn fs_color(@builtin(position) frag:vec4<f32>)->@location(0) vec4<f32> {return render_pixel(frag).color;}



// Beam prepass: one conservative trace per 16x16 tile through 4^4 macrocells.
// Every ray of the tile (main view ray, both ghost origins, anaglyph eyes)
// stays within `spread * t + extra` of the centre ray, so no ray can meet a
// block before the first macrocell within that radius: they start there.
fn view_at(px: vec2<f32>) -> u32 {
    var selected = 0u;
    let count = min(u32(g.misc.w), 9u);
    for (var i = 0u; i < count; i++) {
        let r = g.cameras[i * 5u + 4u];
        if all(px >= r.xy) && all(px < r.xy + r.zw) { selected = i; }
    }
    return selected;
}
fn macro_cell_occupied(m: vec4<i32>) -> bool {
    let cell = m * 4;
    let e = chunk_entry((cell >> vec4<u32>(4u)) + g.center);
    if (e & ENTRY_OUT) != 0u { return true; }
    if (e & ENTRY_DENSE) != 0u { return macro_occupied(e & 0xFFFFu, cell & vec4<i32>(15), false); }
    if (e & ENTRY_UNLOADED) != 0u { return false; }
    return (e & 255u) != AIR;
}
// Any occupied macrocell in the box around segment p0..p1 grown by r? Boxes
// wider than 3 macrocells along an axis are reported occupied (conservative).
fn beam_box_hit(p0: vec4<f32>, p1: vec4<f32>, r: f32) -> bool {
    let a = vec4<i32>(floor((min(p0, p1) - r) / 4.0));
    let b = vec4<i32>(floor((max(p0, p1) + r) / 4.0));
    if any(b - a > vec4<i32>(2)) { return true; }
    for (var w = a.w; w <= b.w; w++) { for (var z = a.z; z <= b.z; z++) {
        for (var y = a.y; y <= b.y; y++) { for (var x = a.x; x <= b.x; x++) {
            if macro_cell_occupied(vec4<i32>(x, y, z, w)) { return true; }
        }}
    }}
    return false;
}
@compute @workgroup_size(8, 8)
fn cs_beam(@builtin(global_invocation_id) id: vec3<u32>) {
    let res = g.view.xy;
    let tiles = (vec2<u32>(res) + BEAM_TILE - 1u) / BEAM_TILE;
    if id.x >= tiles.x || id.y >= tiles.y { return; }
    let index = id.y * tiles.x + id.x;
    let lo = vec2<f32>(id.xy * BEAM_TILE);
    let hi = min(lo + f32(BEAM_TILE - 1u), res - 1.0);
    let view = view_at(lo);
    // Tiles split between views (different rays) start at the eye.
    if view_at(hi) != view || view_at(vec2<f32>(lo.x, hi.y)) != view || view_at(vec2<f32>(hi.x, lo.y)) != view {
        ray_starts_out[index * 2u] = 0.0; ray_starts_out[index * 2u + 1u] = 0.0; return;
    }
    let pixel = pixel_view((lo + hi) * 0.5);
    let o = g.eye; let d = pixel.ray; let tmax = g.view.w;
    // Pixel angles shrink away from the centre, so the centre's is an upper bound.
    let spread = pixel.pixel_angle * f32(BEAM_TILE) * 0.75;
    let extra = 0.06 + select(0.0, 0.035, (g.info.y & FLAG_ANAGLYPH) != 0u);
    // Ghost rays start up to the ghost offset away along ana: a wider beam.
    let ghost_extra = extra + select(0.0, g.misc.x, (g.info.y & FLAG_GHOSTS) != 0u);
    let inv = vec4<f32>(safe_inv(d.x), safe_inv(d.y), safe_inv(d.z), safe_inv(d.w));
    let stepv = vec4<i32>(sign(d));
    var m = vec4<i32>(floor(o / 4.0));
    var tnext = (vec4<f32>((m + max(stepv, vec4<i32>(0))) * 4) - o) * inv;
    tnext = select(tnext, vec4<f32>(1e30), d == vec4<f32>(0.0));
    let tdelta = abs(inv) * 4.0;
    var t = 0.0;
    var start = max(tmax - 1.0, 0.0);
    var wide_start = start;
    var wide_found = false;
    for (var i = 0; i < 96 && t < tmax; i++) {
        let t1 = min(tnext[min_axis(tnext)], tmax);
        let p0 = o + d * t; let p1 = o + d * t1;
        // The tight beam lies inside the wide one: test it only once wide hits.
        if !wide_found && beam_box_hit(p0, p1, spread * t1 + ghost_extra) { wide_found = true; wide_start = t; }
        if wide_found && beam_box_hit(p0, p1, spread * t1 + extra) { start = t; break; }
        let axis = min_axis(tnext);
        t = tnext[axis];
        m[axis] += stepv[axis];
        tnext[axis] += tdelta[axis];
    }
    ray_starts_out[index * 2u] = max(start - 0.01, 0.0);
    ray_starts_out[index * 2u + 1u] = max(min(wide_start, start) - 0.01, 0.0);
}
