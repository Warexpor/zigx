pub fn percent(v: f32) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    format!("{:.0}%", v.max(0.0))
}

pub fn cpu_pct(v: f32) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let v = v.max(0.0);
    if v < 10.0 {
        format!("{v:.1}%")
    } else {
        format!("{v:.0}%")
    }
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < 4 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

pub fn rate(bps: f64) -> String {
    if !bps.is_finite() || bps < 0.0 {
        return "—".into();
    }
    format!("{}/s", bytes(bps.round() as u64))
}

pub fn disk_cell(read: Option<f64>, write: Option<f64>) -> String {
    match (read, write) {
        (None, None) => "—".into(),
        (a, b) => rate(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
    }
}

pub fn duration(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

pub fn freq_ghz(mhz: f32) -> String {
    if !mhz.is_finite() || mhz <= 0.0 {
        "—".into()
    } else {
        format!("{:.2} GHz", mhz / 1000.0)
    }
}

/// Estimated advance of one glyph. `tracking` is letter spacing in em.
/// Uppercase runs are wider than mixed case in every humanist sans.
fn char_w(size: f32, mono: bool, caps: bool, tracking: f32) -> f32 {
    let base = if mono {
        size * 0.60
    } else if caps {
        size * 0.66
    } else {
        size * 0.51
    };
    base + tracking * size
}

fn is_caps(s: &str) -> bool {
    let letters: Vec<char> = s.chars().filter(|c| c.is_alphabetic()).collect();
    !letters.is_empty() && letters.iter().all(|c| c.is_uppercase())
}

pub fn fit_t(s: &str, max_w: f32, size: f32, mono: bool, tracking: f32) -> String {
    let cw = char_w(size, mono, is_caps(s), tracking);
    if cw <= 0.0 || max_w <= 0.0 {
        return String::new();
    }
    let max_chars = (max_w / cw).floor() as usize;
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    if max_chars <= 1 {
        return "…".into();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

pub fn text_width_t(s: &str, size: f32, mono: bool, tracking: f32) -> f32 {
    let n = s.chars().count() as f32;
    // Tracking is added after every glyph but the last.
    (n * char_w(size, mono, is_caps(s), 0.0) + (n - 1.0).max(0.0) * tracking * size).max(0.0)
}

/// Nice axis ceiling (1, 2, 5 × 10^n).
pub fn nice_ceil(v: f32) -> f32 {
    if !v.is_finite() || v <= 0.0 {
        return 1.0;
    }
    let exp = v.log10().floor();
    let base = 10f32.powf(exp);
    let n = v / base;
    let nice = if n <= 1.0 {
        1.0
    } else if n <= 2.0 {
        2.0
    } else if n <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_and_rates() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(rate(1_048_576.0), "1.0 MB/s");
    }

    #[test]
    fn nice_ceil_steps() {
        assert_eq!(nice_ceil(0.0), 1.0);
        assert_eq!(nice_ceil(120.0), 200.0);
        assert_eq!(nice_ceil(900.0), 1000.0);
    }

    #[test]
    fn fit_ellipsizes() {
        let s = fit_t("systemd-journald", 40.0, 13.0, false, 0.0);
        assert!(s.ends_with('…'));
        assert!(s.chars().count() < "systemd-journald".chars().count());
    }

    #[test]
    fn tracking_widens_text() {
        let plain = text_width_t("MONITOR", 10.0, false, 0.0);
        let tracked = text_width_t("MONITOR", 10.0, false, 0.10);
        assert!(tracked > plain);
        assert!((tracked - plain - 6.0 * 1.0).abs() < 1e-4);
    }
}
