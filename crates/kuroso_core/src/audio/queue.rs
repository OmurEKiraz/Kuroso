use crate::audio::state::{RepeatMode, ShuffleMode};
use crate::library::database::LibraryDatabase;
use crate::library::queries::TrackView;
use crate::library::types::TrackId;
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct PlaybackQueue {
    history: VecDeque<TrackId>,
    history_limit: usize,
    current: Option<TrackId>,
    up_next: VecDeque<TrackId>,
    playlist: Vec<TrackId>,
    playlist_index: Option<usize>,
    shuffled_indices: Vec<usize>,
    shuffle_cursor: usize,
    pub repeat_mode: RepeatMode,
    pub shuffle_mode: ShuffleMode,
}

impl Default for PlaybackQueue {
    fn default() -> Self {
        Self::new(100)
    }
}

impl PlaybackQueue {
    /// Initialize with a bounded history capacity
    pub fn new(history_limit: usize) -> Self {
        Self {
            history: VecDeque::with_capacity(history_limit),
            history_limit,
            current: None,
            up_next: VecDeque::new(),
            playlist: Vec::new(),
            playlist_index: None,
            shuffled_indices: Vec::new(),
            shuffle_cursor: 0,
            repeat_mode: RepeatMode::Off,
            shuffle_mode: ShuffleMode::Off,
        }
    }

    /// Load a new collection of tracks (e.g., from an album or search results)
    pub fn load_tracks(&mut self, tracks: Vec<TrackId>, start_index: Option<usize>) {
        self.history.clear();
        self.up_next.clear();
        self.playlist = tracks;
        self.rebuild_shuffle_indices();

        if let Some(idx) = start_index {
            if idx < self.playlist.len() {
                self.playlist_index = Some(idx);
                self.current = Some(self.playlist[idx]);
                if self.shuffle_mode != ShuffleMode::Off {
                    if let Some(pos) = self.shuffled_indices.iter().position(|&i| i == idx) {
                        self.shuffle_cursor = pos;
                    }
                }
            } else {
                self.playlist_index = None;
                self.current = None;
            }
        } else {
            self.playlist_index = None;
            self.current = None;
        }
    }

    /// Prioritize track immediately after the current playing track
    pub fn play_next(&mut self, track_id: TrackId) {
        self.up_next.push_front(track_id);
    }

    /// Append track to the manual "Up Next" queue
    pub fn append_to_queue(&mut self, track_id: TrackId) {
        self.up_next.push_back(track_id);
    }

    /// Append multiple tracks to the main playlist
    pub fn append_playlist_tracks(&mut self, mut tracks: Vec<TrackId>) {
        let old_len = self.playlist.len();
        self.playlist.append(&mut tracks);

        // If in shuffle mode, append the new indices randomly
        if self.shuffle_mode != ShuffleMode::Off {
            let mut new_indices: Vec<usize> = (old_len..self.playlist.len()).collect();
            let mut rng = thread_rng();
            new_indices.shuffle(&mut rng);
            self.shuffled_indices.extend(new_indices);
        }
    }

    /// Move track inside "Up Next" queue (e.g. user drag and drop)
    pub fn move_up_next(&mut self, from: usize, to: usize) -> bool {
        if from >= self.up_next.len() || to >= self.up_next.len() {
            return false;
        }
        if from == to {
            return true;
        }
        if let Some(track) = self.up_next.remove(from) {
            self.up_next.insert(to, track);
            return true;
        }
        false
    }

    /// Move track inside main playlist
    pub fn move_playlist_track(&mut self, from: usize, to: usize) -> bool {
        if from >= self.playlist.len() || to >= self.playlist.len() {
            return false;
        }
        if from == to {
            return true;
        }

        let item = self.playlist.remove(from);
        self.playlist.insert(to, item);

        // Update active index if affected
        if let Some(idx) = self.playlist_index {
            if idx == from {
                self.playlist_index = Some(to);
            } else if from < idx && to >= idx {
                self.playlist_index = Some(idx - 1);
            } else if from > idx && to <= idx {
                self.playlist_index = Some(idx + 1);
            }
        }

        self.rebuild_shuffle_indices();
        true
    }

    /// Remove a specific track from Up Next
    pub fn remove_from_up_next(&mut self, index: usize) -> Option<TrackId> {
        self.up_next.remove(index)
    }

    /// Remove a track from playlist by index
    pub fn remove_from_playlist(&mut self, index: usize) -> Option<TrackId> {
        if index >= self.playlist.len() {
            return None;
        }

        let removed = self.playlist.remove(index);

        if let Some(idx) = self.playlist_index {
            if idx == index {
                self.playlist_index = if self.playlist.is_empty() {
                    None
                } else {
                    Some(index.min(self.playlist.len() - 1))
                };
            } else if index < idx {
                self.playlist_index = Some(idx - 1);
            }
        }

        self.rebuild_shuffle_indices();
        Some(removed)
    }

    /// Clear user Up Next queue
    pub fn clear_up_next(&mut self) {
        self.up_next.clear();
    }

    /// Clear history
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Reset everything
    pub fn clear_all(&mut self) {
        self.history.clear();
        self.up_next.clear();
        self.playlist.clear();
        self.shuffled_indices.clear();
        self.current = None;
        self.playlist_index = None;
        self.shuffle_cursor = 0;
    }

    pub fn current_track(&self) -> Option<TrackId> {
        self.current
    }

    /// Jump directly to a track at a specific index in the current playlist
    pub fn jump_to_playlist_index(&mut self, index: usize) -> Option<TrackId> {
        if index >= self.playlist.len() {
            return None;
        }

        if let Some(curr) = self.current {
            self.push_history(curr);
        }

        self.playlist_index = Some(index);
        self.current = Some(self.playlist[index]);

        if self.shuffle_mode != ShuffleMode::Off {
            if let Some(pos) = self.shuffled_indices.iter().position(|&i| i == index) {
                self.shuffle_cursor = pos;
            }
        }

        self.current
    }

    /// Advances to the next track respecting Up Next, Repeat, and Shuffle
    pub fn next(&mut self) -> Option<TrackId> {
        if let Some(curr) = self.current {
            if self.repeat_mode == RepeatMode::Track {
                return Some(curr);
            }
            self.push_history(curr);
        }

        // 1. Prioritize manual "Up Next"
        if let Some(next_manual) = self.up_next.pop_front() {
            self.current = Some(next_manual);
            return self.current;
        }

        // 2. Play from playlist
        if self.playlist.is_empty() {
            self.current = None;
            self.playlist_index = None;
            return None;
        }

        match self.shuffle_mode {
            ShuffleMode::Off => {
                let next_idx = match self.playlist_index {
                    Some(idx) => idx + 1,
                    None => 0,
                };

                if next_idx < self.playlist.len() {
                    self.playlist_index = Some(next_idx);
                    self.current = Some(self.playlist[next_idx]);
                } else if self.repeat_mode == RepeatMode::Queue {
                    self.playlist_index = Some(0);
                    self.current = self.playlist.first().copied();
                } else {
                    self.playlist_index = None;
                    self.current = None;
                }
            }
            ShuffleMode::Tracks => {
                let next_cursor = if self.current.is_some() {
                    self.shuffle_cursor + 1
                } else {
                    self.shuffle_cursor
                };

                if next_cursor < self.shuffled_indices.len() {
                    self.shuffle_cursor = next_cursor;
                    let playlist_idx = self.shuffled_indices[self.shuffle_cursor];
                    self.playlist_index = Some(playlist_idx);
                    self.current = Some(self.playlist[playlist_idx]);
                } else if self.repeat_mode == RepeatMode::Queue {
                    self.rebuild_shuffle_indices();
                    self.shuffle_cursor = 0;
                    if let Some(&playlist_idx) = self.shuffled_indices.first() {
                        self.playlist_index = Some(playlist_idx);
                        self.current = Some(self.playlist[playlist_idx]);
                    }
                } else {
                    self.playlist_index = None;
                    self.current = None;
                }
            }
            ShuffleMode::Albums => {
                self.playlist_index = None;
                self.current = None;
            }
        }

        self.current
    }

    /// Moves backwards in history or restarts track
    pub fn previous(&mut self) -> Option<TrackId> {
        if let Some(prev) = self.history.pop_back() {
            if let Some(curr) = self.current.take() {
                self.up_next.push_front(curr);
            }
            self.current = Some(prev);
            self.playlist_index = self.playlist.iter().position(|&t| t == prev);
            return self.current;
        }

        self.current
    }

    pub fn set_shuffle(&mut self, mode: ShuffleMode) {
        self.shuffle_mode = mode;
        if mode != ShuffleMode::Off {
            self.rebuild_shuffle_indices();
            if let Some(idx) = self.playlist_index {
                if let Some(pos) = self.shuffled_indices.iter().position(|&i| i == idx) {
                    self.shuffle_cursor = pos;
                }
            }
        }
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat_mode = mode;
    }

    pub fn set_history_limit(&mut self, limit: usize) {
        self.history_limit = limit;
        while self.history.len() > self.history_limit {
            self.history.pop_front();
        }
    }

    pub fn history(&self) -> &VecDeque<TrackId> {
        &self.history
    }

    pub fn up_next(&self) -> &VecDeque<TrackId> {
        &self.up_next
    }

    pub fn playlist(&self) -> &[TrackId] {
        &self.playlist
    }

    pub fn playlist_index(&self) -> Option<usize> {
        self.playlist_index
    }

    /// Validates and purges tracks no longer found in the library database
    /// (e.g. unmounted USB drive or deleted local files)
    pub fn sanitize(&mut self, db: &LibraryDatabase) {
        self.up_next.retain(|id| db.get_track(*id).is_some());
        self.history.retain(|id| db.get_track(*id).is_some());

        let mut new_playlist = Vec::new();
        let mut new_index = None;

        for (idx, id) in self.playlist.iter().enumerate() {
            if db.get_track(*id).is_some() {
                if self.playlist_index == Some(idx) {
                    new_index = Some(new_playlist.len());
                }
                new_playlist.push(*id);
            }
        }

        self.playlist = new_playlist;
        self.playlist_index = new_index;
        if let Some(curr) = self.current {
            if db.get_track(curr).is_none() {
                self.current = None;
            }
        }
        self.rebuild_shuffle_indices();
    }

    /// Resolve UI view models for all tracks currently in "Up Next"
    pub fn resolve_up_next_views(&self, db: &LibraryDatabase) -> Vec<TrackView> {
        self.up_next
            .iter()
            .filter_map(|&id| {
                let track = db.get_track(id)?;
                let artist = db.get_artist(track.artist_id)?;
                let album = track.album_id.and_then(|a_id| db.get_album(a_id));
                let album_artist = track.album_artist_id.and_then(|a_id| db.get_artist(a_id));

                Some(TrackView {
                    id: track.id,
                    path: track.path,
                    title: track.title,
                    artist_id: track.artist_id,
                    artist_name: artist.name,
                    album_artist_id: track.album_artist_id,
                    album_artist_name: album_artist.map(|a| a.name),
                    album_id: track.album_id,
                    album_title: album.map(|a| a.title),
                    duration_ms: track.duration_ms,
                    track_number: track.track_number,
                    disc_number: track.disc_number,
                    year: track.year,
                    format: track.format,
                    bitrate: track.bitrate,
                    sample_rate: track.sample_rate,
                    bit_depth: track.bit_depth,
                    channels: track.channels,
                    track_gain_db: track.track_gain_db,
                    track_peak: track.track_peak,
                    album_gain_db: track.album_gain_db,
                    album_peak: track.album_peak,
                })
            })
            .collect()
    }

    fn push_history(&mut self, track_id: TrackId) {
        if self.history_limit == 0 {
            return;
        }
        if self.history.len() >= self.history_limit {
            self.history.pop_front();
        }
        self.history.push_back(track_id);
    }

    fn rebuild_shuffle_indices(&mut self) {
        self.shuffled_indices = (0..self.playlist.len()).collect();
        let mut rng = thread_rng();
        self.shuffled_indices.shuffle(&mut rng);
        self.shuffle_cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_linear_progression() {
        let mut q = PlaybackQueue::default();
        let tracks = vec![TrackId(1), TrackId(2), TrackId(3)];
        q.load_tracks(tracks, Some(0));

        assert_eq!(q.current_track(), Some(TrackId(1)));
        assert_eq!(q.next(), Some(TrackId(2)));
        assert_eq!(q.next(), Some(TrackId(3)));
        assert_eq!(q.next(), None);
    }

    #[test]
    fn test_up_next_priority() {
        let mut q = PlaybackQueue::default();
        let tracks = vec![TrackId(1), TrackId(2), TrackId(3)];
        q.load_tracks(tracks, Some(0));

        q.play_next(TrackId(99));
        assert_eq!(q.next(), Some(TrackId(99)));
        assert_eq!(q.next(), Some(TrackId(2)));
        assert_eq!(q.next(), Some(TrackId(3)));
    }

    #[test]
    fn test_history_capacity_bound() {
        let mut q = PlaybackQueue::new(2);
        q.load_tracks(vec![TrackId(1), TrackId(2), TrackId(3), TrackId(4)], Some(0));

        q.next(); // current is 2, history: [1]
        q.next(); // current is 3, history: [1, 2]
        q.next(); // current is 4, history: [2, 3] (1 dropped)

        assert_eq!(q.history().len(), 2);
        assert_eq!(q.history()[0], TrackId(2));
        assert_eq!(q.history()[1], TrackId(3));
    }

    #[test]
    fn test_reorder_up_next() {
        let mut q = PlaybackQueue::default();
        q.append_to_queue(TrackId(10));
        q.append_to_queue(TrackId(20));
        q.append_to_queue(TrackId(30));

        assert!(q.move_up_next(2, 0)); // Move 30 to index 0
        assert_eq!(q.up_next()[0], TrackId(30));
        assert_eq!(q.up_next()[1], TrackId(10));
        assert_eq!(q.up_next()[2], TrackId(20));
    }

    #[test]
    fn test_jump_to_index() {
        let mut q = PlaybackQueue::default();
        q.load_tracks(vec![TrackId(1), TrackId(2), TrackId(3), TrackId(4)], Some(0));

        assert_eq!(q.jump_to_playlist_index(2), Some(TrackId(3)));
        assert_eq!(q.current_track(), Some(TrackId(3)));
        assert_eq!(q.next(), Some(TrackId(4)));
    }
}