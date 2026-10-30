#!/usr/bin/env python3
"""Reproduce the Beta1.5 painting catalogue from the local verified JAR.
Requires a JDK with jdk.jdeps; all generated files stay in this repository.
"""
import hashlib,json,re,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
REFERENCE=ROOT/'target/reference/beta-1.5'
jar=REFERENCE/'client.jar'
assert hashlib.sha1(jar.read_bytes()).hexdigest()=='f5ce1699cd728213c21054fa2f1490d162b002b4'
text=subprocess.check_output(['java','-XX:-UsePerfData',f'-Djava.io.tmpdir={ROOT / "target/tooling/tmp"}','--module','jdk.jdeps/com.sun.tools.javap.Main','-classpath',str(jar),'-c','-p','ib'],text=True)
(REFERENCE/'painting-art-bytecode.txt').write_text(text)
arts=[]
for part in re.split(r'new\s+#\d+\s+// class ib',text)[1:]:
    part=part.split('invokespecial')[0];names=re.findall(r'// String (\S+)',part);numbers=[]
    for line in part.splitlines():
        operand=re.search(r': (?:bipush|sipush)\s+(-?\d+)',line)
        small=re.search(r': iconst_(\d)',line)
        if operand:numbers.append(int(operand[1]))
        if small:numbers.append(int(small[1]))
    if names and len(numbers)==5:arts.append(dict(name=names[-1],width=numbers[1],height=numbers[2],u=numbers[3],v=numbers[4]))
assert len(arts)==25
(REFERENCE/'painting-art-verified.json').write_text(json.dumps(arts,indent=2)+'\n')
output='//! Original25 paintings reflected from Beta1.5 ib enum.\n#[derive(Clone,Copy,Debug)]\npub struct Art {pub name:&\'static str,pub width:u32,pub height:u32,pub u:u32,pub v:u32}\npub const ARTS:[Art;25]=[\n'
for a in arts:output+='Art{name:'+json.dumps(a['name'])+','+','.join(f'{key}:{a[key]}'for key in ['width','height','u','v'])+'},\n'
(ROOT/'crates/core/src/paintings.rs').write_text(output+'];\n')
shader=ROOT/'crates/render/shaders/raycast.wgsl';source=shader.read_text();start=source.index('fn painting_art(');end=source.index('fn model_count(',start)
lookup='fn painting_art(index:u32)->vec4<f32>{\n switch (index & 31u) {\n'
for i,a in enumerate(arts):lookup+=f'case {i}u: {{ return vec4<f32>({a["u"]}.0,{a["v"]}.0,{a["width"]}.0,{a["height"]}.0); }}\n'
lookup+='default: {return vec4<f32>(0.0,0.0,16.0,16.0);}\n }\n}\n\n'
shader.write_text(source[:start]+lookup+source[end:])
print('Reflected25 paintings; wrote shared catalogue and shader lookup.')
