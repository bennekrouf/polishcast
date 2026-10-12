//! A recording someone forgot to stop is stopped before it fills the disk: ScreenCaptureKit
//! writes 100–500 MB a minute on a Retina display, before the take is compacted.

use std::path::Path;
use std::time::Duration;

/// Longest take.
pub const MAX_LENGTH: Duration = Duration::from_secs(2 * 60 * 60);
/// Largest recording file.
pub const MAX_BYTES: u64 = 20_000_000_000;
/// Free space left on the disk below which recording stops (compacting needs some too).
pub const MIN_FREE_BYTES: u64 = 2_000_000_000;

/// Why a recording that's running should stop now, if it should: `elapsed` since it started,
/// the recording file's size and the disk's free space (each `None` if unknown).
pub fn reason_to_stop(elapsed: Duration, size: Option<u64>, free: Option<u64>) -> Option<String> {
    if elapsed >= MAX_LENGTH {
        return Some(format!(
            "The recording was stopped after {} hours, the longest a take can be.",
            MAX_LENGTH.as_secs() / 3600
        ));
    }
    if size.is_some_and(|s| s >= MAX_BYTES) {
        return Some(format!(
            "The recording was stopped at {} GB, the largest a take can be.",
            MAX_BYTES / 1_000_000_000
        ));
    }
    if free.is_some_and(|f| f < MIN_FREE_BYTES) {
        return Some("The recording was stopped because the disk is almost full.".into());
    }
    None
}

/// Free space on the disk holding `path`, in bytes.
pub fn free_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: `c` is a valid NUL-terminated path and `stat` is written by the call.
        if unsafe { libc::statvfs(c.as_ptr(), stat.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: statvfs returned 0, so it filled `stat`.
        let stat = unsafe { stat.assume_init() };
        #[allow(clippy::unnecessary_cast)] // the field types differ between platforms
        Some(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_at_the_first_limit_reached() {
        let minute = Duration::from_secs(60);
        assert_eq!(reason_to_stop(minute, Some(1_000), Some(MIN_FREE_BYTES * 10)), None);
        assert_eq!(reason_to_stop(minute, None, None), None, "unknown size and space don't stop it");
        assert!(reason_to_stop(MAX_LENGTH, None, None).unwrap().contains("2 hours"));
        assert!(reason_to_stop(minute, Some(MAX_BYTES), None).unwrap().contains("20 GB"));
        assert!(reason_to_stop(minute, None, Some(MIN_FREE_BYTES - 1)).unwrap().contains("almost full"));
    }

    #[test]
    fn reads_the_free_space_here() {
        if cfg!(unix) {
            assert!(free_space(&std::env::temp_dir()).is_some_and(|f| f > 0));
        }
    }
}
