#!/usr/bin/env python3
"""Generate the native ItemRenderer dispatch/tile/bounds table from DumpHeldBlocks."""
import json
from pathlib import Path
root=Path(__file__).resolve().parents[1]
records=json.loads((root/'crates/render/assets/minecraft-beta-1.5/held-blocks.json').read_text())
by_id={r['reference_id']:r for r in records}
lines=['// Generated from the verified Beta1.5 qa -> cn held-block dispatch.',
       'pub(super) struct HeldBlock {pub render_type:i16,pub bounds:[f32;6],pub tiles:[[i16;6];16]}',
       'pub(super) static BLOCKS:[HeldBlock;96]=[']
for i in range(96):
 r=by_id.get(i,{'render_type':255,'bounds':[0,0,0,1,1,1],'tiles':[[0]*6]*16})
 lines.append('HeldBlock {render_type:'+str(r['render_type'])+',bounds:'+str([float(v) for v in r['bounds']])+',tiles:'+str(r['tiles'])+'}, // '+str(i))
lines.append('];')
(root/'crates/render/src/held_blocks.rs').write_text('\n'.join(lines)+'\n')
