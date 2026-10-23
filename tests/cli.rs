use std::{fs, path::PathBuf, process::Command};

fn output_path(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join("DiscoFM-tests");
    fs::create_dir_all(&directory).unwrap();
    directory.join(format!("{}-{name}", std::process::id()))
}

#[test]
fn cli_compares_independent_ymfm_and_recorded_audio_and_protects_existing_reports() {
    let binary = env!("CARGO_BIN_EXE_discofm");
    let session = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/basic-session.json");
    let report = output_path("independent-report.json");
    let result = Command::new(binary)
        .args(["compare", "--session"])
        .arg(&session)
        .arg("--output")
        .arg(&report)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let original = fs::read(&report).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    assert!(value["source"].as_str().unwrap().contains("ymfm"));
    assert!(
        value["comparison"]["spectral_cosine_full"]
            .as_f64()
            .unwrap()
            > 0.98
    );
    assert_eq!(value["comparison"]["recording_gain"], 1.0);
    assert!(
        !Command::new(binary)
            .args(["compare", "--session"])
            .arg(&session)
            .arg("--output")
            .arg(&report)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&report).unwrap(), original);
    let wav = output_path("independent-recording.wav");
    assert!(
        Command::new(binary)
            .args(["render", "--session"])
            .arg(&session)
            .arg("--output")
            .arg(&wav)
            .output()
            .unwrap()
            .status
            .success()
    );
    let result = Command::new(binary)
        .args(["compare", "--session"])
        .arg(&session)
        .arg("--reference")
        .arg(&wav)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(
        value["comparison"]["spectral_cosine_full"]
            .as_f64()
            .unwrap()
            > 0.999
    );
    for invalid in ["-1", "21", "NaN"] {
        assert!(
            !Command::new(binary)
                .args(["compare", "--session"])
                .arg(&session)
                .arg("--reference")
                .arg(&wav)
                .args(["--align-ms", invalid])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}
#[test]
fn cli_fits_and_refines_real_wav_into_a_reopenable_session_without_overwriting_sources() {
    let wav = output_path("fit-source.wav");
    let fitted = output_path("fit-output.json");
    let refined = output_path("fit-refined.json");
    let invalid = output_path("fit-invalid.json");
    let binary = env!("CARGO_BIN_EXE_discofm");
    assert!(
        Command::new(binary)
            .args([
                "render",
                "--key-on",
                "0.06",
                "--seconds",
                "0.12",
                "--output"
            ])
            .arg(&wav)
            .output()
            .unwrap()
            .status
            .success()
    );
    let result = Command::new(binary)
        .args(["fit", "--reference"])
        .arg(&wav)
        .arg("--output")
        .arg(&fitted)
        .args(["--pitch", "440", "--key-off", "0.06", "--lock", "all"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&fitted).unwrap()).unwrap();
    assert_eq!(json["fitting"]["runs"][0]["evaluations"], 1);
    assert_eq!(json["fitting"]["runs"][0]["status"], "Completed");
    let result = Command::new(binary)
        .args(["fit", "--reference"])
        .arg(&wav)
        .arg("--session")
        .arg(&fitted)
        .arg("--output")
        .arg(&refined)
        .args(["--refine-candidate", "1"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&refined).unwrap()).unwrap();
    assert_eq!(json["fitting"]["runs"].as_array().unwrap().len(), 2);
    assert_eq!(json["fitting"]["runs"][1]["refined"], true);
    assert!(
        Command::new(binary)
            .args(["inspect-session", "--session"])
            .arg(&refined)
            .output()
            .unwrap()
            .status
            .success()
    );
    let before = fs::read(&fitted).unwrap();
    assert!(
        !Command::new(binary)
            .args(["fit", "--reference"])
            .arg(&wav)
            .arg("--output")
            .arg(&fitted)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&fitted).unwrap(), before);
    assert!(
        !Command::new(binary)
            .args(["fit", "--reference"])
            .arg(&wav)
            .arg("--output")
            .arg(&invalid)
            .args(["--lock", "s8.tl"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!invalid.exists());
    let cancelled = output_path("fit-cancelled.json");
    let result = Command::new(binary)
        .args(["fit", "--reference"])
        .arg(&wav)
        .arg("--output")
        .arg(&cancelled)
        .args([
            "--pitch",
            "440",
            "--key-off",
            "0.06",
            "--mode",
            "basic",
            "--evaluations",
            "20000",
            "--population",
            "8",
            "--cancel-after",
            "2",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&cancelled).unwrap()).unwrap();
    let run = &json["fitting"]["runs"][0];
    assert_eq!(run["status"], "Cancelled");
    assert!((2..20000).contains(&run["evaluations"].as_u64().unwrap()));
    assert!(!run["candidates"].as_array().unwrap().is_empty());
    assert!(
        Command::new(binary)
            .args(["inspect-session", "--session"])
            .arg(&cancelled)
            .output()
            .unwrap()
            .status
            .success()
    );
    for path in [wav, fitted, refined, cancelled] {
        fs::remove_file(path).unwrap();
    }
}
#[test]
fn cli_renders_real_stereo_wav_and_saves_session_and_trace() {
    let session = output_path("cli-session.json");
    let wav = output_path("cli-render.wav");
    let trace = output_path("cli-trace.json");
    let binary = env!("CARGO_BIN_EXE_discofm");
    assert!(
        Command::new(binary)
            .args(["new-session", "--output"])
            .arg(&session)
            .status()
            .unwrap()
            .success()
    );
    let result = Command::new(binary)
        .args(["render", "--session"])
        .arg(&session)
        .args([
            "--seconds",
            "0.05",
            "--key-on",
            "0.02",
            "--sample-rate",
            "44100",
            "--output",
        ])
        .arg(&wav)
        .arg("--trace")
        .arg(&trace)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut reader = hound::WavReader::open(&wav).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, 44100);
    assert_eq!(reader.spec().sample_format, hound::SampleFormat::Float);
    assert!((reader.duration() as i64 - 2205).abs() <= 1);
    let samples = reader
        .samples::<f32>()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert!(samples.iter().any(|v| *v > 0.02));
    // Preserve the discrete backend DC residual; test both waveform polarities
    // relative to its mean, instead of assuming the WAV was DC-normalized.
    let active = &samples[..1600];
    let mean = active.iter().sum::<f32>() / active.len() as f32;
    assert!(active.iter().any(|v| *v > mean + 0.005));
    assert!(active.iter().any(|v| *v < mean - 0.005));
    assert!(
        fs::read_to_string(trace)
            .unwrap()
            .contains("diagnostic_trace_version")
    );
}
#[test]
fn cli_reports_unsupported_profiles_and_invalid_pitch() {
    for args in [
        vec!["--profile", "raw-diagnostic"],
        vec!["--note", "127"],
        vec!["--seconds", "NaN"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_discofm"))
            .args(["render", "--output"])
            .arg(output_path("invalid.wav"))
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("DiscoFM:"));
    }
}

#[test]
fn cli_rejects_equivalent_output_paths_before_truncating_files() {
    let output = output_path("collision.wav");
    fs::write(&output, b"preserve existing output").unwrap();
    let equivalent = output
        .parent()
        .unwrap()
        .join(".")
        .join(output.file_name().unwrap());
    let result = Command::new(env!("CARGO_BIN_EXE_discofm"))
        .args(["render", "--output"])
        .arg(&output)
        .arg("--trace")
        .arg(&equivalent)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("destinations must be different"));
    assert_eq!(fs::read(&output).unwrap(), b"preserve existing output");
}

#[test]
fn cli_preserves_session_when_destination_is_a_hard_link() {
    let session = output_path("hard-link-session.json");
    let alias = output_path("hard-link-output.wav");
    let binary = env!("CARGO_BIN_EXE_discofm");
    assert!(
        Command::new(binary)
            .args(["new-session", "--output"])
            .arg(&session)
            .status()
            .unwrap()
            .success()
    );
    let original = fs::read(&session).unwrap();
    fs::hard_link(&session, &alias).unwrap();
    let result = Command::new(binary)
        .args(["render", "--session"])
        .arg(&session)
        .arg("--output")
        .arg(&alias)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(&session).unwrap(), original);
}

#[test]
fn cli_inspects_actual_hardware_and_reports_nested_session_errors() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/channel3-session.json");
    let result = Command::new(env!("CARGO_BIN_EXE_discofm"))
        .args(["inspect-session", "--session"])
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["session_version"], 1);
    assert_eq!(report["channel"], 3);
    assert_eq!(report["algorithm_register"], 7);
    assert_eq!(report["algorithm_display"], 8);
    assert_eq!(
        report["routing"]["carriers"],
        serde_json::json!(["S1", "S2", "S3", "S4"])
    );
    assert_eq!(report["hardware"]["channel3_mode"], 1);
    assert_eq!(
        report["hardware"]["special_frequencies"],
        serde_json::json!([[700, 3], [1100, 4], [1500, 2]])
    );
    let mut session: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
    session["instrument"]["operators"][2]["ar"] = 32.into();
    let invalid = output_path("invalid-session.json");
    fs::write(&invalid, serde_json::to_vec(&session).unwrap()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_discofm"))
        .args(["inspect-session", "--session"])
        .arg(&invalid)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("instrument.operators[2].ar"));
}

#[test]
fn playback_rejects_invalid_settings_before_opening_audio_and_documents_controls() {
    for args in [
        vec!["--queue-ms", "0", "--commands"],
        vec!["--sample-rate", "7999", "--commands"],
        vec!["--octave", "9", "--commands"],
        vec!["--profile", "raw-diagnostic", "--commands"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_discofm"))
            .arg("play")
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
    }
    let result = Command::new(env!("CARGO_BIN_EXE_discofm"))
        .args(["play", "--help"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let help = String::from_utf8_lossy(&result.stdout);
    for option in [
        "--session",
        "--device",
        "--commands",
        "--octave",
        "--profile",
    ] {
        assert!(help.contains(option));
    }
    let result = Command::new(env!("CARGO_BIN_EXE_discofm"))
        .arg("play")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("interactive terminal"));
}

#[test]
fn cli_lists_extracts_and_fits_bank_samples_and_rejects_overwrite() {
    let binary = env!("CARGO_BIN_EXE_discofm");
    let wav = output_path("bank-source.wav");
    let bank = output_path("bank.sfz");
    let extracted = output_path("bank-extracted.wav");
    let session = output_path("bank-fitted.json");
    {
        let mut w = hound::WavWriter::create(
            &wav,
            hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..800 {
            w.write_sample(
                ((i as f32 * std::f32::consts::TAU * 440. / 8000.).sin() * 8000.) as i16,
            )
            .unwrap();
        }
        w.finalize().unwrap();
    }
    fs::write(
        &bank,
        format!(
            "<region> sample={}\n<region> sample={} offset=10",
            wav.file_name().unwrap().to_str().unwrap(),
            wav.file_name().unwrap().to_str().unwrap()
        ),
    )
    .unwrap();
    let result = Command::new(binary)
        .arg("samples")
        .arg(&bank)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["samples"].as_array().unwrap().len(), 2);
    assert_eq!(value["samples"][1]["id"], 2);
    let result = Command::new(binary)
        .arg("extract-sample")
        .arg(&bank)
        .args(["--sample", "2", "--output"])
        .arg(&extracted)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut reader = hound::WavReader::open(&extracted).unwrap();
    assert_eq!(reader.duration(), 790);
    let got: Vec<f32> = reader.samples::<f32>().map(Result::unwrap).collect();
    let mut source = hound::WavReader::open(&wav).unwrap();
    let expected: Vec<f32> = source
        .samples::<i16>()
        .skip(10)
        .flat_map(|v| [v.unwrap() as f32 / 32768.; 2])
        .collect();
    assert_eq!(got, expected);
    let before = fs::read(&extracted).unwrap();
    assert!(
        !Command::new(binary)
            .arg("extract-sample")
            .arg(&bank)
            .args(["--sample", "1", "--output"])
            .arg(&extracted)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&extracted).unwrap(), before);
    assert!(
        !Command::new(binary)
            .arg("extract-sample")
            .arg(&bank)
            .args(["--sample", "99", "--output"])
            .arg(output_path("bank-invalid.wav"))
            .output()
            .unwrap()
            .status
            .success()
    );
    let result = Command::new(binary)
        .args(["fit", "--reference"])
        .arg(&bank)
        .args(["--sample", "1", "--output"])
        .arg(&session)
        .args([
            "--compute",
            "cpu",
            "--workers",
            "1",
            "--pitch",
            "440",
            "--key-off",
            "0.06",
            "--lock",
            "all",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&session).unwrap()).unwrap();
    assert_eq!(value["reference_sample"]["index"], 0);
    assert_eq!(
        value["reference_sample"]["identity"]
            .as_str()
            .unwrap()
            .len(),
        16
    );
    assert_eq!(value["fitting"]["runs"][0]["evaluations"], 1);
    let refined = output_path("bank-refined.json");
    let result = Command::new(binary)
        .args(["fit", "--reference"])
        .arg(&bank)
        .arg("--session")
        .arg(&session)
        .arg("--output")
        .arg(&refined)
        .args(["--refine-candidate", "1"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let after: serde_json::Value = serde_json::from_slice(&fs::read(&refined).unwrap()).unwrap();
    assert_eq!(after["reference_sample"], value["reference_sample"]);
    for p in [wav, bank, extracted, session, refined] {
        fs::remove_file(p).unwrap();
    }
}

#[test]
fn cli_exports_tracker_instruments_and_imports_them_into_new_sessions() {
    let binary = env!("CARGO_BIN_EXE_discofm");
    let session = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/basic-session.json");
    for extension in ["tfi", "vgi", "tyi"] {
        let exported = output_path(&format!("patch.{extension}"));
        let result = Command::new(binary)
            .args(["export", "--session"])
            .arg(&session)
            .arg("--output")
            .arg(&exported)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let bytes = fs::read(&exported).unwrap();
        assert_eq!(
            bytes.len(),
            [42, 43, 32][["tfi", "vgi", "tyi"]
                .iter()
                .position(|e| *e == extension)
                .unwrap()]
        );
        // Existing files are never replaced.
        assert!(
            !Command::new(binary)
                .args(["export", "--session"])
                .arg(&session)
                .arg("--output")
                .arg(&exported)
                .status()
                .unwrap()
                .success()
        );
        let imported = output_path(&format!("imported-{extension}.json"));
        assert!(
            Command::new(binary)
                .arg("import-instrument")
                .arg(&exported)
                .arg("--output")
                .arg(&imported)
                .status()
                .unwrap()
                .success()
        );
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&session).unwrap()).unwrap();
        let reopened: serde_json::Value =
            serde_json::from_slice(&fs::read(&imported).unwrap()).unwrap();
        assert_eq!(
            reopened["instrument"]["operators"], original["instrument"]["operators"],
            "{extension}"
        );
        assert_eq!(
            reopened["instrument"]["channel"]["algorithm"],
            original["instrument"]["channel"]["algorithm"]
        );
    }
    let wrong = output_path("patch.bin");
    assert!(
        !Command::new(binary)
            .args(["export", "--session"])
            .arg(&session)
            .arg("--output")
            .arg(&wrong)
            .status()
            .unwrap()
            .success()
    );
    assert!(!wrong.exists());
}
