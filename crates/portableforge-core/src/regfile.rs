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
pub fn parse_reg(text: &str) -> Vec<RegEntry> {
    parse_reg_with_report(text).0
}

pub fn parse_reg_with_report(text: &str) -> (Vec<RegEntry>, usize) {
    let mut entries = Vec::new();
    let mut skipped = 0usize;
    let mut current_key: Option<String> = None;

    // Join continuation lines first: in a `.reg` export only hex values
    // wrap, and a wrapped line ends with a backslash.
    let mut logical: Vec<String> = Vec::new();
    let mut pending = String::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if pending.is_empty() {
            pending = line.to_string();
        } else {
            pending.push_str(line.trim());
        }
        if pending.contains("=hex") && line.ends_with('\\') {
            pending.pop(); // drop the continuation backslash, keep joining
            continue;
        }
        logical.push(std::mem::take(&mut pending));
    }
    if !pending.is_empty() {
        logical.push(pending);
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
    // `"na\"me"=data` — find the closing quote honouring `\"` escapes.
    let bytes = line.as_bytes();
    let mut i = 1usize;
    let mut name = String::new();
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => {
                name.push(bytes[i + 1] as char);
                i += 2;
            }
            b'"' => {
                i += 1;
                break;
            }
            b => {
                name.push(b as char);
                i += 1;
            }
        }
    }
    let rest = line.get(i..)?.strip_prefix('=')?;
    Some((name, rest.trim().to_string()))
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
}
