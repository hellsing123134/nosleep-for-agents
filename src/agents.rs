//! Finds agent processes and decides whether they are busy: the CPU time of the agent and
//! everything it spawned (builds, tests, language servers) since the previous scan.

use std::collections::{HashMap, HashSet};
use std::time::Instant;
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// A process tree using more than this share of one core counts as working.
const BUSY_SHARE: f64 = 0.03;

pub struct Scan {
    /// Executable names of the agents that are running, one entry per process.
    pub agents: Vec<String>,
    pub busy: bool,
}

pub struct Scanner {
    names: Vec<String>,
    /// Path fragments that disqualify a name match (the Claude desktop app is also claude.exe).
    ignore: Vec<String>,
    /// Whether a matching pid passed the `ignore` check, so each path is looked up once.
    verdict: HashMap<u32, bool>,
    cpu: HashMap<u32, u64>,
    last: Instant,
}

struct Proc {
    pid: u32,
    parent: u32,
    exe: String,
}

fn processes() -> Vec<Proc> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push(Proc {
                pid: e.th32ProcessID,
                parent: e.th32ParentProcessID,
                exe: String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase(),
            });
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    out
}

/// Kernel + user time in 100 ns units.
fn cpu_time(pid: u32) -> Option<u64> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut t: [FILETIME; 4] = std::mem::zeroed();
        let [c, e, k, u] = &mut t;
        let ok = GetProcessTimes(h, c, e, k, u);
        CloseHandle(h);
        let ft = |f: &FILETIME| (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64;
        (ok != 0).then(|| ft(&t[2]) + ft(&t[3]))
    }
}

fn image_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]).to_lowercase())
    }
}

impl Scanner {
    pub fn new(names: Vec<String>, ignore: Vec<String>) -> Self {
        Scanner { names, ignore, verdict: HashMap::new(), cpu: HashMap::new(), last: Instant::now() }
    }

    fn is_agent(&mut self, p: &Proc) -> bool {
        if !self.names.contains(&p.exe) {
            return false;
        }
        let ignore = &self.ignore;
        *self.verdict.entry(p.pid).or_insert_with(|| {
            image_path(p.pid).is_some_and(|path| !ignore.iter().any(|frag| path.contains(frag.as_str())))
        })
    }

    pub fn scan(&mut self) -> Scan {
        let procs = processes();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for p in &procs {
            if p.pid != p.parent {
                children.entry(p.parent).or_default().push(p.pid);
            }
        }
        let alive: HashSet<u32> = procs.iter().map(|p| p.pid).collect();
        self.verdict.retain(|pid, _| alive.contains(pid));
        let roots: Vec<&Proc> = procs.iter().filter(|p| self.is_agent(p)).collect();

        // The agents plus all their descendants; `seen` guards against pid reuse cycles.
        let mut seen = HashSet::new();
        let mut stack: Vec<u32> = roots.iter().map(|p| p.pid).collect();
        while let Some(pid) = stack.pop() {
            if seen.insert(pid) {
                stack.extend(children.get(&pid).into_iter().flatten());
            }
        }

        let mut cpu = HashMap::new();
        let mut delta = 0u64;
        for pid in seen {
            if let Some(t) = cpu_time(pid) {
                if let Some(prev) = self.cpu.get(&pid) {
                    delta += t.saturating_sub(*prev);
                }
                cpu.insert(pid, t);
            }
        }
        self.cpu = cpu;
        let elapsed = self.last.elapsed().as_nanos() as f64 / 100.0;
        self.last = Instant::now();

        Scan {
            agents: roots.iter().map(|p| p.exe.trim_end_matches(".exe").to_string()).collect(),
            busy: elapsed > 0.0 && delta as f64 / elapsed > BUSY_SHARE,
        }
    }
}
