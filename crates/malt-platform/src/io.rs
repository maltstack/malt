//! I/O utilities — pipe creation.

/// Create an anonymous OS pipe.
/// Returns `(read_end, write_end)` as owned file handles.
pub fn create_pipe() -> Result<(std::fs::File, std::fs::File), std::io::Error> {
    #[cfg(unix)]
    {
        let (read_fd, write_fd) = nix::unistd::pipe()
            .map_err(std::io::Error::from)?;
        Ok((std::fs::File::from(read_fd), std::fs::File::from(write_fd)))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Pipes::CreatePipe;
        use std::os::windows::io::FromRawHandle;

        let mut read_h = INVALID_HANDLE_VALUE;
        let mut write_h = INVALID_HANDLE_VALUE;
        let ok = unsafe { CreatePipe(&mut read_h, &mut write_h, std::ptr::null(), 0) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: CreatePipe returns two new valid handles on success.
        // We immediately wrap them in File for RAII ownership.
        Ok(unsafe {
            (
                std::fs::File::from_raw_handle(read_h as *mut _),
                std::fs::File::from_raw_handle(write_h as *mut _),
            )
        })
    }
}
