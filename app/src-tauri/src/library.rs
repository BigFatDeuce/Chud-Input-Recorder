//! The macro library: plain folders and `.chud` files on disk, so it can be
//! browsed, synced or backed up with ordinary tools.
//!
//! * File names follow macro names (unique within a folder).
//! * Deleting moves to `.trash/` (restorable); only "empty trash" deletes.
//! * Each save keeps the previous file in `.versions/<id>/`.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use chud_core::document::MacroDoc;
use chud_core::format::{self, Manifest};
use parking_lot::Mutex;
use serde::Serialize;

const TRASH: &str = ".trash";
const VERSIONS: &str = ".versions";

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: String,
    /// Path relative to the library root, `/` separated.
    pub path: String,
    /// Folder relative to the root ("" for the root).
    pub folder: String,
    pub manifest: Manifest,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    pub root: String,
    pub folders: Vec<String>,
    pub entries: Vec<Entry>,
    /// Files that could not be read (path, error).
    pub broken: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrashItem {
    pub name: String,
    pub is_dir: bool,
    pub deleted_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Version {
    pub ms: i64,
    pub size: u64,
}

pub struct Library {
    root: PathBuf,
    cache: Mutex<HashMap<PathBuf, (SystemTime, u64, Manifest)>>,
    index: Mutex<HashMap<String, PathBuf>>,
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// File-system safe version of a display name.
pub fn sanitize(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    s = s.trim().trim_end_matches('.').trim().to_string();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if s.is_empty() || RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&s)) || s.starts_with('.') {
        s = format!("_{s}");
    }
    s.chars().take(120).collect()
}

fn unique(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mk = |n: usize| {
        let name = if n == 0 { format!("{stem}{ext}") } else { format!("{stem} ({n}){ext}") };
        dir.join(name)
    };
    (0..).map(mk).find(|p| !p.exists()).expect("unbounded")
}

fn now_ms() -> i64 {
    chud_core::document::now_ms()
}

impl Library {
    pub fn new(root: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root, cache: Mutex::default(), index: Mutex::default() })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a relative folder path, refusing anything leaving the root.
    pub fn folder(&self, rel_path: &str) -> Result<PathBuf, String> {
        let mut p = self.root.clone();
        for part in rel_path.split(['/', '\\']).filter(|s| !s.is_empty()) {
            let c = Path::new(part).components().next();
            if !matches!(c, Some(Component::Normal(_))) || part.starts_with('.') {
                return Err(format!("invalid folder: {rel_path}"));
            }
            p.push(part);
        }
        Ok(p)
    }

    fn manifest_cached(&self, path: &Path) -> Result<(Manifest, u64), String> {
        let md = std::fs::metadata(path).map_err(|e| e.to_string())?;
        let mtime = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let size = md.len();
        if let Some((t, s, m)) = self.cache.lock().get(path)
            && *t == mtime && *s == size {
                return Ok((m.clone(), size));
            }
        let m = format::load_manifest(path).map_err(|e| e.to_string())?;
        self.cache.lock().insert(path.to_path_buf(), (mtime, size, m.clone()));
        Ok((m, size))
    }

    pub fn list(&self) -> Listing {
        let mut folders = Vec::new();
        let mut entries = Vec::new();
        let mut broken = Vec::new();
        let mut index = HashMap::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let path = e.path();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    folders.push(rel(&self.root, &path));
                    stack.push(path);
                } else if path.extension().is_some_and(|x| x.eq_ignore_ascii_case(format::EXTENSION)) {
                    match self.manifest_cached(&path) {
                        Ok((manifest, size)) => {
                            let id = manifest.meta.id.clone();
                            if index.contains_key(&id) {
                                broken.push((rel(&self.root, &path), format!("duplicate id {id}")));
                                continue;
                            }
                            index.insert(id.clone(), path.clone());
                            entries.push(Entry {
                                id,
                                path: rel(&self.root, &path),
                                folder: rel(&self.root, &dir),
                                manifest,
                                size,
                            });
                        }
                        Err(err) => broken.push((rel(&self.root, &path), err)),
                    }
                }
            }
        }
        folders.sort_by_key(|f| f.to_lowercase());
        *self.index.lock() = index;
        Listing { root: self.root.to_string_lossy().to_string(), folders, entries, broken }
    }

    pub fn path_of(&self, id: &str) -> Option<PathBuf> {
        if let Some(p) = self.index.lock().get(id).filter(|p| p.exists()) {
            return Some(p.clone());
        }
        self.list();
        self.index.lock().get(id).cloned()
    }

    pub fn load(&self, id: &str) -> Result<(MacroDoc, PathBuf), String> {
        let path = self.path_of(id).ok_or_else(|| format!("macro {id} not found"))?;
        let doc = format::load(&path).map_err(|e| e.to_string())?;
        Ok((doc, path))
    }

    /// Saves a new document into `folder`, named after the macro.
    pub fn save_new(&self, doc: &MacroDoc, folder: &str) -> Result<PathBuf, String> {
        let dir = self.folder(folder)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = unique(&dir, &sanitize(&doc.meta.name), &format!(".{}", format::EXTENSION));
        format::save(doc, &path).map_err(|e| e.to_string())?;
        self.index.lock().insert(doc.meta.id.clone(), path.clone());
        Ok(path)
    }

    /// Overwrites a document, keeping the previous file as a version.
    pub fn save_over(&self, doc: &MacroDoc, path: &Path, keep: usize) -> Result<(), String> {
        if keep > 0 && path.exists() {
            let vdir = self.version_dir(&doc.meta.id);
            if std::fs::create_dir_all(&vdir).is_ok() {
                let _ = std::fs::copy(path, vdir.join(format!("{}.chud", now_ms())));
                self.prune_versions(&doc.meta.id, keep);
            }
        }
        format::save(doc, path).map_err(|e| e.to_string())
    }

    /// Ids come from files and may contain anything: never use them raw.
    fn version_dir(&self, id: &str) -> PathBuf {
        self.root.join(VERSIONS).join(sanitize(id))
    }

    fn prune_versions(&self, id: &str, keep: usize) {
        let mut v = self.versions(id);
        if v.len() > keep {
            v.sort_by_key(|x| x.ms);
            for old in &v[..v.len() - keep] {
                let _ = std::fs::remove_file(self.version_dir(id).join(format!("{}.chud", old.ms)));
            }
        }
    }

    pub fn versions(&self, id: &str) -> Vec<Version> {
        let dir = self.version_dir(id);
        let mut out: Vec<Version> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let ms = e.path().file_stem()?.to_str()?.parse().ok()?;
                Some(Version { ms, size: e.metadata().ok()?.len() })
            })
            .collect();
        out.sort_by_key(|v| std::cmp::Reverse(v.ms));
        out
    }

    pub fn load_version(&self, id: &str, ms: i64) -> Result<MacroDoc, String> {
        let p = self.version_dir(id).join(format!("{ms}.chud"));
        format::load(&p).map_err(|e| e.to_string())
    }

    /// Renames the file to follow a new macro name (same folder).
    pub fn rename_file(&self, id: &str, name: &str) -> Result<PathBuf, String> {
        let path = self.path_of(id).ok_or("not found")?;
        let dir = path.parent().ok_or("no parent")?.to_path_buf();
        let stem = sanitize(name);
        let ext = format!(".{}", format::EXTENSION);
        if path.file_name().is_some_and(|f| f.to_string_lossy() == format!("{stem}{ext}")) {
            return Ok(path);
        }
        let target = unique(&dir, &stem, &ext);
        std::fs::rename(&path, &target).map_err(|e| e.to_string())?;
        self.index.lock().insert(id.to_string(), target.clone());
        Ok(target)
    }

    pub fn move_entry(&self, id: &str, folder: &str) -> Result<PathBuf, String> {
        let path = self.path_of(id).ok_or("not found")?;
        let dir = self.folder(folder)?;
        if path.parent() == Some(dir.as_path()) {
            return Ok(path);
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let target = unique(&dir, &stem, &format!(".{}", format::EXTENSION));
        std::fs::rename(&path, &target).map_err(|e| e.to_string())?;
        self.index.lock().insert(id.to_string(), target.clone());
        Ok(target)
    }

    pub fn create_folder(&self, parent: &str, name: &str) -> Result<String, String> {
        let dir = self.folder(parent)?;
        let target = unique(&dir, &sanitize(name), "");
        std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        Ok(rel(&self.root, &target))
    }

    pub fn rename_folder(&self, folder: &str, name: &str) -> Result<String, String> {
        let dir = self.folder(folder)?;
        if dir == self.root {
            return Err("cannot rename the library root".into());
        }
        let parent = dir.parent().ok_or("no parent")?;
        let target = unique(parent, &sanitize(name), "");
        std::fs::rename(&dir, &target).map_err(|e| e.to_string())?;
        self.list();
        Ok(rel(&self.root, &target))
    }

    pub fn move_folder(&self, folder: &str, into: &str) -> Result<String, String> {
        let dir = self.folder(folder)?;
        let dest = self.folder(into)?;
        if dir == self.root || dest.starts_with(&dir) {
            return Err("cannot move a folder into itself".into());
        }
        let name = dir.file_name().ok_or("no name")?.to_string_lossy().to_string();
        let target = unique(&dest, &name, "");
        std::fs::rename(&dir, &target).map_err(|e| e.to_string())?;
        self.list();
        Ok(rel(&self.root, &target))
    }

    fn to_trash(&self, path: &Path) -> Result<(), String> {
        let trash = self.root.join(TRASH);
        std::fs::create_dir_all(&trash).map_err(|e| e.to_string())?;
        let name = path.file_name().ok_or("no name")?.to_string_lossy().to_string();
        let target = trash.join(format!("{}~{}", now_ms(), name));
        std::fs::rename(path, &target).map_err(|e| e.to_string())
    }

    pub fn trash_entry(&self, id: &str) -> Result<(), String> {
        let path = self.path_of(id).ok_or("not found")?;
        self.to_trash(&path)?;
        self.index.lock().remove(id);
        Ok(())
    }

    pub fn trash_folder(&self, folder: &str) -> Result<(), String> {
        let dir = self.folder(folder)?;
        if dir == self.root {
            return Err("cannot delete the library root".into());
        }
        self.to_trash(&dir)?;
        self.list();
        Ok(())
    }

    pub fn trash(&self) -> Vec<TrashItem> {
        let mut out: Vec<TrashItem> = std::fs::read_dir(self.root.join(TRASH))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                let (ms, name) = n.split_once('~')?;
                Some(TrashItem {
                    name: name.to_string(),
                    is_dir: e.file_type().ok()?.is_dir(),
                    deleted_ms: ms.parse().ok()?,
                })
            })
            .collect();
        out.sort_by_key(|t| std::cmp::Reverse(t.deleted_ms));
        out
    }

    pub fn restore(&self, deleted_ms: i64, name: &str) -> Result<(), String> {
        let src = self.root.join(TRASH).join(format!("{deleted_ms}~{name}"));
        if !src.exists() || name.contains(['/', '\\']) {
            return Err("not in trash".into());
        }
        let p = Path::new(name);
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let target = unique(&self.root, &stem, &ext);
        std::fs::rename(&src, &target).map_err(|e| e.to_string())?;
        self.list();
        Ok(())
    }

    /// Permanently deletes everything in the trash.
    pub fn empty_trash(&self) -> Result<usize, String> {
        let dir = self.root.join(TRASH);
        let n = self.trash().len();
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chud_core::event::{EventKind, Key};

    fn lib() -> (Library, PathBuf) {
        let dir = std::env::temp_dir().join(format!("chud-lib-{}-{}", std::process::id(), now_ms()));
        (Library::new(dir.clone()).unwrap(), dir)
    }

    fn doc(name: &str) -> MacroDoc {
        let k = Key::new(0x1E, 0x41);
        let mut d = MacroDoc::from_recording(
            name,
            vec![(0, EventKind::KeyDown { key: k, repeat: false }), (1000, EventKind::KeyUp { key: k })],
            Default::default(),
        );
        d.meta.id = format!("id-{name}");
        d
    }

    #[test]
    fn lifecycle() {
        let (l, dir) = lib();
        let a = doc("Farm: loop?");
        let p = l.save_new(&a, "").unwrap();
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "Farm_ loop_.chud");
        // Same name: unique file.
        let mut b = doc("Farm: loop?");
        b.meta.id = "other".into();
        let p2 = l.save_new(&b, "").unwrap();
        assert!(p2.to_string_lossy().ends_with("Farm_ loop_ (1).chud"));

        let f = l.create_folder("", "Games").unwrap();
        assert_eq!(f, "Games");
        l.move_entry("other", "Games").unwrap();
        let listing = l.list();
        assert_eq!(listing.entries.len(), 2);
        assert!(listing.entries.iter().any(|e| e.folder == "Games" && e.id == "other"));
        assert_eq!(listing.folders, vec!["Games".to_string()]);

        // Versions on overwrite.
        l.save_over(&a, &p, 3).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        l.save_over(&a, &p, 3).unwrap();
        assert_eq!(l.versions(&a.meta.id).len(), 2);
        assert!(l.load_version(&a.meta.id, l.versions(&a.meta.id)[0].ms).is_ok());

        // Rename follows the name.
        let r = l.rename_file(&a.meta.id, "Renamed").unwrap();
        assert!(r.ends_with("Renamed.chud"));

        // Trash and restore.
        l.trash_entry(&a.meta.id).unwrap();
        assert_eq!(l.list().entries.len(), 1);
        let t = l.trash();
        assert_eq!(t.len(), 1);
        l.restore(t[0].deleted_ms, &t[0].name).unwrap();
        assert_eq!(l.list().entries.len(), 2);

        // Escaping the root is refused.
        assert!(l.folder("../x").is_err());
        assert!(l.folder(".trash").is_err());
        l.trash_folder("Games").unwrap();
        assert_eq!(l.empty_trash().unwrap(), 1);
        assert_eq!(l.list().entries.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("a/b\\c:d"), "a_b_c_d");
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("  .. "), "_");
        assert_eq!(sanitize(".hidden"), "_.hidden");
    }
}
