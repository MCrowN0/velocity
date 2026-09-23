use super::decode::Reader;
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Duration,
};

pub(super) struct Buffer {
    pub frames: VecDeque<[f32; 2]>,
    pub eof: bool,
    pub error: Option<String>,
    stop: bool,
    generation: u64,
    seek: Option<Duration>,
}
struct Shared {
    buffer: Mutex<Buffer>,
    wake: Condvar,
    capacity: usize,
    low_water: usize,
}
pub(super) struct Stream {
    shared: Arc<Shared>,
}
impl Stream {
    pub fn new(mut reader: Reader) -> Result<Self, String> {
        let capacity = reader.rate as usize * 3;
        let shared = Arc::new(Shared {
            buffer: Mutex::new(Buffer {
                frames: VecDeque::with_capacity(capacity),
                eof: false,
                error: None,
                stop: false,
                generation: 0,
                seek: None,
            }),
            wake: Condvar::new(),
            capacity,
            low_water: capacity / 2,
        });
        let worker = shared.clone();
        thread::Builder::new()
            .name("velocity-decode".into())
            .spawn(move || {
                let mut packet = Vec::new();
                let mut offset = 0;
                let mut generation = 0;
                loop {
                    let mut state = worker.buffer.lock().unwrap();
                    while !state.stop
                        && state.seek.is_none()
                        && (state.eof || state.frames.len() == worker.capacity)
                    {
                        state = worker.wake.wait(state).unwrap();
                    }
                    if state.stop {
                        return;
                    }
                    if let Some(position) = state.seek.take() {
                        generation = state.generation;
                        drop(state);
                        let result = reader.seek(position);
                        packet.clear();
                        offset = 0;
                        if let Err(error) = result {
                            let mut state = worker.buffer.lock().unwrap();
                            if state.generation == generation {
                                state.error = Some(error);
                                state.eof = true;
                            }
                        }
                        continue;
                    }
                    drop(state);
                    if offset == packet.len() {
                        let result = reader.packet(&mut packet);
                        offset = 0;
                        match result {
                            Ok(true) => {}
                            result => {
                                let mut state = worker.buffer.lock().unwrap();
                                if state.generation == generation {
                                    state.eof = true;
                                    if let Err(e) = result {
                                        state.error = Some(e);
                                    }
                                }
                                continue;
                            }
                        }
                    }
                    let mut state = worker.buffer.lock().unwrap();
                    if state.generation != generation {
                        continue;
                    }
                    // Bound lock hold time, even for codecs with large packets.
                    let count = (worker.capacity - state.frames.len())
                        .min(packet.len() - offset)
                        .min(1024);
                    state.frames.extend(&packet[offset..offset + count]);
                    offset += count;
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { shared })
    }
    pub fn seek(&self, position: Duration) {
        let mut state = self.shared.buffer.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
        state.frames.clear();
        state.eof = false;
        state.error = None;
        state.seek = Some(position);
        self.shared.wake.notify_one();
    }
    pub fn try_buffer(&self) -> Option<std::sync::MutexGuard<'_, Buffer>> {
        self.shared.buffer.try_lock().ok()
    }
    pub fn consumed(&self, before: usize, after: usize) {
        if before > self.shared.low_water && after <= self.shared.low_water {
            self.shared.wake.notify_one();
        }
    }
    pub fn error(&self) -> Option<String> {
        self.shared.buffer.lock().unwrap().error.clone()
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        let mut state = self.shared.buffer.lock().unwrap();
        state.stop = true;
        self.shared.wake.notify_one();
        // Never join a decoder from the audio callback. The worker releases its file and
        // buffer after the current packet/seek; it cannot retain the player or engine.
    }
}
