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
//! Zero dependencies (raw `kernel32` FFI only). No stdout writes on any path
//! of the normal (proxy) mode.
//!
//! Self-install mode: `pwsh.exe --install` copies this binary into
//! `%USERPROFILE%\.config\opencode\bin\pwsh.exe` and updates `opencode.json`
//! (with a backup). See `--help`.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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

#[derive(Default, Clone)]
struct InstallOpts {
    bin_dir: Option<PathBuf>,
    config: Option<PathBuf>,
    dry_run: bool,
    force: bool,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        run_wrapper();
        return;
    }

    match args[0].to_lowercase().as_str() {
        "--install" | "-install" | "install" => {
            let opts = parse_install_opts(&args[1..]);
            process::exit(cmd_install(opts));
        }
        "--uninstall" | "-uninstall" | "uninstall" => {
            let opts = parse_install_opts(&args[1..]);
            process::exit(cmd_uninstall(opts));
        }
        "--status" | "-status" | "status" => {
            let opts = parse_install_opts(&args[1..]);
            process::exit(cmd_status(opts));
        }
        "--help" | "-h" | "-?" | "help" => {
            print_help();
            process::exit(0);
        }
        _ => run_wrapper(),
    }
}

fn run_wrapper() {
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

fn parse_install_opts(args: &[String]) -> InstallOpts {
    let mut opts = InstallOpts::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bin-dir" => {
                if i + 1 < args.len() {
                    opts.bin_dir = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            "--config" => {
                if i + 1 < args.len() {
                    opts.config = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            "--dry-run" => opts.dry_run = true,
            "--force" => opts.force = true,
            _ => {}
        }
        i += 1;
    }
    opts
}

fn default_bin_dir() -> PathBuf {
    if let Some(up) = env::var_os("USERPROFILE") {
        PathBuf::from(up).join(".config").join("opencode").join("bin")
    } else if let Some(home) = env::var_os("HOME") {
        PathBuf::from(home).join(".config").join("opencode").join("bin")
    } else {
        PathBuf::from(".config").join("opencode").join("bin")
    }
}

fn default_config_path() -> PathBuf {
    let base = if let Some(up) = env::var_os("USERPROFILE") {
        PathBuf::from(up).join(".config").join("opencode")
    } else if let Some(home) = env::var_os("HOME") {
        PathBuf::from(home).join(".config").join("opencode")
    } else {
        PathBuf::from(".config").join("opencode")
    };
    let json = base.join("opencode.json");
    if json.exists() {
        return json;
    }
    let jsonc = base.join("opencode.jsonc");
    if jsonc.exists() {
        return jsonc;
    }
    json
}

fn backup_path(config: &Path) -> PathBuf {
    let parent = config.parent().unwrap_or_else(|| Path::new("."));
    let name = config
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let date = today_string();
    let mut candidate = parent.join(format!("{}.bak-{}", name, date));
    if candidate.exists() {
        // append time to avoid clobbering
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        candidate = parent.join(format!("{}.bak-{}-{}", name, date, secs));
    }
    candidate
}

fn today_string() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = secs / 86400;
    let (y, m, d) = civil_from_days(days);
    format!("{:04}{:02}{:02}", y, m, d)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

fn cmd_install(opts: InstallOpts) -> i32 {
    let self_path = match env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[install] cannot resolve own path: {e}");
            return 1;
        }
    };
    let bin_dir = opts.bin_dir.clone().unwrap_or_else(default_bin_dir);
    let config_path = opts.config.clone().unwrap_or_else(default_config_path);

    println!("[install] wrapper: {}", self_path.display());
    println!("[install] bin dir: {}", bin_dir.display());
    println!("[install] config: {}", config_path.display());

    if opts.dry_run {
        println!("[install] (dry-run) no changes will be made");
    } else if let Err(e) = fs::create_dir_all(&bin_dir) {
        eprintln!("[install] cannot create dir {}: {e}", bin_dir.display());
        return 1;
    }

    let target = bin_dir.join("pwsh.exe");
    let same = paths_equal(&self_path, &target);

    if same {
        println!("[install] already at target location, copy skipped");
    } else {
        if !opts.dry_run {
            if target.exists() {
                // On Windows a running pwsh.exe cannot be overwritten, but it
                // can usually be renamed. Use a unique backup name to avoid
                // collisions with leftover .old files.
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let old = bin_dir.join(format!("pwsh.exe.old.{}", ts));
                if let Err(e) = fs::rename(&target, &old) {
                    eprintln!("[install] warning: cannot rename existing {}: {e}", target.display());
                } else {
                    println!("[install] moved existing binary to {}", old.display());
                }
            }
            if let Err(e) = fs::copy(&self_path, &target) {
                eprintln!("[install] cannot copy to {}: {e}", target.display());
                return 1;
            }
        }
        if opts.dry_run {
            println!("[install] (dry-run) would copy to: {}", target.display());
        } else {
            println!("[install] copied: {}", target.display());
        }
    }

    match resolve_real_pwsh() {
        Some(p) => println!("[install] real pwsh: {}", p.display()),
        None => {
            eprintln!("[install] ERROR: real pwsh not found (PATH / ProgramFiles / Store / fallback)");
            return EXIT_NO_PWSH;
        }
    }

    if config_path.exists() {
        let bp = backup_path(&config_path);
        if !bp.exists() && !opts.dry_run {
            if let Err(e) = fs::copy(&config_path, &bp) {
                eprintln!("[install] cannot backup {}: {e}", bp.display());
                return 1;
            }
            println!("[install] config backup: {}", bp.display());
        } else if bp.exists() {
            println!("[install] backup already exists: {}", bp.display());
        }
    }

    let forward = target.display().to_string().replace('\\', "/");
    let new_content = if config_path.exists() {
        match fs::read_to_string(&config_path) {
            Ok(c) => set_shell_key(&c, &forward),
            Err(e) => {
                eprintln!("[install] cannot read config: {e}");
                return 1;
            }
        }
    } else {
        format!("{{\n  \"shell\": \"{}\"\n}}\n", forward)
    };

    if !opts.dry_run {
        if let Some(parent) = config_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = write_atomic(&config_path, &new_content) {
            eprintln!("[install] cannot write config: {e}");
            return 1;
        }
    }
    println!("[install] config updated: shell = {}", forward);

    if is_in_path(&bin_dir) {
        eprintln!("[install] WARNING: bin dir is on PATH — recursion risk. Remove it from PATH.");
    }

    if !opts.dry_run {
        println!("[install] verifying launch...");
        match Command::new(&target)
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "$PSVersionTable.PSVersion"])
            .status()
        {
            Ok(s) if s.success() => println!("[install] verification passed"),
            Ok(s) => {
                eprintln!(
                    "[install] verification failed, exit code: {}",
                    s.code().unwrap_or(-1)
                );
                return 1;
            }
            Err(e) => {
                eprintln!("[install] cannot launch {}: {e}", target.display());
                return EXIT_SPAWN_FAIL;
            }
        }
    }

    println!("[install] done. Restart the opencode session to apply.");
    0
}

fn cmd_uninstall(opts: InstallOpts) -> i32 {
    let config_path = opts.config.unwrap_or_else(default_config_path);
    println!("[uninstall] config: {}", config_path.display());

    if !config_path.exists() {
        println!("[uninstall] config not found, nothing to do");
        return 0;
    }

    let bp = backup_path(&config_path);
    if !bp.exists() && !opts.dry_run {
        let _ = fs::copy(&config_path, &bp);
    }

    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[uninstall] cannot read config: {e}");
            return 1;
        }
    };
    let new_content = remove_shell_key(&content);

    if !opts.dry_run {
        if let Err(e) = write_atomic(&config_path, &new_content) {
            eprintln!("[uninstall] cannot write config: {e}");
            return 1;
        }
    }
    println!("[uninstall] \"shell\" key removed from config");
    println!("[uninstall] binary left in place; remove manually if desired");
    0
}

fn cmd_status(opts: InstallOpts) -> i32 {
    let self_path = env::current_exe().unwrap_or_default();
    let bin_dir = opts.bin_dir.clone().unwrap_or_else(default_bin_dir);
    let target = bin_dir.join("pwsh.exe");
    let config_path = opts.config.clone().unwrap_or_else(default_config_path);

    println!("wrapper: {}", self_path.display());
    println!("target pwsh.exe: {}", target.display());
    println!(
        "real pwsh: {}",
        resolve_real_pwsh()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".to_string())
    );
    println!("config: {}", config_path.display());

    if config_path.exists() {
        if let Ok(c) = fs::read_to_string(&config_path) {
            let val = extract_shell_value(&c).unwrap_or_else(|| "(not set)".to_string());
            println!("shell in config: {}", val);
            println!(
                "matches target: {}",
                val == target.display().to_string().replace('\\', "/")
            );
        }
    } else {
        println!("config not found");
    }
    0
}

fn print_help() {
    println!("pwsh_utf8_wrapper — UTF-8 wrapper around pwsh.exe");
    println!();
    println!("Usage:");
    println!("  pwsh.exe                          run real pwsh (forward argv/stdio)");
    println!("  pwsh.exe --install [options]      install wrapper into opencode");
    println!("  pwsh.exe --uninstall [options]    remove \"shell\" key from config");
    println!("  pwsh.exe --status [options]       show status");
    println!("  pwsh.exe --help                   this help");
    println!();
    println!("Options:");
    println!("  --bin-dir <path>   directory for pwsh.exe (default ~/.config/opencode/bin)");
    println!("  --config <path>    path to opencode.json/jsonc");
    println!("  --dry-run          show actions without changes");
    println!("  --force            overwrite without prompts");
}

// ---------- JSON helpers ----------

fn set_shell_key(content: &str, value: &str) -> String {
    let key = "\"shell\"";
    if let Some(idx) = content.find(key) {
        let after = &content[idx + key.len()..];
        let bytes = after.as_bytes();
        let mut i = 0usize;
        while i < bytes.len()
            && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
        {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b':' {
            i += 1;
            while i < bytes.len()
                && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
            {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'"' {
                // find closing quote
                let mut j = i + 1;
                let mut escaped = false;
                while j < after.len() {
                    let c = after.as_bytes()[j];
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        break;
                    }
                    j += 1;
                }
                if j < after.len() {
                    // absolute indices
                    let open_abs = idx + key.len() + i; // index of opening quote in content
                    let close_abs = idx + key.len() + j; // index of closing quote
                    let mut out = String::with_capacity(content.len() + value.len());
                    out.push_str(&content[..open_abs + 1]);
                    out.push_str(value);
                    out.push_str(&content[close_abs..]);
                    return out;
                }
            }
        }
    }

    // insert new key before last '}'
    insert_shell_key(content, value)
}

fn insert_shell_key(content: &str, value: &str) -> String {
    let trimmed_input = content.trim();
    if trimmed_input.is_empty() {
        return format!("{{\n  \"shell\": \"{}\"\n}}\n", value);
    }
    let last_brace = match content.rfind('}') {
        Some(i) => i,
        None => {
            // no braces: wrap
            return format!("{{\n  \"shell\": \"{}\"\n}}\n", value);
        }
    };
    let head = &content[..last_brace];
    let trimmed_head = head.trim_end();
    let needs_comma = !trimmed_head.ends_with('{') && !trimmed_head.ends_with(',');
    let mut out = String::with_capacity(content.len() + value.len() + 20);
    if needs_comma {
        out.push_str(trimmed_head);
        out.push(',');
    } else {
        out.push_str(head);
    }
    out.push_str("\n  \"shell\": \"");
    out.push_str(value);
    out.push_str("\"\n");
    out.push_str(&content[last_brace..]);
    out
}

fn remove_shell_key(content: &str) -> String {
    let key = "\"shell\"";
    let Some(idx) = content.find(key) else {
        return content.to_string();
    };
    let bytes = content.as_bytes();
    let after = &content[idx + key.len()..];
    let mut i = 0usize;
    while i < after.len()
        && (after.as_bytes()[i] == b' ' || after.as_bytes()[i] == b'\t' || after.as_bytes()[i] == b'\n' || after.as_bytes()[i] == b'\r')
    {
        i += 1;
    }
    if i >= after.len() || after.as_bytes()[i] != b':' {
        return content.to_string();
    }
    i += 1;
    while i < after.len()
        && (after.as_bytes()[i] == b' ' || after.as_bytes()[i] == b'\t' || after.as_bytes()[i] == b'\n' || after.as_bytes()[i] == b'\r')
    {
        i += 1;
    }
    if i >= after.len() || after.as_bytes()[i] != b'"' {
        return content.to_string();
    }
    let mut j = i + 1;
    let mut escaped = false;
    while j < after.len() {
        let c = after.as_bytes()[j];
        if escaped {
            escaped = false;
        } else if c == b'\\' {
            escaped = true;
        } else if c == b'"' {
            break;
        }
        j += 1;
    }
    if j >= after.len() {
        return content.to_string();
    }
    let value_end = idx + key.len() + j + 1; // exclusive

    // decide comma removal
    let mut left = idx;
    while left > 0
        && (bytes[left - 1] == b' ' || bytes[left - 1] == b'\t' || bytes[left - 1] == b'\n' || bytes[left - 1] == b'\r')
    {
        left -= 1;
    }
    let comma_before = left > 0 && bytes[left - 1] == b',';
    let mut right = value_end;
    while right < bytes.len()
        && (bytes[right] == b' ' || bytes[right] == b'\t' || bytes[right] == b'\n' || bytes[right] == b'\r')
    {
        right += 1;
    }
    let comma_after = right < bytes.len() && bytes[right] == b',';

    let (start, end) = if comma_before {
        (left - 1, value_end)
    } else if comma_after {
        (idx, right + 1)
    } else {
        (idx, value_end)
    };

    let mut out = String::with_capacity(content.len());
    out.push_str(&content[..start]);
    out.push_str(&content[end..]);
    out
}

fn extract_shell_value(content: &str) -> Option<String> {
    let key = "\"shell\"";
    let idx = content.find(key)?;
    let after = &content[idx + key.len()..];
    let bytes = after.as_bytes();
    let mut i = 0usize;
    while i < bytes.len()
        && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
    {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b':' {
        return None;
    }
    i += 1;
    while i < bytes.len()
        && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
    {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'"' {
        return None;
    }
    let mut j = i + 1;
    let mut escaped = false;
    while j < after.len() {
        let c = after.as_bytes()[j];
        if escaped {
            escaped = false;
        } else if c == b'\\' {
            escaped = true;
        } else if c == b'"' {
            break;
        }
        j += 1;
    }
    Some(after[i + 1..j].to_string())
}

fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content)?;
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn is_in_path(dir: &Path) -> bool {
    if let Some(path_var) = env::var_os("PATH") {
        let own = normalize_dir(dir);
        for part in path_var.to_string_lossy().split(';') {
            if part.is_empty() {
                continue;
            }
            if normalize_dir(Path::new(part)) == own {
                return true;
            }
        }
    }
    false
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (fs::canonicalize(a), fs::canonicalize(b)) {
        return ca == cb;
    }
    normalize_dir(a) == normalize_dir(b)
}

// ---------- real pwsh resolution ----------

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
                    continue;
                }
            }
            let candidate = dir.join("pwsh.exe");
            if is_file(&candidate) {
                return Some(candidate);
            }
        }
    }

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

    if let Some(windir) = env::var_os("SystemRoot") {
        let candidate = PathBuf::from(windir).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe");
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

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_shell_insert_empty() {
        let out = set_shell_key("{}", "C:/x/pwsh.exe");
        assert!(out.contains("\"shell\": \"C:/x/pwsh.exe\""));
    }

    #[test]
    fn set_shell_insert_object() {
        let out = set_shell_key("{\n  \"a\": 1\n}", "C:/x/pwsh.exe");
        assert!(out.contains("\"shell\": \"C:/x/pwsh.exe\""));
        assert!(out.contains("\"a\": 1"));
    }

    #[test]
    fn set_shell_replace() {
        let inp = "{\n  \"shell\": \"old\",\n  \"a\": 1\n}";
        let out = set_shell_key(inp, "C:/new/pwsh.exe");
        assert!(out.contains("\"shell\": \"C:/new/pwsh.exe\""));
        assert!(!out.contains("\"old\""));
        assert!(out.contains("\"a\": 1"));
    }

    #[test]
    fn remove_shell_key_basic() {
        let inp = "{\n  \"shell\": \"old\",\n  \"a\": 1\n}";
        let out = remove_shell_key(inp);
        assert!(!out.contains("\"shell\""));
        assert!(out.contains("\"a\": 1"));
    }

    #[test]
    fn remove_shell_key_only() {
        let inp = "{\n  \"shell\": \"old\"\n}";
        let out = remove_shell_key(inp);
        assert!(!out.contains("\"shell\""));
        assert!(out.contains("{"));
        assert!(out.contains("}"));
    }

    #[test]
    fn extract_shell_value_works() {
        let inp = "{\n  \"shell\": \"C:/foo/pwsh.exe\"\n}";
        assert_eq!(extract_shell_value(inp), Some("C:/foo/pwsh.exe".to_string()));
    }

    #[test]
    fn civil_date_sane() {
        let s = today_string();
        assert_eq!(s.len(), 8);
    }
}
