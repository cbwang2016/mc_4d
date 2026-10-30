struct Uniforms {transform:mat4x4<f32>,model:mat4x4<f32>,light:vec4<f32>,light0:vec4<f32>,light1:vec4<f32>,exclusions:array<vec4<f32>,8>,info:vec4<u32>};
@group(0) @binding(0) var<uniform> g:Uniforms;
@group(0) @binding(1) var atlas:texture_2d_array<f32>;
@group(0) @binding(2) var pixels:sampler;
struct VertexIn {@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>,@location(3) tint:vec3<f32>,@location(4) layer:u32};
struct VertexOut {@builtin(position) position:vec4<f32>,@location(0) normal:vec3<f32>,@location(1) uv:vec2<f32>,@location(2) tint:vec3<f32>,@location(3) @interpolate(flat) layer:u32};
@vertex fn vs_main(v:VertexIn)->VertexOut {var o:VertexOut;o.position=g.transform*vec4<f32>(v.position,1.0);o.normal=normalize((g.model*vec4<f32>(v.normal,0.0)).xyz);o.uv=v.uv;o.tint=v.tint;o.layer=v.layer;return o;}
@fragment fn fs_main(v:VertexOut)->@location(0) vec4<f32>{
    for(var i=0u;i<g.info.x;i++){let rect=g.exclusions[i];if all(v.position.xy>=rect.xy)&&all(v.position.xy<rect.xy+rect.zw){discard;}}
    let tex=textureSampleLevel(atlas,pixels,v.uv,i32(v.layer),0.0);if tex.a<0.1 {discard;}
    let normal=normalize(v.normal);
    let lighting=clamp(0.4+0.6*max(dot(normal,normalize(g.light0.xyz)),0.0)+0.6*max(dot(normal,normalize(g.light1.xyz)),0.0),0.0,1.0)*g.light.x;
    let encoded=select(tex.rgb*12.92,1.055*pow(tex.rgb,vec3<f32>(1.0/2.4))-0.055,tex.rgb>vec3<f32>(0.0031308));
    let lit=max(encoded*v.tint*lighting,vec3<f32>(0.0));
    let linear=select(lit/12.92,pow((lit+0.055)/1.055,vec3<f32>(2.4)),lit>vec3<f32>(0.04045));
    return vec4<f32>(linear,tex.a);
}
