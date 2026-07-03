use std::fmt;

pub(crate) enum CliError {
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
