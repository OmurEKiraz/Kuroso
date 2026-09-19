use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Device, Host, SupportedStreamConfigRange};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDeviceDescriptor {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub supported_sample_rates: Vec<u32>,
    pub min_channels: u16,
    pub max_channels: u16,
    pub supports_f32: bool,
    pub supports_i16: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackStrategy {
    /// Source matches DAC native capabilities directly (zero resampling).
    BitPerfect {
        sample_rate: u32,
        channels: u16,
    },
    /// Hardware cannot natively run the source sample rate, so sinc resampling is required.
    ResampleRequired {
        source_sample_rate: u32,
        target_sample_rate: u32,
        channels: u16,
    },
}

pub struct HardwareProber {
    host: Host,
}

impl Default for HardwareProber {
    fn default() -> Self {
        Self::new()
    }
}

impl HardwareProber {
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }

    /// Enumerate all output audio devices and their physical capabilities
    pub fn enumerate_output_devices(&self) -> Result<Vec<AudioDeviceDescriptor>, String> {
        let default_device_name = self.host.default_output_device().and_then(|d| d.name().ok());
        let devices = self.host.output_devices().map_err(|e| e.to_string())?;

        let mut descriptors = Vec::new();

        for dev in devices {
            let name = dev.name().unwrap_or_else(|_| "Unknown Device".to_string());
            let is_default = default_device_name.as_deref() == Some(&name);

            let configs: Vec<SupportedStreamConfigRange> = dev
                .supported_output_configs()
                .map(|c| c.collect())
                .unwrap_or_default();

            let mut rates = Vec::new();
            let mut min_channels = u16::MAX;
            let mut max_channels = 0;
            let mut supports_f32 = false;
            let mut supports_i16 = false;

            for cfg in &configs {
                min_channels = min_channels.min(cfg.channels());
                max_channels = max_channels.max(cfg.channels());

                match cfg.sample_format() {
                    cpal::SampleFormat::F32 => supports_f32 = true,
                    cpal::SampleFormat::I16 => supports_i16 = true,
                    _ => {}
                }

                // Check standard audiophile sample rates against supported ranges
                let standard_rates = [
                    44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 768_000,
                ];
                for &sr in &standard_rates {
                    if sr >= cfg.min_sample_rate().0 && sr <= cfg.max_sample_rate().0 {
                        if !rates.contains(&sr) {
                            rates.push(sr);
                        }
                    }
                }
            }

            rates.sort_unstable();

            if min_channels == u16::MAX {
                min_channels = 0;
            }

            descriptors.push(AudioDeviceDescriptor {
                id: name.clone(),
                name,
                is_default,
                supported_sample_rates: rates,
                min_channels,
                max_channels,
                supports_f32,
                supports_i16,
            });
        }

        Ok(descriptors)
    }

    /// Retrieve the default physical output device
    pub fn get_default_device(&self) -> Option<Device> {
        self.host.default_output_device()
    }

    /// Determine if a track's audio format can be played bit-perfect,
    /// or if high-quality bandlimited resampling is required.
    pub fn negotiate_strategy(
        device: &Device,
        source_sample_rate: u32,
        source_channels: u16,
    ) -> Result<PlaybackStrategy, String> {
        let configs: Vec<SupportedStreamConfigRange> = device
            .supported_output_configs()
            .map_err(|e| e.to_string())?
            .collect();

        // 1. Check if native sample rate and channels are directly supported
        for cfg in &configs {
            if cfg.channels() == source_channels
                && source_sample_rate >= cfg.min_sample_rate().0
                && source_sample_rate <= cfg.max_sample_rate().0
            {
                return Ok(PlaybackStrategy::BitPerfect {
                    sample_rate: source_sample_rate,
                    channels: source_channels,
                });
            }
        }

        // 2. Not natively supported: choose closest best rate (prefer integer multiples: 44.1k -> 88.2k/176.4k, 48k -> 96k/192k)
        let mut supported_rates: Vec<u32> = Vec::new();
        for cfg in &configs {
            if cfg.channels() == source_channels {
                supported_rates.push(cfg.min_sample_rate().0);
                supported_rates.push(cfg.max_sample_rate().0);
            }
        }

        supported_rates.sort_unstable();
        supported_rates.dedup();

        // Prefer integer multiple family (44.1k family vs 48k family)
        let is_44_family = source_sample_rate % 44_100 == 0;
        let mut best_rate = None;

        for &rate in &supported_rates {
            let rate_44 = rate % 44_100 == 0;
            if rate >= source_sample_rate && (rate_44 == is_44_family) {
                best_rate = Some(rate);
                break;
            }
        }

        let target_rate = best_rate
            .or_else(|| supported_rates.last().copied())
            .unwrap_or(48_000);

        Ok(PlaybackStrategy::ResampleRequired {
            source_sample_rate,
            target_sample_rate: target_rate,
            channels: source_channels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_devices_runs_without_panic() {
        let prober = HardwareProber::new();
        let devices = prober.enumerate_output_devices();
        assert!(devices.is_ok(), "Device enumeration should never fail");
    }
}