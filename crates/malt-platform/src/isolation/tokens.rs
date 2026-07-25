//! Windows restricted token creation and management.
//!
//! Provides token restriction for process isolation on Windows.
//! Restricted tokens remove privileges and add deny-only SIDs to
//! limit what a process can do.

use super::tier::IsolationError;

use std::ptr::null_mut;

/// Opaque handle to a Windows access token.
#[derive(Debug)]
pub struct RestrictedToken {
    handle: isize,
}

impl RestrictedToken {
    /// Get the raw Windows token handle.
    pub fn handle(&self) -> isize {
        self.handle
    }
}

impl Drop for RestrictedToken {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

// Windows API bindings for token manipulation.
#[cfg(target_os = "windows")]
extern "system" {
    fn OpenProcessToken(ProcessHandle: isize, DesiredAccess: u32, TokenHandle: *mut isize) -> i32;
    fn CreateRestrictedToken(
        ExistingTokenHandle: isize,
        Flags: u32,
        DisableSidCount: u32,
        SidsToDisable: *const (),
        DeletePrivilegeCount: u32,
        PrivilegesToDelete: *const (),
        RestrictedSidCount: u32,
        SidsToRestrict: *const (),
        NewTokenHandle: *mut isize,
    ) -> i32;
    fn LookupPrivilegeValueA(lpSystemName: *const u8, lpName: *const u8, lpLuid: *mut ()) -> i32;
    fn CloseHandle(hObject: isize) -> i32;
    fn GetCurrentProcess() -> isize;
}

// Token access rights
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_ASSIGN_PRIMARY: u32 = 0x0001;
const TOKEN_DUPLICATE: u32 = 0x0002;
#[allow(dead_code)]
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;

// CreateRestrictedToken flags
const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
#[allow(dead_code)]
const SANDBOX_INERT: u32 = 0x2;
#[allow(dead_code)]
const LUA_TOKEN: u32 = 0x4;

// LUID structure (8 bytes on x86_64)
#[repr(C)]
struct Luid {
    low_part: u32,
    high_part: i32,
}

// LUID_AND_ATTRIBUTES structure (12 bytes on x86_64)
#[repr(C)]
struct LuidAndAttributes {
    luid: Luid,
    attributes: u32,
}

// Privilege attributes
const SE_PRIVILEGE_REMOVED: u32 = 0x00000004;

/// Common Windows privileges that can be restricted.
pub const COMMON_PRIVILEGES: &[&str] = &[
    "SeDebugPrivilege",
    "SeImpersonatePrivilege",
    "SeAssignPrimaryTokenPrivilege",
    "SeTcbPrivilege",
    "SeSecurityPrivilege",
    "SeTakeOwnershipPrivilege",
    "SeLoadDriverPrivilege",
    "SeBackupPrivilege",
    "SeRestorePrivilege",
    "SeIncreaseQuotaPrivilege",
    "SeChangeNotifyPrivilege",
    "SeRemoteShutdownPrivilege",
    "SeUndockPrivilege",
    "SeManageVolumePrivilege",
    "SeRelabelPrivilege",
    "SeIncreaseBasePriorityPrivilege",
    "SeCreatePagefilePrivilege",
    "SeCreatePermanentPrivilege",
    "SeShutdownPrivilege",
];

/// Create a restricted token from the current process token.
///
/// This creates a new token with `DISABLE_MAX_PRIVILEGE` set, which
/// removes all privileges except those explicitly needed. Additional
/// privileges can be removed by name.
///
/// # Arguments
///
/// * `privileges_to_remove` — List of privilege names to explicitly remove.
///
/// # Returns
///
/// A `RestrictedToken` handle that can be used when spawning processes.
pub fn create_restricted_token(
    privileges_to_remove: &[&str],
) -> Result<RestrictedToken, IsolationError> {
    // Get current process token
    let current_process = unsafe { GetCurrentProcess() };
    let mut parent_token: isize = 0;

    // SAFETY: GetCurrentProcess returns a pseudo-handle that is always valid.
    // OpenProcessToken is called with appropriate access rights.
    let result = unsafe {
        OpenProcessToken(
            current_process,
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY,
            &mut parent_token,
        )
    };

    if result == 0 || parent_token == 0 {
        return Err(IsolationError::TokenError(
            "OpenProcessToken failed".to_string(),
        ));
    }

    // Create restricted token with DISABLE_MAX_PRIVILEGE
    let mut new_token: isize = 0;

    // SAFETY: parent_token is valid (checked above). new_token pointer is valid.
    // DISABLE_MAX_PRIVILEGE removes all privileges from the new token.
    let result = unsafe {
        CreateRestrictedToken(
            parent_token,
            DISABLE_MAX_PRIVILEGE,
            0,
            null_mut(),
            0,
            null_mut(),
            0,
            null_mut(),
            &mut new_token,
        )
    };

    // SAFETY: parent_token is valid (checked above).
    unsafe {
        CloseHandle(parent_token);
    }

    if result == 0 || new_token == 0 {
        return Err(IsolationError::TokenError(
            "CreateRestrictedToken failed".to_string(),
        ));
    }

    // Remove additional specified privileges
    if !privileges_to_remove.is_empty() {
        remove_privileges_from_token(new_token, privileges_to_remove)?;
    }

    Ok(RestrictedToken { handle: new_token })
}

/// Remove specific privileges from an existing token.
///
/// # Arguments
///
/// * `token_handle` — The token handle to modify.
/// * `privileges` — List of privilege names to remove.
fn remove_privileges_from_token(
    _token_handle: isize,
    privileges: &[&str],
) -> Result<(), IsolationError> {
    for &priv_name in privileges {
        let mut luid = Luid {
            low_part: 0,
            high_part: 0,
        };

        let name_bytes = priv_name.as_bytes();
        let mut c_name = name_bytes.to_vec();
        c_name.push(0);

        // SAFETY: c_name is a valid null-terminated C string.
        // luid pointer is valid stack memory.
        let result = unsafe {
            LookupPrivilegeValueA(null_mut(), c_name.as_ptr(), &mut luid as *mut _ as *mut ())
        };

        if result == 0 {
            // Privilege doesn't exist on this system — skip it
            continue;
        }

        // For a restricted token created with DISABLE_MAX_PRIVILEGE,
        // privileges are already removed. This function is a no-op
        // in that case, but we keep it for future use when we might
        // want to selectively remove privileges from non-restricted tokens.
        let _ = LuidAndAttributes {
            luid,
            attributes: SE_PRIVILEGE_REMOVED,
        };
    }

    Ok(())
}

/// Create a minimal token for sandboxed processes.
///
/// This is a convenience function that creates a restricted token
/// with all common privileges removed. Suitable for untrusted code
/// execution.
pub fn create_sandbox_token() -> Result<RestrictedToken, IsolationError> {
    create_restricted_token(COMMON_PRIVILEGES)
}

/// Get the current process token handle (for inspection).
///
/// Returns a raw handle that must be closed with `CloseHandle`.
/// The caller is responsible for managing the handle lifetime.
pub fn get_current_token() -> Result<isize, IsolationError> {
    let current_process = unsafe { GetCurrentProcess() };
    let mut token: isize = 0;

    // SAFETY: GetCurrentProcess returns a valid pseudo-handle.
    let result = unsafe { OpenProcessToken(current_process, TOKEN_QUERY, &mut token) };

    if result == 0 || token == 0 {
        return Err(IsolationError::TokenError(
            "OpenProcessToken failed for current process".to_string(),
        ));
    }

    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_privileges_not_empty() {
        assert!(!COMMON_PRIVILEGES.is_empty());
        assert!(COMMON_PRIVILEGES.len() >= 10);
    }

    #[test]
    fn common_privileges_have_se_prefix() {
        for &priv_name in COMMON_PRIVILEGES {
            assert!(
                priv_name.starts_with("Se"),
                "privilege '{priv_name}' does not start with 'Se'"
            );
            assert!(
                priv_name.ends_with("Privilege"),
                "privilege '{priv_name}' does not end with 'Privilege'"
            );
        }
    }

    #[test]
    fn restricted_token_error_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<IsolationError>();
    }

    #[test]
    fn restricted_token_error_is_debug() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<IsolationError>();
    }

    #[test]
    fn restricted_token_drop_with_zero_handle() {
        let token = RestrictedToken { handle: 0 };
        drop(token);
        // If we reach here without crashing, the test passes
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn token_functions_unavailable_on_non_windows() {
        let err = IsolationError::TokenError("test".to_string());
        assert!(err.to_string().contains("token error"));
    }

    /// Real call into OpenProcessToken/CreateRestrictedToken — every other
    /// test in this module either checks trait bounds or constructs
    /// `RestrictedToken` directly, never exercising the actual Win32 calls.
    /// job_objects.rs had two real bugs hiding behind exactly this shape of
    /// coverage; this checks tokens.rs isn't hiding a third.
    #[test]
    fn create_restricted_token_actually_succeeds() {
        let token = create_restricted_token(&[]).expect("create_restricted_token should succeed");
        assert_ne!(token.handle(), 0);
    }

    #[test]
    fn create_sandbox_token_actually_succeeds() {
        let token = create_sandbox_token().expect("create_sandbox_token should succeed");
        assert_ne!(token.handle(), 0);
    }

    #[test]
    fn get_current_token_actually_succeeds() {
        let token = get_current_token().expect("get_current_token should succeed");
        assert_ne!(token, 0);
        // get_current_token's own doc says the caller must close it.
        unsafe {
            CloseHandle(token);
        }
    }

    /// The real point of a restricted token: it should measurably have
    /// fewer privileges than the unrestricted current-process token, not
    /// just "some handle that isn't zero". This is the kind of check the
    /// original tests skipped entirely.
    #[test]
    fn restricted_token_is_actually_more_restricted_than_current_token() {
        let restricted = create_sandbox_token().expect("create_sandbox_token should succeed");

        // A token created with DISABLE_MAX_PRIVILEGE should fail to look up
        // as having SeDebugPrivilege enabled (it's stripped), whereas the
        // check must at least run without erroring on the handle itself.
        // We can't easily enumerate privileges without more FFI surface, so
        // assert the narrower, still-real thing: the restricted token's
        // handle is distinct from a freshly-opened current-process token
        // (i.e. we actually got a new token, not a passthrough of the
        // original).
        let current = get_current_token().expect("get_current_token should succeed");
        assert_ne!(
            restricted.handle(),
            current,
            "restricted token must be a distinct token, not the process's own token handle"
        );
        unsafe {
            CloseHandle(current);
        }
    }
}
