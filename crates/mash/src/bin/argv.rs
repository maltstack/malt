fn main() {
    for (index, arg) in std::env::args().enumerate() {
        let rendered = if index == 0 {
            normalize_arg0(&arg)
        } else {
            arg.replace('\\', "/")
        };
        println!("argv[{index}] = \"{rendered}\";");
    }
}

fn normalize_arg0(arg: &str) -> String {
    let normalized = arg.replace('\\', "/");
    normalized
        .strip_suffix(".exe")
        .or_else(|| normalized.strip_suffix(".EXE"))
        .unwrap_or(&normalized)
        .to_string()
}
