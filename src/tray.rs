//! The tray icon: a coloured dot for the current state, a menu to switch modes, and the
//! balloon notifications (Windows 10/11 show them as toasts).

use crate::engine::{Mode, Phase, SHARED};
use crate::{config, power};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ID_AUTO: usize = 101;
const ID_ALWAYS: usize = 102;
const ID_OFF: usize = 103;
const ID_LID: usize = 104;
const ID_CONFIG: usize = 105;
const ID_EXIT: usize = 106;
const WM_TRAY: u32 = WM_APP + 1;
const WM_REFRESH: u32 = WM_APP + 2;
const NIF_MESSAGE_ICON_TIP: u32 = 1 | 2 | 4;
const NIF_INFO_: u32 = 0x10;

static HWND_: AtomicUsize = AtomicUsize::new(0);
static ICON: AtomicUsize = AtomicUsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn copy_into(dst: &mut [u16], s: &str) {
    let n = dst.len() - 1;
    for (i, c) in s.encode_utf16().take(n).enumerate() {
        dst[i] = c;
    }
}

/// A filled, antialiased circle in the given colour.
fn dot_icon(rgb: u32) -> HICON {
    unsafe {
        let s = GetSystemMetrics(SM_CXSMICON).max(16);
        let (c, r) = (s as f32 / 2.0 - 0.5, s as f32 / 2.0 - 1.5);
        let px: Vec<u32> = (0..s * s)
            .map(|i| {
                let (x, y) = ((i % s) as f32, (i / s) as f32);
                let a = (r - ((x - c).powi(2) + (y - c).powi(2)).sqrt() + 0.5).clamp(0.0, 1.0);
                let ch = |shift: u32| ((((rgb >> shift) & 0xFF) as f32 * a) as u32) << shift; // premultiplied
                ((a * 255.0) as u32) << 24 | ch(16) | ch(8) | ch(0)
            })
            .collect();
        let color = CreateBitmap(s, s, 1, 32, px.as_ptr() as _);
        let mask_bits = vec![0u8; (((s + 15) / 16) * 2 * s) as usize];
        let mask = CreateBitmap(s, s, 1, 1, mask_bits.as_ptr() as _);
        let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

fn base_nid(h: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = h;
    nid.uID = 1;
    nid
}

fn update_icon(h: HWND, op: u32) {
    let (rgb, tip) = {
        let s = SHARED.lock().unwrap();
        let rgb = match (s.mode, s.phase, s.awake) {
            (Mode::Off, ..) => 0x5A5F66,
            (_, Phase::Waiting, _) => 0xE5484D,
            (_, Phase::Working, _) => 0xF5A623,
            (_, _, true) => 0xF7C873,
            _ => 0x9AA0A8,
        };
        let tip = if s.summary.is_empty() { "nosleep".to_string() } else { format!("nosleep: {}", s.summary) };
        (rgb, tip)
    };
    let mut nid = base_nid(h);
    nid.uFlags = NIF_MESSAGE_ICON_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = dot_icon(rgb);
    copy_into(&mut nid.szTip, &tip);
    unsafe {
        Shell_NotifyIconW(op, &nid);
        let old = ICON.swap(nid.hIcon as usize, Relaxed);
        if old != 0 {
            DestroyIcon(old as HICON);
        }
    }
}

/// Asks the tray thread to redraw the icon and tooltip from `SHARED`.
pub fn refresh() {
    let h = HWND_.load(Relaxed);
    if h != 0 {
        unsafe {
            PostMessageW(h as HWND, WM_REFRESH, 0, 0);
        }
    }
}

pub fn balloon(title: &str, body: &str) {
    let h = HWND_.load(Relaxed);
    if h == 0 {
        return;
    }
    let mut nid = base_nid(h as HWND);
    nid.uFlags = NIF_INFO_;
    nid.dwInfoFlags = NIIF_INFO;
    copy_into(&mut nid.szInfoTitle, title);
    copy_into(&mut nid.szInfo, body);
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

pub fn message(text: &str) {
    unsafe {
        MessageBoxW(std::ptr::null_mut(), w(text).as_ptr(), w("nosleep").as_ptr(), MB_OK | MB_ICONINFORMATION);
    }
}

fn menu(h: HWND) {
    let (mode, lid, summary) = {
        let s = SHARED.lock().unwrap();
        (s.mode, s.lid, s.summary.clone())
    };
    let check = |on: bool| if on { MF_CHECKED } else { 0 };
    unsafe {
        let m = CreatePopupMenu();
        AppendMenuW(m, MF_GRAYED, 0, w(if summary.is_empty() { "Starting…" } else { &summary }).as_ptr());
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(m, check(mode == Mode::Auto), ID_AUTO, w("Auto: stay awake while agents work").as_ptr());
        AppendMenuW(m, check(mode == Mode::Always), ID_ALWAYS, w("Always awake").as_ptr());
        AppendMenuW(m, check(mode == Mode::Off), ID_OFF, w("Off").as_ptr());
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(m, check(lid), ID_LID, w("Keep working with the lid closed").as_ptr());
        AppendMenuW(m, 0, ID_CONFIG, w("Open config…").as_ptr());
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(m, 0, ID_EXIT, w("Exit").as_ptr());
        let mut p = std::mem::zeroed();
        GetCursorPos(&mut p);
        SetForegroundWindow(h);
        TrackPopupMenu(m, TPM_RIGHTBUTTON, p.x, p.y, 0, h, std::ptr::null());
        PostMessageW(h, WM_NULL, 0, 0);
        DestroyMenu(m);
    }
}

fn command(h: HWND, id: usize) {
    let set_mode = |m| SHARED.lock().unwrap().mode = m;
    match id {
        ID_AUTO => set_mode(Mode::Auto),
        ID_ALWAYS => set_mode(Mode::Always),
        ID_OFF => set_mode(Mode::Off),
        ID_LID => {
            let mut s = SHARED.lock().unwrap();
            s.lid = !s.lid;
            config::set("lid_override", if s.lid { "true" } else { "false" });
        }
        ID_CONFIG => unsafe {
            ShellExecuteW(
                h,
                w("open").as_ptr(),
                w(&config::path().to_string_lossy()).as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
        },
        ID_EXIT => unsafe {
            power::recover();
            Shell_NotifyIconW(NIM_DELETE, &base_nid(h));
            std::process::exit(0);
        },
        _ => {}
    }
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            command(h, wp & 0xFFFF);
            0
        }
        WM_TRAY => {
            if matches!((lp & 0xFFFF) as u32, WM_RBUTTONUP | WM_LBUTTONUP) {
                menu(h);
            }
            0
        }
        WM_REFRESH => {
            update_icon(h, NIM_MODIFY);
            0
        }
        // Logging off or shutting down: put the lid action back before we are killed.
        WM_ENDSESSION => {
            if wp != 0 {
                power::recover();
            }
            0
        }
        m if m != 0 && m == TASKBAR_CREATED.load(Relaxed) => {
            update_icon(h, NIM_ADD); // explorer restarted
            0
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

/// Creates the hidden window and the tray icon; call `message_loop` on the same thread.
pub fn create() {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = w("nosleep_tray");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
        let h = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            w("nosleep").as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            0 as _,
            0 as _,
            hinst,
            std::ptr::null(),
        );
        HWND_.store(h as usize, Relaxed);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w("TaskbarCreated").as_ptr()), Relaxed);
        update_icon(h, NIM_ADD);
    }
}

pub fn message_loop() {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
