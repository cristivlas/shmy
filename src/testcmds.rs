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
