use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use gtk4::glib;

static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();

pub fn log_path() -> PathBuf {
    crate::config::Config::cache_dir().join("app.log")
}

pub fn init() {
    let dir = crate::config::Config::cache_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("[WARN] [logging] cannot create cache dir {}: {e}", dir.display());
        return;
    }

    let path = log_path();
    if let Ok(meta) = fs::metadata(&path)
        && meta.len() > 1_048_576
    {
        let _ = fs::write(&path, b"");
    }

    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => {
            let _ = LOG_FILE.set(Mutex::new(file));
        }
        Err(e) => {
            eprintln!("[WARN] [logging] cannot open {}: {e}", path.display());
        }
    }
}

fn timestamp() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|dt| dt.format("%Y-%m-%d %H:%M:%S").ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "0000-00-00 00:00:00".to_string())
}

pub fn write_log(level: &str, target: &str, msg: &str) {
    let line = format!("[{}] [{level}] [{target}] {msg}", timestamp());
    eprintln!("{line}");
    if let Some(lock) = LOG_FILE.get()
        && let Ok(mut file) = lock.lock()
    {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

#[macro_export]
macro_rules! log_info {
    ($target:expr, $($arg:tt)*) => {
        $crate::logging::write_log("INFO", $target, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($target:expr, $($arg:tt)*) => {
        $crate::logging::write_log("WARN", $target, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_err {
    ($target:expr, $($arg:tt)*) => {
        $crate::logging::write_log("ERROR", $target, &format!($($arg)*))
    };
}
