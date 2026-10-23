# Dependency license inventory

This inventory records the 218 third-party Rust packages resolved for
`x86_64-pc-windows-msvc` by the checked-in Cargo.lock on 2026-10-22. It includes
normal, build and test dependencies. All 12 workspace packages use
`GPL-3.0-only` and version 1.0.0; they are excluded from the table.

The license expressions come from the exact downloaded package manifests.
They describe each package's terms, without replacing its copyright notices or
license files. `OR` permits a choice; `AND` requires every listed condition.
Legacy `MIT/Apache-2.0` and `Apache-2.0/MIT` labels denote upstream dual licensing.

No incompatible code-license requirement was identified in this Windows graph.
MIT/Apache-2.0 and the other permissive code licenses can be combined with GPLv3;
`self_cell` can use its Apache-2.0 option rather than GPL-2.0-only. Symphonia's
MPL-2.0 source headers contain no secondary-license incompatibility notice.
Fonts retain separate asset licenses and are not relicensed under GPLv3.
See [third-party notices](../THIRD_PARTY_NOTICES.md) for the reviewed native
sources, bundled fonts and distribution requirements.

This target-specific inventory does not validate dependency sets selected by
other operating systems, architectures or feature combinations. Refresh it from
Cargo metadata when those build configurations or Cargo.lock change:

```powershell
cargo metadata --format-version 1 --filter-platform x86_64-pc-windows-msvc --locked
```

| Package | Version | Upstream license expression |
|---|---|---|
| [accesskit](https://crates.io/crates/accesskit/0.24.1) | 0.24.1 | MIT OR Apache-2.0 |
| [adler2](https://crates.io/crates/adler2/2.0.1) | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| [ahash](https://crates.io/crates/ahash/0.8.12) | 0.8.12 | MIT OR Apache-2.0 |
| [allocator-api2](https://crates.io/crates/allocator-api2/0.2.21) | 0.2.21 | MIT OR Apache-2.0 |
| [anstream](https://crates.io/crates/anstream/1.0.0) | 1.0.0 | MIT OR Apache-2.0 |
| [anstyle](https://crates.io/crates/anstyle/1.0.14) | 1.0.14 | MIT OR Apache-2.0 |
| [anstyle-parse](https://crates.io/crates/anstyle-parse/1.0.0) | 1.0.0 | MIT OR Apache-2.0 |
| [anstyle-query](https://crates.io/crates/anstyle-query/1.1.5) | 1.1.5 | MIT OR Apache-2.0 |
| [anstyle-wincon](https://crates.io/crates/anstyle-wincon/3.0.11) | 3.0.11 | MIT OR Apache-2.0 |
| [arboard](https://crates.io/crates/arboard/3.6.1) | 3.6.1 | MIT OR Apache-2.0 |
| [arrayvec](https://crates.io/crates/arrayvec/0.7.8) | 0.7.8 | MIT OR Apache-2.0 |
| [ash](https://crates.io/crates/ash/0.38.0+1.3.281) | 0.38.0+1.3.281 | MIT OR Apache-2.0 |
| [autocfg](https://crates.io/crates/autocfg/1.5.1) | 1.5.1 | Apache-2.0 OR MIT |
| [bit-set](https://crates.io/crates/bit-set/0.10.0) | 0.10.0 | Apache-2.0 OR MIT |
| [bit-vec](https://crates.io/crates/bit-vec/0.9.1) | 0.9.1 | Apache-2.0 OR MIT |
| [bitflags](https://crates.io/crates/bitflags/2.13.2) | 2.13.2 | MIT OR Apache-2.0 |
| [bytemuck](https://crates.io/crates/bytemuck/1.25.2) | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| [bytemuck_derive](https://crates.io/crates/bytemuck_derive/1.12.1) | 1.12.1 | Zlib OR Apache-2.0 OR MIT |
| [byteorder-lite](https://crates.io/crates/byteorder-lite/0.1.0) | 0.1.0 | Unlicense OR MIT |
| [cc](https://crates.io/crates/cc/1.7.0) | 1.7.0 | MIT OR Apache-2.0 |
| [cfg_aliases](https://crates.io/crates/cfg_aliases/0.2.2) | 0.2.2 | MIT |
| [cfg-if](https://crates.io/crates/cfg-if/1.0.5) | 1.0.5 | MIT OR Apache-2.0 |
| [clap](https://crates.io/crates/clap/4.6.7) | 4.6.7 | MIT OR Apache-2.0 |
| [clap_builder](https://crates.io/crates/clap_builder/4.6.7) | 4.6.7 | MIT OR Apache-2.0 |
| [clap_derive](https://crates.io/crates/clap_derive/4.6.7) | 4.6.7 | MIT OR Apache-2.0 |
| [clap_lex](https://crates.io/crates/clap_lex/1.1.1) | 1.1.1 | MIT OR Apache-2.0 |
| [clipboard-win](https://crates.io/crates/clipboard-win/5.4.1) | 5.4.1 | BSL-1.0 |
| [codespan-reporting](https://crates.io/crates/codespan-reporting/0.13.1) | 0.13.1 | Apache-2.0 |
| [color](https://crates.io/crates/color/0.3.3) | 0.3.3 | Apache-2.0 OR MIT |
| [colorchoice](https://crates.io/crates/colorchoice/1.0.5) | 1.0.5 | MIT OR Apache-2.0 |
| [convert_case](https://crates.io/crates/convert_case/0.10.0) | 0.10.0 | MIT |
| [cpal](https://crates.io/crates/cpal/0.18.2) | 0.18.2 | Apache-2.0 |
| [crc32fast](https://crates.io/crates/crc32fast/1.5.2) | 1.5.2 | MIT OR Apache-2.0 |
| [crossbeam-deque](https://crates.io/crates/crossbeam-deque/0.8.8) | 0.8.8 | MIT OR Apache-2.0 |
| [crossbeam-epoch](https://crates.io/crates/crossbeam-epoch/0.9.21) | 0.9.21 | MIT OR Apache-2.0 |
| [crossbeam-utils](https://crates.io/crates/crossbeam-utils/0.8.23) | 0.8.23 | MIT OR Apache-2.0 |
| [crossterm](https://crates.io/crates/crossterm/0.29.0) | 0.29.0 | MIT |
| [crossterm_winapi](https://crates.io/crates/crossterm_winapi/0.9.1) | 0.9.1 | MIT |
| [cursor-icon](https://crates.io/crates/cursor-icon/1.2.0) | 1.2.0 | MIT OR Apache-2.0 OR Zlib |
| [dasp_sample](https://crates.io/crates/dasp_sample/0.11.0) | 0.11.0 | MIT OR Apache-2.0 |
| [derive_more](https://crates.io/crates/derive_more/2.1.1) | 2.1.1 | MIT |
| [derive_more-impl](https://crates.io/crates/derive_more-impl/2.1.1) | 2.1.1 | MIT |
| [document-features](https://crates.io/crates/document-features/0.2.12) | 0.2.12 | MIT OR Apache-2.0 |
| [dpi](https://crates.io/crates/dpi/0.1.2) | 0.1.2 | Apache-2.0 AND MIT |
| [ecolor](https://crates.io/crates/ecolor/0.36.2) | 0.36.2 | MIT OR Apache-2.0 |
| [eframe](https://crates.io/crates/eframe/0.36.2) | 0.36.2 | MIT OR Apache-2.0 |
| [egui](https://crates.io/crates/egui/0.36.2) | 0.36.2 | MIT OR Apache-2.0 |
| [egui_glow](https://crates.io/crates/egui_glow/0.36.2) | 0.36.2 | MIT OR Apache-2.0 |
| [egui-wgpu](https://crates.io/crates/egui-wgpu/0.36.2) | 0.36.2 | MIT OR Apache-2.0 |
| [egui-winit](https://crates.io/crates/egui-winit/0.36.2) | 0.36.2 | MIT OR Apache-2.0 |
| [either](https://crates.io/crates/either/1.19.0) | 1.19.0 | MIT OR Apache-2.0 |
| [emath](https://crates.io/crates/emath/0.36.2) | 0.36.2 | MIT OR Apache-2.0 |
| [epaint](https://crates.io/crates/epaint/0.36.2) | 0.36.2 | MIT OR Apache-2.0 |
| [epaint_default_fonts](https://crates.io/crates/epaint_default_fonts/0.36.2) | 0.36.2 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 |
| [equivalent](https://crates.io/crates/equivalent/1.0.2) | 1.0.2 | Apache-2.0 OR MIT |
| [error-code](https://crates.io/crates/error-code/3.4.0) | 3.4.0 | BSL-1.0 |
| [euclid](https://crates.io/crates/euclid/0.22.14) | 0.22.14 | MIT OR Apache-2.0 |
| [extended](https://crates.io/crates/extended/0.1.0) | 0.1.0 | MIT |
| [fax](https://crates.io/crates/fax/0.2.7) | 0.2.7 | MIT |
| [fdeflate](https://crates.io/crates/fdeflate/0.3.7) | 0.3.7 | MIT OR Apache-2.0 |
| [fearless_simd](https://crates.io/crates/fearless_simd/0.4.1) | 0.4.1 | Apache-2.0 OR MIT |
| [find-msvc-tools](https://crates.io/crates/find-msvc-tools/0.1.14) | 0.1.14 | MIT OR Apache-2.0 |
| [flate2](https://crates.io/crates/flate2/1.1.10) | 1.1.10 | MIT OR Apache-2.0 |
| [foldhash](https://crates.io/crates/foldhash/0.2.0) | 0.2.0 | Zlib |
| [font-types](https://crates.io/crates/font-types/0.12.6) | 0.12.6 | MIT OR Apache-2.0 |
| [getrandom](https://crates.io/crates/getrandom/0.3.4) | 0.3.4 | MIT OR Apache-2.0 |
| [getrandom](https://crates.io/crates/getrandom/0.4.3) | 0.4.3 | MIT OR Apache-2.0 |
| [gl_generator](https://crates.io/crates/gl_generator/0.14.0) | 0.14.0 | Apache-2.0 |
| [glifo](https://crates.io/crates/glifo/0.2.0) | 0.2.0 | Apache-2.0 OR MIT |
| [glow](https://crates.io/crates/glow/0.17.0) | 0.17.0 | MIT OR Apache-2.0 OR Zlib |
| [glutin](https://crates.io/crates/glutin/0.32.3) | 0.32.3 | Apache-2.0 |
| [glutin_egl_sys](https://crates.io/crates/glutin_egl_sys/0.7.1) | 0.7.1 | Apache-2.0 |
| [glutin_wgl_sys](https://crates.io/crates/glutin_wgl_sys/0.6.1) | 0.6.1 | Apache-2.0 |
| [glutin-winit](https://crates.io/crates/glutin-winit/0.5.0) | 0.5.0 | MIT |
| [gpu-allocator](https://crates.io/crates/gpu-allocator/0.28.0) | 0.28.0 | MIT OR Apache-2.0 |
| [guillotiere](https://crates.io/crates/guillotiere/0.7.0) | 0.7.0 | MIT/Apache-2.0 |
| [half](https://crates.io/crates/half/2.7.1) | 2.7.1 | MIT OR Apache-2.0 |
| [harfrust](https://crates.io/crates/harfrust/0.12.0) | 0.12.0 | MIT |
| [hashbrown](https://crates.io/crates/hashbrown/0.16.1) | 0.16.1 | MIT OR Apache-2.0 |
| [hashbrown](https://crates.io/crates/hashbrown/0.17.1) | 0.17.1 | MIT OR Apache-2.0 |
| [heck](https://crates.io/crates/heck/0.5.0) | 0.5.0 | MIT OR Apache-2.0 |
| [hound](https://crates.io/crates/hound/3.5.1) | 3.5.1 | Apache-2.0 |
| [image](https://crates.io/crates/image/0.25.10) | 0.25.10 | MIT OR Apache-2.0 |
| [indexmap](https://crates.io/crates/indexmap/2.14.2) | 2.14.2 | Apache-2.0 OR MIT |
| [is_terminal_polyfill](https://crates.io/crates/is_terminal_polyfill/1.70.2) | 1.70.2 | MIT OR Apache-2.0 |
| [itertools](https://crates.io/crates/itertools/0.15.0) | 0.15.0 | MIT OR Apache-2.0 |
| [itoa](https://crates.io/crates/itoa/1.0.18) | 1.0.18 | MIT OR Apache-2.0 |
| [jobserver](https://crates.io/crates/jobserver/0.1.35) | 0.1.35 | MIT OR Apache-2.0 |
| [khronos_api](https://crates.io/crates/khronos_api/3.1.0) | 3.1.0 | Apache-2.0 |
| [kurbo](https://crates.io/crates/kurbo/0.13.1) | 0.13.1 | Apache-2.0 OR MIT |
| [lazy_static](https://crates.io/crates/lazy_static/1.5.1) | 1.5.1 | MIT OR Apache-2.0 |
| [libloading](https://crates.io/crates/libloading/0.8.9) | 0.8.9 | ISC |
| [libm](https://crates.io/crates/libm/0.2.16) | 0.2.16 | MIT |
| [linebender_resource_handle](https://crates.io/crates/linebender_resource_handle/0.1.1) | 0.1.1 | Apache-2.0 OR MIT |
| [litrs](https://crates.io/crates/litrs/1.0.0) | 1.0.0 | MIT OR Apache-2.0 |
| [lock_api](https://crates.io/crates/lock_api/0.4.14) | 0.4.14 | MIT OR Apache-2.0 |
| [log](https://crates.io/crates/log/0.4.34) | 0.4.34 | MIT OR Apache-2.0 |
| [memchr](https://crates.io/crates/memchr/2.8.3) | 2.8.3 | Unlicense OR MIT |
| [midir](https://crates.io/crates/midir/0.11.1) | 0.11.1 | MIT |
| [miniz_oxide](https://crates.io/crates/miniz_oxide/0.8.9) | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| [miniz_oxide](https://crates.io/crates/miniz_oxide/0.9.1) | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| [moxcms](https://crates.io/crates/moxcms/0.8.1) | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| [naga](https://crates.io/crates/naga/30.0.1) | 30.0.1 | MIT OR Apache-2.0 |
| [naga-types](https://crates.io/crates/naga-types/30.0.1) | 30.0.1 | MIT OR Apache-2.0 |
| [nohash-hasher](https://crates.io/crates/nohash-hasher/0.2.0) | 0.2.0 | Apache-2.0 OR MIT |
| [num-complex](https://crates.io/crates/num-complex/0.4.6) | 0.4.6 | MIT OR Apache-2.0 |
| [num-integer](https://crates.io/crates/num-integer/0.1.47) | 0.1.47 | MIT OR Apache-2.0 |
| [num-traits](https://crates.io/crates/num-traits/0.2.19) | 0.2.19 | MIT OR Apache-2.0 |
| [once_cell](https://crates.io/crates/once_cell/1.21.4) | 1.21.4 | MIT OR Apache-2.0 |
| [once_cell_polyfill](https://crates.io/crates/once_cell_polyfill/1.70.2) | 1.70.2 | MIT OR Apache-2.0 |
| [ordered-float](https://crates.io/crates/ordered-float/5.5.0) | 5.5.0 | MIT |
| [parking_lot](https://crates.io/crates/parking_lot/0.12.5) | 0.12.5 | MIT OR Apache-2.0 |
| [parking_lot_core](https://crates.io/crates/parking_lot_core/0.9.12) | 0.9.12 | MIT OR Apache-2.0 |
| [peniko](https://crates.io/crates/peniko/0.6.1) | 0.6.1 | Apache-2.0 OR MIT |
| [pin-project-lite](https://crates.io/crates/pin-project-lite/0.2.17) | 0.2.17 | Apache-2.0 OR MIT |
| [png](https://crates.io/crates/png/0.18.1) | 0.18.1 | MIT OR Apache-2.0 |
| [pollster](https://crates.io/crates/pollster/0.4.0) | 0.4.0 | Apache-2.0/MIT |
| [polycool](https://crates.io/crates/polycool/0.4.0) | 0.4.0 | MIT OR Apache-2.0 |
| [presser](https://crates.io/crates/presser/0.3.1) | 0.3.1 | MIT OR Apache-2.0 |
| [primal-check](https://crates.io/crates/primal-check/0.3.4) | 0.3.4 | MIT OR Apache-2.0 |
| [proc-macro2](https://crates.io/crates/proc-macro2/1.0.107) | 1.0.107 | MIT OR Apache-2.0 |
| [profiling](https://crates.io/crates/profiling/1.0.18) | 1.0.18 | MIT OR Apache-2.0 |
| [pxfm](https://crates.io/crates/pxfm/0.1.30) | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| [quick-error](https://crates.io/crates/quick-error/2.0.1) | 2.0.1 | MIT/Apache-2.0 |
| [quote](https://crates.io/crates/quote/1.0.47) | 1.0.47 | MIT OR Apache-2.0 |
| [range-alloc](https://crates.io/crates/range-alloc/0.1.5) | 0.1.5 | MIT OR Apache-2.0 |
| [raw-window-handle](https://crates.io/crates/raw-window-handle/0.6.2) | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| [rayon](https://crates.io/crates/rayon/1.12.0) | 1.12.0 | MIT OR Apache-2.0 |
| [rayon-core](https://crates.io/crates/rayon-core/1.13.0) | 1.13.0 | MIT OR Apache-2.0 |
| [read-fonts](https://crates.io/crates/read-fonts/0.41.0) | 0.41.0 | MIT OR Apache-2.0 |
| [realfft](https://crates.io/crates/realfft/3.5.0) | 3.5.0 | MIT |
| [regex-lite](https://crates.io/crates/regex-lite/0.1.9) | 0.1.9 | MIT OR Apache-2.0 |
| [renderdoc-sys](https://crates.io/crates/renderdoc-sys/1.1.0) | 1.1.0 | MIT OR Apache-2.0 |
| [rfd](https://crates.io/crates/rfd/0.17.2) | 0.17.2 | MIT |
| [rtrb](https://crates.io/crates/rtrb/0.4.0) | 0.4.0 | MIT OR Apache-2.0 |
| [rustc_version](https://crates.io/crates/rustc_version/0.4.1) | 0.4.1 | MIT OR Apache-2.0 |
| [rustc-hash](https://crates.io/crates/rustc-hash/1.1.0) | 1.1.0 | Apache-2.0/MIT |
| [rustc-hash](https://crates.io/crates/rustc-hash/2.1.3) | 2.1.3 | Apache-2.0 OR MIT |
| [rustfft](https://crates.io/crates/rustfft/6.4.1) | 6.4.1 | MIT OR Apache-2.0 |
| [scopeguard](https://crates.io/crates/scopeguard/1.2.0) | 1.2.0 | MIT OR Apache-2.0 |
| [self_cell](https://crates.io/crates/self_cell/1.3.0) | 1.3.0 | Apache-2.0 OR GPL-2.0-only |
| [semver](https://crates.io/crates/semver/1.0.28) | 1.0.28 | MIT OR Apache-2.0 |
| [serde](https://crates.io/crates/serde/1.0.229) | 1.0.229 | MIT OR Apache-2.0 |
| [serde_core](https://crates.io/crates/serde_core/1.0.229) | 1.0.229 | MIT OR Apache-2.0 |
| [serde_derive](https://crates.io/crates/serde_derive/1.0.229) | 1.0.229 | MIT OR Apache-2.0 |
| [serde_json](https://crates.io/crates/serde_json/1.0.152) | 1.0.152 | MIT OR Apache-2.0 |
| [serde_path_to_error](https://crates.io/crates/serde_path_to_error/0.1.20) | 0.1.20 | MIT OR Apache-2.0 |
| [shlex](https://crates.io/crates/shlex/2.0.1) | 2.0.1 | MIT OR Apache-2.0 |
| [simd-adler32](https://crates.io/crates/simd-adler32/0.3.10) | 0.3.10 | MIT |
| [skrifa](https://crates.io/crates/skrifa/0.44.0) | 0.44.0 | MIT OR Apache-2.0 |
| [smallvec](https://crates.io/crates/smallvec/1.16.3) | 1.16.3 | MIT OR Apache-2.0 |
| [smol_str](https://crates.io/crates/smol_str/0.2.2) | 0.2.2 | MIT OR Apache-2.0 |
| [spirv](https://crates.io/crates/spirv/0.4.0+sdk-1.4.341.0) | 0.4.0+sdk-1.4.341.0 | Apache-2.0 |
| [static_assertions](https://crates.io/crates/static_assertions/1.1.0) | 1.1.0 | MIT OR Apache-2.0 |
| [strength_reduce](https://crates.io/crates/strength_reduce/0.2.4) | 0.2.4 | MIT OR Apache-2.0 |
| [strsim](https://crates.io/crates/strsim/0.11.1) | 0.11.1 | MIT |
| [symphonia](https://crates.io/crates/symphonia/0.6.1) | 0.6.1 | MPL-2.0 |
| [symphonia-codec-adpcm](https://crates.io/crates/symphonia-codec-adpcm/0.6.1) | 0.6.1 | MPL-2.0 |
| [symphonia-codec-pcm](https://crates.io/crates/symphonia-codec-pcm/0.6.1) | 0.6.1 | MPL-2.0 |
| [symphonia-codec-vorbis](https://crates.io/crates/symphonia-codec-vorbis/0.6.1) | 0.6.1 | MPL-2.0 |
| [symphonia-common](https://crates.io/crates/symphonia-common/0.6.1) | 0.6.1 | MPL-2.0 |
| [symphonia-core](https://crates.io/crates/symphonia-core/0.6.1) | 0.6.1 | MPL-2.0 |
| [symphonia-format-ogg](https://crates.io/crates/symphonia-format-ogg/0.6.1) | 0.6.1 | MPL-2.0 |
| [symphonia-format-riff](https://crates.io/crates/symphonia-format-riff/0.6.1) | 0.6.1 | MPL-2.0 |
| [symphonia-metadata](https://crates.io/crates/symphonia-metadata/0.6.1) | 0.6.1 | MPL-2.0 |
| [syn](https://crates.io/crates/syn/2.0.119) | 2.0.119 | MIT OR Apache-2.0 |
| [syn](https://crates.io/crates/syn/3.0.7) | 3.0.7 | MIT OR Apache-2.0 |
| [thiserror](https://crates.io/crates/thiserror/2.0.21) | 2.0.21 | MIT OR Apache-2.0 |
| [thiserror-impl](https://crates.io/crates/thiserror-impl/2.0.21) | 2.0.21 | MIT OR Apache-2.0 |
| [tiff](https://crates.io/crates/tiff/0.11.3) | 0.11.3 | MIT |
| [tracing](https://crates.io/crates/tracing/0.1.44) | 0.1.44 | MIT |
| [tracing-core](https://crates.io/crates/tracing-core/0.1.36) | 0.1.36 | MIT |
| [transpose](https://crates.io/crates/transpose/0.2.3) | 0.2.3 | MIT OR Apache-2.0 |
| [type-map](https://crates.io/crates/type-map/0.5.1) | 0.5.1 | MIT/Apache-2.0 |
| [unicode-general-category](https://crates.io/crates/unicode-general-category/1.1.0) | 1.1.0 | Apache-2.0 |
| [unicode-ident](https://crates.io/crates/unicode-ident/1.0.26) | 1.0.26 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| [unicode-segmentation](https://crates.io/crates/unicode-segmentation/1.13.3) | 1.13.3 | MIT OR Apache-2.0 |
| [unicode-width](https://crates.io/crates/unicode-width/0.2.2) | 0.2.2 | MIT OR Apache-2.0 |
| [utf8parse](https://crates.io/crates/utf8parse/0.2.2) | 0.2.2 | Apache-2.0 OR MIT |
| [uuid](https://crates.io/crates/uuid/1.28.0) | 1.28.0 | Apache-2.0 OR MIT |
| [vello_common](https://crates.io/crates/vello_common/0.1.0) | 0.1.0 | Apache-2.0 OR MIT |
| [vello_cpu](https://crates.io/crates/vello_cpu/0.1.0) | 0.1.0 | Apache-2.0 OR MIT |
| [version_check](https://crates.io/crates/version_check/0.9.5) | 0.9.5 | MIT/Apache-2.0 |
| [web-time](https://crates.io/crates/web-time/1.1.0) | 1.1.0 | MIT OR Apache-2.0 |
| [weezl](https://crates.io/crates/weezl/0.1.12) | 0.1.12 | MIT OR Apache-2.0 |
| [wgpu](https://crates.io/crates/wgpu/30.0.1) | 30.0.1 | MIT OR Apache-2.0 |
| [wgpu-core](https://crates.io/crates/wgpu-core/30.0.1) | 30.0.1 | MIT OR Apache-2.0 |
| [wgpu-core-deps-windows-linux-android](https://crates.io/crates/wgpu-core-deps-windows-linux-android/30.0.1) | 30.0.1 | MIT OR Apache-2.0 |
| [wgpu-hal](https://crates.io/crates/wgpu-hal/30.0.1) | 30.0.1 | MIT OR Apache-2.0 |
| [wgpu-naga-bridge](https://crates.io/crates/wgpu-naga-bridge/30.0.1) | 30.0.1 | MIT OR Apache-2.0 |
| [wgpu-types](https://crates.io/crates/wgpu-types/30.0.1) | 30.0.1 | MIT OR Apache-2.0 |
| [winapi](https://crates.io/crates/winapi/0.3.9) | 0.3.9 | MIT/Apache-2.0 |
| [windows](https://crates.io/crates/windows/0.62.2) | 0.62.2 | MIT OR Apache-2.0 |
| [windows_x86_64_msvc](https://crates.io/crates/windows_x86_64_msvc/0.52.6) | 0.52.6 | MIT OR Apache-2.0 |
| [windows_x86_64_msvc](https://crates.io/crates/windows_x86_64_msvc/0.53.1) | 0.53.1 | MIT OR Apache-2.0 |
| [windows-collections](https://crates.io/crates/windows-collections/0.3.2) | 0.3.2 | MIT OR Apache-2.0 |
| [windows-core](https://crates.io/crates/windows-core/0.62.2) | 0.62.2 | MIT OR Apache-2.0 |
| [windows-future](https://crates.io/crates/windows-future/0.3.2) | 0.3.2 | MIT OR Apache-2.0 |
| [windows-implement](https://crates.io/crates/windows-implement/0.60.2) | 0.60.2 | MIT OR Apache-2.0 |
| [windows-interface](https://crates.io/crates/windows-interface/0.59.3) | 0.59.3 | MIT OR Apache-2.0 |
| [windows-link](https://crates.io/crates/windows-link/0.2.1) | 0.2.1 | MIT OR Apache-2.0 |
| [windows-numerics](https://crates.io/crates/windows-numerics/0.3.1) | 0.3.1 | MIT OR Apache-2.0 |
| [windows-result](https://crates.io/crates/windows-result/0.4.1) | 0.4.1 | MIT OR Apache-2.0 |
| [windows-strings](https://crates.io/crates/windows-strings/0.5.1) | 0.5.1 | MIT OR Apache-2.0 |
| [windows-sys](https://crates.io/crates/windows-sys/0.52.0) | 0.52.0 | MIT OR Apache-2.0 |
| [windows-sys](https://crates.io/crates/windows-sys/0.60.2) | 0.60.2 | MIT OR Apache-2.0 |
| [windows-sys](https://crates.io/crates/windows-sys/0.61.2) | 0.61.2 | MIT OR Apache-2.0 |
| [windows-targets](https://crates.io/crates/windows-targets/0.52.6) | 0.52.6 | MIT OR Apache-2.0 |
| [windows-targets](https://crates.io/crates/windows-targets/0.53.5) | 0.53.5 | MIT OR Apache-2.0 |
| [windows-threading](https://crates.io/crates/windows-threading/0.2.1) | 0.2.1 | MIT OR Apache-2.0 |
| [winit](https://crates.io/crates/winit/0.30.13) | 0.30.13 | Apache-2.0 |
| [xml-rs](https://crates.io/crates/xml-rs/0.8.29) | 0.8.29 | MIT |
| [zerocopy](https://crates.io/crates/zerocopy/0.8.62) | 0.8.62 | BSD-2-Clause OR Apache-2.0 OR MIT |
| [zerocopy-derive](https://crates.io/crates/zerocopy-derive/0.8.62) | 0.8.62 | BSD-2-Clause OR Apache-2.0 OR MIT |
| [zlib-rs](https://crates.io/crates/zlib-rs/0.6.8) | 0.6.8 | Zlib |
| [zmij](https://crates.io/crates/zmij/1.0.23) | 1.0.23 | MIT |
| [zune-core](https://crates.io/crates/zune-core/0.5.3) | 0.5.3 | MIT OR Apache-2.0 OR Zlib |
| [zune-jpeg](https://crates.io/crates/zune-jpeg/0.5.15) | 0.5.15 | MIT OR Apache-2.0 OR Zlib |
