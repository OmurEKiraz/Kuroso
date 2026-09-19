use kuroso_core::audio::{Player, PlayerSnapshot, PlayerState};
use std::io::{stdout, Write};
use std::thread;
use std::time::Duration;

pub struct KurosoTui {
    player: Player,
}

impl KurosoTui {
    pub fn new(player: Player) -> Self {
        Self { player }
    }

    pub fn run_loop(&self) {
        println!("\x1B[2J\x1B[1;1H");
        loop {
            let snap = self.player.snapshot();
            self.render(&snap);

            if snap.state == PlayerState::Stopped
                && snap.playlist_len > 0
                && snap.playlist_index == Some(snap.playlist_len - 1)
                && snap.elapsed.as_secs() > 0
            {
                break;
            }

            thread::sleep(Duration::from_millis(200));
        }
    }

    fn render(&self, snap: &PlayerSnapshot) {
        print!("\x1B[1;1H");

        let status_badge = match snap.state {
            PlayerState::Playing => "\x1B[1;32m▶ PLAYING\x1B[0m",
            PlayerState::Paused => "\x1B[1;33m⏸ PAUSED \x1B[0m",
            PlayerState::Stopped => "\x1B[1;31m⏹ STOPPED\x1B[0m",
        };

        println!("╭──────────────────────────────────────────────────────────────╮");
        println!("│                    KUROSO AUDIO ENGINE                       │");
        println!("╰──────────────────────────────────────────────────────────────╯");
        println!(
            " Status: {}         Hardware Volume: {:3.0}%",
            status_badge,
            snap.volume * 100.0
        );

        let cur_idx_str = snap
            .playlist_index
            .map(|i| (i + 1).to_string())
            .unwrap_or_else(|| "-".into());
        println!(
            " Playlist: Track {} of {} (Up Next: {})",
            cur_idx_str, snap.playlist_len, snap.up_next_count
        );
        println!("────────────────────────────────────────────────────────────────");

        if let Some(ref t) = snap.current_track {
            println!(" Title:   \x1B[1;37m{}\x1B[0m", t.title);
            println!(" Artist:  {}", t.artist_name);
            println!(" Album:   {}", t.album_title.as_deref().unwrap_or("Unknown Album"));
            println!(
                " Format:  {} Hz | Channels: {} | Format: {:?}",
                t.sample_rate.unwrap_or(48_000),
                t.channels.unwrap_or(2),
                t.format
            );
        } else {
            println!(" No track currently selected.");
            println!();
            println!();
            println!();
        }

        let elapsed_sec = snap.elapsed.as_secs();
        let total_sec = snap.duration.map(|d| d.as_secs()).unwrap_or(0);

        let progress_ratio = if total_sec > 0 {
            (elapsed_sec as f32 / total_sec as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let bar_width: usize = 38;
        let filled = (progress_ratio * bar_width as f32) as usize;
        let empty = bar_width.saturating_sub(filled);

        let bar: String = "━".repeat(filled) + "╸" + &"─".repeat(empty.saturating_sub(1));

        println!("────────────────────────────────────────────────────────────────");
        println!(
            " {:02}:{:02}  \x1B[36m[{}]\x1B[0m  {:02}:{:02}",
            elapsed_sec / 60,
            elapsed_sec % 60,
            bar,
            total_sec / 60,
            total_sec % 60
        );
        println!("────────────────────────────────────────────────────────────────");
        println!(" [Controls] Background Thread Active | PipeWire DAC Lock");

        let _ = stdout().flush();
    }
}