use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::model::{Disk, Gpu, Net, Proc, Snap, HIST_CAP};

const PF_KTHREAD: u64 = 0x00200000;
/// Delay before the priming read so the window can open first.
const PRIME_MS: u64 = 50;
/// Gap after priming before the first real sample. Must exceed
/// [`MIN_DT_SECS`] so rates and history advance on that tick.
const FIRST_SAMPLE_MS: u64 = 220;
/// Shortest delta a rate is computed over; guards against divide-by-tiny.
const MIN_DT_SECS: f64 = 0.2;

/// Longest sleep between checks for a period change or pause.
const POLL_MS: u64 = 50;

pub struct Hub {
    snap: Arc<Mutex<Arc<Snap>>>,
    /// Sampling period in ms; 0 halts sampling.
    period: Arc<AtomicU64>,
}

impl Hub {
    pub fn load(&self) -> Arc<Snap> {
        self.snap.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Change the sampling period, or pause with 0. Takes effect within
    /// `POLL_MS`, without waiting out the old period.
    pub fn set_period(&self, ms: u64) {
        self.period.store(ms, Ordering::Relaxed);
    }
}

pub fn spawn(period_ms: u64, wake: impl Fn() + Send + 'static) -> Hub {
    let snap = Arc::new(Mutex::new(Arc::new(Snap::placeholder())));
    let slot = Arc::clone(&snap);
    let period = Arc::new(AtomicU64::new(period_ms));
    let shared = Arc::clone(&period);
    let hub = Hub { snap, period };
    thread::Builder::new()
        .name("zigx-sample".into())
        .spawn(move || {
            let mut engine = Engine::new();
            let mut ms = shared.load(Ordering::Relaxed);
            // Deadline clock: ticks stay on a fixed grid instead of drifting by
            // the sample cost. The first deadline is short so real numbers
            // replace the priming zeros quickly.
            let mut next = Instant::now() + Duration::from_millis(PRIME_MS);
            loop {
                loop {
                    let want = shared.load(Ordering::Relaxed);
                    let now = Instant::now();
                    if want != ms {
                        if ms == 0 {
                            // Resume: prime again so no rate spans the pause.
                            engine.reprime();
                            next = now;
                        } else if want != 0 {
                            next = next
                                .checked_sub(Duration::from_millis(ms))
                                .map_or(now, |last| last + Duration::from_millis(want))
                                .max(now);
                        }
                        ms = want;
                    }
                    if ms != 0 && now >= next {
                        break;
                    }
                    let wait = if ms == 0 {
                        Duration::from_millis(POLL_MS)
                    } else {
                        (next - now).min(Duration::from_millis(POLL_MS))
                    };
                    thread::sleep(wait);
                }
                let priming = !engine.primed;
                let snap = engine.tick(ms);
                {
                    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
                    *guard = Arc::new(snap);
                }
                wake();
                let now = Instant::now();
                let period = Duration::from_millis(ms);
                if priming {
                    next = now + Duration::from_millis(FIRST_SAMPLE_MS);
                } else {
                    next += period;
                    if next <= now {
                        // Fell behind (suspend, stall): re-anchor instead of bursting.
                        next = now + period;
                    }
                }
            }
        })
        .expect("sampler thread");
    hub
}

struct ProcPrev {
    ticks: u64,
    /// `/proc` starttime. A reused pid has a new one.
    start: u64,
    /// When `ticks` and the io counters were read. The next rate uses this,
    /// not the clock at the start of the scan: a long walk would otherwise
    /// shrink the divisor and report thousands of percent.
    at: Instant,
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
    io_denied: bool,
}

pub struct Engine {
    primed: bool,
    prev_at: Instant,
    clk_tck: f64,
    page_size: u64,
    uid: u32,
    cpu_model: String,
    prev_cpu: Vec<(u64, u64)>,
    prev_proc: HashMap<i32, ProcPrev>,
    prev_disk: HashMap<String, (u64, u64)>,
    prev_net: HashMap<String, (u64, u64)>,
    cpu_hist: VecDeque<f32>,
    cpu_per_hist: Vec<VecDeque<f32>>,
    mem_hist: VecDeque<f32>,
    swap_hist: VecDeque<f32>,
    disk_hist: HashMap<String, (VecDeque<f32>, VecDeque<f32>)>,
    net_hist: HashMap<String, (VecDeque<f32>, VecDeque<f32>)>,
    gpu_hist: Vec<VecDeque<f32>>,
    users: HashMap<u32, String>,
    /// Process title cache, keyed by pid and `starttime` so a recycled pid
    /// cannot keep the previous process's name.
    names: HashMap<i32, (u64, String)>,
    kthreads: u32,
    last_cpu_total: f32,
    last_cpu_per: Vec<f32>,
    /// Last published snapshot. Republished unchanged when a tick arrives too
    /// soon to form an honest rate, so counters are not eaten.
    last_snap: Option<Snap>,
    cached_proc_gpu: HashMap<i32, f32>,
    sample_avg: f32,
    hist_at: Instant,
    hist_seq: u64,
    /// Sampling period the rings were recorded at. A speed change drops them:
    /// the graphs treat every sample as one current period wide.
    hist_period: u64,
    nvml: Option<Nvml>,
    text: String,
    bytes: Vec<u8>,
    path: PathBuf,
}

impl Engine {
    pub fn new() -> Self {
        let clk = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Self {
            primed: false,
            prev_at: Instant::now(),
            clk_tck: if clk > 0 { clk as f64 } else { 100.0 },
            page_size: if page > 0 { page as u64 } else { 4096 },
            uid: unsafe { libc::geteuid() },
            cpu_model: read_cpu_model(),
            prev_cpu: Vec::new(),
            prev_proc: HashMap::new(),
            prev_disk: HashMap::new(),
            prev_net: HashMap::new(),
            cpu_hist: VecDeque::new(),
            cpu_per_hist: Vec::new(),
            mem_hist: VecDeque::new(),
            swap_hist: VecDeque::new(),
            disk_hist: HashMap::new(),
            net_hist: HashMap::new(),
            gpu_hist: Vec::new(),
            users: HashMap::new(),
            names: HashMap::new(),
            kthreads: 0,
            last_cpu_total: 0.0,
            last_cpu_per: Vec::new(),
            last_snap: None,
            cached_proc_gpu: HashMap::new(),
            sample_avg: 0.0,
            hist_at: Instant::now(),
            hist_seq: 0,
            hist_period: 0,
            nvml: Nvml::open(),
            text: String::new(),
            bytes: Vec::new(),
            path: PathBuf::new(),
        }
    }

    /// Treat the next tick as a first read: counters are refreshed but no
    /// rate or history sample is produced from the gap before it.
    pub fn reprime(&mut self) {
        self.primed = false;
    }

    pub fn tick(&mut self, period_ms: u64) -> Snap {
        self.align_period(period_ms);
        let started = Instant::now();
        let now = Instant::now();
        let dt = now.duration_since(self.prev_at).as_secs_f64();
        let min_dt = MIN_DT_SECS;
        // A tick sooner than MIN_DT cannot form an honest rate. Republish the
        // last snap and leave counters alone so the next full period still
        // spans every byte and CPU tick since the previous good sample.
        if self.primed && dt <= min_dt {
            if let Some(snap) = &self.last_snap {
                return snap.clone();
            }
        }
        let priming = !self.primed;
        let advance = self.primed && dt > min_dt;
        let cpu_lines = self.read_cpu_lines();
        let mem = self.read_mem();
        let disks_raw = self.read_disks();
        let (disk_free, disk_total) = read_disk_space();
        let nets_raw = self.read_nets();
        // Every reading, GPUs and the process table included, refreshes on the
        // same 1 s tick so the whole app changes in lockstep.
        let gpu_raw = self.read_gpus();
        self.cached_proc_gpu = self.read_proc_gpus();
        // cpu_lines[0] is the aggregate; the rest are one entry per core.
        let cores = cpu_lines.len().saturating_sub(1).max(1) as u32;
        let procs = self.read_procs(cores, advance, priming);

        let mut cpu_total = 0.0;
        let mut cpu_per = Vec::new();
        if advance {
            if let Some((idle0, total0)) = cpu_lines.first() {
                if let Some((pi, pt)) = self.prev_cpu.first() {
                    cpu_total = pct_busy(*idle0, *total0, *pi, *pt);
                }
            }
            for (idx, (idle, total)) in cpu_lines.iter().enumerate().skip(1) {
                let prev = self.prev_cpu.get(idx).copied().unwrap_or((*idle, *total));
                cpu_per.push(pct_busy(*idle, *total, prev.0, prev.1));
            }
            push_hist(&mut self.cpu_hist, cpu_total);
            if self.cpu_per_hist.len() != cpu_per.len() {
                self.cpu_per_hist = (0..cpu_per.len()).map(|_| VecDeque::new()).collect();
            }
            for (hist, v) in self.cpu_per_hist.iter_mut().zip(&cpu_per) {
                push_hist(hist, *v);
            }
            if mem.total > 0 {
                push_hist(&mut self.mem_hist, mem.used as f32 / mem.total as f32);
            }
            if mem.swap_total > 0 {
                push_hist(
                    &mut self.swap_hist,
                    mem.swap_used as f32 / mem.swap_total as f32,
                );
            }
            self.last_cpu_total = cpu_total;
            self.last_cpu_per = cpu_per.clone();
        } else {
            cpu_total = self.last_cpu_total;
            cpu_per = self.last_cpu_per.clone();
            if cpu_per.is_empty() && !cpu_lines.is_empty() {
                cpu_per = vec![0.0; cpu_lines.len().saturating_sub(1)];
            }
        }

        let disks = self.finish_disks(disks_raw, dt, advance, priming);
        let nets = self.finish_nets(nets_raw, dt, advance, priming);
        let gpus = self.finish_gpus(gpu_raw, advance);
        if advance {
            self.hist_at = now;
            self.hist_seq += 1;
            if period_ms != 0 {
                self.hist_period = period_ms;
            }
        }

        // Only move the rate baselines when this tick produced rates (or is
        // the priming read). A short fall-through after a speed change must
        // not eat the counters the next full period will need.
        if advance || priming {
            self.prev_cpu = cpu_lines.iter().map(|(i, t)| (*i, *t)).collect();
            self.prev_at = now;
        }
        self.primed = true;

        let freq = read_freqs(cpu_per.len());
        let (uptime, load) = read_uptime_load();
        let proc_count = procs.len() as u32;
        let thread_count = procs.iter().map(|p| p.threads).sum();
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        self.sample_avg = if self.sample_avg == 0.0 {
            ms
        } else {
            self.sample_avg * 0.8 + ms * 0.2
        };

        let snap = Snap {
            cpu_model: self.cpu_model.clone(),
            cpu_total,
            cpu_per,
            cpu_freq_mhz: freq,
            cpu_hist: dump(&self.cpu_hist),
            cpu_per_hist: self.cpu_per_hist.iter().map(dump).collect(),
            mem_total: mem.total,
            mem_used: mem.used,
            mem_available: mem.available,
            mem_cached: mem.cached,
            mem_buffers: mem.buffers,
            swap_total: mem.swap_total,
            swap_used: mem.swap_used,
            mem_hist: dump(&self.mem_hist),
            swap_hist: dump(&self.swap_hist),
            disk_free,
            disk_total,
            disks,
            nets,
            gpus,
            procs,
            proc_count,
            thread_count,
            kthreads: self.kthreads,
            uptime_secs: uptime,
            load,
            sample_ms: self.sample_avg,
            hist_at: self.hist_at,
            hist_seq: self.hist_seq,
        };
        self.last_snap = Some(snap.clone());
        snap
    }

    fn read_cpu_lines(&mut self) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        if !slurp(Path::new("/proc/stat"), &mut self.text) {
            return out;
        }
        for line in self.text.lines() {
            if let Some(row) = parse_cpu_line(line) {
                out.push(row);
            } else if !out.is_empty() {
                break;
            }
        }
        out
    }

    fn read_mem(&mut self) -> MemRaw {
        let mut m = MemRaw::default();
        if !slurp(Path::new("/proc/meminfo"), &mut self.text) {
            return m;
        }
        for line in self.text.lines() {
            let mut parts = line.split_whitespace();
            let Some(key) = parts.next() else { continue };
            let Some(val) = parts.next().and_then(|v| v.parse::<u64>().ok()) else {
                continue;
            };
            let bytes = val * 1024;
            match key {
                "MemTotal:" => m.total = bytes,
                "MemAvailable:" => m.available = bytes,
                "Cached:" => m.cached = bytes,
                "Buffers:" => m.buffers = bytes,
                "SwapTotal:" => m.swap_total = bytes,
                "SwapFree:" => m.swap_free = bytes,
                _ => {}
            }
        }
        m.used = m.total.saturating_sub(m.available);
        m.swap_used = m.swap_total.saturating_sub(m.swap_free);
        m
    }

    fn read_disks(&mut self) -> Vec<(String, u64, u64)> {
        let mut out = Vec::new();
        if !slurp(Path::new("/proc/diskstats"), &mut self.text) {
            return out;
        }
        for line in self.text.lines() {
            if let Some(row) = parse_disk_line(line) {
                out.push(row);
            }
        }
        out
    }

    fn read_nets(&mut self) -> Vec<(String, u64, u64)> {
        let mut out = Vec::new();
        if !slurp(Path::new("/proc/net/dev"), &mut self.text) {
            return out;
        }
        for line in self.text.lines().skip(2) {
            if let Some(row) = parse_net_line(line) {
                out.push(row);
            }
        }
        out
    }

    fn read_gpus(&mut self) -> Vec<GpuRaw> {
        if let Some(nv) = self.nvml.as_ref() {
            let rows = nv.sample();
            if !rows.is_empty() {
                return rows;
            }
        }
        sample_sysfs_gpus()
    }

    fn read_proc_gpus(&self) -> HashMap<i32, f32> {
        if let Some(nv) = self.nvml.as_ref() {
            let map = nv.process_util();
            if !map.is_empty() {
                return map;
            }
        }
        HashMap::new()
    }

    fn read_procs(&mut self, cores: u32, advance: bool, priming: bool) -> Vec<Proc> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut kthreads = 0u32;
        let rd = match fs::read_dir("/proc") {
            Ok(rd) => rd,
            Err(_) => {
                self.kthreads = 0;
                return out;
            }
        };
        for ent in rd.flatten() {
            let Some(pid) = ent.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
                continue;
            };
            if pid <= 0 {
                continue;
            }
            let uid = ent.metadata().map(|m| m.uid()).unwrap_or(0);
            if !self.read_proc_file(pid, "stat") {
                continue;
            }
            let stat_text = String::from_utf8_lossy(&self.bytes).into_owned();
            let Some(stat) = parse_proc_stat(&stat_text) else {
                continue;
            };
            if stat.flags & PF_KTHREAD != 0 {
                kthreads += 1;
                continue;
            }
            seen.insert(pid);
            let ticks = stat.utime + stat.stime;
            // A recycled pid must not inherit the previous process's counters.
            let (prev_ticks, prev_at, prev_read, prev_write, prev_denied) =
                match self.prev_proc.get(&pid) {
                    Some(p) if p.start == stat.start => (
                        Some(p.ticks),
                        Some(p.at),
                        p.read_bytes,
                        p.write_bytes,
                        p.io_denied,
                    ),
                    _ => (None, None, None, None, false),
                };
            let (read_bps, write_bps, read_bytes, write_bytes, io_denied) =
                if !advance && !priming {
                    // Short fall-through: leave baselines alone.
                    (None, None, prev_read, prev_write, prev_denied)
                } else if prev_denied {
                    (None, None, None, None, true)
                } else {
                    self.proc_io(pid, prev_read, prev_write, prev_at)
                };
            let at = Instant::now();
            let dt = prev_at
                .map(|t| at.saturating_duration_since(t).as_secs_f64())
                .unwrap_or(0.0);
            let cpu = if advance {
                match prev_ticks {
                    Some(was) if ticks >= was => proc_cpu_pct(ticks - was, self.clk_tck, dt, cores),
                    _ => 0.0,
                }
            } else {
                0.0
            };
            if advance || priming {
                self.prev_proc.insert(
                    pid,
                    ProcPrev {
                        ticks,
                        start: stat.start,
                        at,
                        read_bytes,
                        write_bytes,
                        io_denied,
                    },
                );
            }
            let name = self.proc_name(pid, &stat.comm, stat.start);
            let mem_pages = if self.read_proc_file(pid, "statm") {
                parse_statm_private(&String::from_utf8_lossy(&self.bytes))
            } else {
                None
            }
            .unwrap_or(stat.rss_pages);
            out.push(Proc {
                pid,
                uid,
                user: self.user_name(uid),
                name,
                cpu,
                gpu: self.cached_proc_gpu.get(&pid).copied().unwrap_or(0.0),
                mem: mem_pages * self.page_size,
                read_bps,
                write_bps,
                threads: stat.threads.max(1),
                is_user: uid == self.uid,
                stopped: stat.stopped,
            });
        }
        if advance || priming {
            self.prev_proc.retain(|pid, _| seen.contains(pid));
            self.names.retain(|pid, _| seen.contains(pid));
        }
        self.kthreads = kthreads;
        out
    }

    fn proc_io(
        &mut self,
        pid: i32,
        prev_read: Option<u64>,
        prev_write: Option<u64>,
        prev_at: Option<Instant>,
    ) -> (Option<f64>, Option<f64>, Option<u64>, Option<u64>, bool) {
        if !self.read_proc_file(pid, "io") {
            return (None, None, None, None, true);
        }
        let text = String::from_utf8_lossy(&self.bytes);
        let mut read_b = None;
        let mut write_b = None;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("read_bytes:") {
                read_b = v.trim().parse().ok();
            } else if let Some(v) = line.strip_prefix("write_bytes:") {
                write_b = v.trim().parse().ok();
            }
        }
        let dt = prev_at.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
        if !self.primed || dt <= MIN_DT_SECS {
            return (None, None, read_b, write_b, false);
        }
        let rate = |now: Option<u64>, was: Option<u64>| -> Option<f64> {
            Some((now? as f64 - was? as f64).max(0.0) / dt)
        };
        (
            rate(read_b, prev_read),
            rate(write_b, prev_write),
            read_b,
            write_b,
            false,
        )
    }

    fn proc_name(&mut self, pid: i32, comm: &str, start: u64) -> String {
        if let Some((born, name)) = self.names.get(&pid) {
            if *born == start {
                return name.clone();
            }
        }
        let name = self.read_proc_name(pid, comm);
        self.names.insert(pid, (start, name.clone()));
        name
    }

    /// Drop graph rings when the sample interval changes. Old samples are not
    /// one current period apart, so leaving them in would compress or stretch
    /// the time axis.
    fn align_period(&mut self, period_ms: u64) {
        if period_ms == 0 || self.hist_period == 0 || period_ms == self.hist_period {
            return;
        }
        self.cpu_hist.clear();
        self.cpu_per_hist.clear();
        self.mem_hist.clear();
        self.swap_hist.clear();
        self.disk_hist.clear();
        self.net_hist.clear();
        self.gpu_hist.clear();
        self.hist_seq = 0;
        self.hist_period = period_ms;
        self.last_snap = None;
    }

    fn read_proc_name(&mut self, pid: i32, comm: &str) -> String {
        if self.read_proc_prefix(pid, "cmdline", 240) {
            let bytes = &self.bytes;
            if bytes.iter().any(|&b| b != 0) {
                let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                let raw = String::from_utf8_lossy(&bytes[..end]);
                let base = display_name(raw.as_ref());
                if !base.is_empty() {
                    return base;
                }
            }
        }
        comm.chars().take(64).collect()
    }

    fn read_proc_file(&mut self, pid: i32, leaf: &str) -> bool {
        self.path.clear();
        self.path.push("/proc");
        self.path.push(pid.to_string());
        self.path.push(leaf);
        self.bytes.clear();
        let mut f = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        f.read_to_end(&mut self.bytes).is_ok()
    }

    fn read_proc_prefix(&mut self, pid: i32, leaf: &str, max: usize) -> bool {
        self.path.clear();
        self.path.push("/proc");
        self.path.push(pid.to_string());
        self.path.push(leaf);
        self.bytes.clear();
        let mut f = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let mut tmp = [0u8; 256];
        let cap = max.min(tmp.len());
        let n = f.read(&mut tmp[..cap]).unwrap_or(0);
        self.bytes.extend_from_slice(&tmp[..n]);
        n > 0
    }

    fn user_name(&mut self, uid: u32) -> String {
        if let Some(name) = self.users.get(&uid) {
            return name.clone();
        }
        let name = lookup_user(uid);
        self.users.insert(uid, name.clone());
        name
    }

    fn finish_disks(
        &mut self,
        raw: Vec<(String, u64, u64)>,
        dt: f64,
        advance: bool,
        priming: bool,
    ) -> Vec<Disk> {
        let mut out = Vec::new();
        let mut keep = HashSet::new();
        for (name, sectors_r, sectors_w) in raw {
            keep.insert(name.clone());
            let (read_bps, write_bps) = if advance {
                let rates = if let Some((pr, pw)) = self.prev_disk.get(&name) {
                    (
                        (sectors_r.saturating_sub(*pr) as f64) * 512.0 / dt,
                        (sectors_w.saturating_sub(*pw) as f64) * 512.0 / dt,
                    )
                } else {
                    (0.0, 0.0)
                };
                self.prev_disk.insert(name.clone(), (sectors_r, sectors_w));
                let hist = self
                    .disk_hist
                    .entry(name.clone())
                    .or_insert_with(|| (VecDeque::new(), VecDeque::new()));
                push_hist(&mut hist.0, rates.0 as f32);
                push_hist(&mut hist.1, rates.1 as f32);
                rates
            } else if priming {
                self.prev_disk.insert(name.clone(), (sectors_r, sectors_w));
                self.disk_hist
                    .entry(name.clone())
                    .or_insert_with(|| (VecDeque::new(), VecDeque::new()));
                (0.0, 0.0)
            } else {
                let hist = self
                    .disk_hist
                    .entry(name.clone())
                    .or_insert_with(|| (VecDeque::new(), VecDeque::new()));
                (
                    hist.0.back().copied().unwrap_or(0.0) as f64,
                    hist.1.back().copied().unwrap_or(0.0) as f64,
                )
            };
            let hist = self.disk_hist.get(&name).unwrap();
            out.push(Disk {
                name,
                read_bps,
                write_bps,
                read_hist: dump(&hist.0),
                write_hist: dump(&hist.1),
            });
        }
        if advance || priming {
            self.prev_disk.retain(|k, _| keep.contains(k));
            self.disk_hist.retain(|k, _| keep.contains(k));
        }
        out
    }

    fn finish_nets(
        &mut self,
        raw: Vec<(String, u64, u64)>,
        dt: f64,
        advance: bool,
        priming: bool,
    ) -> Vec<Net> {
        let mut out = Vec::new();
        let mut keep = HashSet::new();
        for (name, rx, tx) in raw {
            keep.insert(name.clone());
            let (rx_bps, tx_bps) = if advance {
                let rates = if let Some((pr, pt)) = self.prev_net.get(&name) {
                    (
                        rx.saturating_sub(*pr) as f64 / dt,
                        tx.saturating_sub(*pt) as f64 / dt,
                    )
                } else {
                    (0.0, 0.0)
                };
                self.prev_net.insert(name.clone(), (rx, tx));
                let hist = self
                    .net_hist
                    .entry(name.clone())
                    .or_insert_with(|| (VecDeque::new(), VecDeque::new()));
                push_hist(&mut hist.0, rates.0 as f32);
                push_hist(&mut hist.1, rates.1 as f32);
                rates
            } else if priming {
                self.prev_net.insert(name.clone(), (rx, tx));
                self.net_hist
                    .entry(name.clone())
                    .or_insert_with(|| (VecDeque::new(), VecDeque::new()));
                (0.0, 0.0)
            } else {
                let hist = self
                    .net_hist
                    .entry(name.clone())
                    .or_insert_with(|| (VecDeque::new(), VecDeque::new()));
                (
                    hist.0.back().copied().unwrap_or(0.0) as f64,
                    hist.1.back().copied().unwrap_or(0.0) as f64,
                )
            };
            let hist = self.net_hist.get(&name).unwrap();
            out.push(Net {
                name,
                rx_bps,
                tx_bps,
                rx_hist: dump(&hist.0),
                tx_hist: dump(&hist.1),
            });
        }
        if advance || priming {
            self.prev_net.retain(|k, _| keep.contains(k));
            self.net_hist.retain(|k, _| keep.contains(k));
        }
        out
    }

    fn finish_gpus(&mut self, raw: Vec<GpuRaw>, advance: bool) -> Vec<Gpu> {
        if self.gpu_hist.len() != raw.len() {
            self.gpu_hist = (0..raw.len()).map(|_| VecDeque::new()).collect();
        }
        raw.into_iter()
            .enumerate()
            .map(|(i, g)| {
                if advance {
                    // Always push so util_hist stays aligned with hist_seq even
                    // when a single NVML/sysfs read misses.
                    let u = g.util.unwrap_or_else(|| {
                        self.gpu_hist[i].back().copied().unwrap_or(0.0)
                    });
                    push_hist(&mut self.gpu_hist[i], u);
                }
                Gpu {
                    name: g.name,
                    util: g.util,
                    mem_used: g.mem_used,
                    mem_total: g.mem_total,
                    temp_c: g.temp_c,
                    power_w: g.power_w,
                    clk_core: g.clk_core,
                    clk_mem: g.clk_mem,
                    enc: g.enc,
                    dec: g.dec,
                    integrated: g.integrated,
                    util_hist: dump(&self.gpu_hist[i]),
                }
            })
            .collect()
    }
}

#[derive(Default)]
struct MemRaw {
    total: u64,
    available: u64,
    cached: u64,
    buffers: u64,
    swap_total: u64,
    swap_free: u64,
    used: u64,
    swap_used: u64,
}

#[derive(Clone)]
struct GpuRaw {
    name: String,
    util: Option<f32>,
    mem_used: u64,
    mem_total: u64,
    temp_c: Option<u32>,
    power_w: Option<f32>,
    clk_core: Option<u32>,
    clk_mem: Option<u32>,
    enc: Option<u32>,
    dec: Option<u32>,
    integrated: bool,
}

struct ParsedStat {
    comm: String,
    stopped: bool,
    flags: u64,
    utime: u64,
    stime: u64,
    threads: u32,
    /// Clock ticks since boot when the process started.
    start: u64,
    rss_pages: u64,
}

fn push_hist(h: &mut VecDeque<f32>, v: f32) {
    h.push_back(v);
    while h.len() > HIST_CAP {
        h.pop_front();
    }
}

fn dump(h: &VecDeque<f32>) -> Vec<f32> {
    h.iter().copied().collect()
}

/// Percent of one core. `cores` caps it at the whole machine: a process
/// cannot have used more CPU than every core running flat out. Per-core mode
/// still shows past 100% when more than one core is busy.
fn proc_cpu_pct(delta_ticks: u64, clk_tck: f64, dt: f64, cores: u32) -> f32 {
    if dt <= MIN_DT_SECS || clk_tck <= 0.0 {
        return 0.0;
    }
    let pct = delta_ticks as f64 / clk_tck / dt * 100.0;
    if !pct.is_finite() {
        return 0.0;
    }
    let cap = cores.max(1) as f64 * 100.0;
    pct.clamp(0.0, cap) as f32
}

fn pct_busy(idle: u64, total: u64, prev_idle: u64, prev_total: u64) -> f32 {
    let dt = total.saturating_sub(prev_total);
    if dt == 0 {
        return 0.0;
    }
    let idle_d = idle.saturating_sub(prev_idle);
    ((dt - idle_d) as f64 / dt as f64 * 100.0).clamp(0.0, 100.0) as f32
}

fn slurp(path: &Path, buf: &mut String) -> bool {
    buf.clear();
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    f.read_to_string(buf).is_ok()
}

/// Chrome rewrites cmdline into one field: "/path/exe --type=...".
/// Cut at " --" before the file name so directories with spaces survive.
fn display_name(field: &str) -> String {
    let exe = field.split(" --").next().unwrap_or(field);
    let base = exe.rsplit('/').next().unwrap_or(exe).trim();
    base.chars().take(64).collect()
}

fn parse_cpu_line(line: &str) -> Option<(u64, u64)> {
    let mut it = line.split_whitespace();
    let name = it.next()?;
    if name == "cpu" || (name.starts_with("cpu") && name.as_bytes().get(3)?.is_ascii_digit()) {
        let nums: Vec<u64> = it.take(8).filter_map(|s| s.parse().ok()).collect();
        if nums.len() < 4 {
            return None;
        }
        let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
        let total: u64 = nums.iter().sum();
        return Some((idle, total));
    }
    None
}

fn parse_disk_line(line: &str) -> Option<(String, u64, u64)> {
    let mut it = line.split_whitespace();
    let _major = it.next()?;
    let _minor = it.next()?;
    let name = it.next()?;
    if !is_physical_disk(name) {
        return None;
    }
    let mut nums = [0u64; 8];
    for slot in &mut nums {
        *slot = it.next()?.parse().ok()?;
    }
    // reads completed, merged, sectors read, ms, writes, merged, sectors written
    Some((name.to_string(), nums[2], nums[6]))
}

/// Free and total bytes across distinct mounted local filesystems.
///
/// Walks `/proc/mounts`, keeps `/dev/*` sources (skipping loop/ram/zram), and
/// dedupes by the canonical block device path. Btrfs subvolumes of one volume
/// share a source (`/dev/mapper/root`) but get distinct `st_dev` values, so
/// device identity — not `st_dev` — is what keeps capacity from being counted
/// four times.
fn read_disk_space() -> (u64, u64) {
    let Ok(text) = fs::read_to_string("/proc/mounts") else {
        return (0, 0);
    };
    let mut seen = HashSet::new();
    let mut free = 0u64;
    let mut total = 0u64;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(src) = parts.next() else { continue };
        let Some(dst) = parts.next() else { continue };
        if !is_local_block_source(src) {
            continue;
        }
        let key = fs::canonicalize(src)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| src.to_string());
        if !seen.insert(key) {
            continue;
        }
        let mount = unescape_mount(dst);
        let Some((avail, blocks)) = path_statvfs(&mount) else {
            continue;
        };
        if blocks == 0 {
            continue;
        }
        free = free.saturating_add(avail);
        total = total.saturating_add(blocks);
    }
    (free, total)
}

fn is_local_block_source(src: &str) -> bool {
    let Some(rest) = src.strip_prefix("/dev/") else {
        return false;
    };
    let name = rest.rsplit('/').next().unwrap_or(rest);
    !(name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("zram")
        || name.starts_with("fd"))
}

fn unescape_mount(raw: &str) -> String {
    if !raw.contains('\\') {
        return raw.to_string();
    }
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let oct = &raw[i + 1..i + 4];
            if let Ok(v) = u8::from_str_radix(oct, 8) {
                out.push(v as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn path_statvfs(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut vfs = unsafe { std::mem::zeroed::<libc::statvfs>() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut vfs) } != 0 {
        return None;
    }
    let fr = vfs.f_frsize as u64;
    if fr == 0 {
        return None;
    }
    Some((vfs.f_bavail as u64 * fr, vfs.f_blocks as u64 * fr))
}

fn is_physical_disk(name: &str) -> bool {
    if name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("dm-")
        || name.starts_with("zram")
        || name.starts_with("sr")
        || name.starts_with("fd")
    {
        return false;
    }
    if let Some(rest) = name.strip_prefix("nvme") {
        return rest.contains('n') && !rest.contains('p');
    }
    if name.starts_with("sd") || name.starts_with("hd") || name.starts_with("vd") {
        return !name.chars().last().is_some_and(|c| c.is_ascii_digit());
    }
    if name.starts_with("mmcblk") {
        return !name.contains('p');
    }
    false
}

fn parse_net_line(line: &str) -> Option<(String, u64, u64)> {
    let (name, rest) = line.split_once(':')?;
    let name = name.trim();
    if name.is_empty() || name == "lo" {
        return None;
    }
    let nums: Vec<u64> = rest
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    if nums.len() < 9 {
        return None;
    }
    Some((name.to_string(), nums[0], nums[8]))
}

fn parse_proc_stat(s: &str) -> Option<ParsedStat> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    if close < open {
        return None;
    }
    let comm = s[open + 1..close].to_string();
    let rest: Vec<&str> = s[close + 1..].split_whitespace().collect();
    let num = |i: usize| rest.get(i)?.parse::<u64>().ok();
    Some(ParsedStat {
        comm,
        stopped: rest.first() == Some(&"T"),
        flags: num(6)?,
        utime: num(11)?,
        stime: num(12)?,
        threads: num(17)? as u32,
        start: num(19)?,
        rss_pages: num(21)?,
    })
}

/// Resident pages minus file-backed and shmem pages (`RssAnon`). Plain RSS
/// counts every shared library and shared buffer once per process, so summing
/// it over a multi-process program (browsers, Electron) overstates RAM badly.
fn parse_statm_private(s: &str) -> Option<u64> {
    let mut it = s.split_whitespace().skip(1);
    let resident: u64 = it.next()?.parse().ok()?;
    let shared: u64 = it.next()?.parse().ok()?;
    Some(resident.saturating_sub(shared))
}

fn read_cpu_model() -> String {
    let Ok(text) = fs::read_to_string("/proc/cpuinfo") else {
        return "CPU".into();
    };
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("model name") {
            if let Some((_, name)) = v.split_once(':') {
                return name.trim().to_string();
            }
        }
    }
    "CPU".into()
}

fn read_freqs(n: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let path = format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq");
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(khz) = text.trim().parse::<f32>() {
                out.push(khz / 1000.0);
                continue;
            }
        }
        out.push(0.0);
    }
    out
}

fn read_uptime_load() -> (u64, [f32; 3]) {
    let uptime = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|t| t.split_whitespace().next()?.parse::<f64>().ok())
        .map(|s| s as u64)
        .unwrap_or(0);
    let mut load = [0.0; 3];
    if let Ok(text) = fs::read_to_string("/proc/loadavg") {
        for (slot, part) in load.iter_mut().zip(text.split_whitespace()) {
            *slot = part.parse().unwrap_or(0.0);
        }
    }
    (uptime, load)
}

fn lookup_user(uid: u32) -> String {
    // NSS can need more than a page (ERANGE). A short buffer used to be cached
    // as the numeric uid for the rest of the session.
    let mut buf_len = 4096usize;
    for _ in 0..6 {
        let mut pwd = unsafe { std::mem::zeroed::<libc::passwd>() };
        let mut buf = vec![0u8; buf_len];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = unsafe {
            libc::getpwuid_r(
                uid,
                &mut pwd,
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE {
            buf_len = buf_len.saturating_mul(2);
            continue;
        }
        if rc == 0 && !result.is_null() && !pwd.pw_name.is_null() {
            let c = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) };
            if let Ok(s) = c.to_str() {
                return s.to_string();
            }
        }
        break;
    }
    uid.to_string()
}

/// Newest SM sample per pid. `samples` is one device's ring.
fn accumulate_proc_utils(map: &mut HashMap<i32, f32>, samples: &[NvmlProcSample]) {
    let mut latest: HashMap<i32, (u64, f32)> = HashMap::new();
    for sample in samples {
        if sample.pid == 0 {
            continue;
        }
        let pid = sample.pid as i32;
        match latest.get(&pid) {
            Some((ts, _)) if *ts >= sample.time_stamp => {}
            _ => {
                latest.insert(pid, (sample.time_stamp, sample.sm_util as f32));
            }
        }
    }
    for (pid, (_, util)) in latest {
        *map.entry(pid).or_insert(0.0) += util;
    }
}

fn read_int(path: &Path) -> Option<u64> {
    let text = fs::read_to_string(path).ok()?;
    text.trim().parse().ok()
}

fn sample_sysfs_gpus() -> Vec<GpuRaw> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir("/sys/class/drm") else {
        return out;
    };
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let device = ent.path().join("device");
        let vendor = fs::read_to_string(device.join("vendor"))
            .unwrap_or_default()
            .trim()
            .to_string();
        match vendor.as_str() {
            "0x1002" | "0x1022" => out.push(sample_amd(&device)),
            "0x8086" => out.push(sample_intel(&ent.path(), &device)),
            "0x10de" => out.push(GpuRaw {
                name: "NVIDIA GPU".into(),
                util: None,
                mem_used: 0,
                mem_total: 0,
                temp_c: None,
                power_w: None,
                clk_core: None,
                clk_mem: None,
                enc: None,
                dec: None,
                integrated: false,
            }),
            _ => {}
        }
    }
    out
}

fn sample_amd(device: &Path) -> GpuRaw {
    let util = read_int(&device.join("gpu_busy_percent")).map(|v| v as f32);
    let mem_total = read_int(&device.join("mem_info_vram_total")).unwrap_or(0);
    let mem_used = read_int(&device.join("mem_info_vram_used")).unwrap_or(0);
    let vcn = read_int(&device.join("vcn_busy_percent")).map(|v| v as u32);
    let hwmon = first_hwmon(device);
    let (temp_c, power_w, clk_core) = hwmon_stats(hwmon.as_deref());
    let name = fs::read_to_string(device.join("product_name"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "AMD GPU".into());
    GpuRaw {
        name,
        util,
        mem_used,
        mem_total,
        temp_c,
        power_w,
        clk_core,
        clk_mem: None,
        enc: vcn,
        dec: vcn,
        integrated: mem_total > 0 && mem_total < 4 * 1024 * 1024 * 1024,
    }
}

fn sample_intel(card: &Path, device: &Path) -> GpuRaw {
    let hwmon = first_hwmon(device);
    let (temp_c, power_w, mut clk_core) = hwmon_stats(hwmon.as_deref());
    if clk_core.is_none() {
        for rel in ["gt_cur_freq_mhz", "device/gt_cur_freq_mhz"] {
            if let Some(v) = read_int(&card.join(rel)) {
                clk_core = Some(v as u32);
                break;
            }
        }
    }
    GpuRaw {
        name: "Intel GPU".into(),
        util: None,
        mem_used: 0,
        mem_total: 0,
        temp_c,
        power_w,
        clk_core,
        clk_mem: None,
        enc: None,
        dec: None,
        integrated: true,
    }
}

fn first_hwmon(device: &Path) -> Option<PathBuf> {
    let dir = device.join("hwmon");
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).next()
}

fn hwmon_stats(hwmon: Option<&Path>) -> (Option<u32>, Option<f32>, Option<u32>) {
    let Some(hw) = hwmon else {
        return (None, None, None);
    };
    let temp = read_int(&hw.join("temp1_input")).map(|v| (v / 1000) as u32);
    let power = read_int(&hw.join("power1_average"))
        .or_else(|| read_int(&hw.join("power1_input")))
        .map(|v| v as f32 / 1_000_000.0);
    let clk = read_int(&hw.join("freq1_input")).map(|v| (v / 1_000_000) as u32);
    (temp, power, clk)
}

struct Nvml {
    _lib: libloading::Library,
    devices: Vec<*mut std::ffi::c_void>,
    util: unsafe extern "C" fn(*mut std::ffi::c_void, *mut NvmlUtil) -> i32,
    mem: unsafe extern "C" fn(*mut std::ffi::c_void, *mut NvmlMem) -> i32,
    temp: unsafe extern "C" fn(*mut std::ffi::c_void, i32, *mut u32) -> i32,
    power: unsafe extern "C" fn(*mut std::ffi::c_void, *mut u32) -> i32,
    clock: unsafe extern "C" fn(*mut std::ffi::c_void, i32, *mut u32) -> i32,
    enc: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut u32, *mut u32) -> i32>,
    dec: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut u32, *mut u32) -> i32>,
    proc_util: Option<
        unsafe extern "C" fn(*mut std::ffi::c_void, *mut NvmlProcSample, *mut u32, u64) -> i32,
    >,
    names: Vec<String>,
}

#[repr(C)]
struct NvmlUtil {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
struct NvmlMem {
    total: u64,
    free: u64,
    used: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NvmlProcSample {
    pid: u32,
    time_stamp: u64,
    sm_util: u32,
    mem_util: u32,
    enc_util: u32,
    dec_util: u32,
}

unsafe impl Send for Nvml {}

impl Nvml {
    fn open() -> Option<Self> {
        unsafe {
            let lib = libloading::Library::new("libnvidia-ml.so.1").ok()?;
            let init_v2: libloading::Symbol<unsafe extern "C" fn() -> i32> =
                lib.get(b"nvmlInit_v2\0").ok()?;
            if init_v2() != 0 {
                return None;
            }
            let get_count: libloading::Symbol<unsafe extern "C" fn(*mut u32) -> i32> =
                lib.get(b"nvmlDeviceGetCount_v2\0").ok()?;
            let mut count = 0u32;
            if get_count(&mut count) != 0 || count == 0 {
                return None;
            }
            let get_handle: libloading::Symbol<
                unsafe extern "C" fn(u32, *mut *mut std::ffi::c_void) -> i32,
            > = lib.get(b"nvmlDeviceGetHandleByIndex_v2\0").ok()?;
            let get_name: libloading::Symbol<
                unsafe extern "C" fn(*mut std::ffi::c_void, *mut libc::c_char, u32) -> i32,
            > = lib.get(b"nvmlDeviceGetName\0").ok()?;
            let mut devices = Vec::new();
            let mut names = Vec::new();
            for i in 0..count {
                let mut dev = std::ptr::null_mut();
                if get_handle(i, &mut dev) != 0 {
                    continue;
                }
                let mut buf = [0u8; 96];
                let _ = get_name(dev, buf.as_mut_ptr() as *mut libc::c_char, buf.len() as u32);
                let name = std::ffi::CStr::from_ptr(buf.as_ptr() as *const libc::c_char)
                    .to_string_lossy()
                    .trim()
                    .to_string();
                devices.push(dev);
                names.push(if name.is_empty() {
                    format!("NVIDIA GPU {i}")
                } else {
                    name
                });
            }
            if devices.is_empty() {
                return None;
            }
            Some(Self {
                util: *lib.get(b"nvmlDeviceGetUtilizationRates\0").ok()?,
                mem: *lib.get(b"nvmlDeviceGetMemoryInfo\0").ok()?,
                temp: *lib.get(b"nvmlDeviceGetTemperature\0").ok()?,
                power: *lib.get(b"nvmlDeviceGetPowerUsage\0").ok()?,
                clock: *lib.get(b"nvmlDeviceGetClockInfo\0").ok()?,
                enc: lib
                    .get(b"nvmlDeviceGetEncoderUtilization\0")
                    .ok()
                    .map(|s| *s),
                dec: lib
                    .get(b"nvmlDeviceGetDecoderUtilization\0")
                    .ok()
                    .map(|s| *s),
                proc_util: lib
                    .get(b"nvmlDeviceGetProcessUtilization\0")
                    .ok()
                    .map(|s| *s),
                _lib: lib,
                devices,
                names,
            })
        }
    }

    fn process_util(&self) -> HashMap<i32, f32> {
        let Some(proc_util) = self.proc_util else {
            return HashMap::new();
        };
        let mut map = HashMap::new();
        for &dev in &self.devices {
            unsafe {
                let mut count = 0u32;
                // Size probe: INSUFFICIENT_SIZE (7) fills `count`.
                let _ = proc_util(dev, std::ptr::null_mut(), &mut count, 0);
                if count == 0 || count > 4096 {
                    continue;
                }
                // Over-allocate; NVML sometimes under-reports the needed size.
                let mut n = count.max(8).saturating_mul(2);
                let mut buf = vec![
                    NvmlProcSample {
                        pid: 0,
                        time_stamp: 0,
                        sm_util: 0,
                        mem_util: 0,
                        enc_util: 0,
                        dec_util: 0,
                    };
                    n as usize
                ];
                if proc_util(dev, buf.as_mut_ptr(), &mut n, 0) != 0 {
                    continue;
                }
                // Timestamp 0 returns the driver's whole ring, several rows
                // per pid. Keep the newest row; summing the ring reports
                // hundreds of percent. Across devices, add the latest sample
                // from each.
                accumulate_proc_utils(&mut map, &buf[..(n as usize).min(buf.len())]);
            }
        }
        map
    }

    fn sample(&self) -> Vec<GpuRaw> {
        let mut out = Vec::with_capacity(self.devices.len());
        for (dev, name) in self.devices.iter().zip(&self.names) {
            unsafe {
                let mut util = NvmlUtil { gpu: 0, memory: 0 };
                let util = if (self.util)(*dev, &mut util) == 0 {
                    Some(util.gpu as f32)
                } else {
                    None
                };
                let mut mem = NvmlMem {
                    total: 0,
                    free: 0,
                    used: 0,
                };
                let (mem_used, mem_total) = if (self.mem)(*dev, &mut mem) == 0 {
                    (mem.used, mem.total)
                } else {
                    (0, 0)
                };
                let mut temp = 0u32;
                let temp_c = if (self.temp)(*dev, 0, &mut temp) == 0 {
                    Some(temp)
                } else {
                    None
                };
                let mut mw = 0u32;
                let power_w = if (self.power)(*dev, &mut mw) == 0 {
                    Some(mw as f32 / 1000.0)
                } else {
                    None
                };
                let mut core = 0u32;
                let clk_core = if (self.clock)(*dev, 0, &mut core) == 0 {
                    Some(core)
                } else {
                    None
                };
                let mut memclk = 0u32;
                let clk_mem = if (self.clock)(*dev, 2, &mut memclk) == 0 {
                    Some(memclk)
                } else {
                    None
                };
                let enc = self.enc.and_then(|f| {
                    let mut v = 0u32;
                    let mut period = 0u32;
                    if f(*dev, &mut v, &mut period) == 0 {
                        Some(v)
                    } else {
                        None
                    }
                });
                let dec = self.dec.and_then(|f| {
                    let mut v = 0u32;
                    let mut period = 0u32;
                    if f(*dev, &mut v, &mut period) == 0 {
                        Some(v)
                    } else {
                        None
                    }
                });
                out.push(GpuRaw {
                    name: name.clone(),
                    util,
                    mem_used,
                    mem_total,
                    temp_c,
                    power_w,
                    clk_core,
                    clk_mem,
                    enc,
                    dec,
                    integrated: false,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_and_disk_and_stat() {
        let (idle, total) = parse_cpu_line("cpu0 10 0 5 80 5 0 0 0").unwrap();
        assert_eq!(idle, 85);
        assert_eq!(total, 100);
        assert!(parse_disk_line("259 0 nvme0n1 1 0 10 0 1 0 20 0").is_some());
        assert!(parse_disk_line("259 1 nvme0n1p1 1 0 10 0 1 0 20 0").is_none());
        assert!(!is_physical_disk("sda1"));
        assert!(is_physical_disk("sda"));
        assert!(is_local_block_source("/dev/mapper/root"));
        assert!(is_local_block_source("/dev/sda1"));
        assert!(!is_local_block_source("/dev/loop0"));
        assert!(!is_local_block_source("tmpfs"));
        assert_eq!(unescape_mount("/mnt/foo\\040bar"), "/mnt/foo bar");
        let (free, total) = read_disk_space();
        assert!(total > 0, "expected a mounted local filesystem");
        assert!(free <= total);
        // One ~1 TB volume must not be counted once per btrfs subvolume.
        assert!(
            total < 2 * 1024 * 1024 * 1024 * 1024,
            "disk total looks multi-counted: {total}"
        );
        let stat = "12 (my proc) S 1 1 1 0 -1 4194560 1 0 0 0 10 20 0 0 20 0 3 0 1 2 99";
        let p = parse_proc_stat(stat).unwrap();
        assert_eq!(p.comm, "my proc");
        assert_eq!(p.utime, 10);
        assert_eq!(p.stime, 20);
        assert_eq!(p.threads, 3);
        assert_eq!(p.start, 1);
        assert_eq!(p.rss_pages, 99);
        assert_eq!(parse_statm_private("5000 1200 300 10 0 900 0\n"), Some(900));
        assert_eq!(parse_statm_private("5000 100 300 10 0 900 0"), Some(0));
        assert_eq!(parse_statm_private("5000"), None);
        assert_eq!(
            display_name("/opt/google/chrome/chrome --type=zygote --no-sandbox"),
            "chrome"
        );
        assert_eq!(
            display_name("/opt/Grok Bot/grok-bot --type=renderer"),
            "grok-bot"
        );
    }

    #[test]
    fn proc_cpu_matches_the_time_between_samples() {
        // 100 ticks at 100 Hz across one second is one core, 100%.
        assert!((proc_cpu_pct(100, 100.0, 1.0, 8) - 100.0).abs() < 0.01);
        // Two busy cores.
        assert!((proc_cpu_pct(200, 100.0, 1.0, 8) - 200.0).abs() < 0.01);
        // The same ticks squeezed into a short divisor used to read as
        // thousands of percent. A gap that short is not a rate.
        assert_eq!(proc_cpu_pct(100, 100.0, 0.05, 8), 0.0);
        // And a process cannot outrun the machine.
        assert!((proc_cpu_pct(5_000, 100.0, 1.0, 8) - 800.0).abs() < 0.01);
    }

    #[test]
    fn gpu_process_util_keeps_the_newest_sample() {
        let ring = [
            NvmlProcSample {
                pid: 5,
                time_stamp: 10,
                sm_util: 20,
                mem_util: 0,
                enc_util: 0,
                dec_util: 0,
            },
            NvmlProcSample {
                pid: 5,
                time_stamp: 30,
                sm_util: 40,
                mem_util: 0,
                enc_util: 0,
                dec_util: 0,
            },
            NvmlProcSample {
                pid: 5,
                time_stamp: 20,
                sm_util: 90,
                mem_util: 0,
                enc_util: 0,
                dec_util: 0,
            },
        ];
        let mut map = HashMap::new();
        accumulate_proc_utils(&mut map, &ring);
        assert_eq!(map.get(&5).copied(), Some(40.0));
        // A second device adds its own latest sample.
        accumulate_proc_utils(
            &mut map,
            &[NvmlProcSample {
                pid: 5,
                time_stamp: 1,
                sm_util: 15,
                mem_util: 0,
                enc_util: 0,
                dec_util: 0,
            }],
        );
        assert_eq!(map.get(&5).copied(), Some(55.0));
    }

    #[test]
    fn speed_change_drops_history_from_the_old_interval() {
        let mut eng = Engine::new();
        push_hist(&mut eng.cpu_hist, 10.0);
        eng.hist_period = 1000;
        eng.hist_seq = 4;
        eng.align_period(1000);
        assert_eq!(eng.cpu_hist.len(), 1, "same period keeps the ring");
        eng.align_period(500);
        assert!(eng.cpu_hist.is_empty());
        assert_eq!(eng.hist_seq, 0);
        assert_eq!(eng.hist_period, 500);
    }

    #[test]
    fn live_sample_sees_this_machine() {
        let mut eng = Engine::new();
        let _ = eng.tick(0);
        thread::sleep(Duration::from_millis(FIRST_SAMPLE_MS));
        let snap = eng.tick(0);
        assert!(snap.mem_total > 0, "meminfo");
        assert!(!snap.cpu_per.is_empty(), "cores");
        assert!(snap.proc_count > 0, "processes");
        let me = std::process::id() as i32;
        assert!(snap.procs.iter().any(|p| p.pid == me));
        assert!(snap.sample_ms < 200.0, "sample took {} ms", snap.sample_ms);
    }

    #[test]
    fn live_nvidia_when_library_is_installed() {
        if !Path::new("/usr/lib/libnvidia-ml.so.1").exists() {
            return;
        }
        let mut eng = Engine::new();
        let snap = eng.tick(0);
        assert!(
            !snap.gpus.is_empty(),
            "NVML is installed but no GPU was reported"
        );
        assert!(snap.gpus[0].mem_total > 0);
    }

    #[test]
    fn hub_pauses_and_resumes_sampling() {
        use std::sync::atomic::AtomicUsize;
        let ticks = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&ticks);
        let hub = spawn(100, move || {
            seen.fetch_add(1, Ordering::Relaxed);
        });
        // Debug builds sample slowly under a parallel test run: wait, don't count.
        let wait_for = |what: &str, ok: &dyn Fn() -> bool| {
            let until = Instant::now() + Duration::from_secs(10);
            while !ok() {
                assert!(Instant::now() < until, "timed out: {what}");
                thread::sleep(Duration::from_millis(20));
            }
        };
        wait_for("samples while live", &|| ticks.load(Ordering::Relaxed) >= 3);
        hub.set_period(0);
        // Let a tick already in flight land.
        thread::sleep(Duration::from_millis(500));
        let held = ticks.load(Ordering::Relaxed);
        let seq = hub.load().hist_seq;
        thread::sleep(Duration::from_millis(600));
        assert_eq!(
            ticks.load(Ordering::Relaxed),
            held,
            "no samples while paused"
        );
        hub.set_period(100);
        wait_for("history advances after resume", &|| {
            hub.load().hist_seq > seq && ticks.load(Ordering::Relaxed) > held + 1
        });
    }
}
