# Example sessions

All sessions use DiscoFM's version 1 JSON format. The application version and
the session format version are independent.

| Session | Contents |
|---|---|
| `basic-session.json` | Default instrument and audition settings; original format compatibility example |
| `channel3-session.json` | Independent channel 3 operator frequencies |
| `fitting-initial-session.json` | Initial patch for the synthetic fitting example |
| `fitting-target-session.json` | Target patch for rendering a synthetic fitting reference |
| `bass-efem-fitted-session.json` | Bass patch and archived fitting results |
| `mandolynn-fitted-session.json` | Mandolin patch and archived fitting results |
| `steeldrum-fitted-session.json` | SteelDrum patch and archived fitting results |

## Synthetic fitting example

Run these commands from the repository root. Output files must not already exist.

```powershell
cargo run -- render --session examples/fitting-target-session.json --output target/fitting-reference.wav
cargo run -- fit --reference target/fitting-reference.wav --session examples/fitting-initial-session.json --output target/fitted.json --pitch 440 --key-off 0.18 --end 0.3
```

## External references

The fitted sessions contain patch data and search history, without audio. Their
reference paths resolve relative to the session file:

- `references/BASS_EfEm_C2.wav`
- `references/STR_MandoLynn_B2.wav`
- `references/PERC_SteelDrum_C5.wav`

These FL Studio samples are copyrighted and are not distributed in this
repository. To reproduce the archived comparisons, provide the corresponding
files from a licensed installation in `examples/references/`, or open a session
and select an available reference in the editor. That local directory is ignored
by Git. A missing reference does not remove the stored instrument or history.

Archived loss versions retain their original weights and scores. Compare scores
only within the same objective version; use rendered-audio comparisons when
evaluating patches from different objective versions.
