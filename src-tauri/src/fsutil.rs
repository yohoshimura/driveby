//! Cross-platform filesystem helpers shared by the backup and restore
//! pipelines: extended-length path handling, Windows file attributes, and
//! the source/destination overlap rejection that both pipelines need.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use tracing::warn;

// ─────────────────────────────────────────────────────────────────────
// Extended-length paths
// ─────────────────────────────────────────────────────────────────────

#[cfg(windows)]
pub fn long_path(p: &Path) -> PathBuf {
    // `\\?\`-prefixed paths require backslashes only — Windows treats forward
    // slashes under that prefix as literal filename characters. Normalize
    // separators *first*, then apply the prefix.
    let normalized: String = p.as_os_str().to_string_lossy().replace('/', r"\");
    if normalized.starts_with(r"\\?\") || normalized.starts_with(r"\\.\") {
        return PathBuf::from(normalized);
    }
    if Path::new(&normalized).is_absolute() {
        if let Some(rest) = normalized.strip_prefix(r"\\") {
            return PathBuf::from(format!(r"\\?\UNC\{}", rest));
        }
        return PathBuf::from(format!(r"\\?\{}", normalized));
    }
    PathBuf::from(normalized)
}

#[cfg(not(windows))]
pub fn long_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// Scratch file a copy streams into before it is renamed onto the real
/// destination. Both pipelines write here first so that an unreadable
/// source, a failed write or a cancellation can never damage the file
/// already sitting at `dest` — the swap only happens once the bytes are
/// safely on disk. It lives in the destination's own directory so the
/// rename stays within one volume, which is what makes it atomic.
///
/// A run killed mid-copy leaves one behind. The backup pipeline's prune
/// sweeps it, since a scratch file is never in `keep`.
pub fn scratch_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".driveby-tmp");
    dest.with_file_name(name)
}

// ─────────────────────────────────────────────────────────────────────
// Windows file attributes (preserves Hidden/System/ReadOnly so that
// custom-folder-icon machinery — `desktop.ini` + the parent's System
// attribute — keeps working in the destination tree)
// ─────────────────────────────────────────────────────────────────────

/// Attribute bits we consider user-meaningful and therefore mirror from
/// source to destination. Everything else (ARCHIVE, REPARSE_POINT, …) is
/// managed by the OS and must not be propagated.
pub const ATTR_KEEP: u32 = 0x1 /*READONLY*/ | 0x2 /*HIDDEN*/ | 0x4 /*SYSTEM*/;
#[cfg(windows)]
const ATTR_READONLY: u32 = 0x1;

#[cfg(windows)]
pub fn read_attrs(p: &Path) -> Option<u32> {
    use std::os::windows::fs::MetadataExt;
    std::fs::metadata(long_path(p))
        .ok()
        .map(|m| m.file_attributes())
}

#[cfg(windows)]
fn set_attrs(p: &Path, attrs: u32) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Storage::FileSystem::SetFileAttributesW;
    let lp = long_path(p);
    let wide: Vec<u16> = lp
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ok = unsafe { SetFileAttributesW(wide.as_ptr(), attrs) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        warn!(
            "SetFileAttributesW failed for {}: GetLastError={}",
            lp.display(),
            err
        );
        return false;
    }
    true
}

#[cfg(windows)]
pub fn apply_attrs(p: &Path, attrs: u32) {
    // The pre-1.4 version ignored the BOOL return value, so a failure to
    // mirror the parent-folder bit (which is what makes Explorer render a
    // custom desktop.ini icon) was silently invisible. set_attrs logs on
    // failure so the bug can't hide again.
    let masked = attrs & ATTR_KEEP;
    if masked == 0 {
        return;
    }
    set_attrs(p, masked);
}

/// Drop the READONLY bit from `p`, if it has one.
///
/// `apply_attrs` deliberately mirrors READONLY from source to destination,
/// so the destination tree accumulates read-only files and directories.
/// Measured behaviour of the std calls we make against them on Windows
/// (NTFS, rustc 1.95):
///
/// | call                        | on a `+R` target |
/// |-----------------------------|------------------|
/// | `fs::remove_file`           | **Ok** — std deletes with `FILE_DISPOSITION_IGNORE_READONLY_ATTRIBUTE` |
/// | `fs::File::create`          | `PermissionDenied` |
/// | `fs::remove_dir`            | `PermissionDenied` |
///
/// So the two paths that actually break are `remove_dir` (the prune pass
/// can never delete an emptied custom-icon folder, which carries `+R` by
/// construction) and any `File::create` that isn't preceded by a delete —
/// which is exactly `restore::copy`.
///
/// The copy path is additionally guarded even though `remove_file` covers
/// it on NTFS: that std fast path needs `FileDispositionInfoEx`, which
/// FAT32/exFAT do not support, and exFAT is the common format for the
/// external drives this app exists to write to. Clearing the bit first
/// costs one metadata call and removes the filesystem dependency.
#[cfg(windows)]
pub fn clear_readonly(p: &Path) {
    let Some(attrs) = read_attrs(p) else {
        return; // absent or unreadable — nothing to clear
    };
    if attrs & ATTR_READONLY == 0 {
        return;
    }
    set_attrs(p, attrs & !ATTR_READONLY);
}

/// The exact on-disk spelling of `p`'s final component, resolved
/// case-insensitively — i.e. what the directory entry is actually called,
/// as opposed to what the caller spelled it as. None if the entry does not
/// exist (or `p` has no final component, e.g. a drive root).
#[cfg(windows)]
pub fn on_disk_name(p: &Path) -> Option<std::ffi::OsString> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{FindClose, FindFirstFileW, WIN32_FIND_DATAW};
    let lp = long_path(p);
    let wide: Vec<u16> = lp
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let handle = unsafe { FindFirstFileW(wide.as_ptr(), &mut data) };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    unsafe { FindClose(handle) };
    let len = data
        .cFileName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(data.cFileName.len());
    Some(std::ffi::OsString::from_wide(&data.cFileName[..len]))
}

// TODO(macOS): APFS is case-insensitive by default too — give this a
// readdir-based lookup (and extend KeepSet::by_lowercase) when directory
// recasing is brought to macOS.
#[cfg(not(windows))]
pub fn on_disk_name(_p: &Path) -> Option<std::ffi::OsString> {
    None
}

#[cfg(not(windows))]
pub fn read_attrs(_p: &Path) -> Option<u32> {
    None
}

#[cfg(not(windows))]
pub fn apply_attrs(_p: &Path, _attrs: u32) {}

#[cfg(not(windows))]
pub fn clear_readonly(_p: &Path) {}

// ─────────────────────────────────────────────────────────────────────
// Source / destination overlap rejection
// ─────────────────────────────────────────────────────────────────────

/// True if `child` equals or is nested under `parent` (case-insensitive on
/// Windows). Both inputs must be absolute. Falls back to lossy string compare
/// if the paths can't be canonicalised yet (e.g. the destination doesn't
/// exist) — callers validate existence first. Critically: on Windows,
/// `canonicalize()` prepends `\\?\` to existing paths but not to non-existing
/// ones, so we strip that prefix on both sides before comparing — otherwise
/// an existing parent would never appear to "contain" a not-yet-created child
/// even when it lexically does.
pub fn path_contains(parent: &Path, child: &Path) -> bool {
    let p = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    let c = std::fs::canonicalize(child).unwrap_or_else(|_| child.to_path_buf());
    let p_norm = normalize_for_compare(&p);
    let c_norm = normalize_for_compare(&c);
    if c_norm == p_norm {
        return true;
    }
    let mut prefix = p_norm.clone();
    if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
        prefix.push(std::path::MAIN_SEPARATOR);
    }
    c_norm.starts_with(&prefix)
}

#[cfg(windows)]
fn normalize_for_compare(p: &Path) -> String {
    let s = p.to_string_lossy().to_lowercase().replace('/', r"\");
    // Strip the verbatim/extended-length prefix Windows' canonicalize adds
    // to paths that exist on disk. We compare a (possibly) extended-length
    // path against a (probably) non-extended one, so they must agree on
    // surface form. UNC variant first to avoid a false match against `\\?\`.
    if let Some(rest) = s.strip_prefix(r"\\?\unc\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    }
}

#[cfg(not(windows))]
fn normalize_for_compare(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// Reject any nesting between a read side and a write side.
///
/// Backup: if the destination sits inside the source, `walk()` would
/// enumerate the destination's own contents, copy them onto themselves, and
/// the prune pass would loop on its own output; if the source sits inside the
/// destination, prune would wipe the source on the next run.
///
/// Restore: the same call protects a far sharper edge — with
/// `destination == backup_path`, `File::create(dst)` truncates the very file
/// `File::open(src)` is about to read, so every file in the backup is emptied
/// and the run still reports success.
pub fn reject_overlap(source: &Path, destination: &Path) -> Result<()> {
    if path_contains(source, destination) {
        return Err(anyhow!("Destination cannot be inside the source folder"));
    }
    if path_contains(destination, source) {
        return Err(anyhow!("Source cannot be inside the destination folder"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn long_path_prefixes_absolute() {
        let p = Path::new(r"C:\Users\me\file.txt");
        assert_eq!(long_path(p).to_string_lossy(), r"\\?\C:\Users\me\file.txt");
    }

    #[cfg(windows)]
    #[test]
    fn long_path_leaves_prefixed_alone() {
        let p = Path::new(r"\\?\C:\foo");
        assert_eq!(long_path(p).to_string_lossy(), r"\\?\C:\foo");
    }

    #[cfg(windows)]
    #[test]
    fn long_path_handles_unc() {
        let p = Path::new(r"\\server\share\file");
        assert_eq!(long_path(p).to_string_lossy(), r"\\?\UNC\server\share\file");
    }

    #[cfg(windows)]
    #[test]
    fn long_path_normalizes_forward_slashes() {
        let p = Path::new("C:/Users/me/sub/file.txt");
        assert_eq!(
            long_path(p).to_string_lossy(),
            r"\\?\C:\Users\me\sub\file.txt"
        );
    }

    // The path-overlap rejection is security-relevant: without it,
    // "destination inside source" would copy the destination onto itself and
    // the prune pass would loop on its own output, "source inside
    // destination" would let prune wipe the source on the next run, and
    // restore-onto-itself would empty every file. Tests are platform-aware
    // because canonicalize() requires the path to exist on Windows; we use
    // temp-dir-relative paths that *do* exist so the comparison is
    // meaningful.
    #[test]
    fn path_contains_self_is_true() {
        let tmp = std::env::temp_dir();
        assert!(path_contains(&tmp, &tmp));
    }

    /// Test helper: create a temp directory under env::temp_dir() so both
    /// paths in path_contains() can canonicalize consistently. On Windows
    /// `temp_dir()` may return an 8.3 short-name path (`YOSHIM~1`) which
    /// `canonicalize()` expands; if one side of the comparison is the
    /// original short form and the other is the expanded long form, the
    /// prefix check fails. In production both paths are validated to exist
    /// before `path_contains` is called, so this is a test-fixture concern,
    /// not a bug.
    pub(crate) fn make_test_dir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("driveby-test-{}", name));
        let _ = std::fs::create_dir_all(&p);
        p
    }

    #[cfg(windows)]
    #[test]
    fn on_disk_name_reports_the_real_spelling() {
        let root = make_test_dir("on-disk-name");
        let dir = root.join("MixedCase");
        let _ = std::fs::remove_dir(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Query with the wrong case: the answer is what's actually on disk.
        assert_eq!(
            on_disk_name(&root.join("mixedcase")).unwrap(),
            std::ffi::OsStr::new("MixedCase")
        );
        assert!(on_disk_name(&root.join("absent")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn path_contains_child_is_true() {
        let parent = make_test_dir("contains-parent");
        let child = parent.join("nested");
        std::fs::create_dir_all(&child).unwrap();
        assert!(path_contains(&parent, &child));
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn path_contains_sibling_is_false() {
        // Siblings: `tmp/foo` and `tmp/bar` should not be considered nested.
        let foo = make_test_dir("sibling-foo");
        let bar = make_test_dir("sibling-bar");
        assert!(!path_contains(&foo, &bar));
        assert!(!path_contains(&bar, &foo));
        let _ = std::fs::remove_dir(&foo);
        let _ = std::fs::remove_dir(&bar);
    }

    #[test]
    fn path_contains_prefix_lookalike_is_false() {
        // Important: "/a/b" must not be considered to contain "/a/bb" just
        // because the string starts with "/a/b". The MAIN_SEPARATOR-padded
        // prefix check guards against this.
        let p = make_test_dir("lookalike-x");
        let q = make_test_dir("lookalike-xx");
        assert!(!path_contains(&p, &q));
        assert!(!path_contains(&q, &p));
        let _ = std::fs::remove_dir(&p);
        let _ = std::fs::remove_dir(&q);
    }

    #[test]
    fn reject_overlap_rejects_both_nesting_directions() {
        let parent = make_test_dir("overlap-parent");
        let child = parent.join("inner");
        std::fs::create_dir_all(&child).unwrap();
        // dest inside source
        assert!(reject_overlap(&parent, &child).is_err());
        // source inside dest
        assert!(reject_overlap(&child, &parent).is_err());
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn reject_overlap_rejects_identical_paths() {
        // The restore-onto-itself case: same folder on both sides truncates
        // every file it is about to read.
        let d = make_test_dir("overlap-same");
        assert!(reject_overlap(&d, &d).is_err());
        let _ = std::fs::remove_dir(&d);
    }

    #[test]
    fn reject_overlap_allows_siblings() {
        let a = make_test_dir("overlap-a");
        let b = make_test_dir("overlap-b");
        assert!(reject_overlap(&a, &b).is_ok());
        let _ = std::fs::remove_dir(&a);
        let _ = std::fs::remove_dir(&b);
    }
}
