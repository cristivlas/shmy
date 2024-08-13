
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

/// Write to stdout without panic
#[macro_export]
macro_rules! my_println {
    // Version with arguments
    ($($arg:tt)*) => {{
        use std::io::Write;

        // Create a formatted string
        let output = format!($($arg)*);
        // Attempt to write to stdout
        std::io::stdout().lock()
            .write_all(output.as_bytes())
            .and_then(|_| std::io::stdout().write_all(b"\n"))
            .map_err(|e| e.to_string())?;

        Ok(()) as Result<(), String>
    }};
}

/// Write to stdout without newline and without panic
#[macro_export]
macro_rules! my_print {
    // Version with arguments
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
