use std::sync::atomic::{AtomicBool, Ordering};

pub fn percent(v: f32) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    format!("{:.0}%", v.max(0.0))
}

pub fn cpu_pct(v: f32) -> String {
    percent(v)
}

static DECIMAL: AtomicBool = AtomicBool::new(false);

/// Byte sizes step by 1000 instead of 1024. A display preference, set once
/// per frame from the settings.
pub fn set_decimal(on: bool) {
    DECIMAL.store(on, Ordering::Relaxed);
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let step = if DECIMAL.load(Ordering::Relaxed) {
        1000.0
    } else {
        1024.0
    };
    let mut v = n as f64;
    let mut i = 0;
    while v >= step && i < 4 {
        v /= step;
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

/// Process disk share of current device throughput, 0..100.
pub fn disk_pct(read: Option<f64>, write: Option<f64>, total_bps: f64) -> String {
    match (read, write) {
        (None, None) => "—".into(),
        (a, b) => {
            let proc = a.unwrap_or(0.0) + b.unwrap_or(0.0);
            if !proc.is_finite() || proc < 0.0 {
                return "—".into();
            }
            if total_bps < 1.0 {
                return percent(0.0);
            }
            percent(((proc / total_bps) * 100.0).clamp(0.0, 100.0) as f32)
        }
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

/// Advance of one glyph in em, tuned to Adwaita/Inter-class humanist sans.
/// A flat 0.51em average was skewing layout: short words like "Flat" got
/// oversized cells, while "Grouped"/"System" ran short and sat off-center.
fn advance_em(c: char, mono: bool) -> f32 {
    if mono {
        return 0.60;
    }
    match c {
        'A' => 0.690,
        'B' => 0.654,
        'C' => 0.730,
        'D' => 0.722,
        'E' => 0.601,
        'F' => 0.590,
        'G' => 0.746,
        'H' => 0.743,
        'I' => 0.269,
        'J' => 0.571,
        'K' => 0.672,
        'L' => 0.565,
        'M' => 0.903,
        'N' => 0.753,
        'O' => 0.765,
        'P' => 0.639,
        'Q' => 0.765,
        'R' => 0.644,
        'S' => 0.642,
        'T' => 0.646,
        'U' => 0.744,
        'V' => 0.690,
        'W' => 0.985,
        'X' => 0.682,
        'Y' => 0.679,
        'Z' => 0.629,
        'a' => 0.562,
        'b' => 0.612,
        'c' => 0.571,
        'd' => 0.612,
        'e' => 0.583,
        'f' => 0.370,
        'g' => 0.613,
        'h' => 0.591,
        'i' => 0.242,
        'j' => 0.242,
        'k' => 0.549,
        'l' => 0.275,
        'm' => 0.876,
        'n' => 0.591,
        'o' => 0.600,
        'p' => 0.612,
        'q' => 0.612,
        'r' => 0.376,
        's' => 0.528,
        't' => 0.327,
        'u' => 0.591,
        'v' => 0.562,
        'w' => 0.818,
        'x' => 0.546,
        'y' => 0.562,
        'z' => 0.552,
        ' ' => 0.281,
        '0' => 0.631,
        '1' => 0.407,
        '2' => 0.610,
        '3' => 0.618,
        '4' => 0.646,
        '5' => 0.593,
        '6' => 0.620,
        '7' => 0.566,
        '8' => 0.619,
        '9' => 0.620,
        '.' | ':' => 0.288,
        '%' => 0.982,
        '/' => 0.360,
        '|' => 0.332,
        '-' => 0.460,
        '…' => 0.800,
        _ => 0.56,
    }
}

pub fn fit_t(s: &str, max_w: f32, size: f32, mono: bool, tracking: f32) -> String {
    if max_w <= 0.0 || size <= 0.0 {
        return String::new();
    }
    if text_width_t(s, size, mono, tracking) <= max_w {
        return s.to_string();
    }
    let ell = '…';
    let ell_w = advance_em(ell, mono) * size;
    if ell_w > max_w {
        return ell.to_string();
    }
    let mut out = String::new();
    let mut w = 0.0;
    for c in s.chars() {
        let next = advance_em(c, mono) * size + if out.is_empty() { 0.0 } else { tracking * size };
        if w + next + ell_w + if out.is_empty() { 0.0 } else { tracking * size } > max_w {
            break;
        }
        w += next;
        out.push(c);
    }
    if out.is_empty() {
        return ell.to_string();
    }
    out.push(ell);
    out
}

pub fn text_width_t(s: &str, size: f32, mono: bool, tracking: f32) -> f32 {
    let mut chars = s.chars().peekable();
    let mut w = 0.0;
    while let Some(c) = chars.next() {
        w += advance_em(c, mono) * size;
        if chars.peek().is_some() {
            w += tracking * size;
        }
    }
    w.max(0.0)
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
    fn cpu_pct_is_whole_numbers() {
        assert_eq!(cpu_pct(0.4), "0%");
        assert_eq!(cpu_pct(3.0), "3%");
        assert_eq!(cpu_pct(9.9), "10%");
        assert_eq!(cpu_pct(10.0), "10%");
        assert_eq!(cpu_pct(42.2), "42%");
        assert_eq!(percent(4.0), "4%");
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
