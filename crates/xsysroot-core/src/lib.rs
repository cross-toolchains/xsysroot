//! Shared terminal progress bars, `cli-table` rendering, LLVM `clang` discovery,
//! wrapper script generation, and safe archive path helpers for `xsysroot` tools.

use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use cli_table::{Cell, Style, Table, print_stdout};
use indicatif::{ProgressBar, ProgressStyle};

/// Formats an aligned icon + status label for progress bars.
pub fn format_status(icon: &str, label: &str) -> String {
    format!("{icon}  {label:>11}")
}

/// Creates a green `xwin`-style aligned bytes progress bar.
pub fn make_bytes_bar(prefix: &'static str, len: u64, icon: &str, label: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    if let Ok(style) = ProgressStyle::with_template(
        "{spinner:.green} {prefix:<9.bold} [{elapsed:>3}] {wide_bar:.green} {binary_bytes:>10}/{binary_total_bytes:>10}  {msg}",
    ) {
        pb.set_style(style.progress_chars("█▄ "));
    }
    pb.set_prefix(prefix);
    pb.set_message(format_status(icon, label));
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Renders a Unicode-bordered table with bold headers to stdout using `cli-table`.
pub fn print_table(headers: &[&str], rows: Vec<Vec<String>>) {
    let title: Vec<_> = headers.iter().map(|h| h.cell().bold(true)).collect();
    let body: Vec<Vec<_>> = rows
        .into_iter()
        .map(|row| row.into_iter().map(|c| c.cell()).collect())
        .collect();
    let table = body.table().title(title);
    let _ = print_stdout(table);
}

/// Resolves `~/.cache/<tool>` (or `%USERPROFILE%\.cache\<tool>` on Windows).
pub fn default_tool_cache_dir(tool: &str) -> PathBuf {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cache").join(tool)
}

/// Resolves an LLVM `clang` binary from an explicit path, Homebrew LLVM, or `PATH`.
pub fn resolve_clang(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        if p.is_file() {
            return Ok(p.to_path_buf());
        }
        return Err(format!("specified --clang does not exist: {}", p.display()));
    }
    let exe = if cfg!(windows) { "clang.exe" } else { "clang" };
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/opt/llvm/bin"),
        PathBuf::from("/usr/local/opt/llvm/bin"),
    ];
    if let Some(path_var) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path_var));
    }
    for dir in dirs {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("clang not found in PATH or Homebrew LLVM (pass --clang <PATH>)".to_string())
}

/// Rejects archive entry paths containing `..`, root prefixes, or drive prefixes.
pub fn sanitize_relative_path(rel: &str) -> Result<PathBuf, String> {
    let mut clean = PathBuf::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => return Err(format!("unsafe archive entry path: `{rel}`")),
        }
    }
    Ok(clean)
}

/// Creates a symlink (replacing any existing file/link at `link`).
#[cfg(unix)]
pub fn create_symlink(target: &str, link: &Path) -> Result<(), String> {
    if link.exists() || link.symlink_metadata().is_ok() {
        let _ = fs::remove_file(link);
    }
    std::os::unix::fs::symlink(target, link).map_err(|e| {
        format!(
            "failed to create symlink {} -> {target}: {e}",
            link.display()
        )
    })
}

#[cfg(not(unix))]
pub fn create_symlink(target: &str, link: &Path) -> Result<(), String> {
    create_windows_symlink_or_fallback(target, link)
}

/// Deferred symlink queue for archive extraction.
///
/// On Unix, symlinks are created immediately. On Windows, symlinks are deferred
/// until `finish()` so that target files and directories already exist when
/// choosing `symlink_file` vs `symlink_dir` (with automatic `hard_link` / copy
/// fallback when Developer Mode / `SeCreateSymbolicLinkPrivilege` is unavailable).
#[derive(Default)]
pub struct SymlinkQueue {
    #[cfg(not(unix))]
    pending: Vec<(String, PathBuf)>,
}

impl SymlinkQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, target: &str, link: &Path) -> Result<(), String> {
        #[cfg(unix)]
        {
            create_symlink(target, link)
        }
        #[cfg(not(unix))]
        {
            self.pending.push((target.to_string(), link.to_path_buf()));
            Ok(())
        }
    }

    pub fn finish(self) -> Result<(), String> {
        #[cfg(unix)]
        {
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let mut remaining = self.pending;
            // Multi-pass resolution handles chained symlinks such as macOS/iOS
            // framework links (`Versions/Current -> A`, `Headers -> Versions/Current/Headers`).
            for _ in 0..4 {
                if remaining.is_empty() {
                    break;
                }
                let mut next = Vec::new();
                let mut progressed = false;
                for (target, link) in remaining {
                    let win_target = target.replace('/', "\\");
                    let resolved = link.parent().unwrap_or(Path::new(".")).join(&win_target);
                    if resolved.exists() {
                        create_windows_symlink_or_fallback(&target, &link)?;
                        progressed = true;
                    } else {
                        next.push((target, link));
                    }
                }
                remaining = next;
                if !progressed {
                    break;
                }
            }
            // Any leftover entries are dangling symlinks: best-effort symlink_file.
            for (target, link) in remaining {
                let _ = create_windows_symlink_or_fallback(&target, &link);
            }
            Ok(())
        }
    }
}

#[cfg(not(unix))]
fn create_windows_symlink_or_fallback(target: &str, link: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed creating {}: {e}", parent.display()))?;
    }
    if let Ok(meta) = fs::symlink_metadata(link) {
        if meta.is_dir() {
            let _ = fs::remove_dir_all(link);
        } else {
            let _ = fs::remove_file(link);
        }
    }

    let win_target = target.replace('/', "\\");
    let resolved = link.parent().unwrap_or(Path::new(".")).join(&win_target);

    if resolved.is_dir() {
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&win_target, link).is_ok() {
            return Ok(());
        }
        copy_dir_recursive(&resolved, link)
    } else if resolved.is_file() {
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(&win_target, link).is_ok() {
            return Ok(());
        }
        if fs::hard_link(&resolved, link).is_ok() {
            return Ok(());
        }
        fs::copy(&resolved, link).map(|_| ()).map_err(|e| {
            format!(
                "failed copying symlink target {} -> {}: {e}",
                resolved.display(),
                link.display()
            )
        })
    } else {
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(&win_target, link).is_ok() {
            return Ok(());
        }
        Ok(())
    }
}

#[cfg(not(unix))]
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("failed creating {}: {e}", dst.display()))?;
    let entries =
        fs::read_dir(src).map_err(|e| format!("failed reading {}: {e}", src.display()))?;
    for entry in entries.flatten() {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if src_path.is_file() && fs::hard_link(&src_path, &dst_path).is_err() {
            fs::copy(&src_path, &dst_path).map_err(|e| {
                format!(
                    "failed copying {} -> {}: {e}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

/// Returns the platform-native wrapper path inside `bin_dir`
/// (`<name>.cmd` on Windows, `<name>` on Unix).
pub fn wrapper_path(bin_dir: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        bin_dir.join(format!("{name}.cmd"))
    } else {
        bin_dir.join(name)
    }
}

/// Writes a platform-native compiler wrapper script (`<name>.cmd` on Windows,
/// executable POSIX `#!/bin/sh` on Unix).
pub fn write_clang_wrapper(
    bin_dir: &Path,
    name: &str,
    compiler: &str,
    args: &str,
) -> Result<PathBuf, String> {
    let path = wrapper_path(bin_dir, name);
    let contents = if cfg!(windows) {
        format!("@echo off\r\n\"{compiler}\" {args} %*\r\n")
    } else {
        format!("#!/bin/sh\nexec \"{compiler}\" {args} \"$@\"\n")
    };
    write_executable_script(&path, &contents)?;
    Ok(path)
}

/// Writes an executable POSIX shell wrapper script at `path`.
pub fn write_executable_script(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|e| format!("failed writing {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .map_err(|e| format!("failed stating {}: {e}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)
            .map_err(|e| format!("failed chmod {}: {e}", path.display()))?;
    }
    Ok(())
}
