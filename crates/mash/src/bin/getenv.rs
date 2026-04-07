use std::env;

fn main() {
    let mut args = env::args();
    let _program = args.next();
    let Some(name) = args.next() else {
        eprintln!("usage: getenv NAME");
        std::process::exit(1);
    };

    match env::var(&name) {
        Ok(value) => println!("{name}='{value}'"),
        Err(_) => println!("{name} is unset"),
    }
}
