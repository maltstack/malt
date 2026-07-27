//! Operating-system facts needed by higher-level policy.
//!
//! This module is the sole owner of the Windows version probe. Callers receive
//! a value, never a Win32/NT FFI handle or structure.

use std::io;

/// A numeric Windows version suitable for image compatibility policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowsVersion {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub revision: u32,
}

impl WindowsVersion {
    /// Parse the `os.version` format reported by Windows OCI images.
    pub fn parse(value: &str) -> Result<Self, String> {
        let mut components = value.split('.');
        let mut parse_component = |name: &str| {
            components
                .next()
                .ok_or_else(|| format!("Windows version is missing {name}"))?
                .parse::<u32>()
                .map_err(|_| format!("Windows version has an invalid {name}"))
        };
        let version = Self {
            major: parse_component("major component")?,
            minor: parse_component("minor component")?,
            build: parse_component("build component")?,
            revision: components
                .next()
                .map(|component| {
                    component.parse::<u32>().map_err(|_| {
                        "Windows version has an invalid revision component".to_string()
                    })
                })
                .transpose()?
                .unwrap_or(0),
        };
        if components.next().is_some() {
            return Err("Windows version has more than four components".to_string());
        }
        Ok(version)
    }

    /// Return whether this host satisfies MALT's conservative baseline policy
    /// for a process-isolated Windows image. HCS remains the final authority;
    /// this rejects only combinations known to be older or a different NT line.
    pub fn supports_container_image(self, image: Self) -> bool {
        self.major == image.major && self.minor == image.minor && self.build >= image.build
    }
}

impl std::fmt::Display for WindowsVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}.{}.{}.{}",
            self.major, self.minor, self.build, self.revision
        )
    }
}

/// Read the actual Windows kernel version rather than the process compatibility
/// version exposed by deprecated Win32 version APIs.
#[cfg(windows)]
pub fn windows_host_version() -> io::Result<WindowsVersion> {
    #[repr(C)]
    struct RtlOsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform_id: u32,
        csd_version: [u16; 128],
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(version_information: *mut RtlOsVersionInfo) -> i32;
    }

    let mut info = RtlOsVersionInfo {
        size: std::mem::size_of::<RtlOsVersionInfo>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform_id: 0,
        csd_version: [0; 128],
    };
    // SAFETY: `info` is initialized with its documented size and remains valid
    // and uniquely borrowed for the duration of the `RtlGetVersion` call.
    let status = unsafe { RtlGetVersion(&mut info) };
    if status < 0 {
        return Err(io::Error::other(format!(
            "RtlGetVersion failed with NTSTATUS 0x{:08x}",
            status as u32
        )));
    }
    Ok(WindowsVersion {
        major: info.major,
        minor: info.minor,
        build: info.build,
        revision: 0,
    })
}

/// Windows container image policy has no meaning on non-Windows hosts.
#[cfg(not(windows))]
pub fn windows_host_version() -> io::Result<WindowsVersion> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows image host assessment requires Windows",
    ))
}

#[cfg(test)]
mod tests {
    use super::WindowsVersion;

    #[test]
    fn parses_oci_windows_versions_and_rejects_ambiguous_text() {
        assert_eq!(
            WindowsVersion::parse("10.0.20348.5386").expect("parse version"),
            WindowsVersion {
                major: 10,
                minor: 0,
                build: 20348,
                revision: 5386,
            }
        );
        assert!(WindowsVersion::parse("10.0").is_err());
        assert!(WindowsVersion::parse("10.0.20348.x").is_err());
        assert!(WindowsVersion::parse("10.0.20348.1.2").is_err());
    }

    #[test]
    fn host_policy_rejects_newer_or_different_nt_lines() {
        let host = WindowsVersion::parse("10.0.26200.1").expect("parse host");
        assert!(host.supports_container_image(
            WindowsVersion::parse("10.0.20348.5386").expect("parse supported image")
        ));
        assert!(!host.supports_container_image(
            WindowsVersion::parse("10.0.27000.1").expect("parse newer image")
        ));
        assert!(!host.supports_container_image(
            WindowsVersion::parse("11.0.20348.1").expect("parse other NT line")
        ));
    }
}
