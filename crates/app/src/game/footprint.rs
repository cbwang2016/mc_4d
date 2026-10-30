//! Slice footprints (which chunks each view needs) computed off the frame
//! thread. Continuous rotation in multi-view layouts invalidates them every few
//! frames, and a synchronous recomputation cost 5-15 ms per occurrence.
use mc4d_core::glam::{IVec4, Vec4};
use mc4d_core::world::{chunks_near_slices, chunks_visible_in_slices, skylight_column_order};
use std::sync::mpsc::{channel, Receiver, Sender};

#[derive(Clone)]
pub struct Job {
    pub eye: Vec4,
    pub normals: Vec<Vec4>,
    pub radius: f32,
    pub margin: f32,
    pub key: (u32, u32, u32),
}

pub struct Footprint {
    pub job: Job,
    pub gpu_wanted: Vec<IVec4>,
    pub load_wanted: Vec<IVec4>,
}

impl Job {
    pub fn compute(self) -> Footprint {
        let prefetch = chunks_near_slices(self.eye, &self.normals, self.radius, 20.0);
        let gpu_wanted = chunks_visible_in_slices(self.eye, &self.normals, self.radius, self.margin);
        let load_wanted = skylight_column_order(&gpu_wanted, &prefetch);
        Footprint { job: self, gpu_wanted, load_wanted }
    }
}

pub struct FootprintWorker {
    jobs: Option<Sender<Job>>,
    results: Receiver<Footprint>,
    busy: bool,
}

impl FootprintWorker {
    pub fn new() -> Self {
        let (jobs, inbox) = channel::<Job>();
        let (outbox, results) = channel();
        std::thread::Builder::new().name("slice-footprints".into()).spawn(move || {
            while let Ok(mut job) = inbox.recv() {
                while let Ok(newer) = inbox.try_recv() { job = newer; }
                if outbox.send(job.compute()).is_err() { break; }
            }
        }).expect("footprint worker");
        Self { jobs: Some(jobs), results, busy: false }
    }

    /// At most one job in flight; later views are covered by the next request.
    pub fn submit(&mut self, job: Job) {
        if !self.busy && self.jobs.as_ref().is_some_and(|s| s.send(job).is_ok()) { self.busy = true; }
    }

    pub fn receive(&mut self) -> Option<Footprint> {
        let result = self.results.try_recv().ok();
        if result.is_some() { self.busy = false; }
        result
    }
}

impl Default for FootprintWorker { fn default() -> Self { Self::new() } }
