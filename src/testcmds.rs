#[cfg(test)]
mod tests {
    use crate::assert_eval_ok;
    use crate::eval::*;
    use crate::testeval::tests::*;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::str::FromStr;
    use tempfile::TempDir;

    macro_rules! assert_err_loc {
        ($expr:literal, $loc:expr) => {
            match eval($expr) {
                Err(EvalError { loc: ref loc, .. }) => {
                    assert_eq!(loc, &$loc);
                }
                Ok(_) => {
                    panic!("Expected error, got Ok")
                }
            }
        };
    }

    #[test]
    fn test_cat_err() {
        assert_eval_ok!("echo abc | cat | x; $x", Value::from_str("abc").unwrap());
        assert_err_loc!("cat   -n bogus", Location::new(1, 9));
    }

    #[test]
    fn test_chmod_err() {
        assert_err_loc!("chmod  -r   -v  w+x bogus", Location::new(1, 20));
    }

    #[test]
    fn test_cp_err() {
        assert_err_loc!("cp -f  -P  -ir fuzz .", Location::new(1, 15));
    }

    #[test]
    fn test_cp() {
        // Create a temporary directory for our test
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();

        // Generate a source file with known content
        let source_path = temp_path.join("source.txt");
        let source_content = "This is a test file for cp command.";
        let mut source_file = File::create(&source_path).unwrap();
        source_file.write_all(source_content.as_bytes()).unwrap();

        // Define the destination path
        let dest_path = temp_path.join("destination.txt");

        // Execute the cp command
        let cmd = format!("cp {} {}", source_path.display(), dest_path.display());
        assert_eval_ok!(&cmd, Value::success());

        // Verify that the destination file exists
        assert!(dest_path.exists(), "Destination file was not created");

        // Read the content of the destination file
        let mut dest_content = String::new();
        File::open(&dest_path)
            .unwrap()
            .read_to_string(&mut dest_content)
            .unwrap();

        // Compare the content of source and destination
        assert_eq!(source_content, dest_content, "File content does not match");

        // Clean up is automatically done by TempDir when it goes out of scope
    }

    #[test]
    fn test_diff_err() {
        assert_err_loc!("diff  --color x y", Location::new(1, 14));
    }

    #[test]
    fn test_ls_err() {
        assert_err_loc!("ls  -u  -h  -l null", Location::new(1, 15));
    }

    #[test]
    fn test_realpath_err() {
        assert_err_loc!("realpath . foo", Location::new(1, 11));
    }
}
