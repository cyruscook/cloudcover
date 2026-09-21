use std::{env, path::Path, process};

fn main() {
    let mut arguments = env::args_os();
    let program = arguments.next().map_or_else(
        || "cloudcover-site-data".to_owned(),
        |value| value.to_string_lossy().into_owned(),
    );
    let Some(output_dir) = arguments.next() else {
        eprintln!("error: usage: {program} OUTPUT_DIR");
        process::exit(2);
    };
    if arguments.next().is_some() {
        eprintln!("error: usage: {program} OUTPUT_DIR");
        process::exit(2);
    }

    if let Err(error) = cloudcover_site_data::generate(Path::new(&output_dir)) {
        eprintln!("error: {error}");
        process::exit(1);
    }
}
