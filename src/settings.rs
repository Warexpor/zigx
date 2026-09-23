//! User preferences edited on the Settings page.
//!
//! Layout state that the app remembers on its own (splitter widths, the last
//! page, sort, density, zoom) stays in `ui.txt`. Everything a user sets on
//! purpose lives here and is written to `settings.txt`.

use std::fs;
use std::io;

use crate::model::Col;
use crate::persist::{atomic_write, config_dir};

/// A setting with a fixed list of values, shown as a segmented control.
pub trait Choice: Copy + PartialEq + 'static {
    const ALL: &'static [Self];
    /// Stable token written to `settings.txt`.
    fn key(self) -> &'static str;
    /// Segment label.
    fn label(self) -> &'static str;

    fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }
    fn from_key(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.key() == s)
    }
}

macro_rules! choice {
    ($name:ident { $($v:ident => $key:literal, $label:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($v),+ }

        impl Choice for $name {
            const ALL: &'static [Self] = &[$($name::$v),+];
            fn key(self) -> &'static str {
                match self { $($name::$v => $key),+ }
            }
            fn label(self) -> &'static str {
                match self { $($name::$v => $label),+ }
            }
        }
    };
}

choice!(Glass {
    Clear => "clear", "Clear",
    Frost => "glass", "Glass",
    Solid => "solid", "Solid",
});

choice!(Motion {
    Smooth => "smooth", "Smooth",
    Reduced => "reduced", "Reduced",
});

choice!(History {
    S30 => "30", "30 s",
    S60 => "60", "1 min",
    S120 => "120", "2 min",
});

choice!(Curve {
    Smooth => "smooth", "Smooth",
    Linear => "linear", "Linear",
});

choice!(Speed {
    Fast => "500", "0.5 s",
    Normal => "1000", "1 s",
    Slow => "2000", "2 s",
});

choice!(ProcCpu {
    Core => "core", "Per core",
    Machine => "machine", "Whole machine",
});

choice!(Units {
    Binary => "binary", "1024",
    Decimal => "decimal", "1000",
});

choice!(Temp {
    Celsius => "c", "°C",
    Fahrenheit => "f", "°F",
});

choice!(OpenOn {
    Last => "last", "Last page",
    Processes => "processes", "Processes",
    Performance => "performance", "Performance",
    Startup => "startup", "Startup",
});

impl Glass {
    /// Alpha of the window sheet over the compositor blur.
    pub fn alpha(self) -> u8 {
        match self {
            Glass::Clear => 140,
            Glass::Frost => 200,
            Glass::Solid => 255,
        }
    }
}

impl History {
    pub fn secs(self) -> u64 {
        match self {
            History::S30 => 30,
            History::S60 => 60,
            History::S120 => 120,
        }
    }
}

impl Speed {
    pub fn ms(self) -> u64 {
        match self {
            Speed::Fast => 500,
            Speed::Normal => 1000,
            Speed::Slow => 2000,
        }
    }
}

/// Process columns that can be hidden. Name, CPU and Memory always show.
pub const OPTIONAL_COLS: [Col; 5] = [Col::Gpu, Col::Disk, Col::Pid, Col::User, Col::Threads];

/// One control on the Settings page. The value carried with it in a hit is a
/// segment index for choices and 0 / 1 for switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Opt {
    Glass,
    Animations,
    Motion,
    Density,
    Heat,
    Readout,
    History,
    Curve,
    Fill,
    Grid,
    /// Segments are the `Speed` values plus a trailing Pause.
    Speed,
    ProcCpu,
    Units,
    Temp,
    Column(Col),
    Confirm,
    OpenOn,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub glass: Glass,
    /// Interface fades, glides and transitions. Graph playback follows `motion`.
    pub animations: bool,
    /// Graph playback: smooth scrolling or whole-sample steps.
    pub motion: Motion,
    /// Amber and red on hot values. Off keeps the whole app monochrome.
    pub heat: bool,
    /// CPU / memory / GPU readout in the title bar.
    pub readout: bool,
    pub history: History,
    pub curve: Curve,
    /// Wash under graph traces.
    pub fill: bool,
    pub grid: bool,
    pub speed: Speed,
    pub proc_cpu: ProcCpu,
    pub units: Units,
    pub temp: Temp,
    pub show_gpu: bool,
    pub show_disk: bool,
    pub show_pid: bool,
    pub show_user: bool,
    pub show_threads: bool,
    /// End task and Force kill ask for a second click.
    pub confirm: bool,
    pub open_on: OpenOn,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            glass: Glass::Frost,
            animations: true,
            motion: Motion::Smooth,
            heat: true,
            readout: true,
            history: History::S30,
            curve: Curve::Smooth,
            fill: true,
            grid: true,
            speed: Speed::Normal,
            proc_cpu: ProcCpu::Core,
            units: Units::Binary,
            temp: Temp::Celsius,
            show_gpu: true,
            show_disk: true,
            show_pid: true,
            show_user: true,
            show_threads: true,
            confirm: true,
            open_on: OpenOn::Last,
        }
    }
}

impl Settings {
    pub fn reduced(&self) -> bool {
        self.motion == Motion::Reduced
    }

    /// Samples spanned by a graph at the current history and speed.
    pub fn window(&self) -> usize {
        (self.history.secs() * 1000 / self.speed.ms()) as usize
    }

    pub fn shows(&self, col: Col) -> bool {
        match col {
            Col::Gpu => self.show_gpu,
            Col::Disk => self.show_disk,
            Col::Pid => self.show_pid,
            Col::User => self.show_user,
            Col::Threads => self.show_threads,
            Col::Name | Col::Cpu | Col::Memory => true,
        }
    }

    pub fn show_mut(&mut self, col: Col) -> Option<&mut bool> {
        Some(match col {
            Col::Gpu => &mut self.show_gpu,
            Col::Disk => &mut self.show_disk,
            Col::Pid => &mut self.show_pid,
            Col::User => &mut self.show_user,
            Col::Threads => &mut self.show_threads,
            Col::Name | Col::Cpu | Col::Memory => return None,
        })
    }

    pub fn temp(&self, c: u32) -> String {
        match self.temp {
            Temp::Celsius => format!("{c} °C"),
            Temp::Fahrenheit => format!("{:.0} °F", c as f32 * 1.8 + 32.0),
        }
    }

    fn parse(text: &str) -> Self {
        let mut s = Self::default();
        let on = |v: &str| matches!(v, "on" | "true" | "yes" | "1");
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            fn set<T: Choice>(slot: &mut T, v: &str) {
                if let Some(x) = T::from_key(v) {
                    *slot = x;
                }
            }
            match k {
                "glass" => set(&mut s.glass, v),
                "animations" => s.animations = on(v),
                "motion" => set(&mut s.motion, v),
                "heat" => s.heat = on(v),
                "readout" => s.readout = on(v),
                "history" => set(&mut s.history, v),
                "curve" => set(&mut s.curve, v),
                "fill" => s.fill = on(v),
                "grid" => s.grid = on(v),
                "speed" => set(&mut s.speed, v),
                "process_cpu" => set(&mut s.proc_cpu, v),
                "units" => set(&mut s.units, v),
                "temperature" => set(&mut s.temp, v),
                "columns" => {
                    for col in OPTIONAL_COLS {
                        if let Some(slot) = s.show_mut(col) {
                            *slot = v.split(',').any(|c| c.trim() == col_key(col));
                        }
                    }
                }
                "confirm" => s.confirm = on(v),
                "open_on" => set(&mut s.open_on, v),
                _ => {}
            }
        }
        s
    }

    fn render(&self) -> String {
        let flag = |b: bool| if b { "on" } else { "off" };
        let cols: Vec<&str> = OPTIONAL_COLS
            .iter()
            .filter(|c| self.shows(**c))
            .map(|c| col_key(*c))
            .collect();
        format!(
            "# ZIGX settings. Written by the Settings page; safe to edit by hand.\n\
             glass={}\nanimations={}\nmotion={}\nheat={}\nreadout={}\n\
             history={}\ncurve={}\nfill={}\ngrid={}\n\
             speed={}\nprocess_cpu={}\nunits={}\ntemperature={}\n\
             columns={}\nconfirm={}\nopen_on={}\n",
            self.glass.key(),
            flag(self.animations),
            self.motion.key(),
            flag(self.heat),
            flag(self.readout),
            self.history.key(),
            self.curve.key(),
            flag(self.fill),
            flag(self.grid),
            self.speed.key(),
            self.proc_cpu.key(),
            self.units.key(),
            self.temp.key(),
            cols.join(","),
            flag(self.confirm),
            self.open_on.key(),
        )
    }
}

fn col_key(col: Col) -> &'static str {
    match col {
        Col::Name => "name",
        Col::Cpu => "cpu",
        Col::Gpu => "gpu",
        Col::Memory => "memory",
        Col::Disk => "disk",
        Col::Pid => "pid",
        Col::User => "user",
        Col::Threads => "threads",
    }
}

pub fn settings_path() -> std::path::PathBuf {
    config_dir().join("settings.txt")
}

pub fn load_settings() -> Settings {
    fs::read_to_string(settings_path())
        .map(|t| Settings::parse(&t))
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) -> io::Result<()> {
    fs::create_dir_all(config_dir())?;
    atomic_write(&settings_path(), &s.render())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_field() {
        let s = Settings {
            glass: Glass::Solid,
            animations: false,
            motion: Motion::Reduced,
            heat: false,
            readout: false,
            history: History::S120,
            curve: Curve::Linear,
            fill: false,
            grid: false,
            speed: Speed::Fast,
            proc_cpu: ProcCpu::Machine,
            units: Units::Decimal,
            temp: Temp::Fahrenheit,
            show_gpu: false,
            show_disk: true,
            show_pid: false,
            show_user: true,
            show_threads: false,
            confirm: false,
            open_on: OpenOn::Performance,
        };
        assert_eq!(Settings::parse(&s.render()), s);
        assert_eq!(
            Settings::parse(&Settings::default().render()),
            Settings::default()
        );
    }

    #[test]
    fn unknown_or_broken_lines_keep_defaults() {
        let s = Settings::parse("glass=neon\nhistory\n# fill=off\nspeed=2000\n");
        assert_eq!(s.glass, Glass::Frost);
        assert!(s.fill);
        assert!(s.animations);
        assert_eq!(s.speed, Speed::Slow);
    }

    #[test]
    fn window_follows_history_and_speed() {
        let mut s = Settings::default();
        assert_eq!(s.window(), 30);
        s.history = History::S120;
        s.speed = Speed::Fast;
        assert_eq!(s.window(), 240);
    }
}
