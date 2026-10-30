// Second occupancy level for ray traversal: one bit per 2x2x2x2 group of
// voxels (4096 bits per chunk slot), rebuilt on the GPU for uploaded slots.
@group(0) @binding(0) var<storage, read> pool: array<u32>;
@group(0) @binding(1) var<storage, read_write> fine: array<u32>;
@group(0) @binding(2) var<storage, read> slots: array<u32>;

const CHUNK_WORDS: u32 = 16384u;

@compute @workgroup_size(128)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) word: u32) {
    let slot = slots[group.x];
    var bits = 0u;
    for (var b = 0u; b < 32u; b++) {
        let sub = word * 32u + b;
        let base = vec4<u32>(sub & 7u, (sub >> 3u) & 7u, (sub >> 6u) & 7u, (sub >> 9u) & 7u) * 2u;
        var any = 0u;
        for (var dw = 0u; dw < 2u; dw++) { for (var dz = 0u; dz < 2u; dz++) { for (var dy = 0u; dy < 2u; dy++) {
            // x and x+1 are adjacent bytes of one word (x is even).
            let index = base.x | ((base.y + dy) << 4u) | ((base.z + dz) << 8u) | ((base.w + dw) << 12u);
            any |= (pool[slot * CHUNK_WORDS + (index >> 2u)] >> ((index & 3u) * 8u)) & 0xffffu;
        }}}
        if any != 0u { bits |= 1u << b; }
    }
    fine[slot * 128u + word] = bits;
}
