//! Read a file another program left behind: only a regular file, never
//! through a symbolic link, never blocking, and never more than a cap.
//!
//! The checks are made on the opened handle, not on the path beforehand, so
//! nothing can be swapped in between the check and the open. The open itself
//! refuses a symbolic link as the last path component (`O_NOFOLLOW`) and
//! cannot block on a FIFO or a device (`O_NONBLOCK`); anything that is then
//! not a regular file is refused before a byte is read.
//!
//! Only Unix hosts are supported.

use std::io::Read;

/// Why a file was not read. No path is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegularFileError {
    /// Nothing is at the path.
    Missing,
    /// A symbolic link, FIFO, folder, device or socket is at the path.
    NotRegular,
    /// The file is larger than the cap.
    TooLarge,
    /// The file could not be opened or read.
    Unreadable,
}

/// The whole content of the regular file at `path`, at most `cap` bytes.
#[cfg(unix)]
pub fn read_capped(path: &std::path::Path, cap: u64) -> Result<Vec<u8>, RegularFileError> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(libc::ENOENT) => RegularFileError::Missing,
            // How an `O_NOFOLLOW` open reports a symbolic link: `ELOOP` on
            // Linux and macOS, `EMLINK` on FreeBSD.
            Some(libc::ELOOP | libc::EMLINK) => RegularFileError::NotRegular,
            _ => RegularFileError::Unreadable,
        })?;
    let metadata = file.metadata().map_err(|_| RegularFileError::Unreadable)?;
    if !metadata.is_file() {
        return Err(RegularFileError::NotRegular);
    }
    if metadata.len() > cap {
        return Err(RegularFileError::TooLarge);
    }
    // The file may still grow while it is read; the cap holds regardless.
    let mut bytes = Vec::new();
    file.take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| RegularFileError::Unreadable)?;
    if bytes.len() as u64 > cap {
        return Err(RegularFileError::TooLarge);
    }
    Ok(bytes)
}

/// Not supported off Unix: every read is refused.
#[cfg(not(unix))]
pub fn read_capped(_: &std::path::Path, _: u64) -> Result<Vec<u8>, RegularFileError> {
    Err(RegularFileError::Unreadable)
}

/// This process's effective user ID, for checking who owns a folder it is
/// about to trust.
#[cfg(unix)]
pub fn effective_user_id() -> u32 {
    // SAFETY: `geteuid` takes no argument, cannot fail and touches no memory.
    unsafe { libc::geteuid() }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{path::PathBuf, time::Duration};

    fn dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new()
            .tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target"))
            .unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        (dir, root)
    }

    /// The read, on another thread, so a read that blocks fails the test
    /// instead of hanging it.
    fn read_soon(path: PathBuf, cap: u64) -> Result<Vec<u8>, RegularFileError> {
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(read_capped(&path, cap));
        });
        receive
            .recv_timeout(Duration::from_secs(5))
            .expect("the read returned without blocking")
    }

    #[test]
    fn reads_a_regular_file_within_the_cap() {
        let (_dir, root) = dir();
        let path = root.join("f.json");
        std::fs::write(&path, b"12345").unwrap();
        assert_eq!(read_soon(path.clone(), 5), Ok(b"12345".to_vec()));
        assert_eq!(read_soon(path, 4), Err(RegularFileError::TooLarge));
        assert_eq!(
            read_soon(root.join("missing"), 5),
            Err(RegularFileError::Missing)
        );
    }

    #[test]
    fn refuses_a_link_a_fifo_and_a_folder_without_blocking() {
        let (_dir, root) = dir();
        std::fs::write(root.join("target.json"), b"{}").unwrap();
        std::os::unix::fs::symlink(root.join("target.json"), root.join("link")).unwrap();
        assert_eq!(
            read_soon(root.join("link"), 64),
            Err(RegularFileError::NotRegular)
        );
        let fifo = root.join("fifo");
        let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path and a plain mode.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert_eq!(read_soon(fifo, 64), Err(RegularFileError::NotRegular));
        std::fs::create_dir(root.join("folder")).unwrap();
        assert_eq!(
            read_soon(root.join("folder"), 64),
            Err(RegularFileError::NotRegular)
        );
    }
}
