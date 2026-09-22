use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::StartupEntry;
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

/// System autostart directories, lowest precedence first.
fn system_autostart_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_CONFIG_DIRS").unwrap_or_default();
    let mut out: Vec<PathBuf> = dirs
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join("autostart"))
        .collect();
    if out.is_empty() {
        out.push(PathBuf::from("/etc/xdg/autostart"));
    }
    out.reverse();
    out
}

struct Loaded {
    path: PathBuf,
    fields: DesktopFields,
}

fn load_dir(dir: &Path) -> Vec<(String, Loaded)> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let Some(file) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        out.push((
            file.to_string(),
            Loaded {
                path,
                fields: parse_desktop(&text),
            },
        ));
    }
    out
}

/// Every autostart entry the session would consider, following the XDG
/// precedence rule: a file in `~/.config/autostart` overrides the system file
/// with the same name, even when the override only says `Hidden=true`.
pub fn load_startup() -> Vec<StartupEntry> {
    let user_dir = autostart_dir();
    let mut system: HashMap<String, Loaded> = HashMap::new();
    for dir in system_autostart_dirs() {
        for (file, loaded) in load_dir(&dir) {
            system.insert(file, loaded);
        }
    }
    let mut user: HashMap<String, Loaded> = load_dir(&user_dir).into_iter().collect();

    let mut out = Vec::new();
    let mut files: Vec<String> = system.keys().chain(user.keys()).cloned().collect();
    files.sort();
    files.dedup();
    for file in files {
        let sys = system.remove(&file);
        let usr = user.remove(&file);
        let effective = usr.as_ref().or(sys.as_ref()).map(|l| &l.fields);
        if effective.is_some_and(|f| f.kind.as_deref() == Some("Link")) {
            continue;
        }
        let pick = |get: fn(&DesktopFields) -> Option<&String>| {
            usr.as_ref()
                .and_then(|l| get(&l.fields))
                .or_else(|| sys.as_ref().and_then(|l| get(&l.fields)))
                .cloned()
        };
        let name = pick(|f| f.name.as_ref())
            .unwrap_or_else(|| file.strip_suffix(".desktop").unwrap_or(&file).to_string());
        let exec = pick(|f| f.exec.as_ref()).unwrap_or_default();
        let enabled = effective.is_none_or(|f| f.enabled);
        out.push(StartupEntry {
            name,
            exec,
            path: user_dir.join(&file),
            system_path: sys.map(|l| l.path),
            enabled,
        });
    }
    out.sort_by_key(|e| e.name.to_lowercase());
    out
}

fn user_path_for(path: &Path) -> io::Result<PathBuf> {
    let dir = autostart_dir();
    let file = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    if path.parent() != Some(dir.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to edit a file outside autostart",
        ));
    }
    Ok(dir.join(file))
}

/// Flip an entry. Existing user files are edited in place with a one-time
/// `.bak`. Entries that only exist system-wide get a user override created,
/// seeded from the system file so the override stays readable on its own.
pub fn write_enabled(path: &Path, system_path: Option<&Path>, enable: bool) -> io::Result<()> {
    let target = user_path_for(path)?;
    let previous = match fs::read_to_string(&target) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    };
    let base = match (&previous, system_path) {
        (Some(text), _) => text.clone(),
        (None, Some(sys)) => fs::read_to_string(sys)?,
        (None, None) => "[Desktop Entry]\nType=Application\n".to_string(),
    };
    if previous.is_some() {
        let mut bak = target.clone().into_os_string();
        bak.push(".bak");
        let bak = PathBuf::from(bak);
        if !bak.exists() {
            fs::copy(&target, &bak)?;
        }
    } else if let Some(dir) = target.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut next = set_key(&base, "Hidden", if enable { "false" } else { "true" });
    if enable {
        next = set_key(&next, "X-GNOME-Autostart-enabled", "true");
    }
    atomic_write(&target, &next)
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

    #[test]
    fn set_key_adds_missing_key_inside_entry_group() {
        let src = "[Desktop Entry]\nName=A\n\n[Desktop Action x]\nName=B\n";
        let out = set_key(src, "Hidden", "true");
        let entry_end = out.find("[Desktop Action").unwrap();
        let hidden = out.find("Hidden=true").unwrap();
        assert!(
            hidden < entry_end,
            "key must land in [Desktop Entry]: {out}"
        );
    }

    #[test]
    fn user_mask_overrides_system_entry() {
        let root = std::env::temp_dir().join(format!("zigx-startup-{}", std::process::id()));
        let sys = root.join("sys").join("autostart");
        let usr = root.join("home").join("autostart");
        fs::create_dir_all(&sys).unwrap();
        fs::create_dir_all(&usr).unwrap();
        fs::write(
            sys.join("a.desktop"),
            "[Desktop Entry]\nType=Application\nName=Alpha\nExec=alpha\n",
        )
        .unwrap();
        fs::write(
            sys.join("b.desktop"),
            "[Desktop Entry]\nType=Application\nName=Beta\nExec=beta\n",
        )
        .unwrap();
        fs::write(usr.join("b.desktop"), "[Desktop Entry]\nHidden=true\n").unwrap();

        // Process-wide env; tests in this module run in one process, and no
        // other test in the crate touches XDG_CONFIG_*.
        std::env::set_var("XDG_CONFIG_DIRS", root.join("sys"));
        std::env::set_var("XDG_CONFIG_HOME", root.join("home"));

        let list = load_startup();
        assert_eq!(list.len(), 2);
        let a = list.iter().find(|e| e.name == "Alpha").unwrap();
        let b = list.iter().find(|e| e.name == "Beta").unwrap();
        assert!(a.enabled);
        assert!(!b.enabled, "user mask must win");
        assert_eq!(b.exec, "beta", "exec falls back to the system file");
        assert!(a.system_path.is_some());
        assert!(!a.path.exists());

        write_enabled(&a.path, a.system_path.as_deref(), false).unwrap();
        assert!(a.path.exists());
        let reloaded = load_startup();
        assert!(!reloaded.iter().find(|e| e.name == "Alpha").unwrap().enabled);
        let _ = fs::remove_dir_all(&root);
    }
}
