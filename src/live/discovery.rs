//! Finding the running Pixmith app: `<app data>/Live/<pid>.json` files (see the Live protocol).

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The protocol version this client speaks.
pub const PROTOCOL: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

pub fn current_os() -> Os {
    if cfg!(target_os = "macos") {
        Os::Mac
    } else if cfg!(windows) {
        Os::Windows
    } else {
        Os::Linux
    }
}

/// The folder the apps keep scripts, recovery files and the `Live` folder in. `env` looks up an environment
/// variable (a parameter so tests never touch the process environment).
pub fn app_data_dir(os: Os, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let get = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    let name = if os == Os::Linux { "pixmith" } else { "Pixmith" };
    if let Some(dir) = get("PIXMITH_SUPPORT_DIR") {
        return Some(PathBuf::from(dir).join(name));
    }
    let home = || get("HOME").or_else(|| (os == Os::Windows).then(|| get("USERPROFILE")).flatten()).map(PathBuf::from);
    match os {
        Os::Mac => Some(home()?.join("Library").join("Application Support").join(name)),
        Os::Windows => Some(PathBuf::from(get("APPDATA")?).join(name)),
        // Absolute by the Linux rule (a leading `/`), so the result does not depend on the OS running the check.
        Os::Linux => match get("XDG_DATA_HOME").filter(|p| p.starts_with('/')) {
            Some(x) => Some(PathBuf::from(x).join(name)),
            None => Some(home()?.join(".local").join("share").join(name)),
        },
    }
}

/// `<app data>/Live` for this machine, from the process environment.
pub fn live_dir() -> Option<PathBuf> {
    app_data_dir(current_os(), &|k| std::env::var(k).ok()).map(|d| d.join("Live"))
}

/// One discovery file.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Entry {
    pub protocol: u32,
    pub pid: u32,
    pub port: u16,
    pub token: String,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub front_end: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub started: u64,
    #[serde(skip)]
    pub file: PathBuf,
}

/// Whether a process with this id is running. Where that cannot be asked (Windows), `true`: a dead process
/// is then found out by its refused port.
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(p) = i32::try_from(pid) else { return false };
        if p <= 0 {
            return false;
        }
        // SAFETY: signal 0 only checks that the process exists and may be signalled.
        let r = unsafe { libc::kill(p, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// The usable discovery files of `dir`, newest `started` first (file modification time breaks ties). Files that
/// are unreadable, malformed or name a dead process are left out; with `cleanup` the dead-process files are
/// deleted, as the protocol allows.
pub fn scan(dir: &Path, alive: &dyn Fn(u32) -> bool, cleanup: bool) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(Entry, std::time::SystemTime)> = Vec::new();
    for item in rd.flatten() {
        let path = item.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(mut entry) = serde_json::from_str::<Entry>(&text) else { continue };
        if !alive(entry.pid) {
            if cleanup {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        entry.file = path;
        let modified = item.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        found.push((entry, modified));
    }
    found.sort_by(|a, b| b.0.started.cmp(&a.0.started).then(b.1.cmp(&a.1)).then(b.0.pid.cmp(&a.0.pid)));
    found.into_iter().map(|(e, _)| e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn app_data_folders_follow_the_protocol() {
        let mac = app_data_dir(Os::Mac, &env(&[("HOME", "/Users/me")])).unwrap();
        assert_eq!(mac, Path::new("/Users/me/Library/Application Support/Pixmith"));
        let win = app_data_dir(Os::Windows, &env(&[("APPDATA", "C:\\Users\\me\\AppData\\Roaming")])).unwrap();
        assert!(win.ends_with("Pixmith") && win.starts_with("C:\\Users\\me\\AppData\\Roaming"));
        let linux = app_data_dir(Os::Linux, &env(&[("HOME", "/home/me")])).unwrap();
        assert_eq!(linux, Path::new("/home/me/.local/share/pixmith"));
        let xdg = app_data_dir(Os::Linux, &env(&[("HOME", "/home/me"), ("XDG_DATA_HOME", "/data")])).unwrap();
        assert_eq!(xdg, Path::new("/data/pixmith"));
        // A relative XDG_DATA_HOME is ignored, as the XDG spec says.
        let rel = app_data_dir(Os::Linux, &env(&[("HOME", "/home/me"), ("XDG_DATA_HOME", "data")])).unwrap();
        assert_eq!(rel, Path::new("/home/me/.local/share/pixmith"));
        assert!(app_data_dir(Os::Mac, &env(&[])).is_none());
    }

    #[test]
    fn support_dir_replaces_the_platform_folder() {
        let e = env(&[("PIXMITH_SUPPORT_DIR", "/tmp/x"), ("HOME", "/Users/me")]);
        assert_eq!(app_data_dir(Os::Mac, &e).unwrap(), Path::new("/tmp/x/Pixmith"));
        assert_eq!(app_data_dir(Os::Windows, &e).unwrap(), Path::new("/tmp/x/Pixmith"));
        assert_eq!(app_data_dir(Os::Linux, &e).unwrap(), Path::new("/tmp/x/pixmith"));
        // An empty value counts as unset.
        let empty = env(&[("PIXMITH_SUPPORT_DIR", " "), ("HOME", "/Users/me")]);
        assert!(app_data_dir(Os::Mac, &empty).unwrap().starts_with("/Users/me/Library"));
    }

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn file(pid: u32, started: u64) -> String {
        format!(
            r#"{{"protocol":1,"pid":{pid},"port":{},"token":"t{pid}","app":"Pixmith","version":"0.5.0","front_end":"swift","platform":"macos","started":{started}}}"#,
            40000 + pid
        )
    }

    #[test]
    fn scan_ignores_dead_and_broken_files_and_sorts_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "10.json", &file(10, 100));
        write(dir.path(), "11.json", &file(11, 300));
        write(dir.path(), "12.json", &file(12, 200));
        write(dir.path(), "13.json", &file(13, 999)); // dead
        write(dir.path(), "junk.json", "not json");
        write(dir.path(), "notes.txt", &file(14, 1000));
        let alive = |pid: u32| pid != 13;
        let found = scan(dir.path(), &alive, false);
        assert_eq!(found.iter().map(|e| e.pid).collect::<Vec<_>>(), vec![11, 12, 10]);
        assert_eq!(found[0].token, "t11");
        assert_eq!(found[0].port, 40011);
        assert!(dir.path().join("13.json").exists(), "no cleanup requested");
        scan(dir.path(), &alive, true);
        assert!(!dir.path().join("13.json").exists());
        assert!(dir.path().join("junk.json").exists(), "malformed files are left alone");
        assert!(scan(&dir.path().join("missing"), &alive, true).is_empty());
    }

    #[test]
    fn pid_alive_knows_this_process() {
        assert!(pid_alive(std::process::id()));
        #[cfg(unix)]
        assert!(!pid_alive(0x7fff_fff0));
    }
}
