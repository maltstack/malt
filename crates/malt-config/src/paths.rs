//! Platform-appropriate config, data, and runtime directory resolution.

use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("malt");
        }
        home_fallback(".config/malt")
    }
    #[cfg(target_os = "macos")]
    {
        home_fallback("Library/Application Support/malt")
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("malt");
        }
        home_fallback(".malt")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        home_fallback(".malt")
    }
}

pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(xdg).join("malt");
        }
        home_fallback(".local/share/malt")
    }
    #[cfg(target_os = "macos")]
    {
        home_fallback("Library/Application Support/malt")
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join("malt");
        }
        home_fallback(".malt/data")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        home_fallback(".malt/data")
    }
}

pub fn runtime_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
            return PathBuf::from(xdg).join("malt");
        }
        // Use /tmp/malt — daemon creates this directory with proper permissions.
        PathBuf::from("/tmp/malt")
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(tmpdir) = std::env::var("TMPDIR") {
            return PathBuf::from(tmpdir).join("malt");
        }
        PathBuf::from("/tmp/malt")
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join("malt").join("run");
        }
        home_fallback(".malt/run")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        PathBuf::from("/tmp/malt")
    }
}

fn home_fallback(suffix: &str) -> PathBuf {
    #[cfg(unix)]
    let home = std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    #[cfg(not(any(unix, windows)))]
    let home: Option<PathBuf> = None;
    home.unwrap_or_else(|| PathBuf::from(".")).join(suffix)
}
