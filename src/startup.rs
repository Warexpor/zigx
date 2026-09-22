use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::{Revert, StartupEntry};
use crate::persist::atomic_write;

pub fn autostart_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("autostart");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config").join("autostart")
}

pub fn load_startup() -> Vec<StartupEntry> {
    let dir = autostart_dir();
    let Ok(rd) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let fields = parse_desktop(&text);
        if fields.kind.as_deref() == Some("Link") {
            continue;
        }
        let name = fields.name.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Autostart")
                .to_string()
        });
        out.push(StartupEntry {
            name,
            exec: fields.exec.unwrap_or_default(),
            path,
            enabled: fields.enabled,
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

pub fn write_enabled(path: &Path, enable: bool) -> io::Result<Revert> {
    let dir = autostart_dir()
        .canonicalize()
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "no autostart directory"))?;
    let canon = path.canonicalize()?;
    if canon.parent() != Some(dir.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to edit a file outside autostart",
        ));
    }
    let previous = fs::read_to_string(&canon)?;
    let mut bak = canon.clone().into_os_string();
    bak.push(".bak");
    let bak = PathBuf::from(bak);
    if !bak.exists() {
        fs::copy(&canon, &bak)?;
    }
    let mut next = set_key(&previous, "Hidden", if enable { "false" } else { "true" });
    if enable {
        next = set_key(&next, "X-GNOME-Autostart-enabled", "true");
    }
    atomic_write(&canon, &next)?;
    Ok(Revert {
        path: canon,
        previous,
    })
}

pub fn restore_startup(revert: &Revert) -> io::Result<()> {
    let dir = autostart_dir().canonicalize()?;
    let canon = revert
        .path
        .canonicalize()
        .unwrap_or_else(|_| revert.path.clone());
    if canon.parent() != Some(dir.as_path()) && revert.path.parent() != Some(dir.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to restore outside autostart",
        ));
    }
    atomic_write(&revert.path, &revert.previous)
}

struct DesktopFields {
    name: Option<String>,
    exec: Option<String>,
    kind: Option<String>,
    enabled: bool,
}

fn parse_desktop(text: &str) -> DesktopFields {
    let mut name = None;
    let mut exec = None;
    let mut kind = None;
    let mut hidden = false;
    let mut gnome_off = false;
    let mut in_entry = false;
    let mut seen_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if seen_group {
                break;
            }
            in_entry = line.eq_ignore_ascii_case("[Desktop Entry]");
            seen_group = true;
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "Name" => name = Some(v.to_string()),
            "Exec" => exec = Some(v.to_string()),
            "Type" => kind = Some(v.to_string()),
            "Hidden" => hidden = v.eq_ignore_ascii_case("true"),
            "X-GNOME-Autostart-enabled" => gnome_off = v.eq_ignore_ascii_case("false"),
            _ => {}
        }
    }
    DesktopFields {
        name,
        exec,
        kind,
        enabled: !hidden && !gnome_off,
    }
}

fn set_key(text: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut out = String::new();
    let mut in_entry = false;
    let mut seen_group = false;
    let mut passed_entry = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if seen_group && in_entry && !found {
                out.push_str(key);
                out.push('=');
                out.push_str(value);
                out.push('\n');
                found = true;
            }
            if seen_group {
                passed_entry = true;
            }
            in_entry = trimmed.eq_ignore_ascii_case("[Desktop Entry]");
            seen_group = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_entry && !passed_entry {
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim() == key {
                    out.push_str(key);
                    out.push('=');
                    out.push_str(value);
                    out.push('\n');
                    found = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !found {
        out.push_str(key);
        out.push('=');
        out.push_str(value);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_toggles_hidden() {
        let src = "[Desktop Entry]\nType=Application\nName=Steam\nExec=steam %U\nHidden=false\n";
        let fields = parse_desktop(src);
        assert_eq!(fields.name.as_deref(), Some("Steam"));
        assert!(fields.enabled);
        let off = set_key(src, "Hidden", "true");
        assert!(!parse_desktop(&off).enabled);
        assert!(off.contains("Hidden=true"));
        assert!(off.contains("Name=Steam"));
    }

    #[test]
    fn ignores_later_groups() {
        let src = "[Desktop Entry]\nName=Keep\n\n[Desktop Action]\nName=Ignore\n";
        assert_eq!(parse_desktop(src).name.as_deref(), Some("Keep"));
    }
}
