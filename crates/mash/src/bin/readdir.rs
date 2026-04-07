fn main() {
    println!(".");
    println!("..");

    let mut entries: Vec<String> = std::fs::read_dir(".")
        .unwrap_or_else(|error| {
            eprintln!("readdir: {error}");
            std::process::exit(1);
        })
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();

    for entry in entries {
        println!("{entry}");
    }
}
