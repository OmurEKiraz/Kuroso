pub struct GainProcessor {
    gain_factor: f32,
}

impl GainProcessor {
    pub fn new(gain_db: f32) -> Self {
        let gain_factor = 10.0f32.powf(gain_db / 20.0);
        Self { gain_factor }
    }

    pub fn from_factor(factor: f32) -> Self {
        Self {
            gain_factor: factor.max(0.0),
        }
    }

    pub fn process_interleaved(&self, samples: &mut [f32]) {
        if (self.gain_factor - 1.0).abs() < 1e-6 {
            return;
        }
        for sample in samples.iter_mut() {
            *sample = (*sample * self.gain_factor).clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gain_db_scaling() {
        let mut buffer = vec![0.5, -0.5];
        let proc = GainProcessor::new(-6.0); // -6dB is roughly 0.5x
        proc.process_interleaved(&mut buffer);
        assert!((buffer[0] - 0.25).abs() < 0.02);
    }
}