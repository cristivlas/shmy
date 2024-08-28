#[cfg(test)]
mod tests {
    use crate::assert_eval_ok;
    use crate::eval::*;
    use crate::testeval::tests::*;
    use std::str::FromStr;

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
        assert_err_loc!("cat src\\main.rs  -n bogus", Location::new(1, 20));
    }

    #[test]
    fn test_realpath_err() {
        assert_err_loc!("realpath . foo", Location::new(1, 11));
    }
}
