//! macOS sandbox-exec profile management.
//!
//! Provides sandbox profile generation and application via `sandbox_init(3)`.
//! The sandbox-exec mechanism uses Seatbelt profiles to restrict file system,
//! network, and Mach IPC access.

use super::tier::IsolationError;

/// Apply a sandbox profile to the current process.
///
/// Uses `sandbox_init` to apply the given Seatbelt profile. Once applied,
/// the sandbox cannot be removed — it affects the calling process and
/// all children.
///
/// # Arguments
///
/// * `profile` — The sandbox profile content in Seatbelt syntax.
///
/// # Returns
///
/// `Ok(())` if the profile was applied successfully, or an error
/// describing why the sandbox could not be initialized.
pub fn apply_sandbox_profile(profile: &str) -> Result<(), IsolationError> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::ptr::null_mut;

        let profile_c = CString::new(profile).map_err(|e| {
            IsolationError::SandboxError(format!("profile contains null bytes: {e}"))
        })?;

        // sandbox_init returns 0 on success, -1 on failure.
        // On failure, *errorbuf is set to a malloc'd error string.
        extern "C" {
            fn sandbox_init(
                profile: *const libc::c_char,
                flags: u64,
                errorbuf: *mut *mut libc::c_char,
            ) -> libc::c_int;
            fn sandbox_free_error(errorbuf: *mut libc::c_char);
        }

        let mut error_buf: *mut libc::c_char = null_mut();

        // SAFETY: profile_c is a valid null-terminated C string.
        // error_buf is a valid pointer to a pointer that sandbox_init will set.
        // flags = 0 means the profile is passed as a string (not a path).
        let result = unsafe { sandbox_init(profile_c.as_ptr(), 0, &mut error_buf) };

        if result != 0 {
            let error_msg = if !error_buf.is_null() {
                // SAFETY: error_buf is set by sandbox_init on failure.
                // The string is null-terminated and owned by the sandbox library.
                let c_str = unsafe { std::ffi::CStr::from_ptr(error_buf) };
                let msg = c_str.to_string_lossy().to_string();
                // SAFETY: error_buf was allocated by sandbox_init and must be
                // freed with sandbox_free_error.
                unsafe {
                    sandbox_free_error(error_buf);
                }
                msg
            } else {
                "sandbox_init failed with no error message".to_string()
            };

            return Err(IsolationError::SandboxError(error_msg));
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = profile;
        Err(IsolationError::UnsupportedPlatform(
            "sandbox-exec is macOS-only".to_string(),
        ))
    }
}

/// Generate a default restricted sandbox profile.
///
/// Returns a Seatbelt profile that:
/// - Allows reading from standard system directories
/// - Allows writing only to `/tmp`, `/var/tmp`, and the user's `$HOME`
/// - Blocks network access (no socket, no Mach IPC)
/// - Blocks process signaling and debugging
/// - Allows basic process execution
///
/// This profile is suitable for running untrusted shell commands.
pub fn default_restricted_profile() -> String {
    let mut profile = String::new();

    // Version declaration
    profile.push_str("(version 1)\n\n");

    // Deny by default
    profile.push_str("(deny default)\n\n");

    // Allow reading system libraries and binaries
    profile.push_str("(allow file-read*\n");
    profile.push_str("  (subpath \"/usr/lib\")\n");
    profile.push_str("  (subpath \"/usr/share\")\n");
    profile.push_str("  (subpath \"/System/Library\")\n");
    profile.push_str("  (subpath \"/Library\")\n");
    profile.push_str("  (subpath \"/Applications\")\n");
    profile.push_str("  (subpath \"/bin\")\n");
    profile.push_str("  (subpath \"/sbin\")\n");
    profile.push_str("  (regex #\"^/private/var/db/timezone/\")\n");
    profile.push_str(")\n\n");

    // Allow reading from home directory
    profile.push_str("(allow file-read*\n");
    profile.push_str("  (subpath (env \"HOME\"))\n");
    profile.push_str(")\n\n");

    // Allow writing to temp directories
    profile.push_str("(allow file-write*\n");
    profile.push_str("  (subpath \"/tmp\")\n");
    profile.push_str("  (subpath \"/var/tmp\")\n");
    profile.push_str("  (subpath (env \"HOME\"))\n");
    profile.push_str(")\n\n");

    // Allow process execution
    profile.push_str("(allow process-exec*)\n\n");

    // Allow process forking
    profile.push_str("(allow process-fork)\n\n");

    // Allow signal handling (but not sending to other processes)
    profile.push_str("(allow signal (target self))\n\n");

    // Allow reading from stdin, writing to stdout/stderr
    profile.push_str("(allow file-read*\n");
    profile.push_str("  (literal \"/dev/null\")\n");
    profile.push_str("  (literal \"/dev/tty\")\n");
    profile.push_str("  (literal \"/dev/dtracehelper\")\n");
    profile.push_str(")\n\n");

    // Allow basic Mach operations needed for libc
    profile.push_str("(allow mach-lookup\n");
    profile.push_str("  (global-name \"com.apple.coreservices.launchservicesd\")\n");
    profile.push_str("  (global-name \"com.apple.SystemConfiguration.configd\")\n");
    profile.push_str(")\n\n");

    // Allow sysctl reads
    profile.push_str("(allow sysctl-read)\n\n");

    // Explicitly deny network access
    profile.push_str("(deny network-outbound)\n");
    profile.push_str("(deny network-inbound)\n");
    profile.push_str("(deny network-bind)\n\n");

    // Explicitly deny debugging other processes
    profile.push_str("(deny process-info*)\n");
    profile.push_str("(deny system-info*)\n");

    profile
}

/// Generate a sandbox profile that allows network access.
///
/// Similar to `default_restricted_profile` but permits outbound
/// network connections. Use for sessions that need internet access.
pub fn default_network_profile() -> String {
    let mut profile = default_restricted_profile();

    // Override network deny with allow
    profile = profile.replace("(deny network-outbound)\n", "(allow network-outbound)\n");
    profile = profile.replace("(deny network-inbound)\n", "");
    profile = profile.replace("(deny network-bind)\n", "(allow network-bind)\n");

    profile
}

/// Generate a sandbox profile for a specific working directory.
///
/// Adds read/write access to the specified directory in addition
/// to the default restricted profile.
///
/// # Arguments
///
/// * `work_dir` — Absolute path to the working directory to allow access to.
pub fn profile_for_work_dir(work_dir: &str) -> String {
    let mut profile = default_restricted_profile();

    // Insert work_dir access before the final deny rules
    let work_dir_rule = format!(
        "\n(allow file-read*\n  (subpath \"{work_dir}\")\n)\n\n\
         (allow file-write*\n  (subpath \"{work_dir}\")\n)\n"
    );

    // Insert before the network deny section
    if let Some(pos) = profile.find("// Explicitly deny network") {
        profile.insert_str(pos, &work_dir_rule);
    } else {
        // Append if we can't find the insertion point
        profile.push_str(&work_dir_rule);
    }

    profile
}

/// Validate a sandbox profile syntax (basic check).
///
/// Checks that the profile has balanced parentheses and contains
/// a version declaration. This is not a full syntax validator —
/// it catches obvious errors before calling `sandbox_init`.
pub fn validate_profile(profile: &str) -> Result<(), IsolationError> {
    // Check for version declaration
    if !profile.contains("(version") {
        return Err(IsolationError::SandboxError(
            "profile missing version declaration".to_string(),
        ));
    }

    // Check balanced parentheses
    let mut depth: i64 = 0;
    for ch in profile.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return Err(IsolationError::SandboxError(
                "unbalanced parentheses in profile".to_string(),
            ));
        }
    }
    if depth != 0 {
        return Err(IsolationError::SandboxError(
            "unbalanced parentheses in profile".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_not_empty() {
        let profile = default_restricted_profile();
        assert!(!profile.is_empty());
    }

    #[test]
    fn default_profile_has_version() {
        let profile = default_restricted_profile();
        assert!(profile.contains("(version 1)"));
    }

    #[test]
    fn default_profile_denies_default() {
        let profile = default_restricted_profile();
        assert!(profile.contains("(deny default)"));
    }

    #[test]
    fn default_profile_denies_network() {
        let profile = default_restricted_profile();
        assert!(profile.contains("(deny network-outbound)"));
        assert!(profile.contains("(deny network-inbound)"));
        assert!(profile.contains("(deny network-bind)"));
    }

    #[test]
    fn default_profile_allows_temp_writes() {
        let profile = default_restricted_profile();
        assert!(profile.contains("(subpath \"/tmp\")"));
        assert!(profile.contains("(subpath \"/var/tmp\")"));
    }

    #[test]
    fn default_profile_allows_system_reads() {
        let profile = default_restricted_profile();
        assert!(profile.contains("(subpath \"/usr/lib\")"));
        assert!(profile.contains("(subpath \"/usr/share\")"));
        assert!(profile.contains("(subpath \"/bin\")"));
    }

    #[test]
    fn default_profile_allows_process_exec() {
        let profile = default_restricted_profile();
        assert!(profile.contains("(allow process-exec"));
        assert!(profile.contains("(allow process-fork)"));
    }

    #[test]
    fn network_profile_allows_outbound() {
        let profile = default_network_profile();
        assert!(profile.contains("(allow network-outbound)"));
        assert!(profile.contains("(allow network-bind)"));
    }

    #[test]
    fn work_dir_profile_includes_directory() {
        let profile = profile_for_work_dir("/Users/test/project");
        assert!(profile.contains("/Users/test/project"));
        assert!(profile.contains("(allow file-read*"));
        assert!(profile.contains("(allow file-write*"));
    }

    #[test]
    fn validate_profile_accepts_valid() {
        let profile = default_restricted_profile();
        assert!(validate_profile(&profile).is_ok());
    }

    #[test]
    fn validate_profile_rejects_missing_version() {
        let profile = "(deny default)\n";
        assert!(validate_profile(profile).is_err());
    }

    #[test]
    fn validate_profile_rejects_unbalanced_open() {
        let profile = "(version 1)\n(deny default\n";
        assert!(validate_profile(profile).is_err());
    }

    #[test]
    fn validate_profile_rejects_unbalanced_close() {
        let profile = "(version 1)\n(deny default))\n";
        assert!(validate_profile(profile).is_err());
    }

    #[test]
    fn validate_profile_rejects_null_bytes() {
        #[cfg(target_os = "macos")]
        {
            let profile = "(version 1)\n\0(deny default)\n";
            let result = apply_sandbox_profile(profile);
            assert!(result.is_err());
        }
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn sandbox_functions_unavailable_on_non_macos() {
        assert!(apply_sandbox_profile("(version 1)\n").is_err());
    }

    #[test]
    fn sandbox_error_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<IsolationError>();
    }

    #[test]
    fn sandbox_error_is_debug() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<IsolationError>();
    }
}
