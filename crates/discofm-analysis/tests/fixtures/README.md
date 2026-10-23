FFmpeg's `sine=frequency=330` source was used to generate these synthetic audio
fixtures for DiscoFM. No third-party recordings are included.

- `sine.ogg`: 44.1 kHz, 0.2 s, Vorbis q4; L=sine, R=0.5*sine.
- `ima.wav`: 22.05 kHz, 0.12 s, Microsoft IMA ADPCM, mono.

The test constructs the Vorbis ACM RIFF wrapper with odd-sized chunks, trailing
metadata, a deliberately non-PCM block size and a fact count.
