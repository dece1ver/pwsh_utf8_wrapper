//! UTF-8 console wrapper around the real `pwsh.exe`, for the OpenCode shell
//! tool on Windows.
//!
//! MUST be deployed under the file name `pwsh.exe`: OpenCode routes shells
//! by executable basename, and only `pwsh`/`powershell` take the PowerShell
//! argv branch (`-NoLogo -NoProfile -NonInteractive -Command <cmd>`).
//!
//! What it does, in order:
//! 1. Switches the inherited (shared) console to UTF-8 **before** the real
//!    pwsh starts. .NET reads `GetConsoleOutputCP()` once at process startup
//!    and caches it in `[Console]::OutputEncoding`, so doing this later (or
//!    from inside the pwsh session) has no effect.
//! 2. Resolves the REAL `pwsh.exe` (never itself — recursion guard) and
//!    forwards argv / stdio / exit code 1:1.
//!
//! Zero dependencies (raw `kernel32` FFI only). No stdout writes on any path.

use std::env;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};

type WinBool = i32;
type WinUint = u32;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleOutputCP(w_code_page_id: WinUint) -> WinBool;
    fn SetConsoleCP(w_code_page_id: WinUint) -> WinBool;
}

const UTF8_CP: WinUint = 65001;

/// Exit code when the real pwsh.exe cannot be found.
const EXIT_NO_PWSH: i32 = 127;
/// Exit code when the real pwsh.exe cannot be launched.
const EXIT_SPAWN_FAIL: i32 = 126;

fn main() {
    // Best effort: if there is no console at all these fail and we simply
    // continue — the wrapper is then neutral (never harmful).
    unsafe {
        SetConsoleOutputCP(UTF8_CP);
        SetConsoleCP(UTF8_CP);
    }

    let real = match resolve_real_pwsh() {
        Some(path) => path,
        None => {
            eprintln!("pwsh_utf8_wrapper: real pwsh.exe not found (not on PATH, no known install location)");
            process::exit(EXIT_NO_PWSH);
        }
    };

    let args: Vec<OsString> = env::args_os().skip(1).collect();
    match Command::new(&real)
        .args(&args)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => process::exit(status.code().unwrap_or(1)),
        Err(err) => {
            eprintln!("pwsh_utf8_wrapper: failed to launch {}: {err}", real.display());
            process::exit(EXIT_SPAWN_FAIL);
        }
    }
}

/// Locate the real `pwsh.exe`, excluding this wrapper's own directory
/// (recursion guard — the wrapper itself is named `pwsh.exe`).
fn resolve_real_pwsh() -> Option<PathBuf> {
    let own_dir: Option<PathBuf> = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));

    if let Some(path_var) = env::var_os("PATH") {
        let wide: Vec<u16> = path_var.encode_wide().collect();
        for part in wide.split(|c| *c == b';' as u16) {
            if part.is_empty() {
                continue;
            }
            let dir = PathBuf::from(OsString::from_wide(part));
            if let Some(ref own) = own_dir {
                if dirs_equal(&dir, own) {
                    continue; // this wrapper's own directory — skip
                }
            }
            let candidate = dir.join("pwsh.exe");
            if is_file(&candidate) {
                return Some(candidate);
            }
        }
    }

    // Fallbacks for machines where pwsh is installed but not on PATH.
    // `%ProgramFiles%` first (portable), then this machine's Store path.
    if let Some(pf) = env::var_os("ProgramFiles") {
        let candidate = PathBuf::from(pf).join("PowerShell\\7\\pwsh.exe");
        if is_file(&candidate) {
            return Some(candidate);
        }
    }
    for fixed in [
        "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
        "C:\\Program Files\\WindowsApps\\Microsoft.PowerShell_7.6.6.0_x64__8wekyb3d8bbwe\\pwsh.exe",
    ] {
        let candidate = PathBuf::from(fixed);
        if is_file(&candidate) {
            return Some(candidate);
        }
    }

    // Last resort: Windows PowerShell 5.1 still honours the UTF-8 console.
    if let Some(windir) = env::var_os("SystemRoot") {
        let candidate = PathBuf::from(windir)
            .join("System32\\WindowsPowerShell\\v1.0\\powershell.exe");
        if is_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_file(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file(),
        Err(_) => false,
    }
}

/// Compare two directories, tolerating case, separators and symlinks.
/// Never panics; on any doubt returns `false` (fail-open: a duplicate PATH
/// entry is harmless, skipping the real pwsh would not be — but callers
/// only use this to *exclude*, and exclusion requires a positive match).
fn dirs_equal(a: &Path, b: &Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return ca == cb;
    }
    normalize_dir(a) == normalize_dir(b)
}

fn normalize_dir(path: &Path) -> String {
    let mut out = String::new();
    for comp in path.components() {
        out.push_str(&comp.as_os_str().to_string_lossy().to_lowercase());
        out.push('\\');
    }
    out
}
