use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

pub struct AudioResampler {
    resampler: SincFixedIn<f32>,
    input_channels: usize,
}

impl AudioResampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> Result<Self, String> {
        if source_rate == target_rate {
            return Err("Resampler requested for identical rates".to_string());
        }

        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };

        let resampler = SincFixedIn::<f32>::new(
            target_rate as f64 / source_rate as f64,
            2.0,
            params,
            1024,
            channels,
        )
        .map_err(|e| format!("Failed to create sinc resampler: {e}"))?;

        Ok(Self {
            resampler,
            input_channels: channels,
        })
    }

    /// Process planar audio channel buffers
    pub fn process(&mut self, input: &[Vec<f32>]) -> Result<Vec<Vec<f32>>, String> {
        if input.len() != self.input_channels {
            return Err("Input channels do not match resampler configuration".to_string());
        }
        self.resampler
            .process(input, None)
            .map_err(|e| format!("Resampling DSP error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_rate_rejected() {
        let res = AudioResampler::new(48_000, 48_000, 2);
        assert!(res.is_err());
    }

    #[test]
    fn test_resampler_creation_44_to_48() {
        let res = AudioResampler::new(44_100, 48_000, 2);
        assert!(res.is_ok());
    }
}