// qa fire overlay: two mirrored animated terrain quads, alpha0.9, translated
// ±0.24 horizontally and−0.3 vertically, yawed∓10 degrees toward the player.
struct Params {viewport:vec4<f32>,exclusions:array<vec4<f32>,8>,info:vec4<f32>};
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var atlas:texture_2d_array<f32>;
@group(0) @binding(2) var nearest:sampler;
@vertex fn vs_main(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));return vec4<f32>(uv*2.0-1.0,0.0,1.0);}
fn rotate_y(v:vec3<f32>,a:f32)->vec3<f32>{return vec3<f32>(cos(a)*v.x+sin(a)*v.z,v.y,-sin(a)*v.x+cos(a)*v.z);}
@fragment fn fs_main(@builtin(position) frag:vec4<f32>)->@location(0) vec4<f32>{
 if any(frag.xy<p.viewport.xy)||any(frag.xy>=p.viewport.xy+p.viewport.zw){discard;}
 for(var i=0u;i<u32(p.info.x);i++){let r=p.exclusions[i];if all(frag.xy>=r.xy)&&all(frag.xy<r.xy+r.zw){discard;}}
 let ndc=(frag.xy-p.viewport.xy)/p.viewport.zw*2.0-1.0;
 let ray=vec3<f32>(ndc.x*p.info.y*p.viewport.z/p.viewport.w,-ndc.y*p.info.y,-1.0);
 var color=vec3<f32>(0.0);var alpha=0.0;
 for(var side=0u;side<2u;side++){
  let sign_side=select(-1.0,1.0,side==1u);let angle=sign_side*0.174532925;
  let origin=rotate_y(-vec3<f32>(-sign_side*0.24,-0.3,0.0),-angle);let direction=rotate_y(ray,-angle);
  let t=(-0.5-origin.z)/direction.z;let hit=origin+direction*t;
  if t<=0.0||abs(hit.x)>0.5||abs(hit.y)>0.5{continue;}
  var uv=vec2<f32>(hit.x+0.5,0.5-hit.y);if side==1u{uv.x=1.0-uv.x;}
  let tile=31u+side*16u;let at=(vec2<f32>(f32(tile%16u),f32(tile/16u))+clamp(uv,vec2<f32>(0.001),vec2<f32>(0.999)))/16.0;
  let sample=textureSampleLevel(atlas,nearest,at,0,0.0);let a=sample.a*0.9;
  color=sample.rgb*a+color*(1.0-a);alpha=a+alpha*(1.0-a);
 }
 if alpha<=0.001{discard;}return vec4<f32>(color/alpha,alpha);
}
