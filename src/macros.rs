#[macro_export]
macro_rules! my_dbg {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            dbg!($($arg)*)
        } else {
            ($($arg)*)
        }
    };
}

/// Write to stdout without panic.
/// More robust than built-in when redirect stdout to pipe.
#[macro_export]
macro_rules! my_println {
    ($($arg:tt)*) => {{
        use std::io::Write;

        // Create a formatted string
        let output = format!($($arg)*);
        // Attempt to write to stdout
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(output.as_bytes())
            .and_then(|_| stdout.write_all(b"\n"))
            .map_err(|e| e.to_string())?;

        Ok(()) as Result<(), String>
    }};
}

/// Write to stdout without newline and without panic.
/// More robust than built-in when redirect stdout to pipe.
#[macro_export]
macro_rules! my_print {
    ($($arg:tt)*) => {{
        use std::io::Write;

        // Create a formatted string
        let output = format!($($arg)*);
        // Attempt to write to stdout
        std::io::stdout().lock()
            .write_all(output.as_bytes())
            .map_err(|e| e.to_string())?;

        Ok(()) as Result<(), String>
    }};
}

#[macro_export]
macro_rules! my_warning {
    ($scope:expr, $($arg:tt)*) => {{
        use colored::*;

        eprintln!("{}", $scope.color(&format!($($arg)*), Color::TrueColor{r:255, g:165, b:0}, &std::io::stderr()));
    }};
}
