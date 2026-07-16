use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::f32::consts::PI;
use std::time::Duration;

const TEST_TONE_HZ: f32 = 440.0;

pub fn list_output_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let devices = host
        .output_devices()
        .context("Falha ao listar dispositivos de áudio")?;

    Ok(devices.filter_map(|d| d.name().ok()).collect())
}

/// Toca um tom de 440Hz por `duration` no dispositivo escolhido, direto por
/// nome (o mesmo retornado por `list_output_devices`).
pub fn play_test_tone(device_name: &str, duration: Duration) -> Result<()> {
    let host = cpal::default_host();
    let device = host
        .output_devices()
        .context("Falha ao listar dispositivos de áudio")?
        .find(|d| d.name().map(|n| n == device_name).unwrap_or(false))
        .ok_or_else(|| anyhow::anyhow!("Dispositivo de áudio não encontrado: {}", device_name))?;

    let supported_config = device
        .default_output_config()
        .context("Falha ao obter configuração padrão do dispositivo")?;
    let sample_format = supported_config.sample_format();
    let config: cpal::StreamConfig = supported_config.into();
    let sample_rate = config.sample_rate.0 as f32;
    let channels = config.channels as usize;

    let mut clock = 0f32;
    let mut next_sample = move || {
        clock = (clock + 1.0) % sample_rate;
        (clock * TEST_TONE_HZ * 2.0 * PI / sample_rate).sin() * 0.3
    };

    let err_fn = |err| log::error!("Erro no stream de áudio: {}", err);

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(channels) {
                    let value = next_sample();
                    frame.iter_mut().for_each(|s| *s = value);
                }
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config,
            move |data: &mut [i16], _| {
                for frame in data.chunks_mut(channels) {
                    let value = (next_sample() * i16::MAX as f32) as i16;
                    frame.iter_mut().for_each(|s| *s = value);
                }
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_output_stream(
            &config,
            move |data: &mut [u16], _| {
                for frame in data.chunks_mut(channels) {
                    let value = ((next_sample() * i16::MAX as f32) + i16::MAX as f32) as u16;
                    frame.iter_mut().for_each(|s| *s = value);
                }
            },
            err_fn,
            None,
        ),
        other => anyhow::bail!("Formato de amostra não suportado: {:?}", other),
    }
    .context("Falha ao construir stream de saída")?;

    stream.play().context("Falha ao tocar tom de teste")?;
    std::thread::sleep(duration);
    stream.pause().context("Falha ao parar tom de teste")?;
    drop(stream);

    Ok(())
}
