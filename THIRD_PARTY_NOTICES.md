# Third-party software and licenses

DiscoFM's application code is licensed under **GPL-3.0-only**. Third-party code,
fonts and their notices retain the licenses identified below; the root license
does not replace them. The inventory in [docs/DEPENDENCIES.md](docs/DEPENDENCIES.md)
records the resolved Windows dependency graph and its review scope.

## Nuked-OPN2

- Author: Alexey Khokholov (Nuke.YKT).
- Source: <https://github.com/nukeykt/Nuked-OPN2>.
- Revision: `335747d78cb0abbc3b55b004e62dad9763140115`.
- License: **LGPL-2.1-or-later**.
- Source, copyright headers, upstream README and full license:
  [crates/discofm-engine/vendor/nuked-opn2](crates/discofm-engine/vendor/nuked-opn2).

The checked-in upstream files are unmodified. `crates/discofm-engine/build.rs`
generates a production copy with instance-local chip-profile state and renamed
`DFM_OPN2_` symbols. Generated files identify that modification and its date,
while preserving the upstream notices. A separate build of the original core
serves the pin-comparison driver. Wrappers and diagnostics are in
`crates/discofm-engine/native/`.

LGPL-2.1 permits combination with GPLv3 code; see the
[GNU license compatibility guidance](https://www.gnu.org/licenses/gpl-faq.html#AllCompatibility).
The library is statically linked. Distributing a binary requires the applicable
LGPL notices and license plus corresponding source and material sufficient to
modify/rebuild/relink the library with the application. The corresponding source
must include the build-time adaptation and wrappers, not just the upstream files.

## ymfm

- Author: Aaron Giles.
- Source: <https://github.com/aaronsgiles/ymfm>.
- Revision: `81aec25ccbb98f4873a255f7551ac4dadac59b4a`.
- License: **BSD-3-Clause**.
- Unmodified source/header files and full license:
  [crates/discofm-engine/vendor/ymfm](crates/discofm-engine/vendor/ymfm).

The C++14 wrapper is separate in
`crates/discofm-engine/native/ymfm_reference.cpp`. ymfm supplies independent
comparisons and fitting-proposal screening; final candidate scores, playback
and production synthesis use Nuked-OPN2. Preserve the BSD copyright notice,
conditions and disclaimer in source distributions and binary accompanying
materials. The license's non-endorsement condition remains applicable. BSD's
GPL compatibility is described in the
[GNU license list](https://www.gnu.org/licenses/license-list.html#ModifiedBSD).

## Symphonia

Symphonia 0.6.1 and the selected ADPCM, PCM and Vorbis codecs, Ogg/RIFF readers,
metadata, common and core packages use **MPL-2.0**. Their upstream source is at
<https://github.com/pdeljanov/Symphonia>; exact package versions are in Cargo.lock.
The downloaded source headers examined for the Windows dependency graph have no
“Incompatible With Secondary Licenses” notice.

MPL-2.0 section 3.3 permits eligible MPL files in a GPL-licensed larger work.
Preserve MPL source notices and make the covered source available under the
applicable MPL/GPL terms when distributing the combination. Individual MPL files
are not silently replaced by a GPL-only declaration. See
[Mozilla's combination guidelines](https://www.mozilla.org/en-US/MPL/2.0/combining-mpl-and-gpl/).

## Bundled fonts

The editor enables egui's `default_fonts` feature. `epaint_default_fonts` 0.36.2
embeds these assets, which retain their own licenses:

| Font | Copyright / source | License |
|---|---|---|
| Hack Regular | Source Foundry Authors; Bitstream Vera and public-domain DejaVu contributions | MIT and the retained Bitstream Vera terms |
| Noto Emoji Regular | Google Inc. | SIL Open Font License 1.1 |
| Ubuntu Light | Canonical Ltd. | Ubuntu Font Licence 1.0 |
| emoji-icon-font | John Slegers | MIT |

The full upstream license texts and the font copyright/license records are in
[LICENSES/epaint_default_fonts](LICENSES/epaint_default_fonts). Include these
notices with editor binary distributions. The fonts remain separate licensed
assets; they are not relicensed under GPLv3. Font bundling conditions are
explained by the [OFL FAQ](https://openfontlicense.org/ofl-faq/) and
[Ubuntu Font Licence](https://canonical.com/legal/font-licence).

## Rust dependencies

The direct dependencies below are resolved from Cargo.lock. The complete
Windows dependency inventory, including transitive/build/test packages, is in
[docs/DEPENDENCIES.md](docs/DEPENDENCIES.md). No dependency versions were changed
by the application version update.

For dual-licensed packages, a GPLv3-compatible option such as MIT or Apache-2.0
can be selected. Packages declaring `AND` terms require all of those notices;
font and Unicode-data licenses remain applicable alongside the code license.
In particular, `self_cell` can be used under Apache-2.0 rather than its
GPL-2.0-only alternative. Permissive-code compatibility is discussed in the
[GNU compatibility overview](https://www.gnu.org/licenses/license-compatibility.html).

| Direct dependency | Version | Upstream license expression |
|---|---|---|
| [arboard](https://crates.io/crates/arboard/3.6.1) | 3.6.1 | MIT OR Apache-2.0 |
| [bytemuck](https://crates.io/crates/bytemuck/1.25.2) | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| [cc](https://crates.io/crates/cc/1.7.0) | 1.7.0 | MIT OR Apache-2.0 |
| [clap](https://crates.io/crates/clap/4.6.7) | 4.6.7 | MIT OR Apache-2.0 |
| [cpal](https://crates.io/crates/cpal/0.18.2) | 0.18.2 | Apache-2.0 |
| [crossterm](https://crates.io/crates/crossterm/0.29.0) | 0.29.0 | MIT |
| [eframe](https://crates.io/crates/eframe/0.36.2) | 0.36.2 | MIT OR Apache-2.0 |
| [hound](https://crates.io/crates/hound/3.5.1) | 3.5.1 | Apache-2.0 |
| [image](https://crates.io/crates/image/0.25.10) | 0.25.10 | MIT OR Apache-2.0 |
| [midir](https://crates.io/crates/midir/0.11.1) | 0.11.1 | MIT |
| [pollster](https://crates.io/crates/pollster/0.4.0) | 0.4.0 | Apache-2.0/MIT |
| [rayon](https://crates.io/crates/rayon/1.12.0) | 1.12.0 | MIT OR Apache-2.0 |
| [realfft](https://crates.io/crates/realfft/3.5.0) | 3.5.0 | MIT |
| [rfd](https://crates.io/crates/rfd/0.17.2) | 0.17.2 | MIT |
| [rtrb](https://crates.io/crates/rtrb/0.4.0) | 0.4.0 | MIT OR Apache-2.0 |
| [rustfft](https://crates.io/crates/rustfft/6.4.1) | 6.4.1 | MIT OR Apache-2.0 |
| [serde](https://crates.io/crates/serde/1.0.229) | 1.0.229 | MIT OR Apache-2.0 |
| [serde_json](https://crates.io/crates/serde_json/1.0.152) | 1.0.152 | MIT OR Apache-2.0 |
| [serde_path_to_error](https://crates.io/crates/serde_path_to_error/0.1.20) | 0.1.20 | MIT OR Apache-2.0 |
| [symphonia](https://crates.io/crates/symphonia/0.6.1) | 0.6.1 | MPL-2.0 |
| [thiserror](https://crates.io/crates/thiserror/2.0.21) | 2.0.21 | MIT OR Apache-2.0 |
| [wgpu](https://crates.io/crates/wgpu/30.0.1) | 30.0.1 | MIT OR Apache-2.0 |

## Source and binary distribution

For a source release, preserve LICENSE.txt, this file, LICENSES/, all vendored
licenses/copyright headers and the pinned upstream source. Keep Cargo.lock and
build.rs so the selected dependency versions and generated native adaptation
can be reproduced. [README.md](README.md) gives the supported build commands.

For a binary release, also provide the complete corresponding source required
by GPLv3 and the linked copyleft components through an applicable license
compliant distribution method. Include required Rust dependency copyright,
license and NOTICE files, including transitive dependencies, and the font
notices above. A short dependency table is not a substitute for those texts.
Provide the sources/build material needed to rebuild and relink the statically
linked library, along with installation information where the licenses require
it. An executable alone is not a complete distribution package.

Sample banks and the external FL Studio references used by optional tests are
not bundled. Synthetic test fixtures have their origin documented beside the
fixtures; no license to redistribute external recordings is implied.
