//! Event-driven WASAPI output and Symphonia decoding. PCM is mono/stereo f32 at the
//! source sample rate; streamed players keep at most three seconds plus one packet.
mod decode;
mod stream;
mod wasapi;

use decode::Reader;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use stream::Stream;

const MEMORY_LIMIT: usize = 32 * 1024 * 1024;
const MAX_PLAYERS: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioLoadMode {
    #[default]
    Auto,
    Memory,
    Stream,
}

/// ASIO is reserved for a future backend; selecting it currently returns an error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum AudioBackend {
    #[default]
    Wasapi,
    Asio,
}

enum Pcm {
    Mono(Arc<[f32]>),
    Stereo(Arc<[[f32; 2]]>),
}
impl Pcm {
    fn len(&self) -> usize {
        match self {
            Self::Mono(p) => p.len(),
            Self::Stereo(p) => p.len(),
        }
    }
    fn frame(&self, i: usize) -> [f32; 2] {
        match self {
            Self::Mono(p) => [p[i]; 2],
            Self::Stereo(p) => p[i],
        }
    }
}
enum Data {
    Memory(Pcm),
    Stream(PathBuf),
}
struct Source {
    data: Data,
    rate: u32,
    duration: Option<Duration>,
}
/// Cheaply cloneable. Memory PCM is shared; every streaming player gets its own cursor.
#[derive(Clone)]
pub struct AudioSource(Arc<Source>);
impl AudioSource {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        Self::load_with_mode(path, AudioLoadMode::Auto)
    }
    pub fn load_with_mode(path: impl AsRef<Path>, mode: AudioLoadMode) -> Result<Self, String> {
        let path = path.as_ref().canonicalize().map_err(|e| e.to_string())?;
        let mut reader = Reader::open(&path)?;
        let rate = reader.rate;
        let channels = reader.channels;
        let frame_limit = MEMORY_LIMIT / (channels * 4);
        let duration = reader
            .frames
            .map(|f| Duration::from_secs_f64(f as f64 / rate as f64));
        let streaming = || {
            Self(Arc::new(Source {
                data: Data::Stream(path.clone()),
                rate,
                duration,
            }))
        };
        if mode == AudioLoadMode::Stream
            || (mode == AudioLoadMode::Auto
                && reader.frames.is_some_and(|n| n > frame_limit as u64))
        {
            return Ok(streaming());
        }
        let reserve = reader.frames.unwrap_or(0).min(frame_limit as u64) as usize;
        let mut pcm = Vec::with_capacity(if channels == 1 { 0 } else { reserve });
        let mut mono = Vec::with_capacity(if channels == 1 { reserve } else { 0 });
        let mut packet = Vec::new();
        while reader.packet(&mut packet)? {
            // Check actual decoded size too: duration metadata can be absent or wrong.
            if mode == AudioLoadMode::Auto && pcm.len() + mono.len() + packet.len() > frame_limit {
                return Ok(streaming());
            }
            if channels == 1 {
                mono.extend(packet.iter().map(|f| f[0]));
            } else {
                pcm.extend_from_slice(&packet);
            }
        }
        let duration = Some(Duration::from_secs_f64(
            (pcm.len() + mono.len()) as f64 / rate as f64,
        ));
        let pcm = if channels == 1 {
            Pcm::Mono(mono.into())
        } else {
            Pcm::Stereo(pcm.into())
        };
        Ok(Self(Arc::new(Source {
            data: Data::Memory(pcm),
            rate,
            duration,
        })))
    }
    pub fn mode(&self) -> AudioLoadMode {
        match self.0.data {
            Data::Memory(_) => AudioLoadMode::Memory,
            Data::Stream(_) => AudioLoadMode::Stream,
        }
    }
    pub fn sample_rate(&self) -> u32 {
        self.0.rate
    }
    pub fn duration(&self) -> Option<Duration> {
        self.0.duration
    }
}

struct Voice {
    source: AudioSource,
    stream: Option<Stream>,
    cursor: f64,
    phase: f64,
    playing: bool,
    ended: bool,
    volume: f32,
    engine: Weak<EngineState>,
}
/// Playback controls are independent even when sources share their decoded PCM.
/// Attach with `AudioEngine::play` or `play_audio` before using `play` to resume.
pub struct AudioPlayer(Arc<Mutex<Voice>>);
impl AudioPlayer {
    pub fn new(source: AudioSource) -> Result<Self, String> {
        let stream = match &source.0.data {
            Data::Stream(path) => Some(Stream::new(Reader::open(path)?)?),
            Data::Memory(_) => None,
        };
        Ok(Self(Arc::new(Mutex::new(Voice {
            source,
            stream,
            cursor: 0.,
            phase: 0.,
            playing: false,
            ended: false,
            volume: 1.,
            engine: Weak::new(),
        }))))
    }
    pub fn play(&self) {
        let mut voice = self.0.lock().unwrap();
        if voice.ended {
            voice.seek(Duration::ZERO);
        }
        voice.playing = true;
    }
    pub fn pause(&self) {
        self.0.lock().unwrap().playing = false;
    }
    pub fn stop(&self) {
        let mut voice = self.0.lock().unwrap();
        voice.playing = false;
        voice.seek(Duration::ZERO);
    }
    /// Memory seeks are immediate, stream seeks are asynchronous and sample accurate.
    /// Silence is output until fresh samples arrive. Check `error()` for seek failures.
    pub fn seek(&self, position: Duration) {
        self.0.lock().unwrap().seek(position);
    }
    pub fn set_volume(&self, volume: f32) -> Result<(), String> {
        if !volume.is_finite() || volume < 0. {
            return Err("volume must be finite and nonnegative".into());
        }
        self.0.lock().unwrap().volume = volume;
        Ok(())
    }
    pub fn is_playing(&self) -> bool {
        self.0.lock().unwrap().playing
    }
    pub fn position(&self) -> Duration {
        let voice = self.0.lock().unwrap();
        Duration::from_secs_f64(voice.cursor / voice.source.0.rate as f64)
    }
    pub fn error(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .stream
            .as_ref()
            .and_then(Stream::error)
    }
}
impl Voice {
    fn seek(&mut self, position: Duration) {
        let position = self
            .source
            .duration()
            .map_or(position, |end| position.min(end));
        self.cursor = position.as_secs_f64() * self.source.0.rate as f64;
        self.phase = 0.;
        self.ended = false;
        if self.source.duration().is_some_and(|end| position >= end) {
            self.ended = true;
            self.playing = false;
            return;
        }
        if let Some(stream) = &self.stream {
            stream.seek(position);
        }
    }
    fn mix(&mut self, output: &mut [[f32; 2]], rate: u32) {
        if !self.playing {
            return;
        }
        let step = self.source.0.rate as f64 / rate as f64;
        if let Some(stream) = &self.stream {
            let Some(mut buffer) = stream.try_buffer() else {
                return;
            };
            let before = buffer.frames.len();
            for out in output {
                let advance = (self.phase + step).floor() as usize;
                if buffer.frames.is_empty() {
                    if buffer.eof {
                        self.playing = false;
                        self.ended = true;
                    }
                    break;
                }
                if !buffer.eof && buffer.frames.len() < (advance + 1).max(2) {
                    break;
                }
                let a = buffer.frames[0];
                let b = buffer.frames.get(1).copied().unwrap_or(a);
                add(out, a, b, self.phase as f32, self.volume);
                self.phase = self.phase + step - advance as f64;
                self.cursor += step;
                let count = advance.min(buffer.frames.len());
                buffer.frames.drain(..count);
            }
            stream.consumed(before, buffer.frames.len());
        } else if let Data::Memory(pcm) = &self.source.0.data {
            for out in output {
                let index = self.cursor as usize;
                if index >= pcm.len() {
                    self.playing = false;
                    self.ended = true;
                    break;
                }
                let a = pcm.frame(index);
                let b = pcm.frame((index + 1).min(pcm.len() - 1));
                add(out, a, b, self.cursor.fract() as f32, self.volume);
                self.cursor = (self.cursor + step).min(pcm.len() as f64);
            }
        }
    }
}
impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let mut voice = self.0.lock().unwrap();
        voice.playing = false;
        // Shut down streaming on the caller, never on the output thread's last Arc drop1!!
        let stream = voice.stream.take();
        drop(voice);
        drop(stream);
    }
}
fn add(out: &mut [f32; 2], a: [f32; 2], b: [f32; 2], phase: f32, volume: f32) {
    for c in 0..2 {
        out[c] += (a[c] + (b[c] - a[c]) * phase) * volume;
    }
}

struct EngineState {
    voices: Mutex<Vec<Weak<Mutex<Voice>>>>,
    stop: AtomicBool,
    error: Mutex<Option<String>>,
}
/// Owns the backend and reusable output buffers on a dedicated output thread.
pub struct AudioEngine {
    backend: AudioBackend,
    state: Arc<EngineState>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}
impl AudioEngine {
    /// Creates a lightweight engine. `start()` opens the device; Game calls it in run().
    pub fn new() -> Self {
        Self::with_backend(AudioBackend::Wasapi)
    }
    pub fn with_backend(backend: AudioBackend) -> Self {
        Self {
            backend,
            state: Arc::new(EngineState {
                voices: Mutex::new(Vec::with_capacity(MAX_PLAYERS)),
                stop: AtomicBool::new(false),
                error: Mutex::new(None),
            }),
            worker: None,
        }
    }
    pub fn backend(&self) -> AudioBackend {
        self.backend
    }
    pub fn start(&mut self) -> Result<(), String> {
        if self.worker.is_some() {
            return self.error().map_or(Ok(()), Err);
        }
        if self.backend == AudioBackend::Asio {
            return Err("ASIO is reserved but not implemented".into());
        }
        let state = self.state.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.worker = Some(
            thread::Builder::new()
                .name("velocity-audio".into())
                .spawn(move || {
                    let result = wasapi::run(&state, &tx);
                    if let Err(error) = result {
                        *state.error.lock().unwrap() = Some(error.clone());
                        let _ = tx.try_send(Err(error));
                    }
                })
                .map_err(|e| e.to_string())?,
        );
        rx.recv()
            .map_err(|_| "audio thread exited during startup".to_string())?
    }
    pub fn play(&self, player: &AudioPlayer) -> Result<(), String> {
        attach(&self.state, player)
    }
    pub fn error(&self) -> Option<String> {
        self.state.error.lock().unwrap().clone()
    }
}
fn attach(state: &Arc<EngineState>, player: &AudioPlayer) -> Result<(), String> {
    let mut voices = state.voices.lock().unwrap();
    let mut voice = player.0.lock().unwrap();
    if let Some(owner) = voice.engine.upgrade() {
        if !Arc::ptr_eq(&owner, state) {
            return Err("player is attached to another audio engine".into());
        }
    } else {
        voices.retain(|v| v.strong_count() > 0);
        if voices.len() == MAX_PLAYERS {
            return Err("audio engine player limit (128) reached".into());
        }
        voices.push(Arc::downgrade(&player.0));
        voice.engine = Arc::downgrade(state);
    }
    if voice.ended {
        voice.seek(Duration::ZERO);
    }
    voice.playing = true;
    Ok(())
}
impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Attach and play using the current Game's engine from any scene callback.
pub fn play_audio(player: &AudioPlayer) -> Result<(), String> {
    crate::game::with_audio(|engine| engine.play(player))
}

#[cfg(test)]
mod tests;
