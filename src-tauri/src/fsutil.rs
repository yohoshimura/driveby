//! Cross-platform filesystem helpers shared by the backup and restore
//! pipelines: extended-length path handling, Windows file attributes, and
//! the source/destination overlap rejection that both pipelines need.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use tracing::warn;

// ─────────────────────────────────────────────────────────────────────
// Case-folding filesystems
// ─────────────────────────────────────────────────────────────────────

/// Whether this target's usual filesystem compares filenames case-
/// insensitively while storing the spelling it was given. NTFS and APFS both
/// do; ext4 and friends do not.
///
/// Everything that has to reconcile a *source* spelling with a *destination*
/// spelling keys off this one constant instead of repeating a platform list.
/// Three places quietly disagreeing about it is what made #R6 possible — an
/// excluded folder deleted from the backup because the exclude matched the
/// source's casing and the prune pass saw the destination's.
///
/// Both filesystems are configurable — NTFS per directory, APFS per volume —
/// so `true` describes the default, not a guarantee. Every reader of it is
/// written to stay correct on a case-sensitive volume anyway: the extra
/// lookups simply never find a second spelling.
pub const CASE_INSENSITIVE_FS: bool = cfg!(any(windows, target_os = "macos"));

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

/// Answers "what is this directory entry *actually* called?" — the on-disk
/// spelling of a path's final component, as opposed to the spelling the
/// caller used to reach it. None when the entry does not exist under any
/// casing, when `p` has no final component (a drive root), or when the answer
/// would be a guess.
///
/// It is a struct rather than a free function because the two platforms
/// answer at very different prices. Windows resolves one path with one
/// `FindFirstFileW`. Everywhere else the only portable answer is to list the
/// parent — and `recase_dirs_phase` asks about every directory in the tree,
/// so a parent holding ten thousand of them would be listed ten thousand
/// times, quadratic in the width of the tree. Listings are therefore memoized
/// by parent, which brings the whole pass back down to one walk of the
/// destination. Windows has nothing to memoize and holds no state.
///
/// Each entry is asked about once, so a listing going stale behind a rename
/// this pass performs itself is never consulted again.
#[derive(Default)]
pub struct NameResolver {
    #[cfg(not(windows))]
    listings: std::collections::HashMap<PathBuf, DirNames>,
}

#[cfg(windows)]
impl NameResolver {
    pub fn on_disk_name(&mut self, p: &Path) -> Option<std::ffi::OsString> {
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
}

#[cfg(not(windows))]
impl NameResolver {
    // Compiled on Linux as well as macOS, though only macOS reaches it in a
    // real run: it is what lets the Linux CI job exercise the lookup that the
    // macOS bundle depends on, on a runner that actually exists.
    pub fn on_disk_name(&mut self, p: &Path) -> Option<std::ffi::OsString> {
        let name = p.file_name()?;
        let parent = p.parent()?;
        if !self.listings.contains_key(parent) {
            // An unreadable parent caches as empty on purpose: the answer is
            // "no idea", and repeating a failing syscall per child would not
            // improve on it.
            let names = DirNames::read(parent).unwrap_or_default();
            self.listings.insert(parent.to_path_buf(), names);
        }
        self.listings.get(parent)?.get(name)
    }
}

/// One directory's entries, indexed both ways.
///
/// `exact` is consulted first and answers on every filesystem: a spelling
/// that is on disk is its own answer. `folded` is the case-drift lookup and
/// is consulted **only where the filesystem folds case**. On a case-sensitive
/// volume — which macOS will format APFS as on request, and where
/// `recase_dirs_phase` still runs — `docs` and `Docs` are two different
/// directories, so answering "the real spelling of `Docs` is `docs`" would
/// have the caller rename a directory that prune should have deleted.
///
/// A fold that two entries share is deliberately unanswerable: renaming one
/// onto the other would destroy a file. On a folding volume that pair cannot
/// normally exist, but the fold here is Rust's `to_lowercase` and the
/// filesystem's is its own table — `K` (U+212A) and `k` lowercase alike while
/// the volume may keep them apart — so the guard stands.
///
/// Plain lowercasing is what the Windows side has always done. It does not
/// normalise Unicode, so a name stored decomposed (HFS+ did this to every
/// filename) will not fold onto its composed spelling. That reads as "no
/// match", the caller leaves the entry alone, and the old spelling survives —
/// exactly what happens today.
#[cfg(not(windows))]
#[derive(Default)]
struct DirNames {
    exact: std::collections::HashSet<std::ffi::OsString>,
    /// Folded name -> the sole entry that folds to it, or None when several
    /// do and no rename can be safely inferred.
    folded: std::collections::HashMap<String, Option<std::ffi::OsString>>,
}

#[cfg(not(windows))]
impl DirNames {
    fn read(dir: &Path) -> Option<Self> {
        let mut out = Self::default();
        for entry in std::fs::read_dir(dir).ok()? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            // readdir never yields the same name twice, so an occupied slot
            // is always a genuine second spelling.
            out.folded
                .entry(name.to_string_lossy().to_lowercase())
                .and_modify(|slot| *slot = None)
                .or_insert_with(|| Some(name.clone()));
            out.exact.insert(name);
        }
        Some(out)
    }

    fn get(&self, want: &std::ffi::OsStr) -> Option<std::ffi::OsString> {
        if self.exact.contains(want) {
            return Some(want.to_os_string());
        }
        if !CASE_INSENSITIVE_FS {
            return None;
        }
        self.fold(want)
    }

    /// The folded lookup on its own, without the platform gate `get` applies.
    /// Only the tests call it directly — to reach the ambiguity guard, whose
    /// fixture needs two entries differing in case and so can only be built
    /// where `get` would have refused to fold in the first place.
    fn fold(&self, want: &std::ffi::OsStr) -> Option<std::ffi::OsString> {
        self.folded
            .get(&want.to_string_lossy().to_lowercase())
            .cloned()
            .flatten()
    }
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
// Hard links (daily versions)
// ─────────────────────────────────────────────────────────────────────

/// Rename `tmp` over `dest` without touching the attributes of the file
/// `dest` names.
///
/// With daily versions `dest` is usually a hard link shared with earlier
/// snapshots, and on Windows the ReadOnly bit belongs to the file, not to the
/// link: `finish_copy`'s `clear_readonly(dest)` strips it from every day that
/// shares the file. `std::fs::rename` will not replace a `+R` file at all.
/// `FileRenameInfoEx` with `FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE`
/// replaces it, atomically, and leaves the other links as they were (NTFS,
/// Windows 10 1809 and later — measured with rustc 1.98). Where the call is
/// not supported, fall back to clear-then-rename, and say so.
#[cfg(windows)]
pub fn replace_link_safe(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileRenameInfoEx, SetFileInformationByHandle, DELETE, FILE_RENAME_INFO,
    };
    // winbase.h. windows-sys 0.59 exports these only under
    // Win32_System_WindowsProgramming, which nothing else here needs.
    const FILE_RENAME_FLAG_REPLACE_IF_EXISTS: u32 = 0x1;
    const FILE_RENAME_FLAG_POSIX_SEMANTICS: u32 = 0x2;
    const FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE: u32 = 0x40;

    let attempt = || -> std::io::Result<()> {
        let file =
            std::fs::OpenOptions::new()
                .access_mode(DELETE)
                .open(long_path(tmp))?;
        let name: Vec<u16> = long_path(dest).as_os_str().encode_wide().collect();
        let size = std::mem::size_of::<FILE_RENAME_INFO>() + name.len() * 2;
        // u64 storage keeps the buffer aligned for the struct's HANDLE field.
        let mut buf = vec![0u64; size.div_ceil(8)];
        let info = buf.as_mut_ptr() as *mut FILE_RENAME_INFO;
        let ok = unsafe {
            (*info).Anonymous.Flags = FILE_RENAME_FLAG_REPLACE_IF_EXISTS
                | FILE_RENAME_FLAG_POSIX_SEMANTICS
                | FILE_RENAME_FLAG_IGNORE_READONLY_ATTRIBUTE;
            (*info).RootDirectory = std::ptr::null_mut();
            (*info).FileNameLength = (name.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                (*info).FileName.as_mut_ptr(),
                name.len(),
            );
            SetFileInformationByHandle(
                file.as_raw_handle() as _,
                FileRenameInfoEx,
                info as *const _,
                size as u32,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    };
    match attempt() {
        Ok(()) => Ok(()),
        // ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_INVALID_PARAMETER:
        // an older Windows, or a filesystem without the Ex information class.
        Err(e) if matches!(e.raw_os_error(), Some(1) | Some(50) | Some(87)) => {
            warn!(
                "link-safe rename unsupported ({}); clearing ReadOnly on {} first",
                e,
                dest.display()
            );
            clear_readonly(dest);
            std::fs::rename(long_path(tmp), long_path(dest))
        }
        Err(e) => Err(e),
    }
}

#[cfg(not(windows))]
pub fn replace_link_safe(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::rename(tmp, dest)
}

/// Delete `path` without touching the attributes of the file it names.
///
/// std's `remove_file` already deletes a `+R` file on NTFS without clearing
/// the bit — it asks for `FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE` —
/// so the other links to it keep theirs (measured with rustc 1.98). What
/// strips the bit is prune's `clear_readonly` *before* the delete. Only a
/// filesystem without that disposition class answers PermissionDenied, and
/// there clear-then-delete is the only way.
pub fn remove_link_safe(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(long_path(path)) {
        Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied => {
            clear_readonly(path);
            std::fs::remove_file(long_path(path))
        }
        other => other,
    }
}

/// Whether `dir`'s filesystem can hard-link: make a file, link it, remove
/// both. exFAT and FAT32 cannot, and neither can some network shares.
///
/// A folder that cannot take the probe file at all is an error, not a "no":
/// it says nothing about links, and the run could not write there either.
pub fn hard_link_supported(dir: &Path) -> Result<bool> {
    let a = dir.join(".driveby-link-probe");
    let b = dir.join(".driveby-link-probe-2");
    // Leftovers from a run killed mid-probe.
    let _ = std::fs::remove_file(long_path(&b));
    let _ = std::fs::remove_file(long_path(&a));
    std::fs::write(long_path(&a), b"probe")
        .map_err(|e| anyhow!("could not write to {}: {}", dir.display(), e))?;
    let linked = std::fs::hard_link(long_path(&a), long_path(&b));
    let _ = std::fs::remove_file(long_path(&b));
    let _ = std::fs::remove_file(long_path(&a));
    if let Err(e) = &linked {
        warn!(dir = %dir.display(), "no hard links here: {}", e);
    }
    Ok(linked.is_ok())
}

/// Whether `a` and `b` are one file reached by two links.
#[cfg(test)]
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
            return false;
        };
        x.dev() == y.dev() && x.ino() == y.ino()
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let id = |p: &Path| -> Option<(u32, u32, u32)> {
            let f = std::fs::File::open(long_path(p)).ok()?;
            let mut info: BY_HANDLE_FILE_INFORMATION =
                unsafe { std::mem::zeroed() };
            let ok = unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) };
            (ok != 0).then_some((
                info.dwVolumeSerialNumber,
                info.nFileIndexHigh,
                info.nFileIndexLow,
            ))
        };
        matches!((id(a), id(b)), (Some(x), Some(y)) if x == y)
    }
}

// ─────────────────────────────────────────────────────────────────────
// Free space
// ─────────────────────────────────────────────────────────────────────

/// Bytes this process may still write on the volume holding `dir`, or
/// `None` when the question has no answer — `dir` missing, or a filesystem
/// that will not say.
///
/// "Available to the caller", not "free": a disk quota, or the blocks ext4
/// reserves for root, are room this process cannot use.
#[cfg(windows)]
pub fn available_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    // A plain path rather than `long_path`: the verbatim prefix is not
    // documented for this call, and a destination root is nowhere near
    // MAX_PATH. The trailing separator is required for a share root.
    let mut wide: Vec<u16> = dir.as_os_str().encode_wide().collect();
    if !matches!(wide.last(), Some(&c) if c == u16::from(b'\\') || c == u16::from(b'/')) {
        wide.push(u16::from(b'\\'));
    }
    wide.push(0);
    let mut available: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

#[cfg(unix)]
// The field types differ by platform — `u64` on Linux, `u32` on macOS — so
// the casts are needed on one and redundant on the other.
#[allow(clippy::unnecessary_cast)]
pub fn available_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    Some((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

// ─────────────────────────────────────────────────────────────────────
// Getting the synchronous calls off the async workers
// ─────────────────────────────────────────────────────────────────────

/// Run a synchronous filesystem call off the async workers.
///
/// Every syscall in this module is synchronous, and against a network
/// destination — supported since 1.7.2, "one that lives at a friend's house" —
/// any of them can take seconds. On an async worker that is worse than slow:
/// the workers also serve Tauri's IPC and the scheduler, and `copy_phase` runs
/// `parallelCopies` copies at once, so a handful of them against a stalled
/// share can occupy every worker the runtime has. The Stop button then stops
/// being answered, which is precisely when the user is reaching for it.
///
/// Callers group the calls that already sit together into one hop rather than
/// wrapping each syscall: one task handoff per file beats six, and on a
/// 100k-file tree the difference is not small.
///
/// Two synchronous callers are deliberately left as they are, so that nobody
/// reads them as an oversight: `path_contains` canonicalises a handful of
/// times when a run starts rather than once per file, and `main`'s startup
/// `create_dir_all` calls run before the runtime has anything else to do.
pub async fn blocking<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(v) => v,
        // A blocking task cannot be cancelled once it has started and this
        // always awaits it, so the only JoinError reachable here is a panic
        // inside `f`. Re-raise rather than invent a value: a panicking
        // attribute call is a bug, and swallowing it would hide it behind a
        // run that reports success.
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

/// The metadata a freshly written scratch file needs before it can be renamed
/// into place: the source's mtime, the source's kept attribute bits, and the
/// read-only bit off whatever it is about to replace.
///
/// Both pipelines made these four calls inline and identically. `mtime: None`
/// is how the backup side spells `preserveMtime: false`; the restore side
/// always passes one, because without the round-trip every following sync
/// re-copies every file.
///
/// The `clear_readonly` is not optional housekeeping: MoveFileEx will not
/// replace a `+R` file, so the rename that follows fails outright without it.
/// It is deliberately *not* applied to the scratch file, which may legitimately
/// be carrying the attribute forward from the source.
pub async fn finish_copy(
    src: PathBuf,
    tmp: PathBuf,
    dest: PathBuf,
    mtime: Option<std::time::SystemTime>,
) {
    blocking(move || {
        stamp_scratch(&src, &tmp, mtime);
        clear_readonly(&dest);
    })
    .await
}

/// `finish_copy` for a scratch file that will replace a file shared with
/// earlier snapshots: the same stamping, and the read-only bit left on the
/// outgoing file, because it is theirs too. `replace_link_safe` does not need
/// it gone.
pub async fn finish_scratch(
    src: PathBuf,
    tmp: PathBuf,
    mtime: Option<std::time::SystemTime>,
) {
    blocking(move || stamp_scratch(&src, &tmp, mtime)).await
}

/// The source's mtime (when kept) and its kept attribute bits, onto the
/// scratch file.
fn stamp_scratch(src: &Path, tmp: &Path, mtime: Option<std::time::SystemTime>) {
    if let Some(t) = mtime {
        let _ = filetime::set_file_mtime(
            long_path(tmp),
            filetime::FileTime::from_system_time(t),
        );
    }
    if let Some(attrs) = read_attrs(src) {
        apply_attrs(tmp, attrs);
    }
}

/// Mirror one directory's attributes onto its destination counterpart, and
/// report whether they stuck: the bits wanted against the bits observed, or
/// None when they matched, when the source had none worth mirroring, or when
/// the source could not be read.
///
/// The mismatch is worth reporting because it is what surfaces filesystem
/// limits rather than letting them pass silently — exFAT drops `+R` on
/// directories, and a custom folder icon then renders as a default one.
pub async fn mirror_dir_attrs(src_dir: PathBuf, dest_dir: PathBuf) -> Option<(u32, u32)> {
    blocking(move || {
        let src_attrs = read_attrs(&src_dir)?;
        let want = src_attrs & ATTR_KEEP;
        apply_attrs(&dest_dir, src_attrs);
        if want == 0 {
            return None;
        }
        let got = read_attrs(&dest_dir)? & ATTR_KEEP;
        (got != want).then_some((want, got))
    })
    .await
}

impl NameResolver {
    /// `on_disk_name`, off the async workers.
    ///
    /// The resolver travels into the blocking task and back out again because
    /// its memoized listings have to survive the hop — losing them would put
    /// `recase_dirs_phase` back to listing each parent once per child, which
    /// is quadratic in the width of the tree.
    pub async fn on_disk_name_async(&mut self, p: &Path) -> Option<std::ffi::OsString> {
        let mut owned = std::mem::take(self);
        let path = p.to_path_buf();
        let (owned, answer) = blocking(move || {
            let answer = owned.on_disk_name(&path);
            (owned, answer)
        })
        .await;
        *self = owned;
        answer
    }
}

// ─────────────────────────────────────────────────────────────────────
// Source / destination overlap rejection
// ─────────────────────────────────────────────────────────────────────

/// True if `child` equals or is nested under `parent` (case-insensitively
/// where the filesystem is). Both inputs must be absolute. Falls back to a
/// lossy string compare if the paths can't be canonicalised yet (e.g. the
/// destination doesn't exist) — callers validate existence first. Critically:
/// on Windows, `canonicalize()` prepends `\\?\` to existing paths but not to
/// non-existing ones, so we strip that prefix on both sides before comparing
/// — otherwise an existing parent would never appear to "contain" a
/// not-yet-created child even when it lexically does.
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
    let s = p.to_string_lossy();
    // Folded on APFS for the same reason the Windows arm folds on NTFS, and
    // the stakes here are the highest in the crate: `/Volumes/D/Photos` and
    // `/Volumes/D/photos` are one directory, so an overlap that goes unspotted
    // is a restore whose `File::create` truncates the very file it is about to
    // read — every file in the backup emptied, and the run reporting success.
    if CASE_INSENSITIVE_FS {
        s.to_lowercase()
    } else {
        s.to_string()
    }
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

    /// Any folder on a mounted volume has an answer, and a folder that does
    /// not exist has none — which the room check reads as "cannot tell"
    /// rather than as a volume with nothing free.
    #[test]
    fn free_space_is_read_off_the_folders_volume() {
        let root = make_test_dir("free-space");
        assert!(available_space(&root).is_some_and(|free| free > 0));
        assert!(available_space(&root.join("does-not-exist/at-all")).is_none());
    }

    #[test]
    fn on_disk_name_reports_the_real_spelling() {
        let root = make_test_dir("on-disk-name");
        let dir = root.join("MixedCase");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut names = NameResolver::default();
        // The spelling that is on disk always resolves to itself, everywhere.
        assert_eq!(
            names.on_disk_name(&dir).unwrap(),
            std::ffi::OsStr::new("MixedCase")
        );
        assert!(names.on_disk_name(&root.join("absent")).is_none());
        // Asked with the wrong case, a case-folding filesystem still finds it
        // and reports what the entry is really called — which is the whole
        // point of the lookup. A case-sensitive one correctly finds nothing.
        let wrong_case = names.on_disk_name(&root.join("mixedcase"));
        if CASE_INSENSITIVE_FS {
            assert_eq!(wrong_case.unwrap(), std::ffi::OsStr::new("MixedCase"));
        } else {
            assert!(wrong_case.is_none());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The ambiguity guard, reached past the platform gate.
    ///
    /// Building the fixture takes two entries differing only in case, which
    /// needs a case-sensitive volume — so this runs on Linux, while the code
    /// path it protects lives on a folding one, where the pair arises only
    /// through Unicode that `to_lowercase` collapses and the filesystem does
    /// not. `fold` is therefore called directly: `get` would refuse to fold
    /// on the very host that can set the fixture up.
    #[cfg(not(windows))]
    #[test]
    fn an_ambiguous_fold_refuses_to_answer() {
        if CASE_INSENSITIVE_FS {
            return; // the fixture cannot be built here
        }
        let root = make_test_dir("on-disk-ambiguous");
        std::fs::create_dir_all(root.join("Foo")).unwrap();
        std::fs::create_dir_all(root.join("foo")).unwrap();
        std::fs::create_dir_all(root.join("Solo")).unwrap();
        let names = DirNames::read(&root).unwrap();
        assert_eq!(
            names.fold(std::ffi::OsStr::new("SOLO")).unwrap(),
            std::ffi::OsStr::new("Solo"),
            "a fold only one entry answers to resolves to that entry"
        );
        assert!(
            names.fold(std::ffi::OsStr::new("FOO")).is_none(),
            "a fold two entries share must not pick one"
        );
        // And the gate itself: on this host `get` must never fold at all.
        assert!(
            names.get(std::ffi::OsStr::new("solo")).is_none(),
            "a case-sensitive volume has no case drift to find"
        );
        assert_eq!(
            names.get(std::ffi::OsStr::new("Solo")).unwrap(),
            std::ffi::OsStr::new("Solo"),
            "but an exact spelling still answers for itself"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The memoized listing must not go stale in the direction that matters:
    /// a second question about the same parent has to be answered from the
    /// cache, and a question about a different parent must still be read.
    #[test]
    fn on_disk_name_caches_per_parent() {
        let root = make_test_dir("on-disk-cache");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("one")).unwrap();
        std::fs::create_dir_all(root.join("two/deep")).unwrap();
        let mut names = NameResolver::default();
        assert!(names.on_disk_name(&root.join("one")).is_some());
        assert!(names.on_disk_name(&root.join("two")).is_some());
        assert!(names.on_disk_name(&root.join("two/deep")).is_some());
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

    /// The same folder spelled two ways is still the same folder on NTFS and
    /// APFS, and this is the check standing between a restore and
    /// `File::create` truncating the file it is about to read. Before
    /// `CASE_INSENSITIVE_FS` the fold was `cfg(windows)`, so a Mac accepted
    /// `~/Photos` against `~/photos` as two unrelated folders.
    #[test]
    fn reject_overlap_folds_case_where_the_filesystem_does() {
        let d = make_test_dir("overlap-case");
        let shouty = d.with_file_name(d.file_name().unwrap().to_string_lossy().to_uppercase());
        assert_eq!(
            reject_overlap(&d, &shouty).is_err(),
            CASE_INSENSITIVE_FS,
            "one directory under two spellings must be rejected exactly where \
             the filesystem considers them one"
        );
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

    /// A panic inside a blocking call must reach the caller. Swallowing it
    /// would leave a run reporting success over metadata that was never
    /// applied — the failure mode the whole module exists to prevent.
    #[tokio::test]
    #[should_panic(expected = "attribute call blew up")]
    async fn a_panic_on_the_blocking_pool_reaches_the_caller() {
        blocking(|| panic!("attribute call blew up")).await
    }

    /// The mtime round-trip both pipelines depend on: without it every sync
    /// re-copies every file, because the destination's timestamp never
    /// matches the source's.
    #[tokio::test]
    async fn finish_copy_carries_the_source_mtime_to_the_scratch_file() {
        let root = make_test_dir("finish-copy-mtime");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("src.txt");
        let tmp = root.join("dest.txt.driveby-tmp");
        let dest = root.join("dest.txt");
        std::fs::write(&src, b"source").unwrap();
        std::fs::write(&tmp, b"source").unwrap();

        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        finish_copy(src, tmp.clone(), dest, Some(when)).await;

        let got = std::fs::metadata(&tmp).unwrap().modified().unwrap();
        assert_eq!(got, when, "the scratch file must carry the source's mtime");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The other three calls, which only Windows can observe: the source's
    /// kept attribute bits land on the scratch file, and the read-only bit
    /// comes off the file the rename is about to replace — MoveFileEx will
    /// not replace a `+R` file, so without that last part the copy fails.
    #[cfg(windows)]
    #[tokio::test]
    async fn finish_copy_mirrors_attrs_and_frees_the_outgoing_file() {
        let root = make_test_dir("finish-copy-attrs");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("src.txt");
        let tmp = root.join("dest.txt.driveby-tmp");
        let dest = root.join("dest.txt");
        std::fs::write(&src, b"source").unwrap();
        std::fs::write(&tmp, b"source").unwrap();
        std::fs::write(&dest, b"stale").unwrap();
        apply_attrs(&src, 0x2); // HIDDEN
        apply_attrs(&dest, 0x1); // the +R that would block the rename

        finish_copy(src, tmp.clone(), dest.clone(), None).await;

        assert_eq!(
            read_attrs(&tmp).unwrap() & ATTR_KEEP,
            0x2,
            "the source's kept bits must be mirrored onto the scratch file"
        );
        assert_eq!(
            read_attrs(&dest).unwrap() & 0x1,
            0,
            "the outgoing file must no longer be read-only"
        );
        assert!(
            std::fs::rename(&tmp, &dest).is_ok(),
            "the rename this exists to make possible must work"
        );
        clear_readonly(&dest);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A directory whose attributes take needs no report; one whose bits do
    /// not stick reports what was wanted against what the filesystem kept —
    /// exFAT silently drops `+R` on directories, and a custom folder icon
    /// then renders as a default one.
    #[cfg(windows)]
    #[tokio::test]
    async fn mirror_dir_attrs_is_quiet_when_the_bits_take() {
        let root = make_test_dir("mirror-dir-attrs");
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src");
        let dst = root.join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        apply_attrs(&src, 0x1); // a custom-icon folder carries +R

        assert_eq!(mirror_dir_attrs(src.clone(), dst.clone()).await, None);
        assert_eq!(read_attrs(&dst).unwrap() & ATTR_KEEP, 0x1);

        clear_readonly(&src);
        clear_readonly(&dst);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    fn set_readonly(p: &Path, on: bool) {
        let mut perms = std::fs::metadata(p).unwrap().permissions();
        perms.set_readonly(on);
        std::fs::set_permissions(p, perms).unwrap();
    }

    /// Replacing today's link must leave yesterday's file exactly as it was:
    /// that is the whole of what a snapshot promises.
    #[test]
    fn replace_link_safe_replaces_only_this_link() {
        let dir = tempfile::tempdir().unwrap();
        let yesterday = dir.path().join("yesterday.txt");
        let today = dir.path().join("today.txt");
        let tmp = dir.path().join("today.txt.driveby-tmp");
        std::fs::write(&yesterday, b"old").unwrap();
        std::fs::hard_link(&yesterday, &today).unwrap();
        std::fs::write(&tmp, b"new").unwrap();

        replace_link_safe(&tmp, &today).unwrap();

        assert_eq!(std::fs::read(&today).unwrap(), b"new");
        assert_eq!(std::fs::read(&yesterday).unwrap(), b"old");
        assert!(!tmp.exists(), "the scratch file is what became today's copy");
        assert!(!same_file(&today, &yesterday));
    }

    /// On Windows ReadOnly belongs to the file, not the link: replacing a
    /// `+R` link the way `finish_copy` does — clear, then rename — strips the
    /// bit from yesterday's copy as well.
    #[cfg(windows)]
    #[test]
    fn replace_link_safe_leaves_the_other_links_readonly_bit() {
        let dir = tempfile::tempdir().unwrap();
        let yesterday = dir.path().join("yesterday.txt");
        let today = dir.path().join("today.txt");
        let tmp = dir.path().join("today.txt.driveby-tmp");
        std::fs::write(&yesterday, b"old").unwrap();
        set_readonly(&yesterday, true);
        std::fs::hard_link(&yesterday, &today).unwrap();
        std::fs::write(&tmp, b"new").unwrap();

        replace_link_safe(&tmp, &today).unwrap();

        assert_eq!(std::fs::read(&today).unwrap(), b"new");
        assert_eq!(std::fs::read(&yesterday).unwrap(), b"old");
        assert_ne!(read_attrs(&yesterday).unwrap() & ATTR_READONLY, 0, "yesterday lost its +R");
        set_readonly(&yesterday, false);
    }

    #[test]
    fn remove_link_safe_removes_one_link() {
        let dir = tempfile::tempdir().unwrap();
        let yesterday = dir.path().join("yesterday.txt");
        let today = dir.path().join("today.txt");
        std::fs::write(&yesterday, b"old").unwrap();
        std::fs::hard_link(&yesterday, &today).unwrap();

        remove_link_safe(&today).unwrap();

        assert!(!today.exists());
        assert_eq!(std::fs::read(&yesterday).unwrap(), b"old");
    }

    #[cfg(windows)]
    #[test]
    fn remove_link_safe_leaves_the_other_links_readonly_bit() {
        let dir = tempfile::tempdir().unwrap();
        let yesterday = dir.path().join("yesterday.txt");
        let today = dir.path().join("today.txt");
        std::fs::write(&yesterday, b"old").unwrap();
        set_readonly(&yesterday, true);
        std::fs::hard_link(&yesterday, &today).unwrap();

        remove_link_safe(&today).unwrap();

        assert!(!today.exists());
        assert_ne!(read_attrs(&yesterday).unwrap() & ATTR_READONLY, 0, "yesterday lost its +R");
        set_readonly(&yesterday, false);
    }

    #[test]
    fn a_local_folder_can_hard_link_and_keeps_no_probe() {
        let dir = tempfile::tempdir().unwrap();
        assert!(hard_link_supported(dir.path()).unwrap());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "the probe files must go");
    }

    /// A folder that cannot be written to says nothing about hard links: the
    /// destination fails with the reason instead of being called a drive
    /// without them.
    #[test]
    fn a_folder_that_cannot_be_written_is_an_error_not_a_drive_without_links() {
        let dir = tempfile::tempdir().unwrap();
        let err = hard_link_supported(&dir.path().join("missing")).unwrap_err();
        assert!(err.to_string().contains("missing"), "{err}");
    }

    #[test]
    fn same_file_tells_a_link_from_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("c"));
        std::fs::write(&a, b"x").unwrap();
        std::fs::hard_link(&a, &b).unwrap();
        std::fs::copy(&a, &c).unwrap();
        assert!(same_file(&a, &b));
        assert!(!same_file(&a, &c));
    }

    /// `finish_scratch` stamps the scratch file as `finish_copy` does, and
    /// leaves the ReadOnly bit of the file it will replace alone — that bit is
    /// shared with earlier days.
    #[cfg(windows)]
    #[tokio::test]
    async fn finish_scratch_leaves_the_outgoing_files_readonly_bit() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let tmp = dir.path().join("dest.txt.driveby-tmp");
        let dest = dir.path().join("dest.txt");
        std::fs::write(&src, b"source").unwrap();
        std::fs::write(&tmp, b"source").unwrap();
        std::fs::write(&dest, b"old").unwrap();
        set_readonly(&dest, true);
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);

        finish_scratch(src, tmp.clone(), Some(when)).await;

        assert_eq!(std::fs::metadata(&tmp).unwrap().modified().unwrap(), when);
        assert_ne!(read_attrs(&dest).unwrap() & ATTR_READONLY, 0);
        set_readonly(&dest, false);
    }
}
