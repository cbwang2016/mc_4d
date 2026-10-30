//! Bounded positional sound playback. The fourth-axis distance also attenuates sound.
use crate::{
    audio_assets::SOUNDS,
    game::{CreatureKind, Game},
    settings::Settings,
};
use mc4d_core::{block, glam::Vec4};
use rodio::{Decoder, Player, Source, SpatialPlayer};
use std::{collections::HashMap, io::Cursor};
type CachedSound = rodio::buffer::SamplesBuffer;
fn decoded_sound(bytes: &'static [u8]) -> Option<CachedSound> {
    let decoder = Decoder::new(Cursor::new(bytes)).ok()?;
    let channels = decoder.channels();
    let rate = decoder.sample_rate();
    let samples: Vec<_> = decoder.collect();
    if samples.is_empty() {
        return None;
    }
    Some(CachedSound::new(channels, rate, samples))
}

#[derive(Clone, Copy)]
pub struct SoundEvent {
    pub name: &'static str,
    pub pos: Vec4,
    pub volume: f32,
    pub pitch: f32,
}
pub struct Audio {
    device: rodio::MixerDeviceSink,
    music: Player,
    rain: Player,
    record: Player,
    record_key: Option<(mc4d_core::glam::IVec4, u64)>,
    active: Vec<SpatialPlayer>,
    cache: HashMap<usize, CachedSound>,
    sequence: usize,
    music_wait: f32,
    ambient_wait: f32,
    walked: f32,
    last_pos: Option<Vec4>,
    was_in_water: bool,
    output_gain: f32,
}
fn sound(prefix: &str, variant: usize) -> Option<&'static [u8]> {
    let choices: Vec<_> = SOUNDS
        .iter()
        .filter(|(name, _)| {
            name.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.is_empty() || suffix.bytes().all(|b| b.is_ascii_digit())
            })
        })
        .collect();
    choices
        .get(variant % choices.len().max(1))
        .map(|(_, bytes)| *bytes)
}
fn gain(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}
pub fn step_sound(id: block::BlockId) -> &'static str {
    match id {
        block::GRASS | block::LEAVES => "step.grass",
        block::DIRT | block::FARMLAND => "step.gravel",
        block::SAND => "step.sand",
        block::SNOW => "step.snow",
        block::WOOL => "step.cloth",
        block::LOG | block::PLANKS | block::CRAFTING_TABLE | block::CHEST => "step.wood",
        _ => "step.stone",
    }
}
pub fn creature_sound(kind: CreatureKind, hurt: bool, dead: bool) -> Option<&'static str> {
    Some(match kind {
        CreatureKind::Pig => {
            if dead {
                "mob.pigdeath"
            } else {
                "mob.pig"
            }
        }
        CreatureKind::Cow => {
            if hurt || dead {
                "mob.cowhurt"
            } else {
                "mob.cow"
            }
        }
        CreatureKind::Sheep => "mob.sheep",
        CreatureKind::Chicken => {
            if hurt || dead {
                "mob.chickenhurt"
            } else {
                "mob.chicken"
            }
        }
        CreatureKind::Zombie => {
            if dead {
                "mob.zombiedeath"
            } else if hurt {
                "mob.zombiehurt"
            } else {
                "mob.zombie"
            }
        }
        CreatureKind::Skeleton => {
            if hurt || dead {
                "mob.skeletonhurt"
            } else {
                "mob.skeleton"
            }
        }
        CreatureKind::Spider => {
            if dead {
                "mob.spiderdeath"
            } else {
                "mob.spider"
            }
        }
        CreatureKind::Creeper => {
            if dead {
                "mob.creeperdeath"
            } else if hurt {
                "mob.creeper"
            } else {
                return None;
            }
        }
        CreatureKind::Slime => "mob.slime",
        CreatureKind::Wolf => {
            if dead {
                "mob.wolf.death"
            } else if hurt {
                "mob.wolf.hurt"
            } else {
                "mob.wolf.bark"
            }
        }
        CreatureKind::Gloom | CreatureKind::Squid => return None,
    })
}
impl Audio {
    pub fn new(muted: bool) -> Option<Self> {
        let mut device = rodio::DeviceSinkBuilder::open_default_sink()
            .map_err(|e| log::warn!("audio device unavailable: {e}"))
            .ok()?;
        device.log_on_drop(false);
        let music = Player::connect_new(device.mixer());
        let rain = Player::connect_new(device.mixer());
        let record = Player::connect_new(device.mixer());
        if let Some(bytes) = sound("ambient.weather.rain", 0) {
            if let Some(source) = decoded_sound(bytes) {
                rain.append(source.repeat_infinite());
                rain.set_volume(0.0);
            }
        }
        Some(Self {
            device,
            music,
            rain,
            record,
            record_key: None,
            active: Vec::new(),
            cache: HashMap::new(),
            sequence: 0,
            music_wait: 20.0,
            ambient_wait: 4.0,
            walked: 0.0,
            last_pos: None,
            was_in_water: false,
            output_gain: if muted { 0.0 } else { 1.0 },
        })
    }
    pub fn update(
        &mut self,
        game: &mut Game,
        settings: &Settings,
        dt: f32,
        click: bool,
        playing: bool,
    ) {
        let dt = dt.clamp(0.0, 0.25);
        self.active.retain(|player| !player.empty());
        self.music
            .set_volume(gain(settings.music_volume) * 0.55 * self.output_gain);
        self.music_wait -= dt;
        if self.music.empty()
            && self.music_wait <= 0.0
            && settings.music_volume > 0.0
            && game.record_playback().is_none()
        {
            self.music.append(AmbientMusic::new(self.sequence as u32));
            self.sequence += 1;
            self.music_wait = 300.0 + (self.sequence % 5) as f32 * 45.0;
        }
        let eye = game.eye();
        let playing_record = game.record_playback();
        let key = playing_record.map(|r| (r.position, r.serial));
        if key != self.record_key {
            self.record.clear();
            self.record_key = key;
            if let Some(record) = playing_record {
                self.music.clear();
                let name = if record.record == mc4d_core::item::RECORD_13 {
                    "13"
                } else {
                    "cat"
                };
                let supplied = (!settings.texture_pack.is_empty())
                    .then(|| std::path::Path::new(&settings.texture_pack))
                    .and_then(|root| {
                        [
                            format!("records/{name}.ogg"),
                            format!("streaming/{name}.ogg"),
                        ]
                        .into_iter()
                        .find_map(|path| {
                            mc4d_render::pack::read(root, &path)
                                .and_then(|bytes| Decoder::new(std::io::Cursor::new(bytes)).ok())
                        })
                    });
                if let Some(mut source) = supplied {
                    let elapsed = std::time::Duration::from_secs_f32(record.elapsed);
                    let ended = source
                        .total_duration()
                        .is_some_and(|duration| elapsed >= duration);
                    if !ended && (record.elapsed < 0.05 || source.try_seek(elapsed).is_ok()) {
                        self.record.append(source);
                    } else {
                        game.finish_record(record.serial);
                    }
                } else {
                    // Original compositions are the bundled fallback; full licensed
                    // recordings may be supplied externally with a texture pack.
                    self.record
                        .append(AmbientMusic::at(record.record as u32, record.elapsed));
                    game.toast("Jukebox: original procedural arrangement");
                }
                self.record.play();
            }
        }
        if let Some(record) = playing_record {
            if self.record.empty() {
                game.finish_record(record.serial);
                self.record_key = None;
            }
        }
        let distance = playing_record.map_or(64.0, |r| {
            (r.position.as_vec4() + Vec4::splat(0.5)).distance(eye)
        });
        self.record.set_volume(
            self.output_gain
                * 0.5
                * gain(settings.sound_volume)
                * (1.0 - distance / 64.0).clamp(0.0, 1.0),
        );
        if playing_record.is_some() {
            self.music.set_volume(0.0);
        }

        let exposed = !(1..=24).any(|dy| {
            game.world
                .block(eye.floor().as_ivec4() + mc4d_core::glam::IVec4::Y * dy)
                .is_some_and(block::is_solid)
        });
        self.rain.set_volume(
            self.output_gain
                * gain(settings.sound_volume)
                * game.environment.rain
                * if exposed { 0.35 } else { 0.06 },
        );
        if click {
            game.emit_sound("random.click", eye, 0.8, 1.0);
        }
        if playing {
            if let Some(previous) = self.last_pos {
                let delta = game.body.pos - previous;
                if delta.length() < 3.0 && game.body.on_ground && !game.flying {
                    self.walked += Vec4::new(delta.x, 0.0, delta.z, delta.w).length();
                    if self.walked > 1.7 {
                        self.walked = 0.0;
                        let id = game
                            .world
                            .block((game.body.pos - Vec4::Y * 0.05).floor().as_ivec4())
                            .unwrap_or(block::AIR);
                        if id != block::AIR {
                            game.emit_sound(step_sound(id), game.body.pos, 0.45, 1.0);
                        }
                    }
                }
            }
            if game.body.in_water && !self.was_in_water {
                game.emit_sound("random.splash", eye, 0.6, 1.0);
            }
            self.ambient_wait -= dt;
            if self.ambient_wait <= 0.0 {
                if let Some(c) = game
                    .creatures
                    .get(self.sequence % game.creatures.len().max(1))
                {
                    if let Some(name) = creature_sound(c.kind, false, false) {
                        game.emit_sound(name, c.center(), 0.6, 1.0);
                    }
                }
                self.ambient_wait = 3.0 + (self.sequence % 5) as f32;
                self.sequence += 1;
            }
        }
        self.last_pos = Some(game.body.pos);
        self.was_in_water = game.body.in_water;
        for event in std::mem::take(&mut game.sounds) {
            if self.active.len() >= 32 || gain(settings.sound_volume) == 0.0 {
                continue;
            }
            let Some(bytes) = sound(event.name, self.sequence) else {
                continue;
            };
            self.sequence += 1;
            let delta = event.pos - eye;
            if delta.length_squared() > 64.0 * 64.0 {
                continue;
            }
            let key = bytes.as_ptr() as usize;
            if let std::collections::hash_map::Entry::Vacant(entry) = self.cache.entry(key) {
                let Some(source) = decoded_sound(bytes) else {
                    continue;
                };
                entry.insert(source);
            }
            let source = self.cache[&key].clone();
            let basis = game.basis();
            let emitter = [delta.dot(basis.right), delta.y, delta.dot(basis.forward_h)];
            let player = SpatialPlayer::connect_new(
                self.device.mixer(),
                emitter,
                [-0.15, 0.0, 0.0],
                [0.15, 0.0, 0.0],
            );
            player.set_volume(
                self.output_gain * gain(settings.sound_volume) * event.volume
                    / (1.0 + delta.dot(basis.ana).powi(2) * 0.04),
            );
            player.append(source.speed(event.pitch.clamp(0.5, 2.0)));
            self.active.push(player);
        }
    }
}

/// Original, slowly evolving bell/pad composition; no recorded song is embedded.
struct AmbientMusic {
    sample: u64,
    seed: u32,
    chord: usize,
    frequencies: [f64; 4],
}
impl AmbientMusic {
    fn at(seed: u32, seconds: f32) -> Self {
        let mut music = Self::new(seed);
        music.sample = (seconds.clamp(0.0, 128.0) * 22050.0) as u64 * 2;
        music
    }

    fn new(seed: u32) -> Self {
        Self {
            sample: 0,
            seed,
            chord: usize::MAX,
            frequencies: [0.0; 4],
        }
    }
}
impl Iterator for AmbientMusic {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        const RATE: f64 = 22050.0;
        if self.sample >= 22050 * 2 * 128 {
            return None;
        }
        let channel = self.sample % 2;
        let time = (self.sample / 2) as f64 / RATE;
        self.sample += 1;
        let chord = (time / 8.0).floor() as usize;
        if chord != self.chord {
            self.chord = chord;
            let roots = [48, 55, 52, 57, 50, 60, 53, 55];
            let root = roots[(chord + self.seed as usize) % roots.len()] as f64;
            for (voice, interval) in [0.0, 7.0, 12.0, 19.0].into_iter().enumerate() {
                self.frequencies[voice] = 440.0 * 2.0f64.powf((root + interval - 69.0) / 12.0);
            }
        }
        let age = time % 8.0;
        let envelope = (age / 1.8).min(1.0) * ((8.0 - age) / 2.5).min(1.0);
        let mut value = 0.0;
        for voice in 0..4 {
            let frequency = self.frequencies[voice] * (1.0 + channel as f64 * 0.0007);
            let phase = std::f64::consts::TAU * frequency * time;
            let delay = voice as f64 * 0.65;
            let bell = if age >= delay {
                (-(age - delay) * 1.7).exp()
            } else {
                0.0
            };
            value += (phase.sin() * 0.012 * envelope
                + (phase.sin() + 0.3 * (phase * 2.01).sin()) * bell * 0.018)
                / (1.0 + voice as f64 * 0.2);
        }
        Some(value as f32)
    }
}
impl Source for AmbientMusic {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        std::num::NonZeroU16::new(2).unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        std::num::NonZeroU32::new(22050).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(128))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_gameplay_audio_decodes_without_opening_a_device() {
        for prefix in [
            "random.click",
            "random.hurt",
            "random.explode",
            "step.stone",
            "step.wood",
            "ambient.weather.rain",
        ] {
            let bytes = sound(prefix, 0).unwrap_or_else(|| panic!("missing sound {prefix}"));
            let mut decoder = Decoder::new(Cursor::new(bytes)).unwrap();
            assert!(decoder.next().is_some(), "{prefix}");
        }
    }
    #[test]
    fn cached_sounds_restart_with_identical_samples_and_rain_loops_without_reopening() {
        let bytes = sound("random.click", 0).unwrap();
        let source = decoded_sound(bytes).unwrap();
        let first: Vec<_> = source.clone().collect();
        let second: Vec<_> = source.clone().collect();
        assert!(!first.is_empty());
        assert_eq!(first, second);
        let repeated: Vec<_> = source.repeat_infinite().take(first.len() * 2).collect();
        assert_eq!(&repeated[..first.len()], first.as_slice());
        assert_eq!(&repeated[first.len()..], first.as_slice());
    }
    #[test]
    fn jukebox_procedural_seek_matches_samples_without_replaying_old_audio() {
        let expected: Vec<_> = AmbientMusic::new(2256).skip(44100).take(128).collect();
        let actual: Vec<_> = AmbientMusic::at(2256, 1.0).take(128).collect();
        assert_eq!(actual, expected);
    }
}
