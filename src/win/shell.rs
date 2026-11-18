//! Per-user install with no admin rights: %LOCALAPPDATA%\Programs\Flashless, Start menu and desktop
//! shortcuts, and an entry in Settings > Apps > Installed apps. The same exe is installer and uninstaller.

use std::path::{Path, PathBuf};
use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
use winreg::RegKey;

pub const DISPLAY_NAME: &str = "Flashless";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Flashless";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn local_app_data() -> PathBuf { std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir) }
fn app_data() -> PathBuf { std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir) }

pub fn install_dir() -> PathBuf { local_app_data().join("Programs").join("Flashless") }
pub fn installed_exe() -> PathBuf { install_dir().join("Flashless.exe") }
fn start_menu_link() -> PathBuf { app_data().join(r"Microsoft\Windows\Start Menu\Programs").join(format!("{DISPLAY_NAME}.lnk")) }
fn desktop_link() -> PathBuf {
    let profile = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    // OneDrive-redirected desktops: prefer the folder that exists
    let od = std::env::var_os("OneDrive").map(|p| PathBuf::from(p).join("Desktop"));
    let desk = od.filter(|p| p.exists()).unwrap_or_else(|| profile.join("Desktop"));
    desk.join(format!("{DISPLAY_NAME}.lnk"))
}

pub fn has_start_menu_link() -> bool { start_menu_link().exists() }
pub fn has_desktop_link() -> bool { desktop_link().exists() }

fn self_path() -> PathBuf { std::env::current_exe().unwrap_or_default() }
fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    n(a) == n(b)
}

pub fn is_installed_copy() -> bool { same_path(&self_path(), &installed_exe()) }

/// The exe acts as installer when named Flashless-Setup*.exe or Flashless-Update*.exe (or run with --install)
/// and is not the installed copy. An update is the same installer: it replaces the exe and keeps settings.
pub fn should_install(args: &[String]) -> bool {
    let name = self_path().file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    args.iter().any(|a| a.eq_ignore_ascii_case("--install")) || ((name.starts_with("flashless-setup") || name.starts_with("flashless-update")) && !is_installed_copy())
}

pub fn installed_version() -> Option<String> {
    if !installed_exe().exists() { return None; }
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL_KEY).and_then(|k| k.get_value::<String, _>("DisplayVersion")).ok().or(Some("unknown".into()))
}

const APP_PATHS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths\Flashless.exe";

/// `start_menu`: the Start menu shortcut is what Windows search finds. `desktop`: optional desktop shortcut.
pub fn install(desktop_shortcut: bool, start_menu: bool) -> Result<(), String> {
    stop_installed_copies();
    std::fs::create_dir_all(install_dir()).map_err(|e| e.to_string())?;
    let mut last = String::new();
    for _ in 0..10 {
        match std::fs::copy(self_path(), installed_exe()) {
            Ok(_) => { last.clear(); break; }
            Err(e) => { last = e.to_string(); std::thread::sleep(std::time::Duration::from_millis(300)); }
        }
    }
    if !last.is_empty() { return Err(format!("could not copy Flashless.exe: {last}")); }

    if start_menu { create_shortcut(&start_menu_link()).map_err(|e| format!("Start menu shortcut: {e}"))?; }
    else { let _ = std::fs::remove_file(start_menu_link()); }
    // App Paths: lets Win+R "flashless" start it too
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(APP_PATHS_KEY) {
        let _ = k.set_value("", &installed_exe().to_string_lossy().to_string());
        let _ = k.set_value("Path", &install_dir().to_string_lossy().to_string());
    }
    if desktop_shortcut { create_shortcut(&desktop_link()).map_err(|e| format!("desktop shortcut: {e}"))?; }
    else { let _ = std::fs::remove_file(desktop_link()); }

    let exe = installed_exe().to_string_lossy().to_string();
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY).map_err(|e| e.to_string())?;
    let size_kb = std::fs::metadata(installed_exe()).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
    let r = (|| -> std::io::Result<()> {
        key.set_value("DisplayName", &DISPLAY_NAME)?;
        key.set_value("DisplayVersion", &VERSION)?;
        key.set_value("Publisher", &"Flashless")?;
        key.set_value("DisplayIcon", &format!("{exe},0"))?;
        key.set_value("InstallLocation", &install_dir().to_string_lossy().to_string())?;
        key.set_value("UninstallString", &format!("\"{exe}\" --uninstall"))?;
        key.set_value("QuietUninstallString", &format!("\"{exe}\" --uninstall --silent"))?;
        key.set_value("EstimatedSize", &size_kb)?;
        key.set_value("NoModify", &1u32)?;
        key.set_value("NoRepair", &1u32)?;
        Ok(())
    })();
    r.map_err(|e| e.to_string())
}

/// Removes shortcuts and the Installed apps entry, then the install folder (after exit if running from it).
/// Settings in %LOCALAPPDATA%\Flashless are kept.
pub fn uninstall() {
    stop_installed_copies();
    let _ = std::fs::remove_file(start_menu_link());
    let _ = std::fs::remove_file(desktop_link());
    if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Uninstall", KEY_WRITE) {
        let _ = k.delete_subkey_all("Flashless");
    }
    if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\App Paths", KEY_WRITE) {
        let _ = k.delete_subkey_all("Flashless.exe");
    }
    let dir = install_dir();
    if !dir.exists() { return; }
    if is_installed_copy() {
        // a running exe can't delete itself: a hidden shell removes the folder after we exit
        use std::os::windows::process::CommandExt;
        // raw_arg: Rust's normal quoting uses \" escapes, which cmd.exe does not understand
        let _ = std::process::Command::new("cmd.exe")
            .raw_arg(format!("/c ping 127.0.0.1 -n 3 >nul & rmdir /s /q \"{}\"", dir.display()))
            .current_dir(std::env::temp_dir())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn();
    } else {
        let _ = std::fs::remove_dir_all(dir);
    }
}

pub fn launch_installed() {
    let _ = std::process::Command::new(installed_exe()).current_dir(install_dir()).spawn();
}

/// Ends Flashless processes running from the install folder so the exe can be replaced or removed.
fn stop_installed_copies() {
    let target = installed_exe();
    let me = std::process::id();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let name = String::from_utf16_lossy(&e.szExeFile[..e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0)]).to_lowercase();
            if e.th32ProcessID != me && name.starts_with("flashless") {
                if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, e.th32ProcessID) {
                    let mut buf = [0u16; MAX_PATH as usize * 2];
                    let mut len = buf.len() as u32;
                    if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
                        let path = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
                        if same_path(&path, &target) {
                            let _ = TerminateProcess(h, 0);
                            WaitForSingleObject(h, 3000);
                        }
                    }
                    let _ = CloseHandle(h);
                }
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
}

fn create_shortcut(link: &Path) -> windows::core::Result<()> {
    if let Some(dir) = link.parent() { let _ = std::fs::create_dir_all(dir); }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let exe = installed_exe();
        sl.SetPath(&HSTRING::from(exe.as_os_str()))?;
        sl.SetWorkingDirectory(&HSTRING::from(install_dir().as_os_str()))?;
        sl.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        sl.SetDescription(&HSTRING::from("Flashless - League of Legends companion app"))?;
        let pf: IPersistFile = sl.cast()?;
        pf.Save(&HSTRING::from(link.as_os_str()), true)?;
    }
    Ok(())
}
