use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: i32,
    pub birth: String,
    pub executable: String,
}
pub fn inspect(pid: i32) -> Result<Option<ProcessIdentity>> {
    ensure!(pid > 1, "refusing non-process PID");
    platform(pid)
}
#[cfg(target_os = "macos")]
fn platform(pid: i32) -> Result<Option<ProcessIdentity>> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    // The kernel writes the fixed-size BSD process record into an adequately
    // sized, aligned buffer; it is read only after the full size is returned.
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size as i32,
        )
    };
    if read == 0 {
        let error = std::io::Error::last_os_error();
        if [Some(libc::ESRCH), Some(libc::ENOENT)].contains(&error.raw_os_error()) {
            return Ok(None);
        }
        return Err(error.into());
    }
    ensure!(read as usize == size, "incomplete process identity");
    let info = unsafe { info.assume_init() };
    if info.pbi_status == 5 {
        return Ok(None);
    } // Darwin SZOMB.
    ensure!(
        info.pbi_uid == unsafe { libc::geteuid() },
        "process is owned by another user"
    );
    let mut bytes = [0u8; 4096];
    let count = unsafe { libc::proc_pidpath(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    ensure!(count > 0, "cannot determine process executable");
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let executable = std::str::from_utf8(&bytes[..end])?.to_owned();
    Ok(Some(ProcessIdentity {
        pid,
        birth: format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec),
        executable,
    }))
}
#[cfg(target_os = "linux")]
fn platform(pid: i32) -> Result<Option<ProcessIdentity>> {
    let path = format!("/proc/{pid}");
    let stat = match std::fs::read_to_string(format!("{path}/stat")) {
        Ok(stat) => stat,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let (_, tail) = stat
        .rsplit_once(')')
        .ok_or_else(|| anyhow::anyhow!("invalid process record"))?;
    let fields: Vec<_> = tail.split_whitespace().collect();
    ensure!(fields.len() >= 20, "incomplete process record");
    if ["Z", "X"].contains(&fields[0]) {
        return Ok(None);
    }
    use std::os::unix::fs::MetadataExt;
    ensure!(
        std::fs::metadata(&path)?.uid() == unsafe { libc::geteuid() },
        "process is owned by another user"
    );
    let executable = std::fs::read_link(format!("{path}/exe"))?
        .to_string_lossy()
        .into_owned();
    Ok(Some(ProcessIdentity {
        pid,
        birth: fields[19].into(),
        executable,
    }))
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform(_: i32) -> Result<Option<ProcessIdentity>> {
    anyhow::bail!("process identity fencing is implemented for Linux and macOS")
}
pub fn matches(expected: &ProcessIdentity) -> Result<bool> {
    Ok(inspect(expected.pid)?.as_ref() == Some(expected))
}
#[cfg(not(target_os = "linux"))]
pub fn signal(expected: &ProcessIdentity, signal: i32) -> Result<bool> {
    if !matches(expected)? {
        return Ok(false);
    }
    // Recheck kernel birth identity immediately before signalling; a stale
    // pidfile never authorizes a signal to a differently identified process.
    if unsafe { libc::kill(expected.pid, signal) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error.into());
        }
    }
    Ok(true)
}

#[cfg(target_os = "linux")]
pub fn signal(expected: &ProcessIdentity, signal: i32) -> Result<bool> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    ensure!(expected.pid > 1, "refusing non-process PID");
    // Bind the kernel process handle before validating its recorded birth.
    // Reuse after this point cannot redirect a signal to a new process.
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, expected.pid, 0) };
    if raw < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        return Err(anyhow::anyhow!(
            "pidfd_open required for safe shutdown: {error}"
        ));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
    if !matches(expected)? {
        return Ok(false);
    }
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        return Err(error.into());
    }
    Ok(true)
}
#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    #[test]
    fn pidfd_signals_only_matching_birth() {
        let mut child = std::process::Command::new("sleep")
            .arg("2")
            .spawn()
            .unwrap();
        let identity = inspect(child.id() as i32).unwrap().unwrap();
        let mut stale = identity.clone();
        stale.birth.push_str("-stale");
        assert!(!signal(&stale, libc::SIGTERM).unwrap());
        assert!(child.try_wait().unwrap().is_none());
        assert!(signal(&identity, libc::SIGTERM).unwrap());
        assert!(!child.wait().unwrap().success());
        assert!(!signal(&identity, libc::SIGTERM).unwrap());
    }
}
