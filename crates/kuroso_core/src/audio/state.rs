use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PlaybackStatus {
    #[default]
    Stopped,
    Playing,
    Paused,
    Buffering,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackProgress {
    pub elapsed: Duration,
    pub duration: Duration,
    pub fraction: f64,
}

impl PlaybackProgress {
    pub fn new(elapsed: Duration, duration: Duration) -> Self {
        let fraction = if duration.as_millis() > 0 {
            (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0)
        } else {
            0.0
        };

        Self {
            elapsed,
            duration,
            fraction,
        }
    }

    pub fn zero() -> Self {
        Self {
            elapsed: Duration::ZERO,
            duration: Duration::ZERO,
            fraction: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RepeatMode {
    #[default]
    Off,
    Track,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ShuffleMode {
    #[default]
    Off,
    Tracks,
    Albums,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_calculation() {
        let prog = PlaybackProgress::new(Duration::from_secs(30), Duration::from_secs(120));
        assert_eq!(prog.fraction, 0.25);

        let over = PlaybackProgress::new(Duration::from_secs(150), Duration::from_secs(120));
        assert_eq!(over.fraction, 1.0);

        let zero = PlaybackProgress::new(Duration::from_secs(10), Duration::ZERO);
        assert_eq!(zero.fraction, 0.0);
    }

    #[test]
    fn test_default_enums() {
        assert_eq!(PlaybackStatus::default(), PlaybackStatus::Stopped);
        assert_eq!(RepeatMode::default(), RepeatMode::Off);
        assert_eq!(ShuffleMode::default(), ShuffleMode::Off);
    }
}