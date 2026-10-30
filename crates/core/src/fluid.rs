//! Bounded, event-driven water and lava in four spatial dimensions.
//!
//! Y remains gravity; all six +/-X, +/-Z and +/-W neighbors carry horizontal
//! flow. Water sources regenerate under the verified Beta 1.5 two-neighbor
//! rule; lava does not. Interactive edits have a bounded priority queue so
//! activating distant ocean chunks cannot delay a newly placed bucket.
//! Existing source IDs and the byte-per-block save encoding remain compatible.

use crate::block::{self, AIR, COBBLE, FluidKind, OBSIDIAN};
use crate::chunk::{CHUNK, CHUNK_VOLUME, WORLD_HEIGHT};
use glam::IVec4;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

pub const HORIZONTAL: [IVec4; 6] = [IVec4::X, IVec4::NEG_X, IVec4::Z, IVec4::NEG_Z, IVec4::W, IVec4::NEG_W];
const NEIGHBORS: [IVec4; 8] = [IVec4::X, IVec4::NEG_X, IVec4::Y, IVec4::NEG_Y, IVec4::Z, IVec4::NEG_Z, IVec4::W, IVec4::NEG_W];

/// The simulator cannot load or generate terrain. Missing cells are barriers;
/// loaded boundary notifications resume flow when the neighbor becomes known.
pub trait FluidWorld {
    fn fluid_chunk_loaded(&self, _c: IVec4) -> bool { true }
    fn fluid_block(&self, p: IVec4) -> Option<block::BlockId>;
    /// Changes a cell without recursively notifying the scheduler.
    fn set_fluid_block(&mut self, p: IVec4, id: block::BlockId) -> bool;
}

#[derive(Clone, Copy, Debug)]
pub struct FluidBudget {
    pub updates: usize,
    pub scan_cells: usize,
}

impl Default for FluidBudget {
    fn default() -> Self { Self { updates: 128, scan_cells: 512 } }
}

#[derive(Default, Debug)]
pub struct FluidWork {
    pub processed: usize,
    pub scanned: usize,
    pub changed: usize,
}

#[derive(Default)]
pub struct FluidSimulator {
    tick: u64,
    rng: u64,
    remainder: f32,
    queue: VecDeque<(IVec4, u64)>,
    queued: HashMap<IVec4, u64>,
    priority: BTreeMap<u64, VecDeque<IVec4>>,
    priority_due: HashMap<IVec4, u64>,
    priority_len: usize,
    scans: VecDeque<(IVec4, usize)>,
    scanning: HashSet<IVec4>,
}

impl FluidSimulator {
    pub fn pending(&self) -> usize { self.queue.len() + self.priority_due.len() + self.scans.len() }

    fn schedule(&mut self, p: IVec4, delay: u64) {
        if (0..WORLD_HEIGHT).contains(&p.y) && !self.priority_due.contains_key(&p) && !self.queued.contains_key(&p) {
            let due = self.tick.saturating_add(delay);
            self.queued.insert(p, due);
            self.queue.push_back((p, due));
        }
    }

    /// Any player/simulation edit wakes the cell and its eight 4D neighbors.
    pub fn notify_change(&mut self, p: IVec4, delay: u64) {
        self.schedule(p, delay);
        for d in NEIGHBORS { self.schedule(p + d, delay); }
    }

    fn schedule_priority(&mut self, p: IVec4, delay: u64) {
        if !(0..WORLD_HEIGHT).contains(&p.y) { return; }
        let due = self.tick.saturating_add(delay);
        if self.priority_due.get(&p).is_some_and(|&old| old <= due) { return; }
        self.queued.remove(&p); // invalidate a slower background task without scanning its FIFO
        self.priority_due.insert(p, due);
        self.priority.entry(due).or_default().push_back(p);
        self.priority_len += 1;
    }

    /// Player/building changes and their resulting flow are serviced ahead of
    /// background chunk activation, while a quarter of work remains fair FIFO.
    pub fn notify_edit(&mut self, p: IVec4, delay: u64) {
        self.schedule_priority(p, delay);
        for d in NEIGHBORS { self.schedule_priority(p + d, delay); }
    }

    /// Call for loaded chunks containing fluid, including loaded fluid neighbors
    /// of a newly loaded chunk. A scan is incremental and deduplicated.
    pub fn activate_chunk(&mut self, chunk: IVec4) {
        if self.scanning.insert(chunk) { self.scans.push_back((chunk, 0)); }
    }

    pub fn advance(&mut self, world: &mut impl FluidWorld, dt: f32, budget: FluidBudget) -> FluidWork {
        self.remainder += dt.clamp(0.0, 0.1);
        let ticks = (self.remainder / 0.05).floor() as u64;
        self.remainder -= ticks as f32 * 0.05;
        self.tick = self.tick.saturating_add(ticks);
        let mut work = FluidWork::default();
        while work.scanned < budget.scan_cells {
            let Some((c, index)) = self.scans.front_mut() else { break };
            if !world.fluid_chunk_loaded(*c) {
                let (c, _) = self.scans.pop_front().unwrap();
                self.scanning.remove(&c);
                work.scanned += 1;
                continue;
            }
            let i = *index;
            let p = *c * CHUNK + IVec4::new((i & 15) as i32, ((i >> 4) & 15) as i32,
                ((i >> 8) & 15) as i32, ((i >> 12) & 15) as i32);
            *index += 1;
            if *index == CHUNK_VOLUME {
                let (c, _) = self.scans.pop_front().unwrap();
                self.scanning.remove(&c);
            }
            work.scanned += 1;
            if let Some((kind, level)) = world.fluid_block(p).and_then(block::fluid) {
                // Still ocean interiors need no work. Flowing states are always
                // reevaluated after reload to reconstruct the transient queue.
                let exposed = std::iter::once(IVec4::NEG_Y).chain(HORIZONTAL).any(|d| {
                    world.fluid_block(p + d).is_some_and(|id| (!blocks_flow(id) && !block::is_fluid(id))
                        || block::fluid(id).is_some_and(|(other, _)| other != kind))
                });
                if level > 0 || exposed { self.notify_change(p, kind.tick_delay()); }
            }
        }
        let count = budget.updates.min(self.queue.len() + self.priority_len);
        let mut routes=HashMap::new();
        for index in 0..count {
            let ready_priority = self.priority.first_key_value().is_some_and(|(&due, _)| due <= self.tick);
            let urgent = ready_priority && (index % 4 != 3 || self.queue.is_empty());
            let (p, due) = if urgent {
                let mut first = self.priority.first_entry().unwrap();
                let due = *first.key(); let p = first.get_mut().pop_front().unwrap();
                if first.get().is_empty() { first.remove_entry(); }
                self.priority_len -= 1;
                work.processed += 1;
                if self.priority_due.get(&p) != Some(&due) { continue; }
                self.priority_due.remove(&p);
                (p, due)
            } else if let Some(entry) = self.queue.pop_front() {
                work.processed += 1;
                entry
            } else { break; };
            if !urgent && self.queued.get(&p) != Some(&due) { continue; }
            if due > self.tick {
                self.queue.push_back((p, due));
                continue;
            }
            if !urgent { self.queued.remove(&p); }
            let Some(old) = world.fluid_block(p) else { continue };
            let new = desired_with_routes(world, p, old, &mut routes);
            if lava_decay_can_stall(old,new) {
                if self.rng==0{self.rng=0x9e3779b97f4a7c15;}
                self.rng^=self.rng<<13;self.rng^=self.rng>>7;self.rng^=self.rng<<17;
                if (self.rng>>32)%4!=0 {if urgent{self.schedule_priority(p,30);}else{self.schedule(p,30);}continue;}
            }
            if new != old && world.set_fluid_block(p, new) {
                work.changed += 1;
                // Flow levels do not alter route topology. New sources and solid
                // cooling products do, so discard only then within this batch.
                if blocks_flow(old)!=blocks_flow(new)||block::fluid(old).is_some_and(|(_,l)|l==0)!=block::fluid(new).is_some_and(|(_,l)|l==0){routes.clear();}
                let delay = block::fluid(new).or_else(|| block::fluid(old))
                    .map_or(5, |(kind, _)| kind.tick_delay());
                if urgent { self.notify_edit(p, delay); } else { self.notify_change(p, delay); }
            }
        }
        work
    }
}

/// Original np.l checks material movement blocking, with explicit doors,
/// standing signs, ladders and reeds as barriers. Collision solidity is not
/// interchangeable (for example, pressure plates block liquid, rails wash away).
/// Verified against client.jar by FluidMaterials.java / fluid-materials.txt.
pub fn blocks_flow(id: block::BlockId) -> bool {
    if id == AIR || block::is_fluid(id) { return false; }
    !matches!(block::definition(id).reference_id,
        6 | 27 | 28 | 37 | 38 | 39 | 40 | 50 | 51 | 55 | 59 | 66 | 69 |
        75 | 76 | 77 | 78 | 93 | 94)
}

fn lava_decay_can_stall(old:u8,new:u8)->bool {
    matches!((block::fluid(old),block::fluid(new)),(Some((FluidKind::Lava,a)),Some((FluidKind::Lava,b)))if a<8&&b<8&&b>a)
}
/// Native np.b/np.k slope search: direct drops cost0; recursion has depth4,
/// which permits an opening five horizontal steps from the source. Ties spread.
fn preferred_flow(world:&impl FluidWorld,source:IVec4,kind:FluidKind)->[bool;6]{
    let passable=|p|world.fluid_block(p).is_some_and(|id|!blocks_flow(id)&&block::fluid(id)!=Some((kind,0)));
    let drop=|p|world.fluid_block(p-IVec4::Y).is_some_and(|id|!blocks_flow(id));
    let allowed=HORIZONTAL.map(|d|passable(source+d));
    // A flat solid floor has no reachable drop; avoid traversing the same
    // branching paths thousands of times in the common pool case.
    let mut any_drop=false;
    'scan:for w in -5i32..=5{for z in -5i32..=5{for x in -5i32..=5{let distance=x.abs()+z.abs()+w.abs();if distance>0&&distance<=5&&drop(source+IVec4::new(x,0,z,w)){any_drop=true;break 'scan;}}}}
    if !any_drop{return allowed;}
    let mut costs=[1000u16;6];
    for (first,&direction)in HORIZONTAL.iter().enumerate(){
        if !allowed[first]{continue;}
        let start=source+direction;if drop(start){costs[first]=0;continue;}
        let mut queue=VecDeque::from([(start,1u16,direction)]);let mut seen=HashSet::from([start]);
        while let Some((p,depth,previous))=queue.pop_front(){
            for d in HORIZONTAL {if d== -previous{continue;}let q=p+d;if !passable(q){continue;}
                if drop(q){costs[first]=depth;queue.clear();break;}
                if depth<4&&seen.insert(q){queue.push_back((q,depth+1,d));}
            }
        }
    }
    let minimum=*costs.iter().min().unwrap();std::array::from_fn(|i|allowed[i]&&costs[i]==minimum)
}
#[cfg(test)]
fn desired(world:&impl FluidWorld,p:IVec4,old:u8)->u8{desired_with_routes(world,p,old,&mut HashMap::new())}
fn desired_with_routes(world: &impl FluidWorld, p: IVec4, old: block::BlockId,routes:&mut HashMap<(IVec4,u8),[bool;6]>) -> block::BlockId {
    let current = block::fluid(old);
    if current.is_none() && blocks_flow(old) { return old; }
    if let Some((FluidKind::Lava, level)) = current {
        if level<=4 && std::iter::once(IVec4::Y).chain(HORIZONTAL).any(|d| world.fluid_block(p+d).is_some_and(block::is_water)) {
            return if level == 0 { OBSIDIAN } else { COBBLE };
        }
    }
    if current.is_some_and(|(_, level)| level == 0) { return old; }
    // Freeze existing flow at unknown borders: absence of loaded terrain is not
    // proof that its supply disappeared. Newly loaded neighbors wake this again.
    if current.is_some() && NEIGHBORS.iter().any(|&d| world.fluid_block(p + d).is_none()) {
        return old;
    }
    // Verified np.a offsets184..250 and et.g: two adjacent source cells
    // regenerate water above an opaque block (to.a()). Beta 1.5's fallback
    // checks the CURRENT cell metadata rather than the cell below; therefore
    // a flowing cell over source water does not regenerate in this version.
    if current.is_some_and(|(kind, level)| kind == FluidKind::Water && level > 0)
        && world.fluid_block(p - IVec4::Y).is_some_and(|id| block::definition(id).opaque)
        && HORIZONTAL.iter().filter(|&&d| world.fluid_block(p + d) == Some(block::WATER)).count() >= 2 {
            return block::WATER;
        }
    for kind in [FluidKind::Water, FluidKind::Lava] {
        if current.is_some_and(|(other, _)| other != kind) { continue; }
        if world.fluid_block(p + IVec4::Y).and_then(block::fluid).is_some_and(|(k, _)| k == kind) {
            return kind.flowing(8);
        }
        let mut best = 8;
        for d in HORIZONTAL {
            let neighbor = p + d;
            let Some((k, level)) = world.fluid_block(neighbor).and_then(block::fluid) else { continue };
            if k != kind { continue; }
            // A falling column spreads sideways only when it meets support.
            // Other horizontal flow likewise favors falling over spreading.
            let supported = world.fluid_block(neighbor - IVec4::Y).is_some_and(|id| {
                blocks_flow(id) || block::fluid(id).is_some_and(|(k, l)| k == kind && l != 8)
            });
            if !supported { continue; }
            if current.is_none() {
                let directions=routes.entry((neighbor,kind as u8)).or_insert_with(||preferred_flow(world,neighbor,kind));
                let toward=HORIZONTAL.iter().position(|&axis|axis== -d).unwrap();if !directions[toward]{continue;}
            }
            let next=if level==8{1}else{level+kind.horizontal_decay()};
            best = best.min(next);
        }
        if best < 8 { return kind.flowing(best); }
    }
    if current.is_some() { AIR } else { old }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{LAVA, STONE, WATER};
    use std::collections::HashMap;

    struct TestWorld { blocks: HashMap<IVec4, u8>, max_x: i32 }
    impl TestWorld {
        fn new() -> Self { Self { blocks: HashMap::new(), max_x: 12 } }
        fn set(&mut self, p: IVec4, id: u8, sim: &mut FluidSimulator) {
            self.blocks.insert(p, id);
            sim.notify_edit(p, block::fluid(id).map_or(5, |(k, _)| k.tick_delay()));
        }
    }
    impl FluidWorld for TestWorld {
        fn fluid_block(&self, p: IVec4) -> Option<u8> {
            if p.x > self.max_x || p.x < -12 || p.z.abs() > 12 || p.w.abs() > 12 { return None; }
            Some(if p.y <= 0 { STONE } else { *self.blocks.get(&p).unwrap_or(&AIR) })
        }
        fn set_fluid_block(&mut self, p: IVec4, id: u8) -> bool {
            if self.fluid_block(p).is_none() { return false; }
            self.blocks.insert(p, id); true
        }
    }
    fn settle(w: &mut TestWorld, s: &mut FluidSimulator, frames: usize) {
        for _ in 0..frames { s.advance(w, 0.1, FluidBudget { updates: 1024, scan_cells: 0 }); }
    }

    #[test]
    fn lava_decay_stall_retains_old_depth_and_schedules_a_real_retry(){
        let mut world=TestWorld::new();let mut sim=FluidSimulator::default();let p=IVec4::Y;let old=FluidKind::Lava.flowing(2);world.blocks.insert(p,old);world.blocks.insert(p+IVec4::X,old);assert_eq!(desired(&world,p,old),FluidKind::Lava.flowing(4));
        sim.rng=4;sim.schedule(p,0);let work=sim.advance(&mut world,0.0,FluidBudget{updates:1,scan_cells:0});assert_eq!(work.changed,0);assert_eq!(world.fluid_block(p),Some(old));assert_eq!(sim.queued.get(&p),Some(&30));
        for _ in 0..14{sim.advance(&mut world,0.1,FluidBudget{updates:1,scan_cells:0});}assert_eq!(world.fluid_block(p),Some(old));
        for _ in 0..300{sim.advance(&mut world,0.1,FluidBudget{updates:1,scan_cells:0});if world.fluid_block(p)!=Some(old){return;}}panic!("stalled lava never retried");
    }

    #[test]
    fn native_downhill_search_prefers_nearest_drop_and_ties_include_w(){
        let source=IVec4::new(0,2,0,0);let mut world=TestWorld::new();world.blocks.insert(source,WATER);
        for x in -6..=6{for z in -6..=6{for w in -6..=6{world.blocks.insert(IVec4::new(x,1,z,w),STONE);}}}
        world.blocks.insert(source+IVec4::X-IVec4::Y,AIR);
        let selected=preferred_flow(&world,source,FluidKind::Water);assert_eq!(selected,[true,false,false,false,false,false]);
        assert_eq!(desired(&world,source+IVec4::W,AIR),AIR);assert_eq!(desired(&world,source+IVec4::X,AIR),FluidKind::Water.flowing(1));
        world.blocks.insert(source+IVec4::W-IVec4::Y,AIR);assert_eq!(preferred_flow(&world,source,FluidKind::Water),[true,false,false,false,true,false]);
    }
    #[test]
    fn native_cooling_ignores_water_below_and_thin_or_falling_lava(){
        let p=IVec4::new(0,3,0,0);let mut world=TestWorld::new();world.blocks.insert(p,LAVA);world.blocks.insert(p-IVec4::Y,WATER);assert_eq!(desired(&world,p,LAVA),LAVA);
        world.blocks.insert(p+IVec4::W,WATER);assert_eq!(desired(&world,p,LAVA),OBSIDIAN);
        let thin=FluidKind::Lava.flowing(5);world.blocks.insert(p,thin);assert_ne!(desired(&world,p,thin),COBBLE);
        assert!(lava_decay_can_stall(FluidKind::Lava.flowing(2),FluidKind::Lava.flowing(4)));assert!(!lava_decay_can_stall(FluidKind::Lava.flowing(6),AIR));assert!(!lava_decay_can_stall(FluidKind::Water.flowing(1),FluidKind::Water.flowing(2)));
    }

    #[test]
    fn liquid_washes_plants_and_rails_but_respects_original_material_barriers() {
        let mut w = TestWorld::new(); let source=IVec4::Y;
        w.blocks.insert(source, WATER);
        for id in [block::SAPLING,block::CROPS,block::TORCH,block::RAIL,block::REPEATER] {
            let p=source+IVec4::X;w.blocks.insert(p,id);
            assert_eq!(desired(&w,p,id),FluidKind::Water.flowing(1),"{id}");
        }
        for id in [block::WOOD_DOOR,block::IRON_DOOR,block::SIGN_POST,block::LADDER,block::REEDS,block::WOOD_PRESSURE_PLATE,block::WEB] {
            let p=source+IVec4::W;w.blocks.insert(p,id);
            assert_eq!(desired(&w,p,id),id,"{id}");
        }
        assert_eq!(desired(&w,IVec4::Y*4,block::SAPLING),block::SAPLING,"dry plants must not disappear");
    }

    #[test]
    fn falls_then_spreads_on_all_three_horizontal_axes() {
        let mut w = TestWorld::new(); let mut s = FluidSimulator::default();
        let source = IVec4::new(0, 4, 0, 0); w.set(source, WATER, &mut s);
        settle(&mut w, &mut s, 250);
        assert_eq!(w.fluid_block(source - IVec4::Y), Some(FluidKind::Water.flowing(8)));
        for d in HORIZONTAL {
            assert!(w.fluid_block(IVec4::Y + d).is_some_and(block::is_water), "missing flow along {d}");
            assert_eq!(w.fluid_block(source + d), Some(AIR), "unsupported source should prefer down");
        }
    }

    #[test]
    fn isolated_source_removal_drains_all_dependent_flow() {
        let mut w = TestWorld::new(); let mut s = FluidSimulator::default();
        w.set(IVec4::Y, WATER, &mut s); settle(&mut w, &mut s, 200);
        assert!(w.blocks.values().filter(|&&id| block::is_water(id)).count() > 10);
        assert_eq!(w.blocks.values().filter(|&&id| id == WATER).count(), 1);
        w.set(IVec4::Y, AIR, &mut s); settle(&mut w, &mut s, 500);
        assert!(!w.blocks.values().any(|&id| block::is_water(id)));
    }

    #[test]
    fn waterfall_drains_and_horizontal_reach_is_finite() {
        let mut w = TestWorld::new(); let mut s = FluidSimulator::default();
        let source = IVec4::new(0, 5, 0, 0);
        w.set(source, WATER, &mut s); settle(&mut w, &mut s, 300);
        assert!(w.fluid_block(IVec4::Y + IVec4::X * 7).is_some_and(block::is_water));
        assert_eq!(w.fluid_block(IVec4::Y + IVec4::X * 8), Some(AIR));
        w.set(source, AIR, &mut s); settle(&mut w, &mut s, 600);
        assert!(!w.blocks.values().any(|&id| block::is_water(id)), "falling columns must not become independent sources");
    }

    #[test]
    fn obstacles_and_unknown_borders_are_not_overwritten() {
        let mut w = TestWorld::new(); let mut s = FluidSimulator::default();
        w.max_x = 0;
        w.set(IVec4::Y - IVec4::X, STONE, &mut s);
        w.set(IVec4::Y, WATER, &mut s); settle(&mut w, &mut s, 100);
        assert_eq!(w.fluid_block(IVec4::Y - IVec4::X), Some(STONE));
        assert_eq!(w.fluid_block(IVec4::Y + IVec4::X), None);
        assert!(!w.blocks.contains_key(&(IVec4::Y + IVec4::X)));
        w.max_x = 12;
        s.activate_chunk(IVec4::ZERO);
        for _ in 0..300 { s.advance(&mut w, 0.1, FluidBudget { updates: 1024, scan_cells: 512 }); }
        assert!(w.fluid_block(IVec4::Y + IVec4::X).is_some_and(block::is_water));
    }

    #[test]
    fn lava_is_slower_and_cools_against_water() {
        let mut water = TestWorld::new(); let mut ws = FluidSimulator::default();
        let mut lava = TestWorld::new(); let mut ls = FluidSimulator::default();
        water.set(IVec4::Y, WATER, &mut ws); lava.set(IVec4::Y, LAVA, &mut ls);
        settle(&mut water, &mut ws, 5); settle(&mut lava, &mut ls, 5);
        assert!(water.fluid_block(IVec4::Y + IVec4::X).is_some_and(block::is_water));
        assert_eq!(lava.fluid_block(IVec4::Y + IVec4::X), Some(AIR));
        settle(&mut lava, &mut ls, 20);
        assert_eq!(lava.fluid_block(IVec4::Y + IVec4::X), Some(FluidKind::Lava.flowing(2)));
        lava.set(IVec4::Y + IVec4::X, WATER, &mut ls); settle(&mut lava, &mut ls, 20);
        assert_eq!(lava.fluid_block(IVec4::Y), Some(OBSIDIAN));
    }

    #[test]
    fn two_sources_restore_a_supported_pool_across_x_or_w() {
        for direction in [IVec4::X, IVec4::W] {
            let mut w = TestWorld::new(); let mut s = FluidSimulator::default();
            let p = IVec4::Y;
            w.set(p - direction, WATER, &mut s); w.set(p + direction, WATER, &mut s);
            settle(&mut w, &mut s, 150);
            assert_eq!(w.fluid_block(p), Some(WATER));
            w.set(p, AIR, &mut s); settle(&mut w, &mut s, 150);
            assert_eq!(w.fluid_block(p), Some(WATER), "bucket removal must recover between two sources");
        }
    }

    #[test]
    fn source_regeneration_keeps_beta_support_quirk_and_excludes_lava() {
        let p = IVec4::new(0, 3, 0, 0);
        let mut w = TestWorld::new();
        for d in [IVec4::X, IVec4::NEG_X] {
            w.blocks.insert(p + d, WATER); w.blocks.insert(p + d - IVec4::Y, STONE);
        }
        let flow = FluidKind::Water.flowing(1);
        w.blocks.insert(p, flow);
        assert_ne!(desired(&w, p, flow), WATER, "air below is unsupported");
        w.blocks.insert(p - IVec4::Y, WATER);
        assert_ne!(desired(&w, p, flow), WATER, "preserve Beta 1.5's current-Y metadata check");
        w.blocks.insert(p - IVec4::Y, block::GLASS);
        assert_ne!(desired(&w, p, flow), WATER, "et.g checks opaque support, not merely collision");
        w.blocks.insert(p - IVec4::Y, STONE);
        assert_eq!(desired(&w, p, flow), WATER);
        for d in [IVec4::X, IVec4::NEG_X] { w.blocks.insert(p + d, LAVA); }
        let lava = FluidKind::Lava.flowing(2);
        w.blocks.insert(p, lava);
        assert_ne!(desired(&w, p, lava), LAVA, "lava has no two-source regeneration");
    }

    #[test]
    fn interactive_flow_is_not_starved_by_a_large_ocean_activation_queue() {
        let mut w = TestWorld::new(); let mut s = FluidSimulator::default();
        for x in -12..=12 { for z in -12..=12 { for ww in -12..=12 {
            let p = IVec4::new(x, 1, z, ww);
            w.blocks.insert(p, WATER); s.notify_change(p, 5);
        }}}
        assert!(s.pending() > 10_000);
        let edited = IVec4::new(0, 4, 0, 0);
        w.set(edited, WATER, &mut s);
        for _ in 0..4 {
            let work = s.advance(&mut w, 0.1, FluidBudget { updates: 128, scan_cells: 0 });
            assert!(work.processed <= 128);
        }
        assert!(w.fluid_block(edited - IVec4::Y).is_some_and(block::is_water), "new bucket must flow within its tick delay despite background backlog");
        assert!(s.pending() > 10_000, "test must not pass merely by draining the ocean queue first");
    }

    #[test]
    fn both_queue_updates_and_load_scans_obey_budgets() {
        let mut w = TestWorld::new(); let mut s = FluidSimulator::default();
        w.set(IVec4::Y, WATER, &mut s); s.activate_chunk(IVec4::ZERO);
        for _ in 0..100 {
            let work = s.advance(&mut w, 0.1, FluidBudget { updates: 3, scan_cells: 7 });
            assert!(work.processed <= 3 && work.changed <= 3 && work.scanned <= 7);
        }
        assert!(s.pending() > 0);
    }
}
