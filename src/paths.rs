//! File access policy. With `--root <dir>` every path a tool reads or writes must resolve (symbolic links
//! followed) to a location inside that directory; relative paths are taken relative to the root. Without a
//! root, relative paths are taken relative to the server's working directory.

use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct Sandbox {
    /// Canonical root directory, if restricted.
    root: Option<PathBuf>,
    /// Base for relative paths when there is no root.
    cwd: PathBuf,
}

impl Sandbox {
    pub fn new(root: Option<&Path>) -> Result<Sandbox, String> {
        let cwd = std::env::current_dir().map_err(|e| format!("cannot read the working directory: {e}"))?;
        let root = match root {
            Some(r) => {
                let c = std::fs::canonicalize(r).map_err(|e| format!("--root {}: {e}", r.display()))?;
                if !c.is_dir() {
                    return Err(format!("--root {} is not a directory", r.display()));
                }
                Some(c)
            }
            None => None,
        };
        Ok(Sandbox { root, cwd })
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// The path as given, with `~/` expanded and relative paths anchored at the root (or working directory).
    fn absolute(&self, p: &str) -> Result<PathBuf, String> {
        let p = p.trim();
        if p.is_empty() {
            return Err("empty path".into());
        }
        if p.contains('\0') {
            return Err("path contains a NUL byte".into());
        }
        let expanded = match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
            Some(rest) => home().ok_or("cannot expand ~: HOME is not set")?.join(rest),
            None if p == "~" => home().ok_or("cannot expand ~: HOME is not set")?,
            None => PathBuf::from(p),
        };
        Ok(if expanded.is_absolute() { expanded } else { self.root.as_ref().unwrap_or(&self.cwd).join(expanded) })
    }

    fn check(&self, resolved: &Path, shown: &str) -> Result<(), String> {
        match &self.root {
            Some(root) if !resolved.starts_with(root) => {
                Err(format!("{shown}: outside the allowed root {} (server started with --root)", root.display()))
            }
            _ => Ok(()),
        }
    }

    /// An existing file or directory to read.
    pub fn read(&self, p: &str) -> Result<PathBuf, String> {
        let abs = self.absolute(p)?;
        let resolved = std::fs::canonicalize(&abs).map_err(|e| format!("{p}: {e}"))?;
        self.check(&resolved, p)?;
        Ok(resolved)
    }

    /// An existing directory.
    pub fn dir(&self, p: &str) -> Result<PathBuf, String> {
        let d = self.read(p)?;
        if !d.is_dir() {
            return Err(format!("{p}: not a directory"));
        }
        Ok(d)
    }

    /// A file to write. Missing parent directories are created (inside the root). An existing symbolic link
    /// is resolved, since the core writes through links.
    pub fn write(&self, p: &str) -> Result<PathBuf, String> {
        let abs = self.absolute(p)?;
        if abs.is_dir() {
            return Err(format!("{p}: is a directory; give a file path"));
        }
        if let Ok(meta) = std::fs::symlink_metadata(&abs)
            && (meta.file_type().is_symlink() || meta.is_file())
            && let Ok(target) = std::fs::canonicalize(&abs)
        {
            self.check(&target, p)?;
            return Ok(target);
        }
        let name = abs.file_name().ok_or_else(|| format!("{p}: no file name"))?.to_owned();
        let parent = abs.parent().ok_or_else(|| format!("{p}: no parent directory"))?;
        // Find the nearest existing ancestor; what is below it must be plain names (no `..`).
        let mut existing = parent.to_path_buf();
        let mut missing: Vec<std::ffi::OsString> = Vec::new();
        while !existing.exists() {
            let last = existing.file_name().ok_or_else(|| format!("{p}: invalid path"))?.to_owned();
            missing.push(last);
            existing = existing.parent().ok_or_else(|| format!("{p}: invalid path"))?.to_path_buf();
        }
        if parent.components().any(|c| c == Component::ParentDir) && !missing.is_empty() {
            return Err(format!("{p}: '..' is not allowed in a directory that does not exist yet"));
        }
        let base = std::fs::canonicalize(&existing).map_err(|e| format!("{p}: {e}"))?;
        let mut dir = base;
        for m in missing.iter().rev() {
            dir.push(m);
        }
        self.check(&dir, p)?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{p}: cannot create {}: {e}", dir.display()))?;
        let out = dir.join(name);
        self.check(&out, p)?;
        Ok(out)
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// Display form of a path for tool results.
pub fn show(p: &Path) -> String {
    let s = p.to_string_lossy();
    // Windows verbatim prefix from canonicalize.
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_confines_reads_and_writes() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.png"), b"x").unwrap();
        std::fs::write(t.path().join("outside.png"), b"x").unwrap();
        let s = Sandbox::new(Some(&root)).unwrap();
        assert!(s.read("a.png").is_ok());
        assert!(s.read("../outside.png").is_err());
        assert!(s.read(&t.path().join("outside.png").to_string_lossy()).is_err());
        assert!(s.write("out/sub/b.png").is_ok());
        assert!(root.join("out/sub").is_dir());
        assert!(s.write("../escape.png").is_err());
        assert!(s.write("new/../../escape.png").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_resolved() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(t.path().join("secret.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(t.path().join("secret.txt"), root.join("link.gpl")).unwrap();
        std::os::unix::fs::symlink(t.path(), root.join("dirlink")).unwrap();
        let s = Sandbox::new(Some(&root)).unwrap();
        assert!(s.read("link.gpl").is_err());
        assert!(s.write("link.gpl").is_err());
        assert!(s.write("dirlink/x.png").is_err());
    }
}
