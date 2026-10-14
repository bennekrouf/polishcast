//! Shrinks a recording in place, as the app does after each take: `cargo run -p
//! polishcast-export --example compact -- <screen.mov> [sound offset]`. With an offset, the
//! `sound.mov` next to it is mixed in first, as after recording one window with the Mac's sound.

fn main() -> anyhow::Result<()> {
    let file = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("usage: compact <screen.mov>"))?;
    if let Some(offset) = std::env::args().nth(2) {
        let sound = std::path::Path::new(&file).with_file_name(polishcast_core::take::SOUND);
        polishcast_export::sound::mix_in(file.as_ref(), &sound, offset.parse()?)?;
    }
    let before = std::fs::metadata(&file)?.len();
    let started = std::time::Instant::now();
    let after = polishcast_export::compact::compact(file.as_ref())?;
    println!(
        "{:.1} MB → {:.1} MB ({:.1}× smaller) in {:.1} s",
        before as f64 / 1e6,
        after as f64 / 1e6,
        before as f64 / after as f64,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
