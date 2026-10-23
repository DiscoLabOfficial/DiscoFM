use crate::{BankError, Entry, SampleInfo, Source, check, invalid};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
type Opcodes = BTreeMap<String, String>;
const MAX_TEXT: usize = 8 * 1024 * 1024;
fn strip_comments(text: &str) -> Result<String, BankError> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut quoted = false;
    let mut block = false;
    while i < b.len() {
        if block {
            if b[i..].starts_with(b"*/") {
                block = false;
                i += 2;
            } else {
                if b[i] == b'\n' {
                    out.push(b'\n');
                }
                i += 1;
            }
        } else if !quoted && b[i..].starts_with(b"/*") {
            block = true;
            out.push(b' ');
            i += 2;
        } else if !quoted && b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else {
            if b[i] == b'"' {
                quoted = !quoted;
            }
            out.push(b[i]);
            i += 1;
        }
    }
    if block || quoted {
        return Err(invalid("SFZ: unterminated comment or quote"));
    }
    String::from_utf8(out).map_err(|_| invalid("SFZ: texto deve ser UTF-8"))
}
fn expand(text: &str, macros: &Opcodes) -> Result<String, BankError> {
    let mut output = String::new();
    let mut rest = text;
    while let Some(pos) = rest.find('$') {
        output.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let length = rest[1..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .map_or(rest.len(), |n| n + 1);
        let key = &rest[..length];
        output.push_str(
            macros
                .get(key)
                .ok_or_else(|| invalid(format!("SFZ: macro indefinida {key}")))?,
        );
        rest = &rest[length..];
    }
    output.push_str(rest);
    Ok(output)
}
fn preprocess(
    path: &Path,
    stack: &mut Vec<PathBuf>,
    macros: &mut Opcodes,
    output: &mut String,
    cancel: &impl Fn() -> bool,
) -> Result<(), BankError> {
    check(cancel)?;
    let path = path.canonicalize()?;
    if stack.contains(&path) || stack.len() >= 32 {
        return Err(invalid("SFZ: ciclo ou excesso de includes"));
    }
    stack.push(path.clone());
    let mut bytes = Vec::new();
    File::open(&path)?
        .take((MAX_TEXT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_TEXT {
        return Err(invalid("SFZ: texto excede 8 MiB"));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| invalid("SFZ: texto deve ser UTF-8"))?
        .trim_start_matches('\u{feff}');
    let text = strip_comments(text)?;
    for line in text.lines() {
        check(cancel)?;
        let trimmed = line.trim();
        if let Some(define) = trimmed.strip_prefix("#define") {
            let (key, value) = define
                .trim()
                .split_once(char::is_whitespace)
                .ok_or_else(|| invalid("SFZ: invalid #define"))?;
            if !key.starts_with('$') {
                return Err(invalid("SFZ: macro must start with $"));
            }
            macros.insert(key.into(), expand(value.trim(), macros)?);
        } else if let Some(include) = trimmed.strip_prefix("#include") {
            let value = expand(include.trim(), macros)?;
            let value = value
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .ok_or_else(|| invalid("SFZ: #include requer caminho entre aspas"))?;
            preprocess(
                &path
                    .parent()
                    .unwrap_or(std::path::Path::new(""))
                    .join(local_path(value)),
                stack,
                macros,
                output,
                cancel,
            )?;
        } else if trimmed.starts_with('#') {
            return Err(invalid("SFZ: unsupported directive"));
        } else {
            output.push_str(&expand(line, macros)?);
            output.push('\n');
        }
        if output.len() > MAX_TEXT {
            return Err(invalid("SFZ: includes expandidos excedem 8 MiB"));
        }
    }
    stack.pop();
    Ok(())
}
fn local_path(value: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(value)
    } else {
        PathBuf::from(value.replace('\\', "/"))
    }
}
fn key(value: &str) -> Option<u8> {
    if let Ok(n) = value.parse::<u8>() {
        return (n <= 127).then_some(n);
    }
    let b = value.as_bytes();
    let first = *b.first()?;
    let mut semitone: i16 = match first.to_ascii_uppercase() {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let mut pos = 1;
    if b.get(pos) == Some(&b'#') {
        semitone += 1;
        pos += 1;
    } else if b.get(pos) == Some(&b'b') {
        semitone -= 1;
        pos += 1;
    }
    let octave = value[pos..].parse::<i16>().ok()?;
    u8::try_from((octave + 1) * 12 + semitone)
        .ok()
        .filter(|n| *n <= 127)
}
fn number(map: &Opcodes, field: &str) -> Result<Option<u64>, BankError> {
    map.get(field)
        .map(|s| {
            s.parse()
                .map_err(|_| invalid(format!("SFZ: invalid {field}")))
        })
        .transpose()
}
fn add(
    entries: &mut Vec<Entry>,
    map: &Opcodes,
    base: &Path,
    default_path: &str,
) -> Result<(), BankError> {
    let Some(sample) = map.get("sample") else {
        return Ok(());
    };
    let index = entries.len();
    let mut warning = None;
    let start = number(map, "offset")?.unwrap_or(0);
    let end = if map.get("end").is_some_and(|v| v == "-1") {
        None
    } else {
        number(map, "end")?
    }
    .map(|n| n.checked_add(1).ok_or_else(|| invalid("SFZ: end overflow")))
    .transpose()?;
    let source = if map.get("end").is_some_and(|v| v == "-1") {
        warning = Some("SFZ: silent region (end=-1)".into());
        Source::Unavailable
    } else if sample.starts_with('*') {
        warning = Some("SFZ: virtual oscillator, no extractable audio sample".into());
        Source::Unavailable
    } else {
        let relative = local_path(&format!("{default_path}{sample}"));
        let path = if relative.is_absolute() {
            relative
        } else {
            base.join(relative)
        };
        if !path.is_file() {
            warning = Some(format!("Sample externo ausente: {}", path.display()));
        }
        Source::External { path, start, end }
    };
    let loop_start = number(map, "loop_start")?;
    let loop_end = number(map, "loop_end")?.and_then(|n| n.checked_add(1));
    let loop_frames = loop_start
        .zip(loop_end)
        .filter(|(s, e)| *s >= start && s < e && end.is_none_or(|last| *e <= last))
        .map(|(s, e)| [s - start, e - start]);
    let root_key = map
        .get("pitch_keycenter")
        .or_else(|| map.get("key"))
        .and_then(|s| key(s))
        .or(Some(60));
    entries.push(Entry {
        info: SampleInfo {
            index,
            name: map
                .get("region_label")
                .cloned()
                .unwrap_or_else(|| sample.clone()),
            sample_rate: None,
            frames: end.and_then(|e| e.checked_sub(start)),
            channels: 2,
            root_key,
            pitch_correction: map.get("tune").and_then(|v| v.parse().ok()).unwrap_or(0),
            loop_frames,
            warning,
        },
        source,
        pair: None,
        left: false,
    });
    Ok(())
}
/// Tokenize opcode assignments without splitting unquoted sample paths on spaces.
fn tokens(text: &str) -> Result<Vec<(String, String)>, BankError> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut output = Vec::new();
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i] == b'<' {
            let end = text[i..]
                .find('>')
                .ok_or_else(|| invalid("SFZ: header sem >"))?
                + i;
            output.push(("header".into(), text[i + 1..end].trim().into()));
            i = end + 1;
            continue;
        }
        let start = i;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
            i += 1;
        }
        let opcode = &text[start..i];
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if opcode.is_empty() || b.get(i) != Some(&b'=') {
            return Err(invalid(format!(
                "SFZ: esperado opcode= perto de {}",
                text[start..].chars().take(30).collect::<String>()
            )));
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        let mut quoted = false;
        while i < b.len() {
            if b[i] == b'"' {
                quoted = !quoted;
                i += 1;
                continue;
            }
            if !quoted && b[i] == b'<' {
                break;
            }
            if !quoted && b[i].is_ascii_whitespace() {
                let mut next = i;
                while next < b.len() && b[next].is_ascii_whitespace() {
                    next += 1;
                }
                let mut equals = next;
                while equals < b.len() && (b[equals].is_ascii_alphanumeric() || b[equals] == b'_') {
                    equals += 1;
                }
                while equals < b.len() && b[equals].is_ascii_whitespace() {
                    equals += 1;
                }
                if equals > next && b.get(equals) == Some(&b'=') {
                    break;
                }
            }
            i += 1;
        }
        let value = text[start..i].trim().trim_matches('"').to_owned();
        output.push((opcode.into(), value));
    }
    Ok(output)
}
pub(crate) fn read(path: &Path, cancel: &impl Fn() -> bool) -> Result<Vec<Entry>, BankError> {
    let mut text = String::new();
    preprocess(
        path,
        &mut Vec::new(),
        &mut Opcodes::new(),
        &mut text,
        cancel,
    )?;
    let mut global = Opcodes::new();
    let mut master = Opcodes::new();
    let mut group = Opcodes::new();
    let mut region = Opcodes::new();
    let mut section = String::new();
    let mut default_path = String::new();
    let mut entries = Vec::new();
    for (opcode, value) in tokens(&text)? {
        check(cancel)?;
        if opcode == "header" {
            if section == "region" {
                add(
                    &mut entries,
                    &region,
                    path.parent().unwrap_or(std::path::Path::new("")),
                    &default_path,
                )?;
            }
            section = value;
            match section.as_str() {
                "global" => {
                    global.clear();
                    master.clear();
                    group.clear();
                }
                "master" => {
                    master = global.clone();
                    group.clear();
                }
                "group" => {
                    group = global.clone();
                    group.extend(master.clone());
                }
                "region" => {
                    region = global.clone();
                    region.extend(master.clone());
                    region.extend(group.clone());
                }
                "sample" => return Err(invalid("SFZ: embedded <sample> blocks are not supported")),
                _ => {}
            }
        } else {
            match section.as_str() {
                "control" if opcode == "default_path" => default_path = value,
                "global" => {
                    global.insert(opcode, value);
                }
                "master" => {
                    master.insert(opcode, value);
                }
                "group" => {
                    group.insert(opcode, value);
                }
                "region" => {
                    region.insert(opcode, value);
                }
                _ => {}
            }
        }
    }
    if section == "region" {
        add(
            &mut entries,
            &region,
            path.parent().unwrap_or(std::path::Path::new("")),
            &default_path,
        )?;
    }
    Ok(entries)
}
