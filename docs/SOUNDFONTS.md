# Soundfont samples

To import **SF2, SF3, SFZ and DLS** sample banks, choose **Import reference…**
or drop a bank into the editor window. A searchable catalog with one-based IDs
appears in the reference panel. Choose an entry to decode its sample for
analysis, A/B and automatic FM fitting. Its original decoded PCM plays once
through CPAL with **Play whole sample** and stops with **Stop** or Esc.
The A/B buttons play the prepared analysis interval.

Samples are extracted and auditioned without synthesizing MIDI presets. Preset
zones, modulators, envelopes, key ranges, articulations and bank loops are not
rendered; root note, fine tuning and explicit loops appear as metadata. Import
has no duration limit, while analysis accepts a region of up to 30 seconds.
The existing 128 MiB queued-clip memory budget also applies to whole-sample
playback, which reports a visible error if the budget is exceeded.

## Supported data

- **SF2:** PCM16 and split PCM24 (`smpl`/`sm24`), exact sample bounds excluding
  guard points, original rate/gain, reciprocal left/right stereo links. Each
  sample header retains its own ID, including duplicate names and stereo sides.
  Invalid links play as mono with a warning. ROM samples are listed but cannot
  play without their external ROM audio.
- **SF3:** individual mono Ogg/Vorbis streams, including banks mixing PCM and
  compressed samples. Both FluidSynth inclusive and MuseScore exclusive end
  conventions, zeroed terminal headers and omitted odd-length padding are
  accepted. Unsupported compression and damaged streams report errors.
- **SFZ:** external WAV and Ogg/Vorbis samples; paths with spaces or quotes,
  UTF-8, `default_path`, comments, macros, includes and global/master/group/region
  inheritance. Fixed `offset`/inclusive `end`, root key, tuning and explicit
  loop positions are read. Virtual oscillators, silent regions and missing
  files are marked unavailable. Embedded `<sample>` data is unsupported.
  Other playback opcodes do not modify the extracted PCM; this is not an SFZ
  instrument engine. Includes are bounded to 32 levels and 8 MiB of text.
- **DLS:** embedded `wvpl/wave` audio, names, `wsmp` root/fine tuning and first
  explicit loop. Audio codecs reuse the WAV reader (PCM/float, supported ADPCM,
  A-law/mu-law and complete Vorbis ACM). Instrument articulations are not applied.

Unpack compressed archives such as SFArk/SFPack before opening them. The bank
catalog is read first; PCM decoding then processes only the selected sample and
its valid stereo mate. Cancellation is checked between file chunks/packets.
Choosing another sample or opening a session invalidates earlier imports.

## Sessions and terminal

The bank path is saved in the session along with `reference_sample`: its
zero-based index, name and catalog/file identity. Relative bank paths are
resolved beside the session. Changes in file size/modification metadata or the
catalog require reopening and selecting again, preventing silent restoration
of a different entry. This checks accidental changes rather than cryptographic
content identity. Sessions predating the optional field remain compatible, and
importing a WAV clears it.

```powershell
.\target\release\discofm.exe samples "C:\Banks\NES.sf2"
.\target\release\discofm.exe gui --reference "C:\Banks\NES.sf2"
.\target\release\discofm.exe gui --reference "C:\Banks\NES.sf2" --sample 2
.\target\release\discofm.exe play-sample "C:\Banks\NES.sf2" --sample 2
.\target\release\discofm.exe extract-sample "C:\Banks\NES.sf2" --sample 2 --output sample.wav
.\target\release\discofm.exe fit --reference "C:\Banks\NES.sf2" --sample 2 --output fitted.json
```

Use **one based** IDs in the CLI. For banks with multiple samples,
`fit`/`compare --reference` require `--sample` or reuse a saved selection for the
same bank. A new float32 stereo WAV is written by `extract-sample`; banks and
existing outputs are never overwritten. The GUI can open a bank before an ID
is chosen.

## Verification and format references

Regression fixtures cover PCM16/24 signed extremes, offsets/guards, duplicate
names, stereo orientation, ROM, malformed bounds, both SF3 end conventions,
odd padding, mixed PCM/Vorbis, SFZ paths/includes/macros/inheritance/ranges,
DLS stereo/metadata, cancellation, replaced files, session restore and CLI
extraction/fitting. The tiny Vorbis fixture was generated with FFmpeg from an
80 ms 440 Hz sine at 22050 Hz (mono and dual-mono stereo fixtures); no bank audio is redistributed.

An external `The_Ultimate_Best_NES_8-Bit_Soundfont.sf2` was also opened and
all **29/29 samples** decoded successfully. This bank is external and not bundled.
Windows `gm.dls` also decoded **495/495 samples**. A separate PCM comparison
verified the first NES extraction exactly against the original SF2 words.
The sample-bank integration checks covered workspace tests, clippy/fmt,
physical CPAL clip completion/switch/Stop and the native GUI catalog.
An optimized build also passed NES catalog/playback, a 32-evaluation
fitting/session/comparison smoke run and GUI opening with `--sample 2`.
These are integration checks, not a fitting-quality benchmark. The source bank
SHA-256 remained unchanged.
Run the opt-in check against another installed bank:

```powershell
$env:DISCOFM_TEST_SOUNDFONT="C:\Banks\NES.sf2"
cargo test -p discofm-soundfont installed_soundfont -- --ignored --nocapture
```

Primary format references: [Creative SF2 2.04](https://www.synthfont.com/sfspec24.pdf),
[FluidSynth SF3](https://www.fluidsynth.org/wiki/SoundFont3Format/),
[MuseScore converter](https://github.com/musescore/sftools/blob/master/sfont.cpp),
[SFZ sample](https://sfzformat.com/opcodes/sample/),
[SFZ hierarchy](https://sfzformat.com/headers/master/),
[SFZ inclusive end/silent regions](https://sfzformat.com/opcodes/end/),
and [Windows DLS structures](https://github.com/microsoft/win32metadata/blob/main/generation/WinSDK/RecompiledIdlHeaders/shared/dls1.h).
