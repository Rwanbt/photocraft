use super::*;
use std::cell::Cell;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("pc-atomic-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Only the target remains: no temporary file was left behind.
fn assert_no_temp(dir: &Path) {
    let leftovers: Vec<_> =
        std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp")).collect();
    assert!(leftovers.is_empty(), "temporary files left behind: {leftovers:?}");
}

const ORIGINAL: &[u8] = b"the user's precious original document";

/// Real file system with failures injected at chosen steps.
#[derive(Default)]
struct Faulty {
    /// Write this many bytes, then fail like a full disk.
    fail_write_after: Option<usize>,
    /// Fail every rename with this kind.
    fail_rename: Option<io::ErrorKind>,
    /// Fail the first N renames with PermissionDenied (a transient lock), then succeed.
    locked_renames: Cell<u32>,
    renames: Cell<u32>,
    /// Simulate name collisions when creating adjacent temporary files.
    colliding_writes: Cell<u32>,
    fail_sync_dir: bool,
    fail_copy_permissions: bool,
}

impl AtomicFs for Faulty {
    fn write_new(&self, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
        if self.colliding_writes.get() > 0 {
            self.colliding_writes.set(self.colliding_writes.get() - 1);
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "injected temp collision"));
        }
        if let Some(n) = self.fail_write_after {
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)?;
            f.write_all(&bytes[..n.min(bytes.len())])?;
            return Err(io::Error::new(io::ErrorKind::StorageFull, "injected: no space left on device"));
        }
        RealFs.write_new(tmp, bytes)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.renames.set(self.renames.get() + 1);
        if let Some(kind) = self.fail_rename {
            return Err(io::Error::new(kind, "injected rename failure"));
        }
        if self.locked_renames.get() > 0 {
            self.locked_renames.set(self.locked_renames.get() - 1);
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected: sharing violation"));
        }
        RealFs.rename(from, to)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        RealFs.remove(path)
    }
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        if self.fail_sync_dir {
            return Err(io::Error::other("injected directory flush failure"));
        }
        RealFs.sync_dir(dir)
    }
    fn copy_permissions(&self, from: &Path, to: &Path) -> io::Result<()> {
        if self.fail_copy_permissions {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected permissions copy failure"));
        }
        RealFs.copy_permissions(from, to)
    }
}

const NO_RETRY: RenameRetry = RenameRetry { attempts: 0, first_delay_ms: 0 };

#[test]
fn replaces_and_creates() {
    let d = TempDir::new("ok");
    let p = d.0.join("doc.psd");
    atomic_write(&p, b"first").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"first");
    atomic_write(&p, b"second, longer").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"second, longer");
    assert_no_temp(&d.0);
}

#[test]
fn creates_missing_parent_folders() {
    let d = TempDir::new("parent");
    let p = d.0.join("a/b/out.png");
    atomic_write(&p, b"x").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"x");
}

#[test]
fn write_failure_halfway_keeps_original() {
    let d = TempDir::new("write");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_write_after: Some(5), ..Default::default() };
    let e = atomic_write_with(&fs, NO_RETRY, &p, &[7u8; 4096]).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::StorageFull);
    assert!(e.to_string().contains("not changed"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_eq!(fs.renames.get(), 0, "never renames after a failed write");
    assert_no_temp(&d.0);
}

#[test]
fn rename_failure_keeps_original() {
    let d = TempDir::new("rename");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_rename: Some(io::ErrorKind::Other), ..Default::default() };
    assert!(atomic_write_with(&fs, NO_RETRY, &p, b"new").is_err());
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

#[test]
fn cross_device_rename_errors_clearly_and_never_writes_in_place() {
    let d = TempDir::new("exdev");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_rename: Some(io::ErrorKind::CrossesDevices), ..Default::default() };
    let e = atomic_write_with(&fs, NO_RETRY, &p, b"new").unwrap_err();
    assert!(e.to_string().contains("different volume"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

#[test]
fn locked_target_rename_is_retried() {
    let d = TempDir::new("locked");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { locked_renames: Cell::new(2), ..Default::default() };
    atomic_write_with(&fs, RenameRetry { attempts: 3, first_delay_ms: 1 }, &p, b"new").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    assert_eq!(fs.renames.get(), 3);
    assert_no_temp(&d.0);
}

#[test]
fn lock_that_outlasts_the_retries_keeps_original() {
    let d = TempDir::new("locked2");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { locked_renames: Cell::new(10), ..Default::default() };
    let e = atomic_write_with(&fs, RenameRetry { attempts: 2, first_delay_ms: 1 }, &p, b"new").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    assert!(e.to_string().contains("locked"), "{e}");
    assert_eq!(fs.renames.get(), 3);
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

#[cfg(unix)]
#[test]
fn read_only_folder_errors_clearly_and_never_writes_in_place() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("ro");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    // The file itself stays writable: an in-place fallback would succeed and must not happen.
    std::fs::set_permissions(&d.0, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(d.0.join("probe")).is_ok() {
        return; // running as root: permissions aren't enforced
    }
    let e = atomic_write(&p, b"new").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    assert!(e.to_string().contains("not writable"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    std::fs::set_permissions(&d.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_no_temp(&d.0);
}

#[cfg(unix)]
#[test]
fn keeps_the_original_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("mode");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    atomic_write(&p, b"new").unwrap();
    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
}

#[cfg(unix)]
#[test]
fn writes_through_a_symlink() {
    let d = TempDir::new("link");
    let real = d.0.join("real.psd");
    let link = d.0.join("link.psd");
    std::fs::write(&real, ORIGINAL).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    atomic_write(&link, b"new").unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read(&real).unwrap(), b"new");
}

#[test]
fn directory_target_or_bare_root_is_an_error_not_a_panic() {
    let d = TempDir::new("dir");
    assert!(atomic_write(&d.0, b"x").is_err());
    assert!(atomic_write(Path::new("/"), b"x").is_err());
    assert!(atomic_write(Path::new(""), b"x").is_err());
}

/// A share that refuses the full flush (`F_FULLFSYNC`, #1336) still saves through a plain fsync on
/// Apple platforms; every other sync error still fails the save.
#[test]
fn a_refused_full_flush_falls_back_to_a_plain_fsync_on_apple_only() {
    let refused = |errno| Err(io::Error::from_raw_os_error(errno));
    for (errno, name) in [(45, "ENOTSUP, SMB"), (25, "ENOTTY")] {
        let fsynced = Cell::new(false);
        let r = or_plain_fsync(refused(errno), || {
            fsynced.set(true);
            Ok(())
        });
        let apple = cfg!(target_vendor = "apple");
        assert_eq!(r.is_ok(), apple, "{name}");
        assert_eq!(fsynced.get(), apple, "{name}: the plain fsync runs only on Apple platforms");
    }
    // The plain fsync's own failure is reported.
    if cfg!(target_vendor = "apple") {
        let r = or_plain_fsync(refused(45), || Err(io::Error::from_raw_os_error(5)));
        assert_eq!(r.map_err(|e| e.raw_os_error()), Err(Some(5)));
    }
    // Real errors (EIO, ENOSPC, EACCES) never fall back, and success needs no fallback.
    for errno in [5, 28, 13] {
        let r = or_plain_fsync(refused(errno), || panic!("no fallback for errno {errno}"));
        assert_eq!(r.map_err(|e| e.raw_os_error()), Err(Some(errno)));
    }
    assert!(or_plain_fsync(Ok(()), || panic!("no fallback after a full flush")).is_ok());
}

#[test]
fn sync_file_flushes_a_local_file() {
    let dir = TempDir::new("sync");
    let path = dir.0.join("f.bin");
    // Opened for writing, as the saves do: Windows can't flush a read-only handle.
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
    sync_file(&file).unwrap();
}

#[test]
fn a_post_rename_directory_flush_error_does_not_report_a_false_save_failure() {
    let dir = TempDir::new("dir-sync");
    let file = dir.0.join("document.pcraft");
    std::fs::write(&file, ORIGINAL).unwrap();
    let fs = Faulty { fail_sync_dir: true, ..Default::default() };

    atomic_write_with(&fs, NO_RETRY, &file, b"committed bytes").unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"committed bytes");
    assert_eq!(fs.renames.get(), 1);
    assert_no_temp(&dir.0);
}

#[test]
fn a_failed_permission_copy_does_not_prevent_a_valid_save() {
    let dir = TempDir::new("permissions");
    let file = dir.0.join("document.pcraft");
    std::fs::write(&file, ORIGINAL).unwrap();
    let fs = Faulty { fail_copy_permissions: true, ..Default::default() };

    atomic_write_with(&fs, NO_RETRY, &file, b"new data").unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"new data");
    assert_eq!(fs.renames.get(), 1);
    assert_no_temp(&dir.0);
}

#[test]
fn eight_temp_name_collisions_fail_safely_without_touching_original() {
    let dir = TempDir::new("collisions");
    let file = dir.0.join("document.pcraft");
    std::fs::write(&file, ORIGINAL).unwrap();
    let fs = Faulty { colliding_writes: Cell::new(8), ..Default::default() };

    let failure = atomic_write_with(&fs, NO_RETRY, &file, b"new data").unwrap_err();
    assert_eq!(failure.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs.colliding_writes.get(), 0);
    assert_eq!(fs.renames.get(), 0);
    assert_eq!(std::fs::read(&file).unwrap(), ORIGINAL);
    assert_no_temp(&dir.0);
}

#[test]
fn a_partial_initial_save_leaves_no_document_or_temp_file() {
    let dir = TempDir::new("first-write-full-disk");
    let file = dir.0.join("new-document.pcraft");
    let fs = Faulty { fail_write_after: Some(3), ..Default::default() };

    let failure = atomic_write_with(&fs, NO_RETRY, &file, b"a longer new document").unwrap_err();
    assert_eq!(failure.kind(), io::ErrorKind::StorageFull);
    assert!(!file.exists());
    assert_eq!(fs.renames.get(), 0);
    assert_no_temp(&dir.0);
}
