use super::EngineState;
use ::wasapi::*;
use std::sync::{Arc, atomic::Ordering, mpsc::SyncSender};

pub(super) fn run(
    state: &Arc<EngineState>,
    ready: &SyncSender<Result<(), String>>,
) -> Result<(), String> {
    initialize_mta().ok().map_err(|e| e.to_string())?;
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            deinitialize();
        }
    }
    let _com = Com;
    run_initialized(state, ready).map_err(|e| e.to_string())
}
fn run_initialized(
    state: &EngineState,
    ready: &SyncSender<Result<(), String>>,
) -> Result<(), WasapiError> {
    let enumerator = DeviceEnumerator::new()?;
    let device = enumerator.get_default_device(&Direction::Render)?;
    let mut client = device.get_iaudioclient()?;
    let rate = client.get_mixformat()?.get_samplespersec();
    let format = WaveFormat::new(32, 32, &SampleType::Float, rate as usize, 2, None);
    let (period, _) = client.get_device_period()?;
    client.initialize_client(
        &format,
        &Direction::Render,
        &StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: period,
        },
    )?;
    let event = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;
    let capacity = client.get_buffer_size()? as usize;
    let mut output = vec![[0f32; 2]; capacity];
    let mut bytes = vec![0u8; capacity * 8];
    render.write_to_device(capacity, &bytes, None)?;
    client.start_stream()?;
    let _ = ready.send(Ok(()));
    let result = (|| {
        while !state.stop.load(Ordering::Acquire) {
            // No polling or busy wait. A timeout also bounds shutdown on device loss.
            event.wait_for_event(100)?;
            let frames = client.get_available_space_in_frames()? as usize;
            if frames == 0 {
                continue;
            }
            output[..frames].fill([0., 0.]);
            if let Ok(mut voices) = state.voices.try_lock() {
                voices.retain(|weak| {
                    let Some(player) = weak.upgrade() else {
                        return false;
                    };
                    if let Ok(mut voice) = player.try_lock() {
                        voice.mix(&mut output[..frames], rate);
                    }
                    true
                });
            }
            for (frame, dst) in output[..frames].iter().zip(bytes.chunks_exact_mut(8)) {
                for (sample, dst) in frame.iter().zip(dst.chunks_exact_mut(4)) {
                    let sample = if sample.is_nan() {
                        0.
                    } else {
                        sample.clamp(-1., 1.)
                    };
                    dst.copy_from_slice(&sample.to_le_bytes());
                }
            }
            render.write_to_device(frames, &bytes[..frames * 8], None)?;
        }
        Ok(())
    })();
    let stopped = client.stop_stream();
    result.and(stopped)
}
