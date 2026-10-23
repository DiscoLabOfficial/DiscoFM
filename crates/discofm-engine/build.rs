fn main() {
    for file in [
        "vendor/nuked-opn2/ym3438.c",
        "vendor/nuked-opn2/ym3438.h",
        "native/bridge.c",
        "native/oracle.c",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }
    // Keep the pinned oracle pristine. Production gets an instance-local mode;
    // the three reads and setter are checked so an upstream update fails loudly.
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("instance-core");
    std::fs::create_dir_all(&out).unwrap();
    let header = std::fs::read_to_string("vendor/nuked-opn2/ym3438.h").unwrap();
    let mut source = std::fs::read_to_string("vendor/nuked-opn2/ym3438.c").unwrap();
    let global = "static Bit32u chip_type = ym3438_mode_readmode;";
    assert_eq!(source.matches(global).count(), 1);
    source = source.replace(global, "");
    let begin = source.find("void OPN2_SetChipType(").unwrap();
    let end = begin + source[begin..].find("\n}").unwrap() + 2;
    source.replace_range(begin..end, "");
    assert_eq!(source.matches("chip_type").count(), 3);
    source = source
        .replace("chip_type", "chip->dfm_chip_type")
        .replace("OPN2_", "DFM_OPN2_");
    let marker = "typedef struct";
    let start = header.find(marker).unwrap();
    let brace = start + header[start..].find('{').unwrap() + 1;
    let mut header = header;
    header.insert_str(
        brace,
        "\n    Bit32u dfm_chip_type; /* instance-local production profile */\n",
    );
    let modification_notice = "/* Modified for DiscoFM on 2026-10-22: instance-local chip profile\n\
                               and DFM_OPN2_ symbols; original copyright and license follow. */\n";
    std::fs::write(
        out.join("ym3438.h"),
        format!(
            "{modification_notice}{}",
            header.replace("OPN2_", "DFM_OPN2_")
        ),
    )
    .unwrap();
    std::fs::write(
        out.join("ym3438.c"),
        format!("{modification_notice}{source}"),
    )
    .unwrap();
    cc::Build::new()
        .include(&out)
        .file(out.join("ym3438.c"))
        .file("native/bridge.c")
        .flag_if_supported("-std=c99")
        .compile("discofm_nuked_instance");
    cc::Build::new()
        .include("vendor/nuked-opn2")
        .file("vendor/nuked-opn2/ym3438.c")
        .file("native/oracle.c")
        .flag_if_supported("-std=c99")
        .compile("discofm_nuked");
    println!("cargo:rerun-if-changed=native/ymfm_reference.cpp");
    println!("cargo:rerun-if-changed=vendor/ymfm");
    cc::Build::new()
        .cpp(true)
        .std("c++14")
        .include("vendor/ymfm")
        .file("native/ymfm_reference.cpp")
        .file("vendor/ymfm/ymfm_opn.cpp")
        .file("vendor/ymfm/ymfm_adpcm.cpp")
        .file("vendor/ymfm/ymfm_ssg.cpp")
        .flag_if_supported("/EHsc")
        .warnings(false)
        .compile("discofm_ymfm_reference");
}
