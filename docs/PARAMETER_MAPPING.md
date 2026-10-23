# Parameter mapping

Construction and JSON loading validate all stored values as raw register
encodings. Operators follow the datasheet's S1/S2/S3/S4 order, with register
offsets **0, 8, 4, 12**. Consecutive offsets would exchange S2 and S3. Bank 0 holds
channel 1–3 and bank 1 holds channel 4–6; the channel offset is `(number-1)%3`.
The editor's functional labels correspond to the register fields below. The
interface uses its own controls and diagrams rather than GENNY artwork.

| Label/field | Range | Register base | Bits and meaning |
|---|---|---|---|
| DETUNE / DT | 0–7 | 30h | 6:4; raw signed-detune encoding |
| FREQ / MUL | 0–15 | 30h | 3:0; zero means half multiplier |
| LEV / TL | 0–127 | 40h | 6:0; zero is least attenuation |
| KSR / RS | 0–3 | 50h | 7:6; frequency-dependent envelope rate scaling |
| ATK / AR | 0–31 | 50h | 4:0 |
| LFO / AM | boolean | 60h | 7; enables operator AM from shared LFO |
| DR1 / D1R | 0–31 | 60h | 4:0 |
| DR2 / D2R | 0–31 | 70h | 4:0; second decay, not sustain duration |
| SUS / SL | 0–15 | 80h | 7:4; attenuation threshold |
| RR | 0–15 | 80h | 3:0 |
| SSG | off or 8–15 | 90h | bit 3 enable; bits 2/1/0 inversion/alternate/hold |
| Algorithm | 0–7 | B0h | 2:0 |
| Feedback | 0–7 | B0h | 5:3 |
| Left/right | boolean each | B4h | 7/6 |
| AMS | 0–3 | B4h | 5:4 |
| FMS/PMS | 0–7 | B4h | 2:0 |
| Global LFO enable/rate | boolean / 0–7 | 22h bank 0 | 3 / 2:0 |
| CH 3 SPECIAL | off/on | 27h bank 0 | bit 6; timers/CSM off for note renderer |
| Channel frequency | FNUM 0–2047, BLOCK 0–7 | A4h then A0h | high latch before low commit |
| Key-on/off | four operator bits | 28h bank 0 | 7:4; channel codes 0,1,2,4,5,6 |
| Diagnostic PCM DAC | byte / boolean | 2Ah / 2Bh | separate 8-bit data / bit 7 enable |

The bases above incorporate the relevant per-operator and per-channel offsets.
Only bank 0 contains globals. Disabled SSG is stored as 0; enabled values use
08h–0Fh. Nuked-OPN2 generates hardware envelopes, including SSG and rate
scaling, without substituting a generic ADSR envelope.

## Operator identities and algorithms

The four-element operator array is indexed by `OperatorId::S1..S4`. In that
order, native pipeline slot offsets are **0, 12, 6, 18**. Both ascending register
addresses and ascending native slot groups traverse S1/S3/S2/S4, so neither
defines the array order. Raw values 0–7 are validated by `Algorithm`, which
also supplies immutable routing/carrier metadata. Session encoding remains
unchanged when `display_number()` returns 1–8. Feedback always feeds back to S1.

| Raw algorithm | Modulation edges | Carriers |
|---:|---|---|
| 0 | S1 → S2 → S3 → S4 | S4 |
| 1 | S1 → S3; S2 → S3; S3 → S4 | S4 |
| 2 | S1 → S4; S2 → S3; S3 → S4 | S4 |
| 3 | S1 → S2; S2 → S4; S3 → S4 | S4 |
| 4 | S1 → S2; S3 → S4 | S2, S4 |
| 5 | S1 → S2; S1 → S3; S1 → S4 | S2, S3, S4 |
| 6 | S1 → S2 | S2, S3, S4 |
| 7 | No modulation edges | S1, S2, S3, S4 |

The pinned core's `fm_algorithm` matrix and `OPN2_FMPrepare` source latches define
these edges. Behavioral tests check carriers and causal paths; in particular,
algorithms 0–3 must retain the correct S2/S3 identities. Refer to
[the pinned implementation](https://github.com/nukeykt/Nuked-OPN2/blob/335747d78cb0abbc3b55b004e62dad9763140115/ym3438.c).

Raw DT values map to signed magnitudes `0,+1,+2,+3,0,-1,-2,-3`. DT=4's negative
zero remains 4 when serialized. MUL=0 displays as 1/2 and remains zero in the
register model. RS=0 still includes the chip's base key-code scaling; it does not
disable all frequency dependence. RR=0 is a slow finite release rate.

## SSG-EG and LFO

`SsgEg` exposes enabled/inverted/alternate/hold bits and validated shape/flag
constructors. Off is canonical zero; enabled encodings are 8–15. With AR=31,
the following describes amplitude direction, not linear amplitude ramps:

| Encoding | Envelope behavior |
|---:|---|
| 8 | Repeated falling envelope |
| 9 | Falls, then holds quiet |
| A | Alternates falling/rising |
| B | Falls, then jumps high and holds |
| C | Repeated rising envelope |
| D | Rises, then holds high |
| E | Alternates rising/falling |
| F | Rises, then jumps quiet and holds |

Eight rate indices belong to the single global hardware LFO. Operator AM gates
amplitude modulation, while channel AMS sets its depth and channel FMS sets
phase modulation depth. Zero sensitivity has no effect. Disabling the LFO stops
movement, though AM attenuation may remain at the core's fixed phase. Tests
compare modulation behavior and rate ordering; this document asserts no nominal
LFO Hz table.

## Special channel and audition

Channel-3 special mode is restricted to hardware channel 3. S1 uses ADh/A9h,
S2 AEh/AAh, S3 ACh/A8h, and S4 the usual A6h/A2h. Special frequencies are
absolute hardware values, so only S4 follows the selected audition note. The
four operators retain the channel's single algorithm and output route.

`audition.velocity_tl_depth` is software-only, per operator. At note-on,
`TL = min(127, TL + round((127-velocity)*depth/127))`. It does not introduce a
velocity register, a separate LFO, or any MIDI device management. Profile,
pitch selection, duration, sample rate and reference path are also software
settings. Fitting metadata and parameter locks are stored separately from the
hardware register model; see [FITTING.md](FITTING.md).

Register reference: [YM2612 registers](https://www.plutiedev.com/ym2612-registers),
cross-checked with the vendored decoder. The core tracing above supplies algorithm
metadata, and synthesis always executes the core's own algorithms.
