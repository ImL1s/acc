//! Minimal Unix `ar` archive reader for static libc/libgcc `.a` members.

use std::path::Path;

/// Return `(member_name, object_bytes)` for every ELF member in an archive.
pub fn read_archive(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.len() < 8 || &bytes[0..8] != b"!<arch>\n" {
        return Err(format!("{}: not a Unix archive", path.display()));
    }
    let mut out = Vec::new();
    let mut off = 8usize;
    while off + 60 <= bytes.len() {
        let hdr = &bytes[off..off + 60];
        let size = parse_decimal_field(std::str::from_utf8(&hdr[48..58]).unwrap_or(""))?;
        let mut name = String::from_utf8_lossy(&hdr[0..16]).trim().to_string();
        off += 60;
        let data_end = off
            .checked_add(size)
            .ok_or("archive member size overflow")?;
        if data_end > bytes.len() {
            return Err(format!("{}: member past EOF", path.display()));
        }
        let data = bytes[off..data_end].to_vec();
        off = data_end;
        if off & 1 != 0 {
            off += 1;
        }
        // Long-name table
        if name.starts_with('#') && name.len() > 1 && name[1..].chars().all(|c| c.is_ascii_digit())
        {
            if let Ok(idx) = name[1..].trim().parse::<usize>() {
                if let Some(slash) = data.iter().position(|&b| b == b'/') {
                    name = String::from_utf8_lossy(&data[..slash]).into_owned();
                } else if idx == 1 {
                    // skip ar extended name table itself
                    continue;
                } else {
                    let _ = idx;
                }
            }
        }
        if name.ends_with('/') {
            name.pop();
        }
        if name == "__.SYMDEF" || name == "__.SYMDEF SORTED" || name.is_empty() {
            continue;
        }
        if data.len() >= 4 && &data[0..4] == b"\x7fELF" {
            out.push((name, data));
        }
    }
    Ok(out)
}

fn parse_decimal_field(s: &str) -> Result<usize, String> {
    let trimmed = s.trim();
    trimmed
        .parse::<usize>()
        .map_err(|e| format!("bad archive size field `{trimmed}`: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_musl_libc_members_when_present() {
        let path = Path::new("/usr/lib/aarch64-linux-musl/libc.a");
        if !path.exists() {
            eprintln!("musl libc.a not installed; skipping");
            return;
        }
        let members = read_archive(path).expect("read archive");
        assert!(
            members.len() > 100,
            "expected many members, got {}",
            members.len()
        );
        assert!(
            members.iter().any(|(n, _)| n.contains("printf")),
            "printf member missing"
        );
    }
}
