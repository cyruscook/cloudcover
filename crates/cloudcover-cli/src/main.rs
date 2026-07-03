mod command;
mod error;
mod policy;

fn main() -> std::process::ExitCode {
    command::run_main()
}
