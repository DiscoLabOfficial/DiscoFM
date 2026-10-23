# Reference analysis

Import has no duration, sample-rate or channel-count admission limit. A checked
RIFF chunk walker handles little endian WAVE, big endian RIFX PCM/float, and RF64
ds64 data sizes, sample counts and chunk tables. PCM containers up to 64 bits,
including padded 24-in-32, float32/64 and standard extensible PCM/float decode
directly. Unsigned 8-bit PCM is centered; other integer samples scale by their
stored precision. Mono is duplicated; more than two channels fold to the mean of
all channels in both stereo lanes. Stereo retains gain, phase and sample rate.

Symphonia 0.6.1 supplies MS/IMA ADPCM, A-law/μ-law and Ogg Vorbis decoding.
Vorbis ACM WAV tags with complete Ogg headers are decoded from the exact data
chunk, excluding padding/trailing metadata. RIFF fact/RF64 sample counts trim
encoder padding at the end. This fixes FL Studio Legacy HIT_2.wav (0x674F): its
blockAlign=1 and bits=16 describe compressed data and must not be validated as PCM.
The local original was verified at 44.1 kHz, 80273 frames, 1.820249 seconds.
Unknown codecs, including WavPack, remain explicit unsupported-codec errors;
separately stored Vorbis headers and compressed RIFX/RF64 beyond the decoder's
capabilities are not claimed supported. No external FFmpeg is required at runtime.

The 30 seconds bound applies only to the analysis region. Source selections and
absolute timing markers may extend beyond 30 seconds. On opening a longer file,
the editor selects its first 30 seconds; omitting the API end selects up to
30 seconds from start. All decoded PCM remains resident, and allocation failures
are reported. Malformed/truncated files and nonfinite float samples also fail
explicitly. Original PCM stays immutable. To prevent nonfinite features, working
amplitudes above 1e10 are rejected before FFT arithmetic.

Preparation uses a manual time region and Mix/Left/Right selection. The selected
region's mean supplies optional DC correction; reported L/R DC and clipping
counts still describe the whole original. Peak and RMS measure the selection
before normalization, whose gain is 1/peak. Samples above max(peak×0.001,1e−7)
survive silence trimming, while a silent selection is left intact. Effective
boundaries keep original-file seconds, and original PCM is never changed.
Reference playback uses the shared comparison preparation described below.

YIN analyzes the strongest available 0.4-second region, resampled with the shared
sinc converter to 24 kHz. Cumulative mean normalized difference chooses the first
local minimum below 0.15; interpolation uses the unnormalized difference to reduce
short-lag bias. A Hann spectrum then refines the fundamental within ±25 cents
of that estimate, using eightfold zero padding and log-power interpolation.
It requires at least four cycles, a resolved local peak and fundamental power
within 40 dB of the strongest peak; otherwise YIN's estimate is retained. This
corrects small decay-related tuning bias without choosing a stronger harmonic
or another octave. Manual pitch takes precedence over both estimates.
The supported fundamental range is 20–4000 Hz; confidence is
1−CMND at the chosen lag. Fewer than 25 ms or insufficient periodic evidence
returns no estimate. This is a single monophonic estimate, not a pitch contour,
polyphonic transcription, or proof of a unique FM patch. Manual Hz/note overrides
control the harmonic grid; the detected estimate remains visible separately.

FFT uses a periodic Hann window with coherent-gain amplitude correction. The UI
selects 512/1024/2048/4096/8192 samples. STFT hop is at least N/4 and grows to bound
the display to roughly 1024 time columns. Spectrograms use up to 256 logarithmic
frequency rows, nearest FFT bins and a −100…0 dBFS display range. Spectrum averages
middle-half windows where available; short audio uses zero padding. Frequency
resolution is sample_rate/N: harmonic levels are local maxima around the expected
harmonic bin, not independently tracked partial frequencies. Up to 32 harmonics
below Nyquist are extracted; the table shows the first eight.

Every 5 ms, temporal analysis measures 10 ms RMS/peak windows. It also extracts
amplitude-weighted centroid, normalized positive spectral flux and separated
energy/flux transient candidates. Those transients are heuristics and can
include boundaries. An amplitude decay is never used to infer the manual
note-on/key-off markers.

The editor compares prepared-reference/chip waveforms, spectra, spectrograms,
RMS envelopes, centroid and flux. Both use 48 kHz, the same channel/DC setting,
reference normalization and source interval from max(selection start, note-on)
to selection end. B renders pre-roll from note-on and is cropped consistently.
Trim helps reference analysis but never removes a missing candidate tail from
comparison. Missing B samples remain zero. There is no automatic phase alignment.
Applying timing preserves the nearest MIDI note plus cents. New imports default
key-off to selection end; it remains an explicit manual marker, not inferred from
the amplitude decay. Duration mismatches and unapplied candidates are visible.
Optional volume matching applies one global RMS gain to B for every comparison
view and clip. The displayed gain includes shared normalization. Common playback
attenuation preserves the A/B ratio while keeping both below 0.98 FS; it is
reported separately and does not alter plots, original PCM or hardware registers.
These comparisons support manual editing; automatic
inverse synthesis and spectral loss optimization are implemented separately in
[FITTING.md](FITTING.md).

Pressing the explicit analyze/apply button queues reference analysis. Completion
is reported independently of native preview rendering. The operation applies
the reliable detected pitch or manual override, the selected duration and manual
note-on/key-off. Pending application is invalidated by edits/import/session
changes. If pitch is unstable, the completed-result message explains what to do;
percussion and noise may need manual pitch or a tonal subregion. The application
never fabricates an approximate pitch.

Acceptance uses tones at multiple sample rates and the 20/4000 Hz boundaries,
alongside harmonic-rich input, silence/noise, coherent FFT amplitude,
channels/DC/selection, normalization/trimming/manual pitch, every supported WAV
encoding, invalid data and cancellation after work begins. All comparison views,
real register changes, short-session preservation, reference restoration,
drag/drop ordering and visible import errors are covered by GUI tests.
Device-free clip checks compare every converted sample/count and tagged
completion; CPAL is exercised separately by an opt-in test.
