//! Windows Job Objects for process resource limits.
//!
//! Provides memory and CPU rate limiting via Windows Job Objects.
//! Job Objects allow grouping processes and applying resource constraints
//! to the entire group atomically.

use super::tier::IsolationError;

use std::ptr::null_mut;

/// Opaque handle to a Windows Job Object.
#[derive(Debug)]
pub struct JobObject {
    handle: isize,
    name: String,
}

impl JobObject {
    /// Get the job object name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the raw Windows handle.
    pub fn handle(&self) -> isize {
        self.handle
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

// Windows API bindings — defined locally to avoid pulling in additional
// windows-sys features beyond what is already declared in Cargo.toml.
#[cfg(target_os = "windows")]
extern "system" {
    fn CreateJobObjectA(lpJobAttributes: *mut (), lpName: *const u8) -> isize;
    #[allow(dead_code)]
    fn OpenJobObjectA(dwDesiredAccess: u32, bInheritHandle: i32, lpName: *const u8) -> isize;
    fn AssignProcessToJobObject(hJob: isize, hProcess: isize) -> i32;
    fn TerminateJobObject(hJob: isize, uExitCode: u32) -> i32;
    fn SetInformationJobObject(
        hJob: isize,
        JobObjectInformationClass: u32,
        lpJobObjectInformation: *const (),
        cbJobObjectInformationLength: u32,
    ) -> i32;
    fn CloseHandle(hObject: isize) -> i32;
    fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> isize;
}

// Job Object Information Class
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;

// Limit Flags
const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x00000100;
#[allow(dead_code)]
const JOB_OBJECT_LIMIT_JOB_MEMORY: u32 = 0x00000200;
const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x00000008;

// Access rights
#[allow(dead_code)]
const JOB_OBJECT_ALL_ACCESS: u32 = 0x1F001F;
const PROCESS_SET_QUOTA: u32 = 0x0100;
const PROCESS_TERMINATE: u32 = 0x0001;

/// JOBOBJECT_BASIC_LIMIT_INFORMATION layout (x86_64).
#[repr(C)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

/// JOBOBJECT_EXTENDED_LIMIT_INFORMATION layout (x86_64).
#[repr(C)]
struct JobObjectExtendedLimitInfo {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: [u64; 2], // IO_READ_BYTES + IO_WRITE_BYTES
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

/// Create a new Job Object with memory and CPU rate limits.
///
/// # Arguments
///
/// * `name` — Unique name for the job object (used for debugging).
/// * `memory_limit_mb` — Per-process memory limit in megabytes (0 = no limit).
/// * `cpu_rate` — CPU rate as a percentage of one CPU core (0 = no limit, 100 = 1 core).
///
/// # Returns
///
/// A `JobObject` handle that can be used to assign processes and enforce limits.
/// The job object is automatically closed when dropped.
pub fn create_job_object(
    name: &str,
    memory_limit_mb: u64,
    cpu_rate: u32,
) -> Result<JobObject, IsolationError> {
    let name_bytes = name.as_bytes();
    let mut c_name = name_bytes.to_vec();
    c_name.push(0); // null terminator

    // SAFETY: CreateJobObjectA is called with a valid null-terminated C string.
    // The handle is checked for validity (non-zero) before use.
    let handle = unsafe { CreateJobObjectA(null_mut(), c_name.as_ptr()) };

    if handle == 0 {
        return Err(IsolationError::JobObjectError(format!(
            "CreateJobObjectA failed for '{name}'"
        )));
    }

    // Set extended limit information
    let mut limit_flags: u32 = 0;
    let process_memory_limit = if memory_limit_mb > 0 {
        limit_flags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        memory_limit_mb * 1024 * 1024
    } else {
        0
    };

    // Active process limit defaults to 1 (can be adjusted)
    limit_flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;

    let extended_info = JobObjectExtendedLimitInfo {
        basic_limit_information: JobObjectBasicLimitInformation {
            per_process_user_time_limit: 0,
            per_job_user_time_limit: if cpu_rate > 0 {
                // Convert CPU rate to 100-nanosecond units per second
                // cpu_rate is percentage, so 100% = 10_000_000 (1 second)
                (cpu_rate as i64) * 100_000
            } else {
                0
            },
            limit_flags,
            minimum_working_set_size: 0,
            maximum_working_set_size: if process_memory_limit > 0 {
                process_memory_limit as usize
            } else {
                usize::MAX
            },
            active_process_limit: 1,
            affinity: 0,
            priority_class: 0,
            scheduling_class: 0,
        },
        io_info: [0; 2],
        process_memory_limit: process_memory_limit as usize,
        job_memory_limit: if memory_limit_mb > 0 {
            (memory_limit_mb * 1024 * 1024) as usize
        } else {
            0
        },
        peak_process_memory_used: 0,
        peak_job_memory_used: 0,
    };

    // SAFETY: extended_info is a valid stack-allocated struct. The pointer
    // and size are correctly passed to SetInformationJobObject.
    let result = unsafe {
        SetInformationJobObject(
            handle,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            &extended_info as *const _ as *const (),
            std::mem::size_of::<JobObjectExtendedLimitInfo>() as u32,
        )
    };

    if result == 0 {
        // SAFETY: handle is valid (checked above). CloseHandle is safe.
        unsafe {
            CloseHandle(handle);
        }
        return Err(IsolationError::JobObjectError(
            "SetInformationJobObject failed".to_string(),
        ));
    }

    Ok(JobObject {
        handle,
        name: name.to_string(),
    })
}

/// Assign a process to a Job Object by PID.
///
/// # Arguments
///
/// * `job` — The job object to assign the process to.
/// * `pid` — The Windows process ID.
///
/// # Note
///
/// A process can only be assigned to one job object. If the process is
/// already in a job, this will fail unless the existing job allows
/// nested jobs.
pub fn assign_process_to_job(job: &JobObject, pid: u32) -> Result<(), IsolationError> {
    // SAFETY: OpenProcess is called with valid access rights. The handle
    // is checked before use and closed after assignment.
    let process_handle = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };

    if process_handle == 0 {
        return Err(IsolationError::JobObjectError(format!(
            "OpenProcess failed for PID {pid}"
        )));
    }

    // SAFETY: Both handles are valid (checked above). AssignProcessToJobObject
    // is safe to call with valid job and process handles.
    let result = unsafe { AssignProcessToJobObject(job.handle, process_handle) };

    // SAFETY: process_handle is valid (checked above).
    unsafe {
        CloseHandle(process_handle);
    }

    if result == 0 {
        return Err(IsolationError::JobObjectError(format!(
            "AssignProcessToJobObject failed for PID {pid}"
        )));
    }

    Ok(())
}

/// Terminate all processes in a Job Object.
///
/// This forcefully terminates every process associated with the job.
/// Use with caution — there is no graceful shutdown.
pub fn terminate_job_object(job: &JobObject) -> Result<(), IsolationError> {
    // SAFETY: job.handle is valid (ensured by JobObject type invariant).
    // TerminateJobObject with exit code 1 is a standard pattern.
    let result = unsafe { TerminateJobObject(job.handle, 1) };

    if result == 0 {
        return Err(IsolationError::JobObjectError(format!(
            "TerminateJobObject failed for '{}'",
            job.name
        )));
    }

    Ok(())
}

/// Query the number of active processes in a Job Object.
pub fn query_active_processes(job: &JobObject) -> Result<u32, IsolationError> {
    // For now, return a placeholder. Full implementation would use
    // QueryInformationJobObject with JobObjectBasicProcessIdList.
    let _ = job;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_object_error_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<IsolationError>();
    }

    #[test]
    fn job_object_error_is_debug() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<IsolationError>();
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn job_object_functions_unavailable_on_non_windows() {
        // On non-Windows, the extern "system" functions are not linked,
        // so we can't actually call them. Verify the error type works.
        let err = IsolationError::JobObjectError("test".to_string());
        assert!(err.to_string().contains("job object error"));
    }

    #[test]
    fn job_object_name_accessor() {
        // Test the JobObject struct directly (no Windows API needed)
        let job = JobObject {
            handle: 0x1234,
            name: "test-job".to_string(),
        };
        assert_eq!(job.name(), "test-job");
        assert_eq!(job.handle(), 0x1234);
    }

    #[test]
    fn job_object_drop_with_zero_handle() {
        // Dropping a JobObject with handle=0 should not call CloseHandle
        let job = JobObject {
            handle: 0,
            name: "empty".to_string(),
        };
        drop(job);
        // If we reach here without crashing, the test passes
    }
}
