use std::{env, ffi::OsString, process::ExitCode};

use cloudcover_core::Language;

use crate::{error::CliError, policy::build_go_policy};

pub(crate) const USAGE: &str = "Usage: cloudcover policy [--language go] <PATH>";

pub(crate) fn run_main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Err(CliError::Usage(message)) => {
            eprintln!("{message}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Runtime(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), CliError> {
    let mut args = env::args_os();
    let _program = args.next();

    match args.next() {
        None => Err(CliError::Usage("missing command".to_owned())),
        Some(command) if is_help_flag(&command) => Err(CliError::Help),
        Some(command) if command == "policy" => {
            let args = args.collect::<Vec<_>>();
            run_policy(&args)
        }
        Some(command) => Err(CliError::Usage(format!(
            "unsupported command: {}",
            command.to_string_lossy()
        ))),
    }
}

fn run_policy(args: &[OsString]) -> Result<(), CliError> {
    if args.len() == 1 && is_help_flag(&args[0]) {
        return Err(CliError::Help);
    }

    let mut language = Language::Go;
    let mut path = None;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if is_help_flag(argument) {
            return Err(CliError::Help);
        }
        if argument == "--language" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| CliError::Usage("missing value for --language".to_owned()))?;
            if value != "go" {
                return Err(CliError::Usage(format!(
                    "unsupported language: {}",
                    value.to_string_lossy()
                )));
            }
            language = Language::Go;
            index += 2;
            continue;
        }
        if argument.to_string_lossy().starts_with('-') {
            return Err(CliError::Usage(format!(
                "unsupported option: {}",
                argument.to_string_lossy()
            )));
        }
        if path.replace(argument.clone()).is_some() {
            return Err(CliError::Usage("too many positional arguments".to_owned()));
        }
        index += 1;
    }

    let path = path.ok_or_else(|| CliError::Usage("missing path".to_owned()))?;
    let policy = build_go_policy(path, language)?;
    let stdout = serde_json::to_string_pretty(&policy)
        .map_err(|error| CliError::Runtime(format!("failed to encode policy JSON: {error}")))?;
    println!("{stdout}");
    Ok(())
}

fn is_help_flag(argument: &OsString) -> bool {
    argument == "-h" || argument == "--help"
}
