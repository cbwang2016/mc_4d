struct Uniforms{size:vec4<f32>,rects:array<vec4<f32>,9>,info:vec4<u32>};
@group(0) @binding(0)var<uniform>u:Uniforms;
@group(0) @binding(1)var atlas:texture_2d_array<f32>;
@group(0) @binding(2)var pixel_sampler:sampler;
@group(0) @binding(3)var scene_depth:texture_2d<f32>;
struct Input{@location(0)clip:vec4<f32>,@location(1)delta:vec4<f32>,@location(2)uv:vec2<f32>,@location(3)color:vec4<f32>,@location(4)view:u32,@location(5)eye:u32,@location(6)layer:u32};
struct Output{@builtin(position)position:vec4<f32>,@location(0)delta:vec4<f32>,@location(1)uv:vec2<f32>,@location(2)color:vec4<f32>,@location(3)@interpolate(flat)view:u32,@location(4)@interpolate(flat)eye:u32,@location(5)@interpolate(flat)layer:u32};
@vertex fn vs_main(i:Input)->Output{return Output(i.clip,i.delta,i.uv,i.color,i.view,i.eye,i.layer);}
@fragment fn fs_main(i:Output)->@location(0)vec4<f32>{
 let rect=u.rects[i.view];if any(i.position.xy<rect.xy)||any(i.position.xy>=rect.xy+rect.zw){discard;}
 for(var v=i.view+1u;v<u.info.x;v++){let r=u.rects[v];if all(i.position.xy>=r.xy)&&all(i.position.xy<r.xy+r.zw){discard;}}
 let coord=clamp(vec2<i32>(i.position.xy/u.size.xy*u.size.zw),vec2<i32>(0),vec2<i32>(u.size.zw)-vec2<i32>(1));
 let depths=textureLoad(scene_depth,coord,0).xy;let depth=select(depths.x,depths.y,i.eye==2u);if length(i.delta)>depth+0.015{discard;}
 let tex=textureSampleLevel(atlas,pixel_sampler,i.uv,i32(i.layer),0.0);if tex.a<0.1{discard;}
 let encoded=select(tex.rgb*12.92,1.055*pow(max(tex.rgb,vec3<f32>(0.0)),vec3<f32>(1.0/2.4))-0.055,tex.rgb>vec3<f32>(0.0031308));let lit=max(encoded*i.color.rgb,vec3<f32>(0.0));var rgb=select(lit/12.92,pow((lit+0.055)/1.055,vec3<f32>(2.4)),lit>vec3<f32>(0.04045));
 if i.eye==1u{rgb=vec3<f32>(dot(rgb,vec3<f32>(0.299,0.587,0.114)),0.0,0.0);}else if i.eye==2u{rgb=vec3<f32>(0.0,rgb.g,rgb.b);}
 return vec4<f32>(rgb,tex.a*i.color.a);
}
