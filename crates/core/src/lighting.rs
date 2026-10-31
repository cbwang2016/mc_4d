//! Derived four-dimensional voxel lighting. Geometry snapshots are copy-on-write;
//! the worker performs column skylight and bounded propagation away from the game
//! thread. Saved chunks contain geometry/state only, never stale derived light.
use crate::{
    block,
    chunk::{CHUNK, CHUNK_VOLUME, Chunk, WORLD_CHUNKS_Y, chunk_of, local_index, local_of},
};
use glam::IVec4;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Duration,
};

const NEIGHBORS: [IVec4; 8] = [
    IVec4::X,
    IVec4::NEG_X,
    IVec4::Y,
    IVec4::NEG_Y,
    IVec4::Z,
    IVec4::NEG_Z,
    IVec4::W,
    IVec4::NEG_W,
];
const BLOCK: u8 = 0;
const SKY: u8 = 1;

/// Beta sky subtraction affects only the sky channel; moonlight remains level4.
/// Rain/thunder use the original5/16 attenuation, and flashes restore daylight
/// only where a propagated sky component exists.
pub fn effective_level(
    block: u8,
    sky: u8,
    daylight: f32,
    rain: f32,
    thunder: bool,
    flash: f32,
) -> u8 {
    let weather = 1.0 - rain.clamp(0.0, 1.0) * 5.0 / 16.0;
    let day = daylight.clamp(0.0, 1.0) * weather * if thunder { weather } else { 1.0 };
    let subtract = ((1.0 - day) * (1.0 - flash.clamp(0.0, 1.0)) * 11.0).floor() as u8;
    block.min(15).max(sky.min(15).saturating_sub(subtract))
}

pub fn brightness(level: u8) -> f32 {
    let value = level.min(15) as f32 / 15.0;
    value / ((1.0 - value) * 3.0 + 1.0) * 0.95 + 0.05
}

#[derive(Clone, Debug)]
pub enum LightData {
    Uniform(u8),
    Dense(Arc<[u8]>),
}

impl LightData {
    pub fn get(&self, index: usize) -> u8 {
        match self {
            Self::Uniform(v) => *v,
            Self::Dense(data) => data[index],
        }
    }
    pub fn uniform(&self) -> Option<u8> {
        match self {
            Self::Uniform(v) => Some(*v),
            _ => None,
        }
    }
    pub fn dense(&self) -> Option<&[u8]> {
        match self {
            Self::Dense(data) => Some(data),
            _ => None,
        }
    }
    fn from_vec(data: Vec<u8>) -> Self {
        let first = data[0];
        if data.iter().all(|&v| v == first) {
            Self::Uniform(first)
        } else {
            Self::Dense(data.into())
        }
    }
    fn set(&mut self, index: usize, value: u8) -> bool {
        if self.get(index) == value {
            return false;
        }
        if let Self::Uniform(old) = self {
            *self = Self::Dense(vec![*old; CHUNK_VOLUME].into());
        }
        if let Self::Dense(data) = self {
            Arc::make_mut(data)[index] = value;
        }
        true
    }
    fn compact(&mut self) {
        if let Self::Dense(data) = self {
            let first = data[0];
            if data.iter().all(|&v| v == first) {
                *self = Self::Uniform(first);
            }
        }
    }
}

/// Low nibble is block light; high nibble is sky light, both in0..15.
#[derive(Clone, Debug)]
pub struct LightChunk {
    pub data: LightData,
    pub version: u64,
    pub geometry_identity: u64,
    pub geometry_version: u32,
    pub sky_known: bool,
    /// Latest FIFO input batch observed when this region became quiescent.
    pub settled_serial: u64,
    /// False for physically computed display data awaiting a local certificate.
    pub settled: bool,
}

impl LightChunk {
    pub fn get(&self, local: IVec4) -> (u8, u8) {
        let packed = self.data.get(local_index(local));
        (packed & 15, packed >> 4)
    }
    pub fn matches(&self, chunk: &Chunk) -> bool {
        self.geometry_identity == chunk.identity
            && self.geometry_version == chunk.light_state_version
    }
}

fn component(value: u8, channel: u8) -> u8 {
    (value >> (channel * 4)) & 15
}
fn with_component(value: u8, channel: u8, level: u8) -> u8 {
    (value & !(15 << (channel * 4))) | ((level & 15) << (channel * 4))
}
fn local(index: usize) -> IVec4 {
    IVec4::new(
        (index & 15) as i32,
        ((index >> 4) & 15) as i32,
        ((index >> 8) & 15) as i32,
        ((index >> 12) & 15) as i32,
    )
}

struct WorkingChunk {
    geometry: Chunk,
    baseline: LightData,
    incoming_sky: LightData,
    outgoing_sky: LightData,
    light: LightChunk,
    max_block: u8,
    last_displayed_version: Option<u64>,
}
fn maximum_block(data: &LightData) -> u8 {
    match data {
        LightData::Uniform(value) => value & 15,
        LightData::Dense(bytes) => bytes
            .iter()
            .fold(0, |maximum, value| maximum.max(value & 15)),
    }
}

#[derive(Default)]
struct Solver {
    #[cfg(test)]
    disable_transfer_cache: bool,
    geometry: HashMap<IVec4, Chunk>,
    fields: HashMap<IVec4, WorkingChunk>,
    columns: VecDeque<IVec4>,
    column_set: HashSet<IVec4>,
    seeds: VecDeque<IVec4>,
    seed_set: HashSet<IVec4>,
    removals: VecDeque<(IVec4, u8, u8)>,
    additions: VecDeque<(IVec4, u8)>,
    addition_set: HashSet<(IVec4, u8)>,
    changed: HashSet<IVec4>,
    voxel_work: HashMap<IVec4, usize>,
    ended_voxel_work: HashSet<IVec4>,
    first_data: VecDeque<IVec4>,
    first_data_set: HashSet<IVec4>,
    provisional: VecDeque<IVec4>,
    provisional_set: HashSet<IVec4>,
    ready_checks: VecDeque<IVec4>,
    ready_check_set: HashSet<IVec4>,
    version: u64,
}

impl Solver {
    fn queue_column(&mut self, c: IVec4) {
        let column = IVec4::new(c.x, 0, c.z, c.w);
        if self.column_set.insert(column) {
            self.columns.push_back(column);
        }
    }
    fn queue_seed(&mut self, c: IVec4) {
        if self.seed_set.insert(c) {
            self.seeds.push_back(c);
        }
    }
    fn queue_add(&mut self, p: IVec4, channel: u8) {
        if self.addition_set.insert((p, channel)) {
            self.additions.push_back((p, channel));
            *self.voxel_work.entry(chunk_of(p)).or_default() += 1;
        }
    }
    fn queue_remove(&mut self, p: IVec4, channel: u8, level: u8) {
        self.removals.push_back((p, channel, level));
        *self.voxel_work.entry(chunk_of(p)).or_default() += 1;
    }
    fn queue_ready_check(&mut self, c: IVec4) {
        if self.changed.contains(&c) && self.ready_check_set.insert(c) {
            self.ready_checks.push_back(c);
        }
    }
    fn check_near_chunk(&mut self, c: IVec4) {
        for dw in -1..=1 {
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        self.queue_ready_check(c + IVec4::new(dx, dy, dz, dw));
                    }
                }
            }
        }
    }
    fn check_near_column(&mut self, c: IVec4) {
        for dw in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    for y in 0..WORLD_CHUNKS_Y {
                        self.queue_ready_check(IVec4::new(c.x + dx, y, c.z + dz, c.w + dw));
                    }
                }
            }
        }
    }
    fn pop_voxel_work(&mut self, p: IVec4) {
        let c = chunk_of(p);
        if let Some(count) = self.voxel_work.get_mut(&c) {
            *count -= 1;
            if *count == 0 {
                self.voxel_work.remove(&c);
                self.ended_voxel_work.insert(c);
            }
        }
    }
    fn region_busy(&self, c: IVec4) -> bool {
        for dw in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if self
                        .column_set
                        .contains(&IVec4::new(c.x + dx, 0, c.z + dz, c.w + dw))
                    {
                        return true;
                    }
                    for dy in -1..=1 {
                        let n = c + IVec4::new(dx, dy, dz, dw);
                        if self.seed_set.contains(&n) || self.voxel_work.contains_key(&n) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
    fn state(&self, p: IVec4) -> Option<block::BlockState> {
        self.geometry
            .get(&chunk_of(p))
            .map(|c| c.get_state(local_of(p)))
    }
    fn level(&self, p: IVec4, channel: u8) -> Option<u8> {
        self.fields
            .get(&chunk_of(p))
            .map(|c| component(c.light.data.get(local_index(local_of(p))), channel))
    }
    fn baseline(&self, p: IVec4, channel: u8) -> u8 {
        self.fields.get(&chunk_of(p)).map_or(0, |c| {
            component(c.baseline.get(local_index(local_of(p))), channel)
        })
    }
    fn set_level(&mut self, p: IVec4, channel: u8, level: u8) -> bool {
        let c = chunk_of(p);
        let Some(field) = self.fields.get_mut(&c) else {
            return false;
        };
        let index = local_index(local_of(p));
        let old = field.light.data.get(index);
        if !field
            .light
            .data
            .set(index, with_component(old, channel, level))
        {
            return false;
        }
        if channel == BLOCK {
            field.max_block = field.max_block.max(level);
        }
        self.version += 1;
        field.light.version = self.version;
        self.changed.insert(c);
        true
    }

    fn apply(&mut self, chunks: Vec<(IVec4, Option<Chunk>)>) {
        let mut affected_columns = HashSet::default();
        let removed: HashSet<_> = chunks
            .iter()
            .filter_map(|(c, ch)| ch.is_none().then_some(*c))
            .collect();
        for (c, chunk) in chunks {
            affected_columns.insert(IVec4::new(c.x, 0, c.z, c.w));
            if let Some(chunk) = chunk {
                self.geometry.insert(c, chunk);
            } else {
                self.geometry.remove(&c);
                if let Some(old) = self.fields.remove(&c) {
                    // Only boundary light can have supported a neighboring chunk.
                    for axis in 0..4 {
                        for side in [0, 15] {
                            let mut neighbor_delta = IVec4::ZERO;
                            neighbor_delta[axis] = if side == 0 { -1 } else { 1 };
                            // Both chunks disappear in this transaction; there is
                            // no surviving receiver whose light can need removal.
                            if removed.contains(&(c + neighbor_delta)) {
                                continue;
                            }
                            let others: Vec<_> = (0..4).filter(|&i| i != axis).collect();
                            for a in 0..16 {
                                for b in 0..16 {
                                    for d in 0..16 {
                                        let mut l = IVec4::ZERO;
                                        l[axis] = side;
                                        l[others[0]] = a;
                                        l[others[1]] = b;
                                        l[others[2]] = d;
                                        let p = c * CHUNK + l;
                                        let mut direction = IVec4::ZERO;
                                        direction[axis] = if side == 0 { -1 } else { 1 };
                                        for channel in [BLOCK, SKY] {
                                            let level = component(
                                                old.light.data.get(local_index(l)),
                                                channel,
                                            );
                                            if level > 0
                                                && self.level(p + direction, channel).is_some_and(
                                                    |v| v > self.baseline(p + direction, channel),
                                                )
                                            {
                                                self.queue_remove(p, channel, level);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                self.changed.remove(&c);
            }
            self.queue_column(c);
        }
        // Even unchanged neighbor values need a new dependency certificate:
        // they may only become ready after this input's propagation settles.
        for c in affected_columns {
            for dw in -1..=1 {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        for y in 0..WORLD_CHUNKS_Y {
                            let n = IVec4::new(c.x + dx, y, c.z + dz, c.w + dw);
                            if self.fields.contains_key(&n) {
                                self.changed.insert(n);
                                self.queue_ready_check(n);
                            }
                        }
                    }
                }
            }
        }
    }

    fn direct_plane(chunk: &Chunk, incoming: &mut [u8; 4096]) -> LightData {
        if incoming.iter().all(|&v| v == 0) {
            if chunk.emissive_cells == 0 {
                return LightData::Uniform(0);
            }
            return LightData::from_vec(
                (0..CHUNK_VOLUME)
                    .map(|i| block::light_emission(chunk.get_state(local(i))).min(15))
                    .collect(),
            );
        }
        if let Some(id) = chunk
            .uniform_id()
            .filter(|_| chunk.metadata_plane().is_none())
        {
            let state = block::BlockState::from(id);
            let opacity = block::light_opacity(state);
            let emission = block::light_emission(state).min(15);
            if opacity >= 15 {
                incoming.fill(0);
                return LightData::Uniform(emission);
            }
            if opacity == 0 && incoming.iter().all(|&v| v == incoming[0]) {
                return LightData::Uniform((incoming[0] << 4) | emission);
            }
        }
        let mut bytes = vec![0; CHUNK_VOLUME];
        for w in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let column = (x + 16 * (z + 16 * w)) as usize;
                    let mut sky = incoming[column];
                    for y in (0..16).rev() {
                        let p = IVec4::new(x, y, z, w);
                        let state = chunk.get_state(p);
                        sky = sky.saturating_sub(block::light_opacity(state));
                        bytes[local_index(p)] = (sky << 4) | block::light_emission(state).min(15);
                    }
                    incoming[column] = sky;
                }
            }
        }
        LightData::from_vec(bytes)
    }

    fn block_source_has_frontier(
        &self,
        p: IVec4,
        level: u8,
        baseline: &LightData,
        current: &LightData,
    ) -> bool {
        let c = chunk_of(p);
        NEIGHBORS.iter().any(|&d| {
            let n = p + d;
            let nc = chunk_of(n);
            let Some(state) = self.state(n) else {
                return false;
            };
            let candidate = level.saturating_sub(block::light_opacity(state).max(1));
            let present = if nc == c {
                let index = local_index(local_of(n));
                component(current.get(index), BLOCK).max(component(baseline.get(index), BLOCK))
            } else {
                self.level(n, BLOCK)
                    .unwrap_or(0)
                    .max(block::light_emission(state))
            };
            candidate > present
        })
    }

    fn process_column(&mut self) {
        let Some(column) = self.columns.pop_front() else {
            return;
        };
        self.column_set.remove(&column);
        self.check_near_column(column);
        let mut incoming = [15u8; 4096];
        let mut known = true;
        for y in (0..WORLD_CHUNKS_Y).rev() {
            let c = IVec4::new(column.x, y, column.z, column.w);
            let Some(geometry) = self.geometry.get(&c).cloned() else {
                incoming.fill(0);
                known = false;
                continue;
            };
            // Adding another chunk in this column does not change every other
            // chunk's direct light. Reuse a prior boundary transfer only when
            // geometry provenance, sky knowledge and all incoming rays match.
            #[cfg(test)]
            let cache_enabled = !self.disable_transfer_cache;
            #[cfg(not(test))]
            let cache_enabled = true;
            if let Some(field) = self.fields.get(&c).filter(|_| cache_enabled) {
                let same_geometry = field.geometry.identity == geometry.identity
                    && field.geometry.light_state_version == geometry.light_state_version;
                let same_input = match &field.incoming_sky {
                    LightData::Uniform(value) => incoming.iter().all(|v| v == value),
                    LightData::Dense(values) => values.as_ref() == incoming,
                };
                if same_geometry && field.light.sky_known == known && same_input {
                    match &field.outgoing_sky {
                        LightData::Uniform(value) => incoming.fill(*value),
                        LightData::Dense(values) => incoming.copy_from_slice(values),
                    }
                    continue;
                }
            }
            let incoming_sky = LightData::from_vec(incoming.to_vec());
            let baseline = Self::direct_plane(&geometry, &mut incoming);
            let outgoing_sky = LightData::from_vec(incoming.to_vec());
            let existing = self.fields.remove(&c);
            if let Some(mut field) = existing {
                let geometry_changed = field.geometry.identity != geometry.identity
                    || field.geometry.light_state_version != geometry.light_state_version;
                let same_baseline = match (&field.baseline, &baseline) {
                    (LightData::Uniform(a), LightData::Uniform(b)) => a == b,
                    (LightData::Dense(a), LightData::Dense(b)) => a == b,
                    _ => false,
                };
                field.incoming_sky = incoming_sky;
                field.outgoing_sky = outgoing_sky;
                if !geometry_changed && same_baseline && field.light.sky_known == known {
                    self.fields.insert(c, field);
                    continue;
                }
                for index in 0..CHUNK_VOLUME {
                    let p = c * CHUNK + local(index);
                    let force = geometry_changed
                        && block::light_opacity(field.geometry.get_state(local(index)))
                            != block::light_opacity(geometry.get_state(local(index)));
                    let old_base = field.baseline.get(index);
                    let new_base = baseline.get(index);
                    if !force && old_base == new_base {
                        continue;
                    }
                    let mut current = field.light.data.get(index);
                    for channel in [BLOCK, SKY] {
                        let before = component(current, channel);
                        let new = component(new_base, channel);
                        let previous_base = component(old_base, channel);
                        if force || (new < previous_base && before > new) {
                            current = with_component(current, channel, new);
                            if before > new {
                                self.queue_remove(p, channel, before);
                            }
                            if new > 0 {
                                self.queue_add(p, channel);
                            }
                            for delta in NEIGHBORS {
                                self.queue_add(p + delta, channel);
                            }
                        } else if new > before {
                            // Direct light is initialized in bulk. The chunk and its
                            // neighbors are seeded below, which queues only borders
                            // where propagation can actually improve a dark voxel.
                            // Enqueuing every newly sunlit air voxel here floods the
                            // worker when the top of a streamed column arrives last.
                            current = with_component(current, channel, new);
                            // Local emitter edits must not wait behind thousands of
                            // terrain seed chunks. Only seed a useful frontier;
                            // equal-emission lava interiors cannot improve a neighbor.
                            if channel == BLOCK
                                && field.geometry.identity == geometry.identity
                                && self.block_source_has_frontier(
                                    p,
                                    new,
                                    &baseline,
                                    &field.light.data,
                                )
                            {
                                self.queue_add(p, BLOCK);
                            }
                        }
                    }
                    field.light.data.set(index, current);
                }
                self.version += 1;
                field.max_block = field.max_block.max(maximum_block(&baseline));
                if field.geometry.identity != geometry.identity {
                    field.last_displayed_version = None;
                }
                field.geometry = geometry.clone();
                field.baseline = baseline;
                field.light.version = self.version;
                field.light.geometry_identity = geometry.identity;
                field.light.geometry_version = geometry.light_state_version;
                field.light.sky_known = known;
                self.fields.insert(c, field);
            } else {
                self.version += 1;
                let light = LightChunk {
                    data: baseline.clone(),
                    version: self.version,
                    geometry_identity: geometry.identity,
                    geometry_version: geometry.light_state_version,
                    sky_known: known,
                    settled_serial: 0,
                    settled: false,
                };
                let max_block = maximum_block(&baseline);
                self.fields.insert(
                    c,
                    WorkingChunk {
                        geometry,
                        baseline,
                        incoming_sky,
                        outgoing_sky,
                        light,
                        max_block,
                        last_displayed_version: None,
                    },
                );
            }
            self.changed.insert(c);
            // Publish a column's actual computed plane promptly for display.
            // This is coalesced per chunk and never certifies unfinished work.
            if self.fields[&c].last_displayed_version.is_none() {
                if self.first_data_set.insert(c) {
                    self.first_data.push_back(c);
                }
            } else if self.provisional_set.insert(c) {
                self.provisional.push_back(c);
            }
            self.queue_ready_check(c);
            self.queue_seed(c);
            for delta in NEIGHBORS {
                self.queue_seed(c + delta);
            }
        }
    }

    /// Seed every source voxel of one chunk face whose light can improve the
    /// adjacent voxel across the face, resolving the neighbour
    /// chunk's light and geometry once per face instead of per voxel (the
    /// per-voxel hash lookups dominated seeding on generated terrain).
    fn seed_face(&mut self, c: IVec4, axis: usize, side: i32, source: &LightData) {
        let mut direction = IVec4::ZERO;
        direction[axis] = if side == 0 { -1 } else { 1 };
        let Some(neighbor) = self.fields.get(&(c + direction)) else { return };
        let (light, sky_known) = (neighbor.light.data.clone(), neighbor.light.sky_known);
        let geometry = self.geometry.get(&(c + direction)).cloned();
        let others: [usize; 3] = match axis { 0 => [1, 2, 3], 1 => [0, 2, 3], 2 => [0, 1, 3], _ => [0, 1, 2] };
        for a in 0..CHUNK { for b in 0..CHUNK { for d in 0..CHUNK {
            let mut l = IVec4::ZERO;
            l[axis] = side; l[others[0]] = a; l[others[1]] = b; l[others[2]] = d;
            let packed = source.get(local_index(l));
            if component(packed, BLOCK) <= 1 && component(packed, SKY) <= 1 { continue; }
            let mut n = l;
            n[axis] = CHUNK - 1 - side;
            let current = light.get(local_index(n));
            let block_possible = component(packed, BLOCK).saturating_sub(1) > component(current, BLOCK);
            let sky_possible = sky_known && component(packed, SKY).saturating_sub(1) > component(current, SKY);
            if !block_possible && !sky_possible { continue; }
            let Some(geometry) = &geometry else { return };
            let attenuation = block::light_opacity(geometry.get_state(n)).max(1);
            let p = c * CHUNK + l;
            if block_possible && component(packed, BLOCK).saturating_sub(attenuation) > component(current, BLOCK) { self.queue_add(p, BLOCK); }
            if sky_possible && component(packed, SKY).saturating_sub(attenuation) > component(current, SKY) { self.queue_add(p, SKY); }
        }}}
    }

    fn process_seed(&mut self) {
        let Some(c) = self.seeds.pop_front() else {
            return;
        };
        self.seed_set.remove(&c);
        self.check_near_chunk(c);
        let Some(data) = self.fields.get(&c).map(|f| f.light.data.clone()) else {
            return;
        };
        if let LightData::Uniform(value) = data {
            if component(value, BLOCK) <= 1 && component(value, SKY) <= 1 {
                return;
            }
            for axis in 0..4 {
                for side in [0, 15] {
                    let mut direction = IVec4::ZERO;
                    direction[axis] = if side == 0 { -1 } else { 1 };
                    let Some(neighbor) = self.fields.get(&(c + direction)) else {
                        continue;
                    };
                    if component(value, BLOCK) <= 1 && !neighbor.light.sky_known {
                        continue;
                    }
                    if neighbor.light.data.uniform().is_some_and(|n| {
                        component(n, BLOCK) >= component(value, BLOCK).saturating_sub(1)
                            && component(n, SKY) >= component(value, SKY).saturating_sub(1)
                    }) {
                        continue;
                    }
                    if self
                        .geometry
                        .get(&(c + direction))
                        .and_then(Chunk::uniform_id)
                        .is_some_and(|id| block::light_opacity(id.into()) >= 15)
                    {
                        continue;
                    }
                    self.seed_face(c, axis, side, &LightData::Uniform(value));
                }
            }
        } else {
            let own_max = maximum_block(&data);
            self.fields.get_mut(&c).unwrap().max_block = own_max;
            let block_max = NEIGHBORS
                .iter()
                .filter_map(|&d| self.fields.get(&(c + d)).map(|f| f.max_block))
                .fold(own_max, u8::max);
            let geometry = self.geometry[&c].clone();
            let sky_known = self.fields[&c].light.sky_known;
            // Preserve outgoing boundary propagation even when the adjacent
            // field is uniform dark and therefore has no sources to seed.
            for axis in 0..4 { for side in [0, CHUNK - 1] { self.seed_face(c, axis, side, &data); } }
            for index in 0..CHUNK_VOLUME {
                let current = data.get(index);
                let l = local(index);
                let p = c * CHUNK + l;
                // Most terrain is opaque, and most exposed air is already at
                // maximum skylight. Search from improvable receivers instead of
                // testing all eight neighbors of every fully lit air voxel.
                let block_level = component(current, BLOCK);
                let sky_level = component(current, SKY);
                // Attenuation is at least 1: such voxels cannot improve, so skip
                // them before the comparatively costly block-state lookup.
                if block_level + 1 >= block_max && (!sky_known || sky_level >= 14) {
                    continue;
                }
                let attenuation = block::light_opacity(geometry.get_state(l)).max(1);
                if attenuation >= 15 {
                    continue;
                }
                let possible_block = block_level < block_max.saturating_sub(attenuation);
                let possible_sky = sky_known && sky_level < 15 - attenuation;
                if !possible_block && !possible_sky {
                    continue;
                }
                for direction in NEIGHBORS {
                    let neighbor = l + direction;
                    let source = if neighbor.cmpge(IVec4::ZERO).all()
                        && neighbor.cmplt(IVec4::splat(CHUNK)).all()
                    {
                        data.get(local_index(neighbor))
                    } else if let Some(field) = self.fields.get(&chunk_of(p + direction)) {
                        field.light.data.get(local_index(local_of(p + direction)))
                    } else {
                        continue;
                    };
                    if possible_block
                        && component(source, BLOCK).saturating_sub(attenuation) > block_level
                    {
                        self.queue_add(p + direction, BLOCK);
                    }
                    if possible_sky
                        && component(source, SKY).saturating_sub(attenuation) > sky_level
                    {
                        self.queue_add(p + direction, SKY);
                    }
                }
            }
        }
    }

    fn propagate(&mut self, budget: usize) {
        for _ in 0..budget {
            if let Some((p, channel, removed)) = self.removals.pop_front() {
                self.pop_voxel_work(p);
                for direction in NEIGHBORS {
                    let n = p + direction;
                    let Some(level) = self.level(n, channel) else {
                        continue;
                    };
                    if level == 0 {
                        continue;
                    }
                    if level < removed {
                        let source = self.baseline(n, channel);
                        if level > source && self.set_level(n, channel, source) {
                            self.queue_remove(n, channel, level);
                        }
                        if source > 0 {
                            self.queue_add(n, channel);
                        }
                    } else {
                        self.queue_add(n, channel);
                    }
                }
                continue;
            }
            let Some((p, channel)) = self.additions.pop_front() else {
                break;
            };
            self.addition_set.remove(&(p, channel));
            self.pop_voxel_work(p);
            let Some(level) = self.level(p, channel) else {
                continue;
            };
            if level <= 1 {
                continue;
            }
            // Most neighbours share this voxel's chunk: resolve its geometry and
            // field once instead of three hash lookups per neighbour. Writes
            // only touch each neighbour itself, so they are applied afterwards.
            let c = chunk_of(p);
            let (here_geometry, here_field) = (self.geometry.get(&c), self.fields.get(&c));
            let mut raised = [(IVec4::ZERO, 0u8); NEIGHBORS.len()];
            let mut count = 0;
            for direction in NEIGHBORS {
                let n = p + direction;
                let local = n - c * CHUNK;
                let same = local.cmpge(IVec4::ZERO).all() && local.cmplt(IVec4::splat(CHUNK)).all();
                let (state, sky_known, current) = if same {
                    (here_geometry.map(|g| g.get_state(local)), here_field.is_some_and(|f| f.light.sky_known),
                        here_field.map(|f| component(f.light.data.get(local_index(local)), channel)))
                } else {
                    (self.state(n), self.fields.get(&chunk_of(n)).is_some_and(|f| f.light.sky_known), self.level(n, channel))
                };
                let Some(state) = state else { continue };
                if channel == SKY && !sky_known {
                    continue;
                }
                let candidate = level.saturating_sub(block::light_opacity(state).max(1));
                if candidate > 0 && current.is_some_and(|v| candidate > v) {
                    raised[count] = (n, candidate);
                    count += 1;
                }
            }
            for &(n, candidate) in &raised[..count] {
                if self.set_level(n, channel, candidate) {
                    self.queue_add(n, channel);
                }
            }
        }
        // A queue can transiently empty while this very batch immediately
        // propagates into another voxel of the same chunk. Check dependents
        // once after the batch, only for chunks which really stayed quiescent.
        let ended: Vec<_> = self.ended_voxel_work.drain().collect();
        for c in ended {
            if !self.voxel_work.contains_key(&c) {
                self.check_near_chunk(c);
            }
        }
    }

    fn pending(&self) -> usize {
        self.columns.len() + self.seeds.len() + self.removals.len() + self.additions.len()
    }
    fn step(&mut self) {
        self.process_column();
        self.process_seed();
        self.propagate(4096);
    }
    fn take_results(&mut self, budget: usize, serial: u64) -> Vec<(IVec4, LightChunk)> {
        let mut result = Vec::with_capacity(budget);
        // Do not let thousands of unchanged-neighbor certificates starve the
        // first real display plane of newly visible geometry. Reserve half the
        // bounded publication budget for first data, leaving settled progress.
        let first_budget = if budget == 0 { 0 } else { (budget / 2).max(1) };
        for _ in 0..first_budget.saturating_mul(16) {
            if result.len() == first_budget {
                break;
            }
            let Some(c) = self.first_data.pop_front() else {
                break;
            };
            self.first_data_set.remove(&c);
            if !self.changed.contains(&c) {
                continue;
            }
            let ready = !self.region_busy(c);
            let Some(field) = self
                .fields
                .get_mut(&c)
                .filter(|f| f.light.sky_known && f.last_displayed_version.is_none())
            else {
                continue;
            };
            if ready {
                field.light.data.compact();
                field.light.settled_serial = serial;
                field.light.settled = true;
                self.changed.remove(&c);
            }
            let mut light = field.light.clone();
            light.settled = ready;
            field.last_displayed_version = Some(light.version);
            result.push((c, light));
        }
        for _ in 0..budget.saturating_mul(16) {
            if result.len() == budget {
                break;
            }
            let Some(c) = self.ready_checks.pop_front() else {
                break;
            };
            self.ready_check_set.remove(&c);
            if !self.changed.contains(&c) || self.region_busy(c) {
                continue;
            }
            self.changed.remove(&c);
            if let Some(field) = self.fields.get_mut(&c) {
                field.light.data.compact();
                field.light.settled_serial = serial;
                field.light.settled = true;
                field.last_displayed_version = Some(field.light.version);
                result.push((c, field.light.clone()));
            }
        }
        // First data and settled certificates have reserved progress. The remaining bounded
        // slots may expose a real direct-light baseline (plus propagation already
        // computed), so new terrain need not stay black while neighbors settle.
        for _ in 0..budget.saturating_mul(16) {
            if result.len() == budget {
                break;
            }
            let Some(c) = self.provisional.pop_front() else {
                break;
            };
            self.provisional_set.remove(&c);
            if !self.changed.contains(&c) {
                continue;
            }
            let Some(field) = self
                .fields
                .get_mut(&c)
                .filter(|f| f.light.sky_known && f.last_displayed_version != Some(f.light.version))
            else {
                continue;
            };
            let mut light = field.light.clone();
            light.settled = false;
            field.last_displayed_version = Some(light.version);
            result.push((c, light));
        }
        result
    }
}

struct Batch {
    serial: u64,
    chunks: Vec<(IVec4, Option<Chunk>)>,
}

pub struct LightingEngine {
    sender: Option<SyncSender<Batch>>,
    results: Receiver<(IVec4, LightChunk)>,
    submitted: u64,
    completed: Arc<AtomicU64>,
    pending: Arc<AtomicUsize>,
    stages: Arc<[AtomicUsize; 4]>,
    results_pending: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    deferred: HashMap<IVec4, Option<Chunk>>,
    deferred_order: VecDeque<IVec4>,
    sent: HashMap<IVec4, (u64, Option<(u64, u32)>)>,
}

impl Default for LightingEngine {
    fn default() -> Self {
        Self::new()
    }
}
impl LightingEngine {
    pub fn new() -> Self {
        let (sender, jobs) = mpsc::sync_channel::<Batch>(2);
        let (output, results) = mpsc::sync_channel(256);
        let completed = Arc::new(AtomicU64::new(0));
        let pending = Arc::new(AtomicUsize::new(0));
        let stages = Arc::new(std::array::from_fn(|_| AtomicUsize::new(0)));
        let stage_counts = stages.clone();
        let results_pending = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (finished, work, result_count, stopped) = (
            completed.clone(),
            pending.clone(),
            results_pending.clone(),
            stop.clone(),
        );
        let worker = thread::Builder::new()
            .name("voxel-lighting".into())
            .spawn(move || {
                let mut solver = Solver::default();
                let mut serial = 0;
                while !stopped.load(Ordering::Relaxed) {
                    // A bounded channel does not bound a drain-until-empty loop:
                    // streamed snapshots can refill it faster than apply consumes
                    // them, starving propagation and publication indefinitely.
                    // Consume one batch, then always advance the solver.
                    if let Ok(batch) = jobs.try_recv() {
                        serial = batch.serial;
                        solver.apply(batch.chunks);
                    }
                    if solver.pending() == 0
                        && !solver.changed.is_empty()
                        && result_count.load(Ordering::Acquire) >= 256
                    {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    if solver.pending() == 0 && solver.changed.is_empty() {
                        solver.ready_checks.clear();
                        solver.ready_check_set.clear();
                        solver.first_data.clear();
                        solver.first_data_set.clear();
                        solver.provisional.clear();
                        solver.provisional_set.clear();
                        for count in stage_counts.iter() {
                            count.store(0, Ordering::Relaxed);
                        }
                        finished.store(serial, Ordering::Release);
                        work.store(0, Ordering::Release);
                        match jobs.recv_timeout(Duration::from_millis(10)) {
                            Ok(batch) => {
                                serial = batch.serial;
                                solver.apply(batch.chunks);
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        }
                    }
                    let step_start=std::time::Instant::now();
                    solver.step();
                    // On integrated GPUs a permanently busy core takes package power
                    // from rendering: one saturated solver tripled GPU frame time in
                    // measurements, while a 50% duty cycle removed almost all of it.
                    let duty=crate::background_duty();
                    if duty<1.0 {thread::sleep(step_start.elapsed().mul_f32((1.0-duty)/duty));}
                    work.store(solver.pending(), Ordering::Release);
                    for (counter, value) in stage_counts.iter().zip([
                        solver.columns.len(),
                        solver.seeds.len(),
                        solver.removals.len() + solver.additions.len(),
                        solver.changed.len(),
                    ]) {
                        counter.store(value, Ordering::Relaxed);
                    }
                    let room = 256usize
                        .saturating_sub(result_count.load(Ordering::Acquire))
                        .min(64);
                    for result in solver.take_results(room, serial) {
                        result_count.fetch_add(1, Ordering::AcqRel);
                        match output.try_send(result) {
                            Ok(()) => {}
                            Err(mpsc::TrySendError::Full((c, _))) => {
                                result_count.fetch_sub(1, Ordering::AcqRel);
                                solver.changed.insert(c);
                                solver.queue_ready_check(c);
                            }
                            Err(mpsc::TrySendError::Disconnected(_)) => return,
                        }
                    }
                }
            })
            .expect("spawn lighting worker");
        Self {
            sender: Some(sender),
            results,
            submitted: 0,
            completed,
            pending,
            stages,
            results_pending,
            stop,
            worker: Some(worker),
            deferred: HashMap::default(),
            deferred_order: VecDeque::new(),
            sent: HashMap::default(),
        }
    }
    pub fn submit(&mut self, chunks: Vec<(IVec4, Option<Chunk>)>) {
        for (c, chunk) in chunks {
            if !self.deferred.contains_key(&c) {
                self.deferred_order.push_back(c);
            }
            self.deferred.insert(c, chunk);
        }
        self.flush();
    }
    /// Backpressure retains only the newest snapshot for each chunk. Neither a
    /// slow worker nor repeated fluid edits can accumulate unlimited old copies.
    pub fn flush(&mut self) {
        if self.deferred.is_empty() {
            return;
        }
        let chunks = self.take_deferred_batch();
        let Some(sender) = &self.sender else { return };
        let records: Vec<_> = chunks
            .iter()
            .map(|(c, ch)| {
                (
                    *c,
                    ch.as_ref().map(|ch| (ch.identity, ch.light_state_version)),
                )
            })
            .collect();
        match sender.try_send(Batch {
            serial: self.submitted + 1,
            chunks,
        }) {
            Ok(()) => {
                self.submitted += 1;
                for (c, version) in records {
                    self.sent.insert(c, (self.submitted, version));
                }
            }
            Err(mpsc::TrySendError::Full(batch)) | Err(mpsc::TrySendError::Disconnected(batch)) => {
                for (c, chunk) in batch.chunks.into_iter().rev() {
                    self.deferred.insert(c, chunk);
                    self.deferred_order.push_front(c);
                }
            }
        }
    }
    fn take_deferred_batch(&mut self) -> Vec<(IVec4, Option<Chunk>)> {
        let mut chunks = Vec::with_capacity(64);
        while chunks.len() < 64 {
            let Some(c) = self.deferred_order.pop_front() else {
                break;
            };
            if !self.deferred.contains_key(&c) {
                continue;
            }
            let count = (0..WORLD_CHUNKS_Y)
                .filter(|&y| self.deferred.contains_key(&IVec4::new(c.x, y, c.z, c.w)))
                .count();
            if chunks.len() + count > 64 {
                self.deferred_order.push_front(c);
                break;
            }
            for y in (0..WORLD_CHUNKS_Y).rev() {
                let coordinate = IVec4::new(c.x, y, c.z, c.w);
                if let Some(chunk) = self.deferred.remove(&coordinate) {
                    chunks.push((coordinate, chunk));
                }
            }
        }
        chunks
    }
    /// Acknowledges actual FIFO submission, not merely a deferred snapshot.
    pub fn submitted_serial(&self, c: IVec4, current: Option<&Chunk>) -> Option<u64> {
        let (serial, version) = self.sent.get(&c)?;
        (*version == current.map(|ch| (ch.identity, ch.light_state_version))).then_some(*serial)
    }
    pub fn forget_removed_submission(&mut self, c: IVec4) {
        if self
            .sent
            .get(&c)
            .is_some_and(|(_, version)| version.is_none())
        {
            self.sent.remove(&c);
        }
    }
    pub fn poll(&mut self) -> Option<(IVec4, LightChunk)> {
        let result = self.results.try_recv().ok()?;
        self.results_pending.fetch_sub(1, Ordering::AcqRel);
        Some(result)
    }
    pub fn idle(&self) -> bool {
        self.deferred.is_empty()
            && self.completed.load(Ordering::Acquire) >= self.submitted
            && self.results_pending.load(Ordering::Acquire) == 0
    }
    /// Columns, seed chunks, propagation voxels and unpublished chunks.
    pub fn stages(&self) -> [usize; 4] {
        std::array::from_fn(|i| self.stages[i].load(Ordering::Relaxed))
    }
    pub fn pending(&self) -> usize {
        self.deferred.len()
            + self.pending.load(Ordering::Acquire)
            + self.results_pending.load(Ordering::Acquire)
            + (!self.idle()) as usize
    }
}
impl Drop for LightingEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn day_weather_subtraction_preserves_moonlight_and_independent_torches() {
        assert_eq!(effective_level(0, 15, 1.0, 0.0, false, 0.0), 15);
        assert_eq!(effective_level(0, 15, 0.0, 0.0, false, 0.0), 4);
        assert_eq!(effective_level(14, 0, 0.0, 1.0, true, 0.0), 14);
        assert_eq!(effective_level(0, 15, 1.0, 1.0, false, 0.0), 12);
        assert_eq!(effective_level(0, 15, 1.0, 1.0, true, 0.0), 10);
        assert_eq!(effective_level(0, 0, 0.0, 1.0, true, 1.0), 0);
        assert_eq!(brightness(0), 0.05);
        assert_eq!(brightness(15), 1.0);
    }
    fn settle(s: &mut Solver) {
        for _ in 0..2000 {
            if s.pending() == 0 {
                return;
            }
            s.step();
        }
        panic!("lighting did not converge: {} pending", s.pending());
    }
    fn air_columns(count_w: i32) -> Solver {
        let mut solver = Solver::default();
        let mut chunks = Vec::new();
        for w in 0..count_w {
            for y in 0..WORLD_CHUNKS_Y {
                chunks.push((IVec4::new(0, y, 0, w), Some(Chunk::uniform(block::AIR))));
            }
        }
        solver.apply(chunks);
        settle(&mut solver);
        solver
    }
    fn edit(s: &mut Solver, p: IVec4, id: u8) {
        let c = chunk_of(p);
        let mut chunk = s.geometry[&c].clone();
        chunk.set(local_of(p), id);
        s.apply(vec![(c, Some(chunk))]);
        settle(s);
    }
    #[test]
    fn cached_sky_boundaries_match_uncached_transfer_after_edits() {
        let mut cached = air_columns(1);
        let mut reference = Solver::default();
        reference.disable_transfer_cache = true;
        reference.apply(
            cached
                .geometry
                .iter()
                .map(|(&c, ch)| (c, Some(ch.clone())))
                .collect(),
        );
        settle(&mut reference);
        for (position, id) in [
            (IVec4::new(8, 100, 8, 8), block::STONE),
            (IVec4::new(8, 60, 8, 8), block::TORCH),
            (IVec4::new(8, 100, 8, 8), block::AIR),
            (IVec4::new(8, 60, 8, 8), block::AIR),
        ] {
            edit(&mut cached, position, id);
            edit(&mut reference, position, id);
            for (c, field) in &cached.fields {
                let other = &reference.fields[c];
                assert_eq!(field.light.sky_known, other.light.sky_known);
                for index in 0..CHUNK_VOLUME {
                    assert_eq!(
                        field.light.data.get(index),
                        other.light.data.get(index),
                        "cached sky differs at{c:?}:{index}"
                    );
                }
            }
        }
    }
    #[test]
    fn continuous_snapshot_input_cannot_starve_local_light_publication() {
        let mut engine = LightingEngine::new();
        let mut local = Vec::new();
        for y in 0..WORLD_CHUNKS_Y {
            local.push((IVec4::new(0, y, 0, 0), Some(Chunk::uniform(block::AIR))));
        }
        engine.submit(local.clone());
        let initial = std::time::Instant::now();
        while !engine.idle() {
            while engine.poll().is_some() {}
            assert!(
                initial.elapsed() < Duration::from_secs(5),
                "initial column did not settle"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let position = IVec4::new(8, 60, 8, 8);
        let coordinate = chunk_of(position);
        let mut changed = local[coordinate.y as usize].1.clone().unwrap();
        changed.set(local_of(position), block::TORCH);
        engine.submit(vec![(coordinate, Some(changed.clone()))]);
        // Keep the real two-slot worker channel full of distant 64-chunk
        // snapshots. This must not delay a local torch until input stops.
        let sender = engine.sender.as_ref().unwrap().clone();
        let distant: Vec<_> = (100..104)
            .flat_map(|x| {
                (0..WORLD_CHUNKS_Y)
                    .map(move |y| (IVec4::new(x, y, 0, 0), Some(Chunk::uniform(block::AIR))))
            })
            .collect();
        // Queue the first snapshots before polling: a producer thread that
        // starts late (as on Windows CI) would let the torch publish unopposed.
        let mut serial = 3;
        for _ in 0..3 {
            sender
                .send(Batch {
                    serial,
                    chunks: distant.clone(),
                })
                .unwrap();
            serial += 1;
        }
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let sent = Arc::new(AtomicUsize::new(3));
        let count = sent.clone();
        let producer = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match sender.try_send(Batch {
                    serial,
                    chunks: distant.clone(),
                }) {
                    Ok(()) => {
                        serial += 1;
                        count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(mpsc::TrySendError::Full(_)) => thread::yield_now(),
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
        });
        let start = std::time::Instant::now();
        let mut published = false;
        while start.elapsed() < Duration::from_secs(4) {
            while let Some((c, light)) = engine.poll() {
                if c == coordinate
                    && light.matches(&changed)
                    && light.settled
                    && light.get(local_of(position)).0 == 14
                {
                    published = true;
                    break;
                }
            }
            if published {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        stopped.store(true, Ordering::Relaxed);
        producer.join().unwrap();
        assert!(
            sent.load(Ordering::Relaxed) > 2,
            "producer did not sustain channel pressure"
        );
        assert!(
            published,
            "local lighting was starved for four seconds by continuously queued distant snapshots"
        );
    }
    #[test]
    fn whole_column_eviction_avoids_obsolete_sky_removal_wave() {
        let geometry: Vec<_> = (13..16)
            .map(|y| (IVec4::new(0, y, 0, 0), Some(Chunk::uniform(block::AIR))))
            .collect();
        let mut split = Solver::default();
        split.apply(geometry.clone());
        settle(&mut split);
        let mut atomic = Solver::default();
        atomic.apply(geometry);
        settle(&mut atomic);
        split.apply(vec![(IVec4::new(0, 15, 0, 0), None)]);
        split.process_column();
        let split_work = split.pending();
        atomic.apply((13..16).map(|y| (IVec4::new(0, y, 0, 0), None)).collect());
        atomic.process_column();
        eprintln!(
            "column eviction queued work: split={split_work}, atomic={}",
            atomic.pending()
        );
        assert!(
            split_work > 100_000,
            "split eviction did not reproduce the skylight-removal flood:{split_work}"
        );
        assert!(
            atomic.pending() < 100,
            "atomic eviction retained obsolete light work:{}",
            atomic.pending()
        );
        settle(&mut atomic);
        assert!(atomic.fields.is_empty());
    }
    #[test]
    fn deferred_backpressure_never_splits_a_complete_column() {
        let mut engine = LightingEngine::new();
        // Interleave ten columns vertically, matching arbitrary HashMap dirty
        // iteration. A 64-coordinate cut used to split all ten columns.
        for y in (0..WORLD_CHUNKS_Y).rev() {
            for x in 0..10 {
                let c = IVec4::new(x, y, 0, 0);
                engine.deferred.insert(c, None);
                engine.deferred_order.push_back(c);
            }
        }
        let mut total = 0;
        while !engine.deferred.is_empty() {
            let batch = engine.take_deferred_batch();
            assert!(!batch.is_empty());
            assert!(batch.len() <= 64);
            total += batch.len();
            let mut columns = HashMap::<i32, usize>::default();
            for (c, _) in batch {
                *columns.entry(c.x).or_default() += 1;
            }
            assert!(
                columns
                    .values()
                    .all(|&count| count == WORLD_CHUNKS_Y as usize)
            );
        }
        assert_eq!(total, 10 * WORLD_CHUNKS_Y as usize);
    }
    #[test]
    fn provisional_planes_show_real_sky_but_keep_roofed_caves_dark() {
        let cave = IVec4::new(0, 3, 0, 0);
        let mut chunks = Vec::new();
        for y in 0..WORLD_CHUNKS_Y {
            let mut chunk = Chunk::uniform(if y == 3 { block::STONE } else { block::AIR });
            if y == 3 {
                for w in 2..14 {
                    for z in 2..14 {
                        for yy in 2..14 {
                            for x in 2..14 {
                                chunk.set(IVec4::new(x, yy, z, w), block::AIR);
                            }
                        }
                    }
                }
            }
            chunks.push((IVec4::new(0, y, 0, 0), Some(chunk)));
        }
        let mut solver = Solver::default();
        solver.apply(chunks);
        solver.process_column();
        assert!(solver.pending() > 0);
        let preview = solver.take_results(128, 1);
        let top = preview.iter().find(|(c, _)| c.y == 15).unwrap();
        let underground = preview.iter().find(|(c, _)| *c == cave).unwrap();
        assert!(!top.1.settled && !underground.1.settled);
        assert_eq!(top.1.get(IVec4::splat(8)), (0, 15));
        assert_eq!(underground.1.get(IVec4::splat(8)), (0, 0));
        let version = top.1.version;
        settle(&mut solver);
        let settled = solver.take_results(128, 1);
        let final_top = settled.iter().find(|(c, _)| c.y == 15).unwrap();
        assert!(final_top.1.settled);
        assert_eq!(
            final_top.1.version, version,
            "certification should not need an invented color change"
        );
    }
    #[test]
    fn edited_torch_spreads_before_distant_seed_backlog_is_consumed() {
        let mut solver = air_columns(1);
        for x in 100..10_100 {
            solver.queue_seed(IVec4::new(x, 3, 0, 0));
        }
        let p = IVec4::new(8, 60, 8, 8);
        let c = chunk_of(p);
        let mut chunk = solver.geometry[&c].clone();
        chunk.set(local_of(p), block::TORCH);
        solver.apply(vec![(c, Some(chunk))]);
        solver.process_column();
        assert!(solver.seeds.len() > 9000);
        assert!(solver.additions.len() < 10);
        solver.propagate(4096);
        assert_eq!(solver.level(p + IVec4::W, BLOCK), Some(13));
        assert!(
            solver.seeds.len() > 9000,
            "local emitter should not need to drain distant seed work first"
        );
    }
    #[test]
    fn equal_emission_pool_interior_is_not_a_propagation_frontier() {
        let mut solver = air_columns(1);
        let c = IVec4::new(0, 3, 0, 0);
        let geometry = Chunk::uniform(block::LAVA);
        solver.geometry.insert(c, geometry);
        let baseline = LightData::Uniform(15 << 4 | 15);
        let current = LightData::Uniform(0);
        // At the center, all eight neighboring emitters already supply at least
        // the candidate light. A bulk pool must not enqueue every interior cell.
        assert!(!solver.block_source_has_frontier(
            c * CHUNK + IVec4::splat(8),
            15,
            &baseline,
            &current
        ));
    }
    #[test]
    fn first_display_plane_is_not_starved_by_certificate_backlog() {
        let mut solver = air_columns(2);
        let _ = solver.take_results(128, 1);
        for w in 0..2 {
            for y in 0..10 {
                let c = IVec4::new(0, y, 0, w);
                solver.changed.insert(c);
                solver.queue_ready_check(c);
            }
        }
        let new = IVec4::new(0, 15, 0, 2);
        solver.apply(vec![(new, Some(Chunk::uniform(block::AIR)))]);
        solver.process_column();
        assert!(solver.ready_checks.len() > 8);
        let results = solver.take_results(8, 2);
        let display = results
            .iter()
            .find(|(c, _)| *c == new)
            .expect("new real display data must precede the old certificate backlog");
        assert_eq!(display.1.get(IVec4::splat(8)), (0, 15));
        assert!(!display.1.settled);
        assert!(
            results.iter().any(|(c, l)| *c != new && l.settled),
            "settled publications must retain bounded progress too"
        );
        assert!(results.len() <= 8);
    }
    #[test]
    fn torch_propagates_across_w_and_removal_preserves_other_sources() {
        let mut s = air_columns(2);
        let a = IVec4::new(8, 60, 8, 15);
        let b = a + IVec4::W * 6;
        edit(&mut s, a, block::TORCH);
        assert_eq!(s.level(a, BLOCK), Some(14));
        for delta in NEIGHBORS {
            assert_eq!(s.level(a + delta, BLOCK), Some(13));
        }
        assert_eq!(s.level(a + IVec4::W * 3, BLOCK), Some(11));
        edit(&mut s, b, block::TORCH);
        edit(&mut s, a, block::AIR);
        assert_eq!(s.level(a, BLOCK), Some(8));
        edit(&mut s, b, block::AIR);
        assert_eq!(s.level(a, BLOCK), Some(0));
        assert_eq!(s.level(b, BLOCK), Some(0));
    }
    #[test]
    fn roof_blocks_sky_and_opening_restores_a_vertical_beam() {
        let mut s = air_columns(1);
        let roof_y = 100;
        let c = IVec4::new(0, roof_y / CHUNK, 0, 0);
        let mut roof = s.geometry[&c].clone();
        for w in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    roof.set(IVec4::new(x, roof_y % CHUNK, z, w), block::STONE);
                }
            }
        }
        s.apply(vec![(c, Some(roof))]);
        settle(&mut s);
        let below = IVec4::new(8, 90, 8, 8);
        assert_eq!(s.level(below, SKY), Some(0));
        edit(&mut s, IVec4::new(8, roof_y, 8, 8), block::GLASS);
        assert_eq!(s.level(below, SKY), Some(15));
        assert_eq!(s.level(below + IVec4::W, SKY), Some(14));
    }
    #[test]
    fn late_column_top_does_not_queue_every_sunlit_voxel_and_matches_full_seed_reference() {
        let mut chunks = Vec::new();
        for y in 0..WORLD_CHUNKS_Y - 1 {
            let mut chunk = Chunk::uniform(block::AIR);
            if y == 2 {
                for w in 0..16 {
                    for z in 0..16 {
                        for x in 0..16 {
                            if (x, z, w) != (8, 8, 8) {
                                chunk.set(IVec4::new(x, 8, z, w), block::STONE);
                            }
                        }
                    }
                }
            }
            chunks.push((IVec4::new(0, y, 0, 0), Some(chunk)));
        }
        let mut fast = Solver::default();
        let mut reference = Solver::default();
        fast.apply(chunks.clone());
        reference.apply(chunks);
        settle(&mut fast);
        settle(&mut reference);
        let top = (
            IVec4::new(0, WORLD_CHUNKS_Y - 1, 0, 0),
            Some(Chunk::uniform(block::AIR)),
        );
        fast.apply(vec![top.clone()]);
        reference.apply(vec![top]);
        fast.process_column();
        reference.process_column();
        // Reproduce the old, conservative policy: every newly sunlit voxel is
        // a propagation source, including the interiors of uniform-air chunks.
        let mut full_sources = Vec::new();
        for (&c, field) in &reference.fields {
            for index in 0..CHUNK_VOLUME {
                if component(field.baseline.get(index), SKY) > 0 {
                    full_sources.push(c * CHUNK + local(index));
                }
            }
        }
        for p in full_sources {
            reference.queue_add(p, SKY);
        }
        assert!(reference.additions.len() > 100_000);
        assert!(
            fast.additions.len() < 1_000,
            "only boundary propagation should be queued"
        );
        settle(&mut fast);
        settle(&mut reference);
        for (c, field) in &fast.fields {
            for index in 0..CHUNK_VOLUME {
                assert_eq!(
                    field.light.data.get(index),
                    reference.fields[c].light.data.get(index),
                    "different light at {:?}",
                    *c * CHUNK + local(index)
                );
            }
        }
        assert_eq!(fast.level(IVec4::new(8, 30, 8, 8), SKY), Some(15));
        assert_eq!(fast.level(IVec4::new(8, 30, 8, 9), SKY), Some(14));
    }

    #[test]
    fn settled_regions_publish_while_unrelated_columns_still_have_work() {
        let mut solver = air_columns(1);
        while !solver.take_results(128, 1).is_empty() {}
        let source = IVec4::new(8, 60, 8, 8);
        let c = chunk_of(source);
        let mut chunk = solver.geometry[&c].clone();
        chunk.set(local_of(source), block::TORCH);
        solver.apply(vec![(c, Some(chunk))]);
        // More distant queued columns than this local propagation needs steps.
        for x in 100..2100 {
            solver.queue_column(IVec4::new(x, 0, 0, 0));
        }
        assert!(
            solver
                .take_results(128, 2)
                .iter()
                .all(|(p, light)| *p != c || !light.settled),
            "a nearby seed/propagation task forbids a settled certificate"
        );
        let mut published = None;
        for _ in 0..1000 {
            solver.step();
            assert_eq!(
                solver.voxel_work.values().sum::<usize>(),
                solver.additions.len() + solver.removals.len()
            );
            for (p, light) in solver.take_results(128, 2) {
                if p == c && light.settled {
                    published = Some(light);
                }
            }
            if published.is_some() {
                break;
            }
        }
        let light = published.expect("local region never became settled");
        assert!(solver.pending() > 0, "distant work should still be present");
        assert_eq!(light.settled_serial, 2);
        assert_eq!(light.get(local_of(source)).0, 14);
        assert_eq!(light.get(local_of(source + IVec4::X)).0, 13);
    }

    #[test]
    fn opacity_and_emission_use_verified_beta_values() {
        let mut s = air_columns(1);
        let p = IVec4::new(8, 60, 8, 8);
        edit(&mut s, p, block::LAVA);
        assert_eq!(s.level(p, BLOCK), Some(15));
        assert_eq!(s.level(p + IVec4::X, BLOCK), Some(14));
        edit(&mut s, p + IVec4::X, block::WATER);
        assert_eq!(s.level(p + IVec4::X, BLOCK), Some(12));
        edit(&mut s, p + IVec4::X, block::STONE);
        assert_eq!(s.level(p + IVec4::X, BLOCK), Some(0));
    }

    #[test]
    #[ignore = "timing harness: cargo test --release -p mc4d-core lighting_throughput -- --ignored --nocapture"]
    fn lighting_throughput_on_generated_terrain() {
        let terrain = crate::terrain::TerrainGen::new(1337);
        let mut chunks = Vec::new();
        for w in -2..=2 { for z in -2..=2 { for x in -2..=2 { for y in 0..WORLD_CHUNKS_Y {
            let c = IVec4::new(x, y, z, w);
            chunks.push((c, Some(terrain.generate(c))));
        }}}}
        let count = chunks.len();
        let mut s = Solver::default();
        let start = std::time::Instant::now();
        s.apply(chunks);
        let applied = start.elapsed();
        let (mut tc, mut ts, mut tp) = (std::time::Duration::ZERO, std::time::Duration::ZERO, std::time::Duration::ZERO);
        let mut steps = 0;
        while s.pending() > 0 && steps < 10_000_000 {
            let a = std::time::Instant::now(); s.process_column(); tc += a.elapsed();
            let a = std::time::Instant::now(); s.process_seed(); ts += a.elapsed();
            let a = std::time::Instant::now(); s.propagate(4096); tp += a.elapsed();
            steps += 1;
        }
        println!("{count} chunks: apply {applied:?}, columns {tc:?}, seeds {ts:?}, propagate {tp:?}, total {:?}, steps {steps}", start.elapsed());
    }
}
