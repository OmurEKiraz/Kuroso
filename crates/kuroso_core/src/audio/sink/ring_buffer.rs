use ringbuf::traits::Split;
use ringbuf::HeapRb;

pub type AudioProducer = ringbuf::CachingProd<std::sync::Arc<HeapRb<f32>>>;
pub type AudioConsumer = ringbuf::CachingCons<std::sync::Arc<HeapRb<f32>>>;

pub fn create_audio_ring_buffer(capacity_frames: usize, channels: usize) -> (AudioProducer, AudioConsumer) {
    let total_samples = capacity_frames * channels;
    let rb = HeapRb::<f32>::new(total_samples);
    rb.split()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::traits::{Consumer, Producer};

    #[test]
    fn test_ring_buffer_transfer() {
        let (mut prod, mut cons) = create_audio_ring_buffer(1024, 2);
        let samples = vec![0.5f32, -0.5f32, 0.25f32, -0.25f32];
        
        let written = prod.push_slice(&samples);
        assert_eq!(written, 4);

        let mut read_buf = vec![0.0f32; 4];
        let read = cons.pop_slice(&mut read_buf);
        assert_eq!(read, 4);
        assert_eq!(read_buf, samples);
    }
}