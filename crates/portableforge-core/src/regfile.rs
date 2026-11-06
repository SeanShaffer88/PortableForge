//! Parser for `reg.exe export` (`.reg`) files — the agent's registry
//! capture format. Handles the `Windows Registry Editor Version 5.00`
//! dialect: key headers, string / dword / hex / multi-string values,
//! line continuations for hex data, and key deletions (`[-KEY]`).
//! Enough to build a faithful [`RegEntry`] list for diffing; unknown
//! value kinds are preserved in their textual form.

use crate::snapshot::RegEntry;

/// Parse `.reg` text into registry entries. Deletion markers produce no
/// entries (a snapshot records what exists). Malformed lines are skipped
/// rather than failing the whole capture — a snapshot with a warning is
/// more useful than no snapshot; the caller counts skipped lines via
/// [`parse_reg_with_report`].
/// Decode a `.reg` export's bytes: UTF-16LE with a BOM (what
/// `reg.exe` writes) or plain UTF-8. The same decoding the agent
/// does, kept here so the host side (Sandboxie captures) reads
/// exports identically.
pub fn decode_reg_export(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&units).map_err(|e| format!("Bad UTF-16 export: {e}"));
    }
    String::from_utf8(bytes.to_vec()).map_err(|e| format!("Bad export text: {e}"))
}

pub fn parse_reg(text: &str) -> Vec<RegEntry> {
    parse_reg_with_report(text).0
}

pub fn parse_reg_with_report(text: &str) -> (Vec<RegEntry>, usize) {
    let mut entries = Vec::new();
    let mut skipped = 0usize;
    let mut current_key: Option<String> = None;

    // reg.exe emits both wrapped hex and literal newlines in quoted strings.
    let mut logical: Vec<String> = Vec::new();
    let mut pending = String::new();
    let mut in_quoted_value = false;
    let mut in_hex_value = false;
    for raw in text.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if in_quoted_value {
            pending.push_str(line);
            in_quoted_value = closing_quote(line).is_none();
        } else if in_hex_value {
            pending.push_str(line.trim());
            in_hex_value = pending.ends_with('\\');
        } else {
            pending.push_str(line);
            if let Some(data) = value_data(line.trim_start()) {
                let data = data.trim_start();
                in_quoted_value = data
                    .strip_prefix('"')
                    .is_some_and(|rest| closing_quote(rest).is_none());
                in_hex_value = (data.starts_with("hex:") || data.starts_with("hex("))
                    && pending.ends_with('\\');
            }
        }
        if in_quoted_value {
            pending.push_str(&raw[line.len()..]);
            continue;
        }
        if in_hex_value {
            pending.pop();
            continue;
        }
        logical.push(std::mem::take(&mut pending));
    }
    if in_quoted_value || in_hex_value {
        skipped += 1;
    }

    for line in &logical {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with("Windows Registry Editor")
            || line.starts_with("REGEDIT4")
        {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let Some(end) = rest.rfind(']') else {
                skipped += 1;
                continue;
            };
            let key = &rest[..end];
            if let Some(deleted) = key.strip_prefix('-') {
                let _ = deleted;
                current_key = None;
            } else {
                current_key = Some(normalise_hive(key));
            }
            continue;
        }
        let Some(key) = current_key.clone() else {
            skipped += 1;
            continue;
        };
        // Value line: `"name"=data` or `@=data` (default value).
        let (name, data) = if let Some(rest) = line.strip_prefix('@') {
            let Some(data) = rest.strip_prefix('=') else {
                skipped += 1;
                continue;
            };
            ("(Default)".to_string(), data.trim().to_string())
        } else if line.starts_with('"') {
            match split_value_line(line) {
                Some(pair) => pair,
                None => {
                    skipped += 1;
                    continue;
                }
            }
        } else {
            skipped += 1;
            continue;
        };
        entries.push(RegEntry {
            key,
            name,
            data: normalise_data(&data),
        });
    }
    (entries, skipped)
}

fn split_value_line(line: &str) -> Option<(String, String)> {
    let (escaped_name, data) = named_value_parts(line)?;
    let mut name = String::new();
    let mut chars = escaped_name.chars();
    while let Some(ch) = chars.next() {
        name.push(if ch == '\\' { chars.next()? } else { ch });
    }
    Some((name, data.trim().to_string()))
}

fn value_data(line: &str) -> Option<&str> {
    line.strip_prefix("@=")
        .or_else(|| named_value_parts(line).map(|(_, data)| data))
}

fn named_value_parts(line: &str) -> Option<(&str, &str)> {
    let quoted = line.strip_prefix('"')?;
    let end = closing_quote(quoted)?;
    Some((&quoted[..end], quoted[end + 1..].strip_prefix('=')?))
}

fn closing_quote(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            '"' => return Some(index),
            _ => {}
        }
    }
    None
}

/// Render data canonically so equivalent exports diff cleanly:
/// dword values lose leading zeros grouping differences, hex bytes are
/// lower-cased without spaces.
fn normalise_data(data: &str) -> String {
    let d = data.trim();
    if let Some(hex) = d.strip_prefix("dword:") {
        if let Ok(v) = u32::from_str_radix(hex.trim(), 16) {
            return format!("dword:{v:08x}");
        }
        return d.to_string();
    }
    if let Some(rest) = d.strip_prefix("hex") {
        let bytes: String = rest
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect::<String>()
            .to_ascii_lowercase();
        return format!("hex:{bytes}");
    }
    d.to_string()
}

fn normalise_hive(key: &str) -> String {
    let (hive, rest) = match key.split_once('\\') {
        Some(pair) => pair,
        None => (key, ""),
    };
    let hive_upper = hive.to_ascii_uppercase();
    let full = match hive_upper.as_str() {
        "HKEY_LOCAL_MACHINE" | "HKLM" => "HKEY_LOCAL_MACHINE",
        "HKEY_CURRENT_USER" | "HKCU" => "HKEY_CURRENT_USER",
        "HKEY_CLASSES_ROOT" | "HKCR" => "HKEY_CLASSES_ROOT",
        "HKEY_USERS" | "HKU" => "HKEY_USERS",
        "HKEY_CURRENT_CONFIG" | "HKCC" => "HKEY_CURRENT_CONFIG",
        other => other,
    };
    if rest.is_empty() {
        full.to_string()
    } else {
        format!("{full}\\{rest}")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hostile_inputs_never_panic_the_parser() {
        // Item 81: the parser eats whatever a damaged or hostile
        // export contains. Every input must return — entries or
        // not — without panicking.
        let big = "a".repeat(1_000_000);
        let inputs: Vec<String> = vec![
            String::new(),
            "\u{0}\u{1}\u{2}".into(),
            "Windows Registry Editor Version 5.00\n[".into(),
            "[unclosed".into(),
            "\"unterminated".into(),
            "[HKEY_CURRENT_USER\\Software]\n\"x\"=".into(),
            "[HKEY_CURRENT_USER\\Software]\n\"x\"=\"y".into(),
            "a=b\r\n[b]\n\"x\"=\"y\"".into(),
            "[".repeat(10_000),
            "[HKEY_CURRENT_USER\\Software]\n\"d\"=dword:zzzz".into(),
            "[HKEY_CURRENT_USER\\Software]\n\"d\"=dword:99999999999999999999".into(),
            "[HKEY_CURRENT_USER\\Software]\n\"h\"=hex:0,1,zz,\\".into(),
            "[HKEY_CURRENT_USER\\Software]\n\"h\"=hex:\\\n  zz".into(),
            "[-HKEY_CURRENT_USER\\Software]".into(),
            "[..\\..\\..]".into(),
            "[HKEY_CURRENT_USER\\🎉 Emoji]\n\"🎉\"=\"🎉\"".into(),
            format!("[HKEY_CURRENT_USER\\Software]\n\"big\"=\"{big}\""),
            "REGEDIT4\n[HKEY_CURRENT_USER\\Software]\n\"a\"=\"1\"".into(),
            "\n\n\n   \n\t\n".into(),
            "[HKEY_CURRENT_USER\\Software]extra\n\"a\"=\"1\"".into(),
            "\"orphan\"=\"value with no key\"".into(),
            "[HKEY_CURRENT_USER\\Software]\n@=\"default\" extra junk".into(),
            "[HKEY_CURRENT_USER\\Software]\n\"esc\"=\"a\\\\\"".into(),
        ];
        for input in &inputs {
            let entries = super::parse_reg(input);
            let (entries2, skipped) = super::parse_reg_with_report(input);
            assert_eq!(entries.len(), entries2.len());
            let _ = skipped;
        }
        // A sane export in the middle of the junk still parses.
        let good = "Windows Registry Editor Version 5.00\n\n[HKEY_CURRENT_USER\\Software\\Acme]\n\"Mode\"=\"dark\"\n";
        assert_eq!(super::parse_reg(good).len(), 1);
    }
    use super::*;

    const SAMPLE: &str = "Windows Registry Editor Version 5.00\r\n\r\n\
[HKEY_CURRENT_USER\\Software\\Acme\\Widget]\r\n\
\"Mode\"=\"fast\"\r\n\
\"Count\"=dword:0000002a\r\n\
@=\"default text\"\r\n\r\n\
[HKEY_LOCAL_MACHINE\\SOFTWARE\\Acme]\r\n\
\"Blob\"=hex:01,02,ff\r\n\r\n\
[-HKEY_CURRENT_USER\\Software\\Gone]\r\n";

    #[test]
    fn parses_keys_values_and_default() {
        let entries = parse_reg(SAMPLE);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].key, "HKEY_CURRENT_USER\\Software\\Acme\\Widget");
        assert_eq!(entries[0].name, "Mode");
        assert_eq!(entries[0].data, "\"fast\"");
        assert_eq!(entries[1].data, "dword:0000002a");
        assert_eq!(entries[2].name, "(Default)");
        assert_eq!(entries[3].key, "HKEY_LOCAL_MACHINE\\SOFTWARE\\Acme");
        assert_eq!(entries[3].data, "hex:0102ff");
    }

    #[test]
    fn deleted_keys_produce_no_entries() {
        let entries = parse_reg(SAMPLE);
        assert!(entries.iter().all(|e| !e.key.contains("Gone")));
    }

    #[test]
    fn preserves_multiline_strings_and_unicode_value_names() {
        let input = concat!(
            "[HKEY_CURRENT_USER\\Software\\Fixture]\r\n",
            "\"Résumé\"=\"first line with \\\"quotation\\\"\r\n",
            "  second line\r\n",
            "[literal brackets] and =hex\"\r\n",
            "\"Blob\"=hex:01,02,\\\r\n",
            "  ff\r\n",
        );
        let (entries, skipped) = parse_reg_with_report(input);
        assert_eq!(skipped, 0);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "Résumé");
        assert_eq!(
            entries[0].data,
            "\"first line with \\\"quotation\\\"\r\n  second line\r\n[literal brackets] and =hex\""
        );
        assert_eq!(entries[1].data, "hex:0102ff");
    }

    #[test]
    fn reports_incomplete_continued_values() {
        for value in ["\"x\"=\"unfinished\r\n", "\"x\"=hex:01,\\\r\n"] {
            let input = format!("[HKEY_CURRENT_USER\\Software\\Fixture]\r\n{value}");
            let (entries, skipped) = parse_reg_with_report(&input);
            assert!(entries.is_empty());
            assert_eq!(skipped, 1);
        }
    }
}
