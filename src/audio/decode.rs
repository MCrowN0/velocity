use std::{fs::File, path::Path, time::Duration};
use symphonia::core::{
    audio::{Channels, SampleBuffer},
    codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    probe::Hint,
};

pub(super) struct Reader {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    pub rate: u32,
    pub frames: Option<u64>,
    pub channels: usize,
    samples: Option<SampleBuffer<f32>>,
    skip_until: Option<u64>,
    time_base: symphonia::core::units::TimeBase,
    start_ts: u64,
}
impl Reader {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )
            .map_err(|e| e.to_string())?
            .format;
        let track = format
            .default_track()
            .filter(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .or_else(|| {
                format
                    .tracks()
                    .iter()
                    .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            })
            .ok_or("no audio track")?;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| e.to_string())?;
        let rate = track
            .codec_params
            .sample_rate
            .ok_or("audio sample rate is unknown")?;
        if rate == 0 || rate > 384_000 {
            return Err("unsupported audio sample rate".into());
        }
        let channels = track.codec_params.channels.map_or(2, |c| c.count());
        if channels == 0 {
            return Err("audio has no channels".into());
        }
        let time_base = track
            .codec_params
            .time_base
            .unwrap_or(symphonia::core::units::TimeBase::new(1, rate));
        if time_base.numer == 0 || time_base.denom == 0 {
            return Err("invalid audio time base".into());
        }
        let frames = track.codec_params.n_frames.map(|n| {
            ((n as u128 * time_base.numer as u128 * rate as u128) / time_base.denom as u128)
                .min(u64::MAX as u128) as u64
        });
        Ok(Self {
            track: track.id,
            frames,
            time_base,
            start_ts: track.codec_params.start_ts,
            rate,
            channels,
            decoder,
            format,
            samples: None,
            skip_until: None,
        })
    }
    pub fn seek(&mut self, position: Duration) -> Result<(), String> {
        let time_base = self.time_base;
        let divisor = time_base.numer as u128 * 1_000_000_000;
        let ts = ((position.as_nanos() * time_base.denom as u128 + divisor / 2) / divisor)
            .min(u64::MAX as u128) as u64;
        let ts = ts.saturating_add(self.start_ts);
        let sought = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::TimeStamp {
                    ts,
                    track_id: self.track,
                },
            )
            .map_err(|e| e.to_string())?;
        self.decoder.reset();
        self.skip_until = Some(sought.required_ts);
        Ok(())
    }
    // Reuses packet conversion storage. The caller reuses `out` as well.
    pub fn packet(&mut self, out: &mut Vec<[f32; 2]>) -> Result<bool, String> {
        out.clear();
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                Err(e) => return Err(e.to_string()),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = self.decoder.decode(&packet).map_err(|e| e.to_string())?;
            if decoded.spec().rate != self.rate {
                return Err("sample rate changed within audio track".into());
            }
            let channels = decoded.spec().channels;
            let count = channels.count();
            if count == 0 {
                return Err("audio has no channels".into());
            }
            let needed = decoded.capacity() * count;
            if self.samples.as_ref().is_none_or(|s| s.capacity() < needed) {
                self.samples = Some(SampleBuffer::new(
                    decoded.capacity() as u64,
                    *decoded.spec(),
                ));
            }
            let samples = self.samples.as_mut().unwrap();
            samples.copy_interleaved_ref(decoded);
            // Timestamps are in the track time base, which need not be sample frames.
            let skip = self.skip_until.map_or(0, |target| {
                let ticks = target.saturating_sub(packet.ts());
                let numerator = ticks as u128 * self.time_base.numer as u128 * self.rate as u128;
                ((numerator + self.time_base.denom as u128 / 2) / self.time_base.denom as u128)
                    .min(usize::MAX as u128) as usize
            });
            let frame_count = samples.samples().len() / count;
            if skip >= frame_count {
                continue;
            }
            self.skip_until = None;
            let mut weights = [[0.; 2]; 32];
            for (weight, channel) in weights.iter_mut().zip(channels.iter()) {
                *weight = match channel {
                    Channels::FRONT_LEFT => [1., 0.],
                    Channels::FRONT_RIGHT => [0., 1.],
                    Channels::LFE1 | Channels::LFE2 => [0., 0.],
                    Channels::REAR_LEFT | Channels::SIDE_LEFT => [0.707, 0.],
                    Channels::REAR_RIGHT | Channels::SIDE_RIGHT => [0., 0.707],
                    _ => [0.707, 0.707],
                };
            }
            for frame in samples.samples().chunks_exact(count).skip(skip) {
                let stereo = if count == 1 {
                    [frame[0]; 2]
                } else if count == 2 {
                    [frame[0], frame[1]]
                } else {
                    frame
                        .iter()
                        .zip(&weights)
                        .fold([0., 0.], |mut sum, (s, w)| {
                            sum[0] += s * w[0];
                            sum[1] += s * w[1];
                            sum
                        })
                };
                out.push(stereo.map(|s| if s.is_finite() { s } else { 0. }));
            }
            return Ok(true);
        }
    }
}
