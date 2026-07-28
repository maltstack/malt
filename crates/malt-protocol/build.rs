use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let schemas_dir = manifest_dir.join("../../schemas");
    let schemas_dir = schemas_dir.canonicalize()?;
    let vexilc = env::var("VEXILC_PATH").unwrap_or_else(|_| "vexilc".to_string());

    let mut schema_files = Vec::new();
    collect_vexil_files(&schemas_dir, &mut schema_files)?;

    for schema in &schema_files {
        let status = Command::new(&vexilc)
            .arg("build")
            .arg(schema)
            .arg("--include")
            .arg(&schemas_dir)
            .arg("--output")
            .arg(&out_dir)
            .arg("--target")
            .arg("rust")
            .status()?;

        if !status.success() {
            return Err(
                io::Error::other(format!("vexilc build failed for {}", schema.display())).into(),
            );
        }
    }

    normalize_generated_sources(&out_dir)?;
    write_mod_rs(&out_dir)?;

    println!("cargo::rerun-if-changed={}", schemas_dir.display());
    for schema in &schema_files {
        println!("cargo::rerun-if-changed={}", schema.display());
    }
    Ok(())
}

fn normalize_generated_sources(out_dir: &Path) -> io::Result<()> {
    let malt_dir = out_dir.join("malt");
    if !malt_dir.is_dir() {
        return Ok(());
    }
    normalize_generated_dir(&malt_dir)
}

fn normalize_generated_dir(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            normalize_generated_dir(&path)?;
            continue;
        }
        if path.extension().is_some_and(|e| e == "rs") {
            normalize_generated_file(&path)?;
        }
    }
    Ok(())
}

fn normalize_generated_file(path: &Path) -> io::Result<()> {
    let src = fs::read_to_string(path)?;
    let mut out = String::with_capacity(src.len());

    let mut in_pack_impl = false;
    let mut pack_impl_depth = 0usize;
    let mut in_pack_fn = false;
    let mut pack_fn_depth = 0usize;
    let mut in_match_self = false;
    let mut match_depth = 0usize;

    for line in src.lines() {
        let trimmed = line.trim_start();
        let opens = line.chars().filter(|&c| c == '{').count();
        let closes = line.chars().filter(|&c| c == '}').count();

        if trimmed.starts_with("impl vexil_runtime::Pack for ") {
            in_pack_impl = true;
            pack_impl_depth = 0;
        }
        if in_pack_impl && trimmed.starts_with("fn pack(&self, ") {
            in_pack_fn = true;
            pack_fn_depth = 0;
        }
        if in_pack_fn && trimmed.starts_with("match self {") {
            in_match_self = true;
            match_depth = 0;
        }

        let indent_len = line.len().saturating_sub(trimmed.len());
        let indent = &line[..indent_len];
        let mut rewritten = line.to_string();
        if in_match_self {
            rewritten = rewrite_pack_match_loop_line(line);
        }
        if trimmed == "use vexil_runtime::*;" {
            rewritten = format!("{indent}#[allow(unused_imports)]\n{indent}use vexil_runtime::*;");
        }

        out.push_str(&rewritten);
        out.push('\n');

        if in_match_self {
            match_depth = match_depth.saturating_add(opens);
            match_depth = match_depth.saturating_sub(closes);
            if match_depth == 0 {
                in_match_self = false;
            }
        }
        if in_pack_fn {
            pack_fn_depth = pack_fn_depth.saturating_add(opens);
            pack_fn_depth = pack_fn_depth.saturating_sub(closes);
            if pack_fn_depth == 0 {
                in_pack_fn = false;
            }
        }
        if in_pack_impl {
            pack_impl_depth = pack_impl_depth.saturating_add(opens);
            pack_impl_depth = pack_impl_depth.saturating_sub(closes);
            if pack_impl_depth == 0 {
                in_pack_impl = false;
            }
        }
    }

    if path.file_name().is_some_and(|f| f == "envelope.rs") {
        out = out.replace("pub timestamp: u8,", "pub timestamp: u64,");
        out = out.replace(
            "let timestamp = r.read_bits(48_u8)? as u8;",
            "let timestamp = r.read_bits(48_u8)? as u64;",
        );
    }

    if out != src {
        fs::write(path, out)?;
    }
    Ok(())
}

fn rewrite_pack_match_loop_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent_len = line.len().saturating_sub(trimmed.len());
    let indent = &line[..indent_len];
    let Some(rest) = trimmed.strip_prefix("for item in &") else {
        return line.to_string();
    };
    let Some(ident) = rest.strip_suffix(" {") else {
        return line.to_string();
    };
    if ident.is_empty() || !is_simple_ident(ident) {
        return line.to_string();
    }
    format!("{indent}for item in {ident} {{")
}

fn is_simple_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn collect_vexil_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_vexil_files(&path, files)?;
        } else if path.extension().is_some_and(|e| e == "vexil") {
            files.push(path);
        }
    }
    Ok(())
}

fn write_mod_rs(out_dir: &Path) -> io::Result<()> {
    let malt_dir = out_dir.join("malt");
    let mut modules = Vec::new();

    if let Ok(entries) = fs::read_dir(&malt_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path.extension().is_some_and(|e| e == "rs")
                && path.file_stem().is_some_and(|s| s != "mod")
            {
                if let Some(name) = path.file_stem() {
                    modules.push(name.to_string_lossy().to_string());
                }
            } else if path.is_dir() && path.join("mod.rs").exists() {
                if let Some(name) = path.file_name() {
                    modules.push(name.to_string_lossy().to_string());
                }
            }
        }
    }
    modules.sort();

    let mut content = String::from("// Generated by malt-protocol build.rs. DO NOT EDIT.\n\n");
    for m in &modules {
        content.push_str(&format!("pub mod {m};\n"));
    }
    fs::write(malt_dir.join("mod.rs"), &content)?;

    // Handle persist/ subdirectory
    let persist_dir = malt_dir.join("persist");
    if persist_dir.is_dir() {
        let mut persist_mods = Vec::new();
        if let Ok(entries) = fs::read_dir(&persist_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && path.extension().is_some_and(|e| e == "rs")
                    && path.file_stem().is_some_and(|s| s != "mod")
                {
                    if let Some(name) = path.file_stem() {
                        persist_mods.push(name.to_string_lossy().to_string());
                    }
                }
            }
        }
        persist_mods.sort();

        let mut pc = String::from("// Generated by malt-protocol build.rs. DO NOT EDIT.\n\n");
        for m in &persist_mods {
            pc.push_str(&format!("pub mod {m};\n"));
        }
        fs::write(persist_dir.join("mod.rs"), &pc)?;
    }

    // Root mod.rs
    fs::write(
        out_dir.join("mod.rs"),
        "// Generated by malt-protocol build.rs. DO NOT EDIT.\n\npub mod malt;\n",
    )?;
    Ok(())
}
