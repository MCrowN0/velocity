use super::*;
use std::{fs::File, io::Write, sync::atomic::AtomicU64, time::Instant};

struct Wave(PathBuf);
impl Wave {
    fn new(frames: u32, channels: u16, rate: u32, sparse: bool) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "velocity-audio-{}-{}.wav",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = File::create(&path).unwrap();
        let bytes = frames * channels as u32 * 4;
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36 + bytes).to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16u32.to_le_bytes()).unwrap();
        file.write_all(&3u16.to_le_bytes()).unwrap();
        file.write_all(&channels.to_le_bytes()).unwrap();
        file.write_all(&rate.to_le_bytes()).unwrap();
        file.write_all(&(rate * channels as u32 * 4).to_le_bytes())
            .unwrap();
        file.write_all(&(channels * 4).to_le_bytes()).unwrap();
        file.write_all(&32u16.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&bytes.to_le_bytes()).unwrap();
        if sparse {
            file.set_len(44 + bytes as u64).unwrap();
        } else {
            for frame in 0..frames {
                for _ in 0..channels {
                    file.write_all(&((frame % 100) as f32 / 100.).to_le_bytes())
                        .unwrap();
                }
            }
        }
        Self(path)
    }
}
impl Drop for Wave {
    fn drop(&mut self) {
        // Decode workers close files asynchronously after player drop.
        for _ in 0..100 {
            if std::fs::remove_file(&self.0).is_ok() {
                return;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
}
fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "decoder failed to make progress");
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn repeated_player_drop_releases_voice_source_and_decoder_buffer() {
    let wave = Wave::new(8000, 2, 1000, false);
    for mode in [AudioLoadMode::Memory, AudioLoadMode::Stream] {
        let source = AudioSource::load_with_mode(&wave.0, mode).unwrap();
        let source_weak = Arc::downgrade(&source.0);
        let engine = AudioEngine::new();
        for _ in 0..32 {
            let player = AudioPlayer::new(source.clone()).unwrap();
            engine.play(&player).unwrap();
            let voice = Arc::downgrade(&player.0);
            let released = player
                .0
                .lock()
                .unwrap()
                .stream
                .as_ref()
                .map(|s| s.released_probe());
            drop(player);
            assert!(voice.upgrade().is_none());
            if let Some(released) = released {
                wait_for(released);
            }
        }
        drop(source);
        assert!(source_weak.upgrade().is_none());
    }
}
#[test]
fn memory_decode_controls_and_independent_players() {
    let wave = Wave::new(100, 1, 1000, false);
    let source = AudioSource::load(&wave.0).unwrap();
    assert_eq!(source.mode(), AudioLoadMode::Memory);
    assert_eq!(source.duration(), Some(Duration::from_millis(100)));
    let a = AudioPlayer::new(source.clone()).unwrap();
    let b = AudioPlayer::new(source).unwrap();
    assert!(Arc::ptr_eq(
        &a.0.lock().unwrap().source.0,
        &b.0.lock().unwrap().source.0
    ));
    a.play();
    a.seek(Duration::from_millis(20));
    a.set_volume(0.5).unwrap();
    let mut out = [[0.; 2]; 4];
    a.0.lock().unwrap().mix(&mut out, 2000);
    for (i, frame) in out.iter().enumerate() {
        assert!((frame[0] - (0.20 + i as f32 * 0.005) * 0.5).abs() < 1e-6);
        assert_eq!(frame[0], frame[1]);
    }
    assert_eq!(b.position(), Duration::ZERO);
    a.pause();
    out.fill([0.; 2]);
    a.0.lock().unwrap().mix(&mut out, 1000);
    assert_eq!(out, [[0.; 2]; 4]);
    a.stop();
    assert_eq!(a.position(), Duration::ZERO);
    assert!(a.set_volume(f32::NAN).is_err());
    assert!(a.set_volume(-1.).is_err());
}
#[test]
fn auto_uses_decoded_f32_size_and_modes_override_it() {
    for channels in [1, 2] {
        let frames = (MEMORY_LIMIT / (channels as usize * 4)) as u32;
        let big = Wave::new(frames + 1, channels, 48000, true);
        assert_eq!(
            AudioSource::load(&big.0).unwrap().mode(),
            AudioLoadMode::Stream
        );
        let exact = Wave::new(frames, channels, 48000, true);
        assert_eq!(
            AudioSource::load(&exact.0).unwrap().mode(),
            AudioLoadMode::Memory
        );
    }
    let small = Wave::new(10, 2, 1000, false);
    assert_eq!(
        AudioSource::load_with_mode(&small.0, AudioLoadMode::Stream)
            .unwrap()
            .mode(),
        AudioLoadMode::Stream
    );
    assert_eq!(
        AudioSource::load_with_mode(&small.0, AudioLoadMode::Memory)
            .unwrap()
            .mode(),
        AudioLoadMode::Memory
    );
}
#[test]
fn streaming_is_bounded_refills_and_seeks_without_stale_samples() {
    let wave = Wave::new(8000, 2, 1000, false);
    let player =
        AudioPlayer::new(AudioSource::load_with_mode(&wave.0, AudioLoadMode::Stream).unwrap())
            .unwrap();
    let buffered = || {
        player
            .0
            .lock()
            .unwrap()
            .stream
            .as_ref()
            .unwrap()
            .try_buffer()
            .map_or(0, |b| b.frames.len())
    };
    wait_for(|| buffered() == 3000);
    thread::sleep(Duration::from_millis(20));
    assert_eq!(buffered(), 3000);
    player.play();
    player.0.lock().unwrap().mix(&mut [[0.; 2]; 2000], 1000);
    wait_for(|| buffered() == 3000);
    player.seek(Duration::from_millis(5143));
    wait_for(|| {
        player
            .0
            .lock()
            .unwrap()
            .stream
            .as_ref()
            .unwrap()
            .try_buffer()
            .is_some_and(|b| b.eof && b.frames.len() == 2857)
    });
    let mut out = [[0.; 2]; 10];
    player.0.lock().unwrap().mix(&mut out, 1000);
    assert!((out[0][0] - 0.43).abs() < 1e-6, "{out:?}");
    assert!(player.error().is_none());
    player.seek(Duration::from_millis(120));
    player.seek(Duration::from_millis(740));
    wait_for(|| buffered() == 3000);
    out.fill([0.; 2]);
    player.0.lock().unwrap().mix(&mut out, 1000);
    assert!((out[0][0] - 0.40).abs() < 1e-6);
}
#[test]
fn eof_and_replay_and_engine_ownership() {
    let wave = Wave::new(10, 2, 1000, false);
    let player = AudioPlayer::new(AudioSource::load(&wave.0).unwrap()).unwrap();
    let engine = AudioEngine::new();
    engine.play(&player).unwrap();
    engine.play(&player).unwrap();
    assert_eq!(engine.state.voices.lock().unwrap().len(), 1);
    assert!(AudioEngine::new().play(&player).is_err());
    player.0.lock().unwrap().mix(&mut [[0.; 2]; 20], 1000);
    assert!(!player.is_playing());
    player.play();
    assert_eq!(player.position(), Duration::ZERO);
    assert!(play_audio(&player).is_err());
    assert!(
        AudioEngine::with_backend(AudioBackend::Asio)
            .start()
            .is_err()
    );
    drop(engine);
    assert!(AudioEngine::new().play(&player).is_ok());
}
#[test]
fn invalid_file_returns_an_error() {
    let wave = Wave::new(0, 1, 1000, false);
    std::fs::write(&wave.0, b"not audio").unwrap();
    assert!(AudioSource::load(&wave.0).is_err());
}

#[test]
fn flac_decode_and_sample_accurate_seek() {
    // A deterministic FLAC stream with one verbatim mono subframe. No encoder tool
    // or network fixture is needed; both FLAC CRCs are generated here.
    let fixture = Wave::new(0, 1, 1000, false);
    let mut bytes = b"fLaC\x80\x00\x00\x22".to_vec();
    bytes.extend_from_slice(&100u16.to_be_bytes()); // min/max block size
    bytes.extend_from_slice(&100u16.to_be_bytes());
    bytes.extend_from_slice(&[0; 6]); // unknown min/max encoded frame size
    let info = (1000u64 << 44) | (15u64 << 36) | 100;
    bytes.extend_from_slice(&info.to_be_bytes());
    bytes.extend_from_slice(&[0; 16]); // MD5 unspecified
    let mut frame = vec![0xff, 0xf8, 0x60, 0x08, 0, 99];
    let mut crc = 0u8;
    for byte in &frame {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 7
            } else {
                crc << 1
            };
        }
    }
    frame.push(crc);
    frame.push(2); // verbatim, no wasted bits
    for i in 0..100i16 {
        frame.extend_from_slice(&(i * 200).to_be_bytes());
    }
    let mut crc = 0u16;
    for byte in &frame {
        crc ^= (*byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    frame.extend_from_slice(&crc.to_be_bytes());
    bytes.extend_from_slice(&frame);
    std::fs::write(&fixture.0, bytes).unwrap();
    let source = AudioSource::load(&fixture.0).unwrap();
    assert_eq!(source.duration(), Some(Duration::from_millis(100)));
    let mut reader = Reader::open(&fixture.0).unwrap();
    reader.seek(Duration::from_millis(43)).unwrap();
    let mut packet = Vec::new();
    assert!(reader.packet(&mut packet).unwrap());
    assert_eq!(packet.len(), 57);
    assert_eq!(packet[0], [8600. / 32768.; 2]);
    drop(reader);
}

#[test]
fn streamed_resampling_matches_memory_at_eof_and_seek_to_end_is_clean() {
    let wave = Wave::new(100, 1, 1000, false);
    let memory = AudioPlayer::new(AudioSource::load(&wave.0).unwrap()).unwrap();
    let stream =
        AudioPlayer::new(AudioSource::load_with_mode(&wave.0, AudioLoadMode::Stream).unwrap())
            .unwrap();
    wait_for(|| {
        stream
            .0
            .lock()
            .unwrap()
            .stream
            .as_ref()
            .unwrap()
            .try_buffer()
            .is_some_and(|b| b.eof)
    });
    for rate in [500, 1000, 2000] {
        memory.play();
        stream.play();
        let mut expected = [[0.; 2]; 250];
        let mut actual = [[0.; 2]; 250];
        memory.0.lock().unwrap().mix(&mut expected, rate);
        stream.0.lock().unwrap().mix(&mut actual, rate);
        assert_eq!(expected, actual);
        assert!(!stream.is_playing());
        memory.stop();
        stream.stop();
        wait_for(|| {
            stream
                .0
                .lock()
                .unwrap()
                .stream
                .as_ref()
                .unwrap()
                .try_buffer()
                .is_some_and(|b| b.eof)
        });
    }
    stream.seek(Duration::from_secs(99));
    assert_eq!(stream.position(), Duration::from_millis(100));
    assert!(!stream.is_playing());
    assert!(stream.error().is_none());
}

/// Manual release-mode CPU check, deliberately excludes decoding/device waits.
#[test]
#[ignore]
fn mixer_benchmark() {
    for count in [1, 32] {
        let source = AudioSource(Arc::new(Source {
            rate: 44100,
            duration: Some(Duration::from_secs(11)),
            data: Data::Memory(Pcm::Mono(vec![0.01; 44100 * 11].into())),
        }));
        let mut players: Vec<_> = (0..count)
            .map(|_| AudioPlayer::new(source.clone()).unwrap())
            .collect();
        let mut voices: Vec<_> = players.iter_mut().map(|p| p.0.lock().unwrap()).collect();
        for voice in &mut voices {
            voice.playing = true;
        }
        let mut output = [[0.; 2]; 480];
        let start = Instant::now();
        for _ in 0..1000 {
            output.fill([0.; 2]);
            for voice in &mut voices {
                voice.mix(&mut output, 48000);
            }
            std::hint::black_box(&output);
        }
        eprintln!(
            "{count} voices, 10 seconds at 48 kHz: {:?} mixer time",
            start.elapsed()
        );
    }
}

#[test]
fn matching_rate_blocks_preserve_mix_seek_and_eof() {
    for pcm in [
        Pcm::Mono(vec![0.25, -0.5, 0.75].into()),
        Pcm::Stereo(vec![[0.25, -0.25], [-0.5, 0.5], [0.75, -0.75]].into()),
    ] {
        let expected: Vec<_> = (0..pcm.len()).map(|i| pcm.frame(i)).collect();
        let source = AudioSource(Arc::new(Source {
            rate: 48000,
            duration: None,
            data: Data::Memory(pcm),
        }));
        let player = AudioPlayer::new(source).unwrap();
        let mut voice = player.0.lock().unwrap();
        voice.playing = true;
        voice.volume = 0.5;
        let mut output = [[0.1; 2]; 5];
        voice.mix(&mut output, 48000);
        for i in 0..3 {
            for c in 0..2 {
                assert_eq!(output[i][c], 0.1 + expected[i][c] * 0.5);
            }
        }
        assert_eq!(&output[3..], &[[0.1; 2]; 2]);
        assert_eq!(voice.cursor, 3.);
        assert!(voice.ended && !voice.playing);
        voice.cursor = 0.5;
        voice.playing = true;
        let mut output = [[0.; 2]; 1];
        voice.mix(&mut output, 48000);
        for c in 0..2 {
            assert_eq!(
                output[0][c],
                (expected[0][c] + (expected[1][c] - expected[0][c]) * 0.5) * 0.5
            );
        }
    }
}

#[test]
#[ignore = "release CPU mixer percentiles"]
fn mixer_percentiles() {
    for rate in [44100, 48000] {
        for stereo in [false, true] {
            let pcm = if stereo {
                Pcm::Stereo(vec![[0.01, -0.01]; 48000].into())
            } else {
                Pcm::Mono(vec![0.01; 48000].into())
            };
            let source = AudioSource(Arc::new(Source {
                rate,
                duration: None,
                data: Data::Memory(pcm),
            }));
            let players: Vec<_> = (0..32)
                .map(|_| AudioPlayer::new(source.clone()).unwrap())
                .collect();
            let mut voices: Vec<_> = players.iter().map(|p| p.0.lock().unwrap()).collect();
            let mut samples = Vec::new();
            let mut output = [[0.; 2]; 480];
            for i in 0..2200 {
                for voice in &mut voices {
                    voice.cursor = 0.;
                    voice.playing = true;
                }
                output.fill([0.; 2]);
                let start = Instant::now();
                for voice in &mut voices {
                    voice.mix(std::hint::black_box(&mut output), 48000);
                }
                let ms = start.elapsed().as_secs_f64() * 1000.;
                std::hint::black_box(&output);
                if i >= 200 {
                    samples.push(ms);
                }
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "audio 32 voices rate={rate} stereo={stereo}, 480 frames: median={:.6} p95={:.6} p99={:.6} ms",
                samples[1000], samples[1900], samples[1980]
            );
        }
    }
}

/// Requires a Windows audio output device. Run explicitly to exercise real WASAPI.
#[test]
#[ignore]
fn wasapi_device_smoke() {
    let wave = Wave::new(48000, 1, 48000, true);
    let memory = AudioPlayer::new(AudioSource::load(&wave.0).unwrap()).unwrap();
    let stream =
        AudioPlayer::new(AudioSource::load_with_mode(&wave.0, AudioLoadMode::Stream).unwrap())
            .unwrap();
    memory.set_volume(0.).unwrap();
    stream.set_volume(0.).unwrap();
    let mut engine = AudioEngine::new();
    engine.start().unwrap();
    engine.play(&memory).unwrap();
    engine.play(&stream).unwrap();
    wait_for(|| {
        memory.position() >= Duration::from_millis(20)
            && stream.position() >= Duration::from_millis(20)
    });
    stream.seek(Duration::from_millis(500));
    wait_for(|| stream.position() >= Duration::from_millis(520));
    assert!(stream.error().is_none());
    assert!(engine.error().is_none(), "{:?}", engine.error());
}
