// Globals, camera/pixel selection and precipitation functions are inserted from
// raycast.wgsl by weather_pass.rs. This pass does not trace world voxels/entities.
@group(1) @binding(0) var scene_color:texture_2d<f32>;
@group(1) @binding(1) var scene_distances:texture_2d<f32>;
@vertex
fn vs_main(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));
    return vec4<f32>(uv*2.0-1.0,0.0,1.0);
}
@fragment
fn fs_main(@builtin(position) frag:vec4<f32>)->@location(0) vec4<f32> {
    let coord=vec2<i32>(frag.xy);
    let base=textureLoad(scene_color,coord,0);
    let pixel=pixel_view(frag.xy);
    if pixel.border {return base;}
    let depths=textureLoad(scene_distances,coord,0).rg;
    let cam=pixel.camera;let ray=pixel.ray;
    var color:vec3<f32>;
    if (g.info.y & FLAG_ANAGLYPH)!=0u {
        let left=world_precipitation(g.eye-cam.right*0.035,ray,depths.x);
        let right=world_precipitation(g.eye+cam.right*0.035,ray,depths.y);
        color=vec3<f32>(base.r*(1.0-left.a)+dot(left.rgb,vec3<f32>(0.299,0.587,0.114)),base.gb*(1.0-right.a)+right.gb);
    } else {
        let rain=world_precipitation(g.eye,ray,depths.x);
        color=base.rgb*(1.0-rain.a)+rain.rgb;
    }
    return vec4<f32>(color,base.a);
}
