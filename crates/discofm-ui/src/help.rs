//! Tooltip texts. Kept together so wording stays consistent and claims stay
//! within what the emulated chip and the analysis actually do.

// Operator registers.
pub(crate) const TL: &str = "Total Level — attenuation in 0.75 dB steps.\n0 = loudest, 127 = silent.\nOn a carrier this is volume; on a modulator it sets modulation depth (brightness).";
pub(crate) const AR: &str = "Attack Rate — how fast the envelope rises after key-on.\n0 = never rises (silent), 31 = instant.";
pub(crate) const D1R: &str = "First Decay Rate — speed from the peak down to the sustain level (SL).\n0 = stays at the peak.";
pub(crate) const SL: &str = "Sustain Level — where the first decay hands over to the second.\n0 = at the peak, each step is 3 dB lower, 15 = fully decayed.";
pub(crate) const D2R: &str = "Second Decay Rate — how the level keeps falling while the key is held.\n0 = holds steady at SL.";
pub(crate) const RR: &str =
    "Release Rate — speed after key-off.\n0 is slow but still finite; 15 is fastest.";
pub(crate) const DT: &str = "Detune — small pitch offset that grows with the key code.\nRaw 0–3 = +0…+3, 4 = −0, 5–7 = −1…−3.";
pub(crate) const MUL: &str = "Frequency multiple of the channel pitch.\n0 = ×½, 1–15 = ×1…×15. Sets which harmonic this operator runs at.";
pub(crate) const RS: &str = "Rate Scaling (key scaling) — 0–3.\nHigher values make every envelope rate faster on higher notes.";
pub(crate) const VEL: &str = "Velocity depth — editor audition setting, not a chip register.\nAt velocity v this adds round((127 − v) · depth / 127) to TL. 0 = velocity has no effect.";
pub(crate) const AM: &str = "Amplitude-modulation enable — lets the LFO tremolo (channel AMS depth) reach this operator.\nNeeds LFO on and AMS > 0.";
pub(crate) const SSG: &str = "SSG-EG — repeating / inverted envelope shapes (raw 8–15).\nINV inverts, ALT alternates direction each cycle, HOLD stops after one cycle.\nOff for normal envelopes.";
pub(crate) const EG_PLOT: &str = "Envelope captured from the emulated chip during the preview render (not an idealised ADSR).\nGrey: raw generator attenuation. Colour: final operator attenuation after TL, AM and SSG.\nTop = loud (0), bottom = silent (1023).";

// Channel.
pub(crate) const ALGORITHM: &str = "Operator routing. Carriers (OUT) are heard directly; modulators shape the timbre of the operator they feed.";
pub(crate) const FEEDBACK: &str = "S1 self-modulation, 0–7.\nLow values add brightness; 6–7 turn S1 into a buzzy, noise-like source.";
pub(crate) const LFO: &str =
    "Global LFO (shared by all six channels). Required for tremolo (AMS) and vibrato (FMS).";
pub(crate) const LFO_RATE: &str = "LFO speed, 0 = slowest … 7 = fastest.";
pub(crate) const AMS: &str = "Tremolo depth, 0–3. Applies only to operators with AM enabled.";
pub(crate) const FMS: &str = "Vibrato depth for this channel, 0–7.";
pub(crate) const STEREO: &str = "Channel output enables. With both off the channel is silent.";
pub(crate) const CH3: &str = "Channel 3 special mode: S1–S3 get independent FNUM/BLOCK frequencies, S4 follows the played note.\nForces audition on hardware channel 3.";
pub(crate) const FNUM: &str = "Frequency number, 0–2047 (pitch within the octave).";
pub(crate) const BLOCK: &str = "Block, 0–7 (octave).";

// Audition / transport.
pub(crate) const PLAY: &str = "Play the audition note live through the selected audio output.";
pub(crate) const RELEASE: &str =
    "Send key-off to the sounding note so its release can be heard. Shortcut: Space.";
pub(crate) const STOP: &str = "Stop all sound immediately. Shortcut: Esc.";
pub(crate) const NOTE: &str = "MIDI note used for the preview render and the Play button.";
pub(crate) const FINE: &str = "Fine tuning in cents, applied through the chip frequency registers.";
pub(crate) const VELOCITY: &str =
    "Audition velocity. Only operators with a velocity depth above 0 respond.";
pub(crate) const PROFILE: &str = "Chip variant emulated by Nuked-OPN2.\nYM2612 includes the discrete Mega Drive DAC behaviour (unverified upstream); YM3438 is the integrated CMOS variant without it.";
pub(crate) const CLOCK: &str = "Master clock: NTSC (≈7.67 MHz) or PAL (≈7.60 MHz). Changes the pitch mapping and envelope timing.";
pub(crate) const CHANNEL: &str =
    "Hardware channel 1–6 used for audition. Locked to 3 in channel 3 special mode.";
pub(crate) const GATE: &str = "How long the key is held before key-off.";
pub(crate) const LENGTH: &str = "Total render length, including the release tail.";
pub(crate) const OCTAVE: &str =
    "Octave of the on-screen and computer keyboard (Z S X D C V G B H N J M ,).";
pub(crate) const DEVICE: &str = "Audio output. The device opens on first playback.";

// Reference & analysis.
pub(crate) const IMPORT: &str = "Load a reference: WAV, or a sample from an SF2, SF3, SFZ or DLS bank. You can also drop a file anywhere on the window.";
pub(crate) const OPEN: &str = "Open a DiscoFM session (.json).";
pub(crate) const SAVE: &str =
    "Save patch, audition, reference settings and fitting history as a session (.json).";
pub(crate) const PLAY_A: &str = "Play the prepared reference region (A).";
pub(crate) const PLAY_B: &str = "Play the chip render for the same region (B).";
pub(crate) const MATCH_LEVEL: &str = "Apply a single gain to B in plots and clips so A and B have equal RMS. The envelope is preserved, both are attenuated together if needed to avoid clipping, and chip registers are unchanged.";
pub(crate) const CHANNEL_MODE: &str = "Which channel of the recording is analysed.";
pub(crate) const FFT: &str =
    "FFT size for spectrum and spectrogram. Larger = finer frequency, coarser time.";
pub(crate) const REMOVE_DC: &str = "Subtract the region's mean before analysis.";
pub(crate) const NORMALIZE: &str =
    "Scale the analysed region to full peak level. The original audio is not modified.";
pub(crate) const TRIM: &str =
    "Skip leading and trailing audio more than 60 dB below the region's peak.";
pub(crate) const REGION: &str = "Part of the recording to analyse and compare (at most 30 s).";
pub(crate) const MANUAL_PITCH: &str = "Use this pitch instead of the YIN estimate. Needed for noisy, percussive or inharmonic sounds.";
pub(crate) const MANUAL_NOTE: &str = "Pick the manual pitch from a MIDI note.";
pub(crate) const NOTE_ON: &str = "Where the note starts in the recording (seconds).";
pub(crate) const KEY_OFF: &str = "Where key-off happens in the recording. It is not inferred from the decay; set it by ear or from the plots.";
pub(crate) const APPLY_ANALYSIS: &str = "Analyse the region, then set the audition note, fine tune, gate and length from the detected (or manual) pitch and markers.";

// Fitting.
pub(crate) const FIT_MODE_BASIC: &str =
    "Searches algorithm, ratios, levels and detune; envelopes keep their initial values.";
pub(crate) const FIT_MODE_ADVANCED: &str =
    "Also searches envelopes, SSG-EG, LFO and sensitivities. Use this for anything with a decay.";
pub(crate) const COMPUTE_AUTO: &str = "Experimental: measure CPU and GPU spectral analysis and use the faster one. Synthesis (most of the time) always runs on the CPU, so the GPU rarely saves more than a few percent.";
pub(crate) const COMPUTE_CPU: &str = "CPU spectral analysis (default). With a fixed seed, results are identical for any worker count.";
pub(crate) const COMPUTE_GPU: &str = "Experimental: force GPU spectral analysis (wgpu). Fails loudly instead of falling back to the CPU.";
pub(crate) const RECORDING_GAIN: &str = "Allow one gain for the whole note when comparing. Keeps envelope and timbre differences; the chip output itself is unchanged.";
pub(crate) const UNPITCHED: &str = "For drums and noise: skips the pitch-confidence check and keys the patch on the lowest strong spectral peak (the body), or on the manual pitch when set. Uses band weights and drops pitch, partials, periodicity and harmonic terms.";
pub(crate) const BUDGET: &str = "Number of candidate renders the search may evaluate.";
pub(crate) const POPULATION: &str =
    "Candidates per generation of the differential-evolution search.";
pub(crate) const SEED: &str =
    "Random seed. The same seed and settings reproduce the same search on CPU.";
pub(crate) const MAX_SECONDS: &str = "Longest note length the fitter will render.";
pub(crate) const OPEN_LOG: &str =
    "Open the folder with discofm.log and crash records to send to the developer.";
pub(crate) const WORKERS: &str =
    "CPU workers that render and score candidates. 0 = automatic (every logical CPU, up to 16).";
pub(crate) const CANDIDATES: &str = "How many ranked candidates to keep.";
pub(crate) const DE_F: &str =
    "Differential weight (F): mutation step size. The starting value when self-adapting.";
pub(crate) const DE_ADAPTIVE: &str = "jDE: every population member carries its own F and CR, occasionally resampled and kept only when they produce a better patch. Off (default) uses the fixed F and CR below, which did better on long runs in tests.";
pub(crate) const DE_CR: &str = "Crossover rate (CR): fraction of coordinates taken from the mutant. The starting value when self-adapting.";
pub(crate) const POLICY: &str = "Auto lets the search try this chip mode; Keep preserves the initial on/off state. Individual locks always win.";
pub(crate) const SPECIAL_FREQ: &str = "Also search the channel 3 special FNUM/BLOCK values of S1–S3 (only with channel 3 set to Keep).";
pub(crate) const LOCKS: &str =
    "Checked parameters keep the initial value of each run, including when refining a candidate.";
pub(crate) const START_FIT: &str = "Search for chip settings whose real render matches the reference. Uses the region, pitch and markers from the reference panel.";
pub(crate) const REFINE: &str =
    "Run a new local search seeded with this run's candidates, starting from the selected one.";
pub(crate) const APPLY: &str = "Copy the selected candidate (patch and audition) into the editor.";
pub(crate) const APPLY_PLAY: &str =
    "Apply the selected candidate and play B as soon as its preview is ready.";

pub(crate) fn weight(name: &str) -> &'static str {
    match name {
        "Pitch" => "Fundamental frequency track.",
        "Partials" => {
            "Frequencies and relative energy of up to 12 partials, including inharmonic peaks."
        }
        "AM periodicity" => "Periodic amplitude movement (tremolo-like) of the log-RMS envelope.",
        "Log-STFT" => "Log-magnitude spectrogram distance at 512/1024/4096 FFT sizes.",
        "Convergence" => "Spectral convergence: relative magnitude error of the spectrograms.",
        "Envelope" => "RMS envelope over time (linear, relative and log errors).",
        "Harmonics" => "Normalised energies of up to 32 harmonics.",
        "Transients" => "Attack/onset shape from the RMS derivative.",
        "Modulation" => "Spectral centroid and flux trajectories.",
        "Tail" => "Decay and release tail level.",
        "Bands" => "Smoothed energy in 24 log-spaced bands, 40 Hz–11 kHz.",
        "Off-grid" => {
            "Energy between the reference's harmonics (an octave-low ×½ ratio, inharmonic sidebands), beyond the share the reference itself has."
        }
        _ => "",
    }
}
pub(crate) const PATCH_MENU: &str = "Copy, paste, import, export and the patch library. Undo/redo cover every patch edit (Ctrl+Z / Ctrl+Y).";
pub(crate) const COPY_PATCH: &str = "Copy this patch to the clipboard as text (Ctrl+C outside text fields). Paste it into another session or another DiscoFM window.";
pub(crate) const PASTE_PATCH: &str = "Replace the patch with one copied from DiscoFM (Ctrl+V outside text fields). Undo restores the current patch.";
pub(crate) const IMPORT_PATCH: &str = "Load a patch from a TFI, VGI or TYI tracker instrument or from a DiscoFM session. Undo restores the current patch.";
pub(crate) const EXPORT: &str = "Save the patch as a tracker instrument, as GENNY writes them:\nTFI (TFM Music Maker; no AMS/FMS or AM), VGI (VGM Music Maker; no panning), TYI (register dump with panning).\nNone stores the global LFO, channel 3 special frequencies or velocity depth; the status bar lists what was left out.";
pub(crate) const LIBRARY: &str = "A folder of patches: DiscoFM sessions (.json) and TFI/VGI/TYI files. Click one to load it; undo restores the previous patch. Set DISCOFM_LIBRARY to change the default folder.";
pub(crate) const LIBRARY_ADD: &str =
    "Save the current patch and audition settings into the library as a session file.";
pub(crate) const MIDI: &str = "Play the audition voice from a MIDI keyboard: note on/off with velocity, any channel. The audition is monophonic, like the on-screen piano.";
pub(crate) const SCREENING: &str = "Local refinement first scores each move with ymfm (about ten times cheaper than Nuked-OPN2 and nearly rank-identical on chip targets) and skips moves predicted worse in both directions. Every kept score still comes from Nuked-OPN2. Better results for the same budget, about 40% more time.";
