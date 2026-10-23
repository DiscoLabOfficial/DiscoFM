use discofm_soundfont::{SampleSelection, SoundBank};
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
pub(crate) fn selection(
    path: &Path,
    id: Option<u32>,
    saved: Option<&SampleSelection>,
) -> Result<Option<SampleSelection>, Box<dyn Error>> {
    if !discofm_soundfont::is_bank(path) {
        if id.is_some() {
            return Err("--sample requer SF2/SF3/SFZ/DLS".into());
        }
        return Ok(None);
    }
    let bank = SoundBank::open(path, &|| false)?;
    if let Some(id) = id {
        return Ok(Some(bank.selection((id - 1) as usize)?));
    }
    if let Some(saved) = saved {
        return Ok(Some(saved.clone()));
    }
    if bank.samples.len() == 1 {
        return Ok(Some(bank.selection(0)?));
    }
    Err(
        "Bank has several samples: pass --sample ID; use `discofm samples BANK` to list them"
            .into(),
    )
}
pub(crate) fn saved_selection<'a>(
    session: &'a discofm_formats::Session,
    session_path: Option<&Path>,
    reference: &Path,
) -> Option<&'a SampleSelection> {
    let path = PathBuf::from(session.reference_path.as_ref()?);
    let path = if path.is_absolute() {
        path
    } else {
        session_path?.parent()?.join(path)
    };
    if crate::same_path(&path, reference).unwrap_or(false) {
        session.reference_sample.as_ref()
    } else {
        None
    }
}
pub(crate) fn list(path: &Path) -> Result<(), Box<dyn Error>> {
    let bank = SoundBank::open(path, &|| false)?;
    let samples: Vec<_> = bank
        .samples
        .iter()
        .map(|s| serde_json::json!({"id":s.index+1,"sample":s}))
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"bank":bank.path,"format":bank.format,"samples":samples})
        )?
    );
    Ok(())
}
pub(crate) fn extract(path: &Path, id: u32, output: &Path) -> Result<(), Box<dyn Error>> {
    if crate::same_path(path, output)? {
        return Err("The output WAV cannot overwrite the bank".into());
    }
    let bank = SoundBank::open(path, &|| false)?;
    let audio = bank.read_sample((id - 1) as usize, &|| false)?;
    discofm_audio::write_wav(output, &audio)?;
    println!(
        "{} · {} frames · {} Hz",
        output.display(),
        audio.frames.len(),
        audio.sample_rate
    );
    Ok(())
}
pub(crate) fn play(path: &Path, id: u32, device_id: Option<String>) -> Result<(), Box<dyn Error>> {
    let bank = SoundBank::open(path, &|| false)?;
    let audio = Arc::new(bank.read_sample((id - 1) as usize, &|| false)?);
    let seconds = audio.frames.len() as f64 / f64::from(audio.sample_rate);
    let mut playback = discofm_audio::Playback::open(
        discofm_chip::Instrument::default(),
        discofm_engine::LiveConfig::default(),
        discofm_audio::DeviceOptions {
            device_id,
            ..Default::default()
        },
    )?;
    playback.controls().play_clip(audio)?;
    println!(
        "{} · {:.3} s",
        bank.samples[(id - 1) as usize].name,
        seconds
    );
    while !playback.controls().clip_finished() {
        playback.check_health()?;
        std::thread::sleep(Duration::from_millis(25));
    }
    playback.controls().stop();
    Ok(())
}
