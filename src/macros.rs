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
    () => {{
        $crate::my_println!("")
    }};

    ($($arg:tt)*) => {{
        use std::io::{ErrorKind, Write};

        // Create a formatted string
        let output = format!($($arg)*);

        // Attempt to write to stdout, ignoring broken pipe errors.
        let mut stdout = std::io::stdout().lock();
        match stdout
            .write_all(output.as_bytes())
            .and_then(|_| stdout.write_all(b"\n")) {
                Ok(_) => Ok(()),
                Err(e) => match e.kind() {
                    ErrorKind::BrokenPipe => Ok(()),
                    _ => Err(e.to_string()),
                },
            }
    }};
}

/// Write to stdout without newline and without panic.
/// More robust than built-in when redirect stdout to pipe.
#[macro_export]
macro_rules! my_print {
    ($($arg:tt)*) => {{
        use std::io::{ ErrorKind, Write };

        // Create a formatted string
        let output = format!($($arg)*);

        // Attempt to write to stdout, ignoring broken pipe errors.
        match std::io::stdout().lock().write_all(output.as_bytes()) {
            Ok(_) => Ok(()),
            Err(e) => match e.kind() {
                ErrorKind::BrokenPipe => Ok(()),
                _ => Err(e.to_string()),
            },
        }
    }};
}

#[macro_export]
macro_rules! my_warning {
    ($scope:expr, $($arg:tt)*) => {{
        use colored::*;

        eprintln!("{}", $scope.color(&format!($($arg)*), Color::TrueColor{r:255, g:165, b:0}, &std::io::stderr()));
    }};
}
