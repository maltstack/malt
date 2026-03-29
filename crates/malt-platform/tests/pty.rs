use malt_platform::pty::{open_pty, WinSize};

#[test]
fn open_and_resize() {
    let size = WinSize { cols: 80, rows: 24 };
    let (pty, _reader, _writer) = open_pty(size).unwrap();
    let new_size = WinSize {
        cols: 120,
        rows: 40,
    };
    pty.resize(new_size).unwrap();
}

#[test]
fn open_multiple_ptys() {
    // Verify we can open several PTYs concurrently without handle leaks.
    let mut handles = Vec::new();
    for i in 0..4 {
        let size = WinSize {
            cols: 80 + i * 10,
            rows: 24,
        };
        let (pty, reader, writer) = open_pty(size).unwrap();
        handles.push((pty, reader, writer));
    }
    // Resize each one.
    for (pty, _, _) in &handles {
        pty.resize(WinSize { cols: 100, rows: 30 }).unwrap();
    }
    // Drop all — exercises cleanup paths.
    drop(handles);
}

#[test]
fn resize_multiple_times() {
    let size = WinSize { cols: 80, rows: 24 };
    let (pty, _reader, _writer) = open_pty(size).unwrap();

    for cols in [40, 80, 120, 200, 10] {
        pty.resize(WinSize { cols, rows: 24 }).unwrap();
    }
    for rows in [10, 24, 40, 60, 5] {
        pty.resize(WinSize { cols: 80, rows }).unwrap();
    }
}

/// ConPTY (and Unix PTY) I/O requires a child process attached to the
/// pseudo console — without one, reads block and writes go nowhere.
/// Full read/write integration tests belong with the process spawning
/// module. This test verifies that the writer handle is writable.
#[test]
fn writer_is_writable() {
    use std::io::Write;

    let size = WinSize { cols: 80, rows: 24 };
    let (_pty, _reader, mut writer) = open_pty(size).unwrap();

    // Writing to the input pipe should succeed even without a child process.
    writer.write_all(b"hello").unwrap();
    writer.flush().unwrap();
}
