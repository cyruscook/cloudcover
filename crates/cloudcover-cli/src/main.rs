use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fmt,
    process::ExitCode,
};

use cloudcover_aws::AwsProvider;
use cloudcover_core::{
    ApiMethod, CloudProvider, GoMethodReference, Language, MethodReference, Sdk,
};

const USAGE: &str = "Usage: cloudcover policy [--language go] <PATH>";

fn main() -> ExitCode {
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

fn build_go_policy(path: OsString, language: Language) -> Result<serde_json::Value, CliError> {
    if language != Language::Go {
        return Err(CliError::Usage(format!(
            "unsupported language: {language:?}"
        )));
    }

    let provider = AwsProvider::new();
    let sdk = Sdk::new("aws-sdk-go-v2", Language::Go);
    let mut methods_by_sdk = BTreeMap::<(String, Option<String>, String), Vec<ApiMethod>>::new();
    for mapping in provider.sdk_method_mappings() {
        if mapping.sdk() != &sdk {
            continue;
        }
        let MethodReference::Go(go_method) = mapping.method() else {
            continue;
        };
        methods_by_sdk
            .entry(go_method_key(go_method))
            .or_default()
            .extend(mapping.api_methods().iter().cloned());
    }

    let mut api_methods = BTreeSet::new();
    for go_method in cloudcover_go::analyze_dir(path)
        .map_err(|error| CliError::Runtime(format!("failed to analyze Go code: {error}")))?
    {
        if let Some(mapped_methods) = methods_by_sdk.get(&go_method_key(&go_method)) {
            api_methods.extend(mapped_methods.iter().cloned());
        }
    }

    provider
        .permissions_policy(&api_methods.into_iter().collect::<Vec<_>>())
        .map_err(|error| CliError::Runtime(error.to_string()))
}

fn go_method_key(method: &GoMethodReference) -> (String, Option<String>, String) {
    (
        method.package().to_owned(),
        method.receiver().map(str::to_owned),
        method.name().to_owned(),
    )
}

fn is_help_flag(argument: &OsString) -> bool {
    argument == "-h" || argument == "--help"
}

enum CliError {
    Help,
    Usage(String),
    Runtime(String),
}

impl fmt::Debug for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Help => formatter.write_str("Help"),
            Self::Usage(message) => formatter.debug_tuple("Usage").field(message).finish(),
            Self::Runtime(message) => formatter.debug_tuple("Runtime").field(message).finish(),
        }
    }
}
