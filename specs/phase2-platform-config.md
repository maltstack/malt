# Phase 2: `malt-platform` Core + `malt-config` Design Spec

## Goal

Build the two L0 foundation crates: `malt-platform` (OS abstractions for PTY, process spawning, signals, sockets) and `malt-config` (Vexil Store config loading with schema validation). These unblock all L1 crates (`mash`, `malt-term`, `malt-tools`, `malt-layout`, `malt-session`).

## Architecture

Both crates are L0 — zero internal workspace dependencies (except `malt-protocol` for type re-exports). `malt-platform` abstracts all OS interactions behind traits with platform-specific implementations split into separate files. `malt-config` wraps `vexil-store` with config-specific responsibilities (path hierarchy, schema validation, defaults). The isolation tier model (Bare → Contained) is a separate spec — this covers only the core platform abstractions.

## Reference Implementation

`C:\Users\mamuk\projects\vexil-v2\vexil-platform\src\` (15,365 lines) contains working code to port. **Port the logic and architecture, rewrite the code with proper quality.** The reference's code organization and style were weak — we're building from scratch with the same abstractions but better structure.

## Spec References

- `malt/specs/architecture.md` §2 (bus/signals), §5 (shell/PTY), §6 (VNP/sockets), §10 (isolation overview)
- `malt/specs/phase1-vnp-schema-design.md` (SignalKind enum in common.vexil)

---

## Crate Structure

```
orix/malt/crates/
  malt-config/
    Cargo.toml
    build.rs                    # vexilc codegen for config schemas
    src/
      lib.rs                    # Config<T>, load functions, public API
      paths.rs                  # XDG/platform config path resolution
    tests/
      load.rs                   # Load/validate/default tests
      paths.rs                  # Path resolution tests

  malt-platform/
    Cargo.toml
    src/
      lib.rs                    # Re-exports all modules
      pty/
        mod.rs                  # Pty trait, WinSize, open_pty(), PtyError
        unix.rs                 # UnixPty via nix::pty::openpty
        windows.rs              # ConPty via CreatePseudoConsole
      process/
        mod.rs                  # SpawnConfig, Child, Io, spawn()
        unix.rs                 # fork/exec
        windows.rs              # CreateProcessW
      signals/
        mod.rs                  # SignalBroker trait, send_signal(), name/number lookups
        unix.rs                 # Unix signal impl (nix + optional tokio)
        windows.rs              # Windows signal impl (GenerateConsoleCtrlEvent, TerminateProcess)
      sockets/
        mod.rs                  # Transport enum, connect/listen
        unix.rs                 # Unix domain sockets
        windows.rs              # Named pipes
      env.rs                    # Environment/cwd helpers
      io.rs                     # Io enum, pipe creation
    tests/
      pty.rs                    # PTY open/read/write/resize
      process.rs                # Spawn, capture, wait, process groups
      signals.rs                # Send, name lookup, number mapping
      sockets.rs                # Transport roundtrip
      env.rs                    # Environment helpers

orix/malt/schemas/config/
  daemon.vexil                  # Daemon config schema
  user.vexil                    # User settings schema
```

---

## `malt-platform` Core Traits

### PTY

```rust
// pty/mod.rs

pub trait Pty: Send + Sync {
    fn resize(&self, size: WinSize) -> Result<(), PtyError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinSize {
    pub cols: u16,
    pub rows: u16,
}

/// Open a new PTY pair. Returns the PTY handle (for resize),
/// a read half (PTY output), and a write half (PTY input).
/// The reader/writer have independent ownership — no lifetime coupling.
pub fn open_pty(size: WinSize) -> Result<(Arc<dyn Pty>, OwnedReadHalf, OwnedWriteHalf), PtyError>;

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("failed to open PTY: {0}")]
    Open(std::io::Error),
    #[error("failed to resize PTY: {0}")]
    Resize(std::io::Error),
}
```

**Unix impl:** `nix::pty::openpty()`, `O_CLOEXEC` on both fds, resize via `TIOCSWINSZ` ioctl. Master fd duplicated into independent reader/writer `File` handles.

**Windows impl:** `CreatePseudoConsole()`, input/output pipe pair, resize via `ResizePseudoConsole()`.

**No `as_any()` downcast** — if platform-specific access is ever needed, it goes through a platform extension trait.

### Process

```rust
// process/mod.rs

pub struct SpawnConfig {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub env_clear: bool,
    pub cwd: Option<PathBuf>,
    pub stdin: Io,
    pub stdout: Io,
    pub stderr: Io,
    pub pty: Option<Arc<dyn Pty>>,
    pub process_group: ProcessGroup,
}

pub enum Io {
    Inherit,
    Null,
    Pipe,
    File(std::fs::File),
}

pub enum ProcessGroup {
    Inherit,
    New,
    Join(u32),
}

pub struct ExitStatus(i32);

impl ExitStatus {
    pub fn code(&self) -> i32;
    pub fn success(&self) -> bool;
}

pub fn spawn(config: SpawnConfig) -> Result<Child, SpawnError>;

pub struct Child { /* opaque, platform-specific inner */ }

impl Child {
    pub fn pid(&self) -> u32;
    pub fn wait(&mut self) -> Result<ExitStatus, SpawnError>;
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, SpawnError>;
    pub fn take_stdin(&mut self) -> Option<ChildStdin>;
    pub fn take_stdout(&mut self) -> Option<ChildStdout>;
    pub fn take_stderr(&mut self) -> Option<ChildStderr>;
}

// With tokio feature:
// pub async fn wait_async(child: &mut Child) -> Result<ExitStatus, SpawnError>;

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("executable not found: {}", path.display())]
    NotFound { path: PathBuf },
    #[error("permission denied: {}", path.display())]
    PermissionDenied { path: PathBuf },
    #[error("spawn failed: {0}")]
    Io(#[from] std::io::Error),
}
```

**Unix impl:** `nix::unistd::fork()` + `execvp()` in pre_exec closure. PTY slave attached via `dup2`. Process group via `setpgid`. `O_CLOEXEC` on all fds. Wait via `waitpid`. Drop impl reaps zombie with `WNOHANG`.

**Windows impl:** `CreateProcessW()`. Console handles via `STARTUPINFO`. Process group available for later isolation. Wait via `WaitForSingleObject`. Drop impl closes handle.

**No `ResourceLimits` or `IsolationSpawnConfig`** — those belong in the isolation spec.

### Signals

```rust
// signals/mod.rs

pub use malt_protocol::common::SignalKind;

pub trait SignalBroker: Send + Sync {
    fn send(&self, pid: u32, signal: SignalKind) -> Result<(), SignalError>;
    fn subscribe(&self) -> SignalReceiver;
}

/// One-off signal send without a broker instance.
pub fn send_signal(pid: u32, signal: SignalKind) -> Result<(), SignalError>;

/// Lookup helpers — single source of truth for signal mapping.
pub fn signal_by_name(name: &str) -> Option<SignalKind>;
pub fn signal_name(signal: SignalKind) -> &'static str;
pub fn signal_number(signal: SignalKind) -> i32;

/// Check if a process is alive.
pub fn process_exists(pid: u32) -> bool;

#[derive(Debug, thiserror::Error)]
pub enum SignalError {
    #[error("no such process: {pid}")]
    NoSuchProcess { pid: u32 },
    #[error("permission denied for pid {pid}")]
    PermissionDenied { pid: u32 },
    #[error("signal delivery failed: {0}")]
    Io(#[from] std::io::Error),
}
```

`SignalKind` is re-exported from `malt_protocol::common::SignalKind` — the enum defined in `common.vexil`. No duplicate definition.

**Unix impl:** `nix::sys::signal::kill()` for send. `tokio::signal::unix` for subscribe (behind feature flag). `signal_by_name` accepts `"TERM"`, `"SIGTERM"`, or `"15"`.

**Windows impl:** `GenerateConsoleCtrlEvent` for INT/QUIT, `TerminateProcess` for TERM/KILL.

### Sockets

```rust
// sockets/mod.rs

pub enum Transport {
    #[cfg(unix)]
    UnixSocket(PathBuf),
    #[cfg(windows)]
    NamedPipe(String),
    Tcp(std::net::SocketAddr),
}

impl Transport {
    /// Platform-appropriate default local transport.
    /// Unix: ~/.malt/daemon.sock
    /// Windows: \\.\pipe\malt-daemon
    pub fn default_local() -> Self;

    /// Serialize for MALT_SOCKET env var.
    pub fn to_env_string(&self) -> String;

    /// Parse from MALT_SOCKET env var.
    pub fn from_env_string(s: &str) -> Result<Self, TransportError>;
}
```

Async connect/listen behind `tokio` feature flag.

### Environment Helpers

```rust
// env.rs — small, no platform split needed

pub fn current_dir() -> Result<PathBuf, std::io::Error>;
pub fn home_dir() -> Option<PathBuf>;
pub fn is_interactive_terminal() -> bool;
```

### I/O Utilities

```rust
// io.rs

/// Create an anonymous OS pipe. Returns (read_end, write_end).
pub fn create_pipe() -> Result<(std::fs::File, std::fs::File), std::io::Error>;
```

---

## `malt-config` Design

### Dependencies

```toml
[dependencies]
vexil-runtime = { git = "https://github.com/vexil-lang/vexil", branch = "main" }
vexil-store = { git = "https://github.com/vexil-lang/vexil", branch = "main" }
thiserror = "2"
```

No `malt-platform` dependency. Path resolution uses `std::env` directly.

### Config Schemas

```vexil
// schemas/config/daemon.vexil
@version("0.1.0")
namespace malt.config.daemon

config DaemonConfig {
    socket_path         : optional<string> = none
    max_sessions        : u32              = 64
    default_tier        : string           = "Bare"
    scrollback_lines    : u32              = 10000
    log_level           : string           = "info"
    persist_dir         : optional<string> = none
    persist_interval_secs : u32            = 30
}
```

```vexil
// schemas/config/user.vexil
@version("0.1.0")
namespace malt.config.user

config UserConfig {
    edit_mode           : string           = "emacs"
    theme               : optional<string> = none
    shell               : optional<string> = none
    startup_commands    : array<string>    = []
}
```

### Public API

```rust
pub struct Config<T> {
    inner: T,
    path: Option<PathBuf>,  // None if loaded from defaults only
}

impl<T> Config<T> {
    pub fn get(&self) -> &T;
    pub fn path(&self) -> Option<&Path>;
}

/// Load daemon config. Searches system → user → env override paths.
/// Missing files use schema defaults. Invalid files return ConfigError.
pub fn load_daemon_config() -> Result<Config<DaemonConfig>, ConfigError>;

/// Load user config from user config directory.
pub fn load_user_config() -> Result<Config<UserConfig>, ConfigError>;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file invalid: {path}: {reason}")]
    Invalid { path: PathBuf, reason: String },
    #[error("config I/O error: {0}")]
    Io(#[from] std::io::Error),
}
```

### Path Resolution

```rust
// paths.rs

/// Config directory: XDG_CONFIG_HOME/malt/ or platform fallback.
pub fn config_dir() -> PathBuf;

/// Data directory: XDG_DATA_HOME/malt/ or platform fallback.
pub fn data_dir() -> PathBuf;

/// Runtime directory: XDG_RUNTIME_DIR/malt/ or platform fallback.
pub fn runtime_dir() -> PathBuf;
```

| Platform | Config | Data | Runtime |
|----------|--------|------|---------|
| Linux | `$XDG_CONFIG_HOME/malt/` or `~/.config/malt/` | `$XDG_DATA_HOME/malt/` or `~/.local/share/malt/` | `$XDG_RUNTIME_DIR/malt/` or `/tmp/malt-$UID/` |
| macOS | `~/Library/Application Support/malt/` | same | `$TMPDIR/malt/` |
| Windows | `%APPDATA%\malt\` | `%LOCALAPPDATA%\malt\` | `%LOCALAPPDATA%\malt\run\` |

---

## `malt-platform` Dependencies

```toml
[dependencies]
thiserror = "2"
tracing = "0.1"
malt-protocol = { path = "../malt-protocol" }

[target.'cfg(unix)'.dependencies]
nix = { version = "0.31", features = ["process", "signal", "term", "ioctl", "fs"] }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61", features = [
    "Win32_Foundation",
    "Win32_System_Console",
    "Win32_System_Threading",
    "Win32_System_Pipes",
    "Win32_Security",
    "Win32_Storage_FileSystem",
] }

[features]
default = []
tokio = ["dep:tokio"]

[dependencies.tokio]
version = "1"
features = ["process", "signal", "net", "io-util"]
optional = true
```

Minimal feature sets. Isolation-specific features (Job Objects, HCS, mount, sched, resource) added in the isolation spec later.

---

## Tokio Feature Flag

Default: sync-only. With `features = ["tokio"]`:

- `signals::AsyncSignalBroker` — async signal subscription via `tokio::signal`
- `process::wait_async()` — async child process wait
- `sockets` — async connect/accept for `Transport`

Consumers opt in:
```toml
# malt-daemon enables async
malt-platform = { path = "../malt-platform", features = ["tokio"] }

# malt-tools stays sync
malt-platform = { path = "../malt-platform" }
```

---

## Testing Strategy

### `malt-platform` Tests

1. **PTY** — Open, write bytes to master, read from slave (or vice versa), verify roundtrip. Resize and verify new dimensions. Skip on CI environments without PTY support (`#[cfg_attr(not(feature = "pty-tests"), ignore)]`).

2. **Process** — Spawn `echo hello`, capture stdout, verify `"hello\n"`. Spawn with `Io::Null`, verify empty stdout. Spawn nonexistent binary, verify `SpawnError::NotFound`. Test `try_wait` returns `None` for running process, `Some` after exit. Test process group creation.

3. **Signals** — `signal_by_name("TERM")` returns `Some(SignalKind::Term)`. `signal_by_name("SIGTERM")` same. `signal_by_name("15")` same. `signal_name(SignalKind::Term)` returns `"TERM"`. Spawn sleep, send SIGTERM, verify process exits.

4. **Sockets** — `Transport::default_local()` returns platform-appropriate value. `to_env_string()` / `from_env_string()` roundtrip. Invalid env string returns error.

5. **Env** — `current_dir()` matches `std::env::current_dir()`. `home_dir()` returns `Some` in normal environments.

### `malt-config` Tests

1. **Defaults** — No config file, `load_daemon_config()` returns schema defaults (`max_sessions=64`, `log_level="info"`).

2. **Load from file** — Write `.vx` to tempdir, load, verify overrides. Partial override (only `log_level`) preserves other defaults.

3. **Invalid file** — Malformed `.vx` returns `ConfigError::Invalid`.

4. **Path resolution** — `config_dir()` respects `$XDG_CONFIG_HOME` when set. Falls back correctly when unset.

No mocking. Real filesystem via tempdir for config tests.

---

## Architecture Spec Updates

Three lines to change in `specs/architecture.md`:

1. Crate description: `L0 malt-config` — change "TOML config: typed structs, validated at load" to "Vexil Store config: schema-validated `.vx` files, loaded at startup"

2. Persistence format section — remove "User-facing config (`config.toml`, `startup.toml`) stays TOML" and replace with "All structured configuration uses Vexil Store `.vx` format, validated against Vexil schemas at load time"

3. Any remaining references to `config.toml` or `startup.toml` → `config.vx` / `startup.vx`
