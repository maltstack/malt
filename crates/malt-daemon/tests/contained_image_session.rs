//! Real Windows HCS contained-image lifecycle coverage.
//!
//! This test intentionally drives the installed helper and daemon rather than
//! constructing HCS types. It is opt-in because the process under test must be
//! enrolled through the explicit UAC-backed helper workflow; normal `cargo
//! test` must never install or authorise a privileged service as a side effect.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const IMAGE_ID_ENV: &str = "MALT_REAL_HCS_IMAGE_ID";

fn malt_executable() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("debug")
        .join("malt.exe")
}

fn run_malt(arguments: &[&str]) -> Output {
    Command::new(malt_executable())
        .args(arguments)
        .output()
        .expect("run the rebuilt MALT CLI")
}

fn output_text(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn created_session_id(output: &Output) -> u32 {
    let text = output_text(output);
    let fields = text.split_whitespace().collect::<Vec<_>>();
    let Some(position) = fields.iter().position(|field| *field == "session") else {
        panic!("contained creation did not report a session id: {text}");
    };
    fields
        .get(position + 1)
        .expect("session id follows the session label")
        .parse()
        .expect("contained session id is numeric")
}

struct SessionCleanup {
    id: u32,
}

impl Drop for SessionCleanup {
    fn drop(&mut self) {
        let id = self.id.to_string();
        let _ = run_malt(&["kill", &id]);
    }
}

/// Run only after an operator has installed/reached the helper, started and
/// enrolled the daemon, and provided a MALT-owned image identity. A missing
/// value is an explicit environment skip, not a pass that claims HCS worked.
#[test]
#[ignore = "requires an installed/enrolled helper and MALT_REAL_HCS_IMAGE_ID"]
fn real_contained_image_executes_and_removes_its_workspace() {
    let Ok(image_id) = std::env::var(IMAGE_ID_ENV) else {
        eprintln!("SKIP: set {IMAGE_ID_ENV} to a prepared MALT-owned image digest");
        return;
    };
    if image_id.is_empty() {
        eprintln!("SKIP: {IMAGE_ID_ENV} was empty");
        return;
    }
    if !malt_executable().is_file() {
        eprintln!(
            "SKIP: rebuilt MALT CLI is absent at {}",
            malt_executable().display()
        );
        return;
    }

    let name = format!("hcs-integration-{}", std::process::id());
    let created = run_malt(&[
        "new",
        "--name",
        &name,
        "--isolation",
        "contained",
        "--image",
        &image_id,
    ]);
    assert!(created.status.success(), "{}", output_text(&created));
    let session_id = created_session_id(&created);
    let cleanup = SessionCleanup { id: session_id };

    let command = run_malt(&["exec", &session_id.to_string(), "cmd /c ver"]);
    assert!(command.status.success(), "{}", output_text(&command));
    let command_text = output_text(&command);
    assert!(
        command_text.contains("Microsoft Windows [Version"),
        "contained command did not return the Windows version: {command_text}"
    );

    let active = run_malt(&["image", "inspect", &image_id]);
    assert!(active.status.success(), "{}", output_text(&active));
    assert!(
        output_text(&active).contains("active:      1"),
        "the contained session was not retained as an image use reference: {}",
        output_text(&active)
    );

    let killed = run_malt(&["kill", &session_id.to_string()]);
    assert!(killed.status.success(), "{}", output_text(&killed));
    std::mem::forget(cleanup);

    let inactive = run_malt(&["image", "inspect", &image_id]);
    assert!(inactive.status.success(), "{}", output_text(&inactive));
    assert!(
        output_text(&inactive).contains("active:      0"),
        "destroyed contained session still holds an image reference: {}",
        output_text(&inactive)
    );

    let workspace_root =
        PathBuf::from(std::env::var_os("ProgramData").expect("ProgramData is set on Windows"))
            .join("MALT")
            .join("images")
            .join("sessions")
            .join(session_id.to_string());
    if workspace_root.exists() {
        let entries = std::fs::read_dir(&workspace_root)
            .expect("read helper-owned session workspace root")
            .collect::<Result<Vec<_>, _>>()
            .expect("enumerate helper-owned session workspace root");
        assert!(
            entries.is_empty(),
            "post-destroy helper workspace still contains entries: {}",
            workspace_root.display()
        );
    }
}
