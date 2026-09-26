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
    } else if !out.iter().any(|p| p == Path::new("/etc/xdg/autostart")) {
        // Keep the distro defaults even when XDG_CONFIG_DIRS is customized.
        out.insert(0, PathBuf::from("/etc/xdg/autostart"));
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

/// Every autostart `.desktop` under the XDG autostart dirs, following the
/// precedence rule: a file in `~/.config/autostart` overrides the system file
/// with the same name, even when the override only says `Hidden=true`.
///
/// Entries are listed for management even when `OnlyShowIn` / `NotShowIn` /
/// `TryExec` would keep the session from launching them — those keys decide
/// launch, not whether the file belongs in the Startup page.
pub fn load_startup() -> Vec<StartupEntry> {
    load_startup_from(
        &system_autostart_dirs(),
        &autostart_dir(),
        &hypr_autostart_path(),
    )
}

/// [`load_startup`] over explicit locations. `system_dirs` is lowest
/// precedence first.
fn load_startup_from(system_dirs: &[PathBuf], user_dir: &Path, hypr: &Path) -> Vec<StartupEntry> {
    let mut system: HashMap<String, Loaded> = HashMap::new();
    for dir in system_dirs {
        for (file, loaded) in load_dir(dir) {
            system.insert(file, loaded);
        }
    }
    let mut user: HashMap<String, Loaded> = load_dir(user_dir).into_iter().collect();

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
            hypr_index: None,
            enabled,
        });
    }
    out.extend(load_hypr_launches(hypr));
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

/// Toggle the `index`-th `o.launch_on_start` in a Hypr/Omarchy `autostart.lua`.
pub fn write_hypr_enabled(path: &Path, index: usize, enable: bool) -> io::Result<()> {
    let text = fs::read_to_string(path)?;
    let Some(next) = set_hypr_launch_enabled(&text, index, enable) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "launch_on_start entry not found",
        ));
    };
    let mut bak = path.as_os_str().to_os_string();
    bak.push(".bak");
    let bak = PathBuf::from(bak);
    if !bak.exists() {
        fs::copy(path, &bak)?;
    }
    atomic_write(path, &next)
}

fn hypr_autostart_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("hypr").join("autostart.lua");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".config")
        .join("hypr")
        .join("autostart.lua")
}

fn load_hypr_launches(path: &Path) -> Vec<StartupEntry> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_hypr_launches(&text, path)
}

fn parse_hypr_launches(text: &str, path: &Path) -> Vec<StartupEntry> {
    let mut out = Vec::new();
    for (index, call) in find_hypr_launches(text).into_iter().enumerate() {
        out.push(StartupEntry {
            name: hypr_launch_name(&call.command),
            exec: call.command,
            path: path.to_path_buf(),
            system_path: None,
            hypr_index: Some(index),
            enabled: call.enabled,
        });
    }
    out
}

struct HyprLaunch {
    command: String,
    enabled: bool,
    /// Inclusive line range in the source file (0-based).
    start_line: usize,
    end_line: usize,
}

fn find_hypr_launches(text: &str) -> Vec<HyprLaunch> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        let commented = trimmed.starts_with("--");
        let code = if commented {
            trimmed
                .strip_prefix("--")
                .map(|s| s.trim_start())
                .unwrap_or(trimmed)
        } else {
            trimmed
        };
        if !code.starts_with("o.launch_on_start") {
            i += 1;
            continue;
        }
        let start_line = i;
        let mut block = String::new();
        let mut depth = 0i32;
        let mut saw_paren = false;
        while i < lines.len() {
            let raw = lines[i];
            let t = raw.trim_start();
            let body = if let Some(rest) = t.strip_prefix("--") {
                rest.trim_start()
            } else {
                t
            };
            block.push_str(body);
            block.push('\n');
            for ch in body.chars() {
                match ch {
                    '(' => {
                        depth += 1;
                        saw_paren = true;
                    }
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            if saw_paren && depth <= 0 {
                break;
            }
            i += 1;
        }
        if let Some(cmd) = extract_lua_string(&block) {
            out.push(HyprLaunch {
                command: cmd,
                enabled: !commented,
                start_line,
                end_line: i.min(lines.len().saturating_sub(1)),
            });
        }
        i += 1;
    }
    out
}

fn extract_lua_string(block: &str) -> Option<String> {
    let bytes = block.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i += 1;
            let mut out = String::new();
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' if i + 1 < bytes.len() => {
                        out.push(bytes[i + 1] as char);
                        i += 2;
                    }
                    b'"' => return Some(out),
                    b => {
                        out.push(b as char);
                        i += 1;
                    }
                }
            }
            return None;
        }
        i += 1;
    }
    None
}

fn hypr_launch_name(cmd: &str) -> String {
    let first = cmd.split_whitespace().next().unwrap_or(cmd);
    if first.contains('/') {
        return Path::new(first)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(first)
            .to_string();
    }
    if cmd.len() > 42 {
        format!("{}...", &cmd[..39])
    } else {
        cmd.to_string()
    }
}

fn set_hypr_launch_enabled(text: &str, index: usize, enable: bool) -> Option<String> {
    let launches = find_hypr_launches(text);
    let call = launches.get(index)?;
    if call.enabled == enable {
        return Some(text.to_string());
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        if i < call.start_line || i > call.end_line {
            out.push((*line).to_string());
            continue;
        }
        if enable {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("--") {
                let indent_len = line.len() - t.len();
                let mut s = line[..indent_len].to_string();
                s.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                out.push(s);
            } else {
                out.push((*line).to_string());
            }
        } else {
            let t = line.trim_start();
            if t.starts_with("--") || t.is_empty() {
                out.push((*line).to_string());
            } else {
                let indent_len = line.len() - t.len();
                let mut s = line[..indent_len].to_string();
                s.push_str("-- ");
                s.push_str(t);
                out.push(s);
            }
        }
    }
    let mut next = out.join("\n");
    if text.ends_with('\n') {
        next.push('\n');
    }
    Some(next)
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

        // write_enabled only edits files inside autostart_dir(), which follows
        // XDG_CONFIG_HOME. No other test sets it, so parallel tests cannot race.
        std::env::set_var("XDG_CONFIG_HOME", root.join("home"));

        let hypr = root.join("home").join("hypr").join("autostart.lua");
        let load = || load_startup_from(std::slice::from_ref(&sys), &usr, &hypr);
        let list = load();
        assert_eq!(list.len(), 2, "{list:?}");
        let a = list.iter().find(|e| e.name == "Alpha").unwrap();
        let b = list.iter().find(|e| e.name == "Beta").unwrap();
        assert!(a.enabled);
        assert!(!b.enabled, "user mask must win");
        assert_eq!(b.exec, "beta", "exec falls back to the system file");
        assert!(a.system_path.is_some());
        assert!(!a.path.exists());

        write_enabled(&a.path, a.system_path.as_deref(), false).unwrap();
        assert!(a.path.exists());
        let reloaded = load();
        assert!(!reloaded.iter().find(|e| e.name == "Alpha").unwrap().enabled);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn lists_entries_even_when_only_show_in_excludes_desktop() {
        let root = std::env::temp_dir().join(format!("zigx-startup-osi-{}", std::process::id()));
        let sys = root.join("sys").join("autostart");
        let usr = root.join("home").join("autostart");
        fs::create_dir_all(&sys).unwrap();
        fs::create_dir_all(&usr).unwrap();
        fs::write(
            sys.join("gnome-only.desktop"),
            "[Desktop Entry]\nType=Application\nName=GNOME Only\nExec=true\nOnlyShowIn=GNOME;\n",
        )
        .unwrap();
        fs::write(
            sys.join("missing-bin.desktop"),
            "[Desktop Entry]\nType=Application\nName=Missing Bin\nExec=missing\nTryExec=/no/such/zigx-tryexec\n",
        )
        .unwrap();

        std::env::set_var("XDG_CURRENT_DESKTOP", "Hyprland");

        let hypr = root.join("home").join("hypr").join("autostart.lua");
        let list = load_startup_from(std::slice::from_ref(&sys), &usr, &hypr);
        assert!(
            list.iter().any(|e| e.name == "GNOME Only"),
            "OnlyShowIn must not hide management entries: {list:?}"
        );
        assert!(
            list.iter().any(|e| e.name == "Missing Bin"),
            "TryExec must not hide management entries: {list:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_hypr_launch_on_start_including_comments() {
        let src = r#"-- Extra
-- o.launch_on_start("my-service")
o.launch_on_start("/opt/v2rayn-bin/v2rayN")

o.launch_on_start(
  "gsettings set org.gnome.desktop.interface gtk-enable-primary-paste false"
)
"#;
        let path = PathBuf::from("/tmp/fake-autostart.lua");
        let list = parse_hypr_launches(src, &path);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name, "my-service");
        assert!(!list[0].enabled);
        assert_eq!(list[1].name, "v2rayN");
        assert!(list[1].enabled);
        assert_eq!(list[1].exec, "/opt/v2rayn-bin/v2rayN");
        assert!(list[2].enabled);
        assert!(list[2].exec.contains("gtk-enable-primary-paste"));

        let off = set_hypr_launch_enabled(src, 1, false).unwrap();
        let again = parse_hypr_launches(&off, &path);
        assert!(!again[1].enabled);
        assert!(again[1].enabled == false);
        let on = set_hypr_launch_enabled(&off, 1, true).unwrap();
        let restored = parse_hypr_launches(&on, &path);
        assert!(restored[1].enabled);
        assert!(restored[1].exec.contains("v2rayN"));
    }
}
