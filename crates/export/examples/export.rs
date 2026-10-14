//! Exports a take without the app: `cargo run --release -p polishcast-export --example export
//! -- <take dir> <out.mp4> [long side] [fps]`.

use polishcast_export::{export, Progress, Settings};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, out, ..] = args.as_slice() else {
        anyhow::bail!("usage: export <take dir> <out.mp4> [long side] [fps]");
    };
    let mut settings = Settings::default();
    if let Some(v) = args.get(2) {
        settings.long_side = v.parse()?;
    }
    if let Some(v) = args.get(3) {
        settings.fps = v.parse()?;
    }
    let progress = Progress::default();
    let started = Instant::now();
    std::thread::scope(|s| {
        let job = s.spawn(|| export(dir.as_ref(), out.as_ref(), settings, &progress, &AtomicBool::new(false)));
        while !job.is_finished() {
            eprint!("\r{:5.1}%", progress.fraction() * 100.0);
            std::thread::sleep(Duration::from_millis(200));
        }
        eprintln!("\r100.0% in {:.1} s", started.elapsed().as_secs_f64());
        job.join().unwrap()
    })
}
