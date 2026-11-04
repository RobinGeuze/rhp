use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;

use super::*;

/// A fresh directory below the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let p = base.join(format!("rhp-vcwd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }

    fn bytes(&self) -> Vec<u8> {
        self.0.as_os_str().as_bytes().to_vec()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn relative_paths_follow_the_request_directory() {
    reset_state();
    assert_eq!(resolve(b"a/b"), &b"a/b"[..]);
    let s = Scratch::new("resolve");
    assert!(set_cwd(&s.bytes()));
    let mut want = s.bytes();
    want.extend_from_slice(b"/a/../b");
    assert_eq!(resolve(b"a/../b"), &want[..]);
    assert_eq!(resolve(b"/abs"), &b"/abs"[..]);
    assert_eq!(resolve(b""), &b""[..]);
    assert_eq!(getcwd().unwrap(), s.bytes());
    reset_state();
    assert_eq!(resolve(b"a"), &b"a"[..]);
}

#[test]
fn chdir_checks_like_chdir2() {
    reset_state();
    let s = Scratch::new("chdir");
    std::fs::create_dir(s.0.join("sub")).unwrap();
    std::fs::write(s.0.join("file"), b"x").unwrap();
    std::os::unix::fs::symlink(s.0.join("sub"), s.0.join("link")).unwrap();
    assert!(set_cwd(&s.bytes()));
    assert_eq!(chdir(b""), Err(libc::ENOENT));
    assert_eq!(chdir(b"missing"), Err(libc::ENOENT));
    assert_eq!(chdir(b"file"), Err(libc::ENOTDIR));
    assert_eq!(chdir(b"file/.."), Err(libc::ENOTDIR));
    // A symlink resolves to the directory it names, as getcwd(3) reports.
    assert_eq!(chdir(b"link"), Ok(()));
    let mut sub = s.bytes();
    sub.extend_from_slice(b"/sub");
    assert_eq!(getcwd().unwrap(), sub);
    assert_eq!(chdir(b".."), Ok(()));
    assert_eq!(getcwd().unwrap(), s.bytes());
    reset_state();
}

#[test]
fn normalize_folds_dots() {
    assert_eq!(normalize(b"/a/./b/../c//d/"), b"/a/c/d");
    assert_eq!(normalize(b"/../.."), b"/");
    assert_eq!(normalize(b""), b"/");
}

#[test]
fn expand_filepath_resolves_the_existing_part() {
    reset_state();
    let s = Scratch::new("expand");
    std::fs::create_dir(s.0.join("real")).unwrap();
    std::os::unix::fs::symlink(s.0.join("real"), s.0.join("link")).unwrap();
    assert!(set_cwd(&s.bytes()));
    let mut want = s.bytes();
    want.extend_from_slice(b"/real/new/file");
    assert_eq!(expand_filepath(b"link/new/./file"), want);
    let mut kept = s.bytes();
    kept.extend_from_slice(b"/link/x");
    assert_eq!(expand_path(b"link/y/../x"), kept);
    assert_eq!(expand_filepath(b"phar://a/../b"), b"phar://a/../b");
    reset_state();
}

#[test]
fn files_are_created_under_the_request_umask() {
    reset_state();
    let s = Scratch::new("umask");
    assert!(set_cwd(&s.bytes()));
    let perms = |name: &str| {
        std::fs::metadata(s.0.join(name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    let process = umask();
    assert_eq!(set_umask(0), process);
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC;
    drop(open(b"open0", flags, 0o666).unwrap());
    assert_eq!(perms("open0"), 0o666);
    mkdir(b"dir0", 0o777).unwrap();
    assert_eq!(perms("dir0"), 0o777);
    // An existing file keeps its mode.
    std::fs::set_permissions(s.0.join("open0"), std::fs::Permissions::from_mode(0o600)).unwrap();
    drop(open(b"open0", flags, 0o666).unwrap());
    assert_eq!(perms("open0"), 0o600);
    assert_eq!(set_umask(0o077), 0);
    write_file(b"open77", b"x").unwrap();
    assert_eq!(perms("open77"), 0o600);
    mkdir(b"dir77", 0o777).unwrap();
    assert_eq!(perms("dir77"), 0o700);
    // What a C library creates under the process's umask is fixed up.
    set_umask(0o002);
    let lib = s.0.join("lib");
    library_created(b"lib", 0o666, || {
        let c = CString::new(lib.as_os_str().as_bytes()).unwrap();
        unsafe { libc::close(libc::open(c.as_ptr(), flags, 0o666)) };
    });
    assert_eq!(perms("lib"), 0o664);
    reset_state();
    assert_eq!(umask(), process);
}

#[test]
fn hot_calls_start_from_the_directory() {
    reset_state();
    let s = Scratch::new("at");
    std::fs::create_dir(s.0.join("sub")).unwrap();
    std::fs::write(s.0.join("sub/f"), b"data").unwrap();
    assert!(set_cwd(&s.bytes()));
    let at = |p: &[u8]| with_at(p, |dir, c| (dir, c.to_bytes().to_vec()));
    let (dir, c) = at(b"sub/f").unwrap();
    assert_ne!(dir, libc::AT_FDCWD);
    assert_eq!(c, b"sub/f");
    assert_eq!(at(b"/abs").unwrap().0, libc::AT_FDCWD);
    assert!(at(b"a\0b").is_none());
    assert_eq!(stat(b"sub/f", true).unwrap().st_size, 4);
    assert_eq!(read_file(b"sub/f").unwrap(), b"data");
    assert!(access(b"sub/f", libc::R_OK));
    assert_eq!(
        stat(b"missing", true).unwrap_err().raw_os_error(),
        Some(libc::ENOENT)
    );
    // A directory that can be searched but not read: paths are joined.
    std::fs::set_permissions(s.0.join("sub"), std::fs::Permissions::from_mode(0o311)).unwrap();
    assert_eq!(chdir(b"sub"), Ok(()));
    let (dir, c) = at(b"f").unwrap();
    // (root reads any directory)
    if unsafe { libc::geteuid() } != 0 {
        assert_eq!(dir, libc::AT_FDCWD);
        assert!(c.ends_with(b"/sub/f"));
    }
    assert_eq!(read_file(b"f").unwrap(), b"data");
    std::fs::set_permissions(s.0.join("sub"), std::fs::Permissions::from_mode(0o755)).unwrap();
    reset_state();
    assert_eq!(at(b"rel").unwrap().0, libc::AT_FDCWD);
}
