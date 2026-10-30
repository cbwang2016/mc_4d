#!/usr/bin/env python3
"""Regenerate native model cases from the verified Beta constructor snapshot.

DumpModels.java extracts the source snapshot from the unmodified Beta1.5 JAR.
This generator only changes the delimited model table in raycast.wgsl.
"""
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
import json
models={m['class']:m['parts'] for m in json.load(open(ROOT / 'crates/render/assets/minecraft-beta-1.5/models.json'))}
classes={1:'dx',2:'gv',3:'ma',4:'rz',5:'vf',6:'dz',7:'kg',8:'qz',9:'hw',10:'ms',11:'n',18:'ep',19:'mu'}
def vec(v):return 'vec3<f32>('+', '.join(str(float(x)) for x in v)+')'
def part(p,kind,animate=True):
 angles=list(p['pose'][3:]);pivot=p['pose'][:3]
 if kind==18:pivot[1]+=17
 if kind==19:pivot[1]+=19
 if kind==11:
  if p['name']=='j':angles[0]=0.8
 a=vec(angles)
 # Retain per-limb animation about original pivots instead of moving whole boxes.
 leg=(kind in (1,2,3) and p['name'] in 'fghi') or (kind in (6,7) and p['name'] in 'fg') or (kind==4 and p['name'] in 'cd') or (kind==9 and p['name'] in 'defg') or (kind==11 and p['name'] in 'cdef')
 if leg and animate:a+=' + vec3<f32>(sin(phase + '+str(3.1415927 if ord(p['name'])%2 else 0.0)+') * 0.45, 0.0, 0.0)'
 return ('vehicle_model_box(' if kind in (18,19) else 'native_model_box(')+vec(p['lo'])+', '+vec(p['hi'])+', '+vec(pivot)+', '+a+', vec2<f32>('+', '.join(map(str,p['uv']))+'), '+vec([round(v+0.2) for v in p['size']])+(', 0.75' if kind==18 else ', 0.49' if kind==19 else '')+')'
out=['// Generated from verified Beta1.5 constructor vertices, pivots, skin offsets and neutral poses.','fn classic_model_count(kind: u32) -> u32 {','    switch kind {']
filtered={}
for kind,c in classes.items():
 ps=models[c]
 if kind in (6,7):ps=[p for p in ps if p['name'] in 'acdefg']
 if kind==9:ps=[p for p in ps if p['name']!='b']
 filtered[kind]=ps;out.append(f'        case {kind}u: {{ return {12 if kind==3 else len(ps)}u; }}')
out+=['        default: { return 0u; }','    }','}','fn classic_model_part(kind: u32, part: u32, phase: f32, state: u32) -> ModelPart {','    switch kind {']
for kind,ps in filtered.items():
 out.append(f'        case {kind}u: {{');out.append('            switch part {')
 for i,p in enumerate(ps):
  if kind==3:
   wool=next(w for w in models['dr'] if w['name']==p['name'])
   out.append(f'                case {i}u: {{ return {part(p,kind)}; }}')
   out.append(f'                case {i+6}u: {{ return {part(wool,kind)}; }}')
  elif kind==11:
   sitting=next(w for w in models['n_sitting'] if w['name']==p['name'])
   out.append(f'                case {i}u: {{ if (state & 4u) != 0u {{ return {part(sitting,kind,False)}; }} return {part(p,kind)}; }}')
  else:out.append(f'                case {i}u: {{ return {part(p,kind)}; }}')
 out+=['                default: {}','            }','        }']
out+=['        default: {}','    }','    return model_box(vec3<f32>(0.0), vec3<f32>(1.0), vec2<f32>(0.0));','}']
shader = ROOT / 'crates/render/shaders/raycast.wgsl'
source = shader.read_text()
start = source.index('// Generated from verified Beta1.5 constructor')
end = source.index('fn model_count(', start)
shader.write_text(source[:start] + '\n'.join(out) + '\n' + source[end:])
