//! Keeping the machine awake, overriding the lid-close action, and reading the battery.
//!
//! `SetThreadExecutionState` stops idle sleep but not the lid: closing it runs the power
//! plan's lid action regardless. So while agents work we set that action to "do nothing" in
//! the active plan, and put the original back afterwards. The original is saved to a file
//! first, so a crash or a kill can be undone on the next start (or by `nosleep restore`).

use crate::config;
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::System::Power::*;

const SUB_BUTTONS: GUID = GUID::from_u128(0x4f971e89_eebd_4455_a8de_9e59040e7347);
const LID_ACTION: GUID = GUID::from_u128(0x5ca83367_6e45_459f_a27b_476b1d01c936);
const DO_NOTHING: u32 = 0;

/// Must always be called from the same long-lived thread: the request belongs to the thread.
pub fn set_awake(on: bool) {
    let flags = if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS };
    unsafe {
        SetThreadExecutionState(flags);
    }
}

/// Battery percentage when running on battery, `None` on AC power or without a battery.
pub fn on_battery() -> Option<u8> {
    unsafe {
        let mut s: SYSTEM_POWER_STATUS = std::mem::zeroed();
        if GetSystemPowerStatus(&mut s) == 0 || s.ACLineStatus != 0 || s.BatteryLifePercent > 100 {
            return None;
        }
        Some(s.BatteryLifePercent)
    }
}

fn active_scheme() -> Option<GUID> {
    unsafe {
        let mut p: *mut GUID = std::ptr::null_mut();
        if PowerGetActiveScheme(std::ptr::null_mut(), &mut p) != 0 || p.is_null() {
            return None;
        }
        let g = *p;
        LocalFree(p as _);
        Some(g)
    }
}

fn read_lid(scheme: &GUID) -> Option<(u32, u32)> {
    let (mut ac, mut dc) = (0, 0);
    unsafe {
        let a = PowerReadACValueIndex(std::ptr::null_mut(), scheme, &SUB_BUTTONS, &LID_ACTION, &mut ac);
        let d = PowerReadDCValueIndex(std::ptr::null_mut(), scheme, &SUB_BUTTONS, &LID_ACTION, &mut dc);
        (a == 0 && d == 0).then_some((ac, dc))
    }
}

fn write_lid(scheme: &GUID, ac: u32, dc: u32) -> bool {
    unsafe {
        PowerWriteACValueIndex(std::ptr::null_mut(), scheme, &SUB_BUTTONS, &LID_ACTION, ac) == 0
            && PowerWriteDCValueIndex(std::ptr::null_mut(), scheme, &SUB_BUTTONS, &LID_ACTION, dc) == 0
            && PowerSetActiveScheme(std::ptr::null_mut(), scheme) == 0
    }
}

fn guid_u128(g: &GUID) -> u128 {
    (g.data1 as u128) << 96 | (g.data2 as u128) << 80 | (g.data3 as u128) << 64 | u64::from_be_bytes(g.data4) as u128
}

fn backup_path() -> std::path::PathBuf {
    config::dir().join("lid.backup")
}

/// Sets the lid action to "do nothing", remembering the original. False if Windows refused.
pub fn hold_lid() -> bool {
    let Some(scheme) = active_scheme() else { return false };
    let Some((ac, dc)) = read_lid(&scheme) else { return false };
    if !backup_path().exists() {
        let _ = std::fs::create_dir_all(config::dir());
        let line = format!("{:032x} {ac} {dc}", guid_u128(&scheme));
        if std::fs::write(backup_path(), line).is_err() {
            return false; // never change a setting we could not promise to restore
        }
    }
    write_lid(&scheme, DO_NOTHING, DO_NOTHING)
}

/// Restores the saved lid action, if one was saved. Returns whether anything was restored.
pub fn recover() -> bool {
    let Ok(text) = std::fs::read_to_string(backup_path()) else { return false };
    let mut it = text.split_whitespace();
    let parsed = (|| {
        let scheme = GUID::from_u128(u128::from_str_radix(it.next()?, 16).ok()?);
        Some((scheme, it.next()?.parse().ok()?, it.next()?.parse().ok()?))
    })();
    let ok = match parsed {
        Some((scheme, ac, dc)) => write_lid(&scheme, ac, dc),
        None => true, // garbage: nothing sensible to restore, drop it
    };
    if ok {
        let _ = std::fs::remove_file(backup_path());
    }
    ok
}
