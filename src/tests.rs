#[cfg(test)]
mod tests {
    use crate::eval::*;
    use std::str::FromStr;

    fn eval(input: &str) -> EvalResult<Value> {
        let interp = Interp::new();
        let mut quit = false;
        let result = interp.eval(&mut quit, input);
        dbg!(&result);
        result
    }

    macro_rules! assert_eval_ok {
        ($expr:literal, $val:pat) => {
            assert!(matches!(eval($expr), Ok($val)));
        };
        ($expr:literal, $val:expr) => {
            assert!(matches!(eval($expr), Ok(ref v) if *v == $val));
        };
    }

    macro_rules! assert_eval_err {
        ($expr:literal, $message:literal) => {
            match eval($expr) {
                Err(EvalError {
                    message: ref msg, ..
                }) => {
                    assert_eq!(msg, $message);
                }
                Ok(_) => panic!("Expected an error for expression '{}', but got Ok", $expr),
            }
        };
    }

    #[test]
    fn test_assign() {
        assert_eval_ok!("i = 3; $i", Value::Int(3));
    }

    #[test]
    fn test_assign_chain() {
        assert_eval_ok!("i = j = 3; $i == $j && $i == 3 && $j == 3", Value::Int(1));
    }

    #[test]
    fn test_equals() {
        assert_eval_ok!("i = 42; $i == 42", Value::Int(1));
        assert_eval_ok!("i = 42; $i != 13", Value::Int(1));
    }

    #[test]
    fn test_gt() {
        assert_eval_ok!("i = 42; $i > 42", Value::Int(0));
        assert_eval_ok!("i = 50; $i > 42", Value::Int(1));
        assert_eval_ok!("i = 42; $i >= 42", Value::Int(1));
    }

    #[test]
    fn test_if() {
        assert_eval_ok!("i = 1; if $i (True)", Value::from_str("True").unwrap());
    }

    #[test]
    fn test_if_no_group() {
        assert_eval_err!(
            "i = 1; if $i true",
            "Parentheses are required around IF block"
        )
    }

    #[test]
    fn test_else() {
        assert_eval_ok!(
            "i = 1; if ($i < 0) (Apple) else (Orange)",
            Value::from_str("Orange").unwrap()
        );
    }

    #[test]
    fn test_else_no_if() {
        assert_eval_err!("else fail", "ELSE without IF")
    }

    #[test]
    fn test_else_no_group() {
        assert_eval_err!(
            "i = 1; if $i (1) else 0",
            "Parentheses are required around ELSE block"
        )
    }

    #[test]
    fn test_for() {
        assert_eval_ok!(
            "i = \"\"; for j in a b c d; ($i = $i + $j)",
            Value::from_str("abcd").unwrap()
        );
    }

    #[test]
    fn test_for_tilde() {
        let interp = Interp::new();
        let mut quit = false;
        interp
            .get_scope()
            .insert("HOME".to_string(), Value::from_str("abc").unwrap());
        let result = interp.eval(&mut quit, "for i in ~/foo; ($i)");
        dbg!(&result);
        assert!(matches!(result, Ok(ref v) if v.to_string() == "abc/foo"));
    }

    #[test]
    fn test_for_no_group() {
        assert_eval_err!(
            "for i in _; hello",
            "Parentheses are required around FOR body"
        )
    }

    #[test]
    fn test_while() {
        assert_eval_ok!(
            "i = 3; j = 0; while ($i > 0) ($i = $i - 1; $j = $j + 1)",
            Value::Int(3)
        );
        // nested loops
        assert_eval_ok!(
            "i = 5; while ($i > 0) (j = $i; $i = $i - 1; k = $j; while ($j > 1) ($j = $j - 1); $k)",
            Value::Int(1)
        );
    }

    #[test]
    fn test_while_no_group() {
        assert_eval_err!(
            "while (1) hello",
            "Parentheses are required around WHILE body"
        )
    }

    #[test]
    fn test_var_subst() {
        assert_eval_ok!(
            "TEST=/tmp/foobar/baz/bam; $TEST",
            Value::from_str("/tmp/foobar/baz/bam").unwrap()
        );
        assert_eval_ok!(
            "TEST=/tmp/foobar/baz/bam; ${TEST}",
            Value::from_str("/tmp/foobar/baz/bam").unwrap()
        );
        assert_eval_ok!(
            "TEST=/tmp/foobar/baz/bam; aaa${TEST}bbb",
            Value::from_str("aaa/tmp/foobar/baz/bambbb").unwrap()
        );
        assert_eval_ok!(
            "TEST=/tmp/foobar/baz/bam; aaa${TEST/.a/}",
            Value::from_str("aaa/tmp/foor/z/m").unwrap()
        );
        assert_eval_ok!(
            "TEST=\"/tmp/f  bar/baz/bam\"; \"${TEST/ +/_}\"",
            Value::from_str("/tmp/f_bar/baz/bam").unwrap()
        );
        assert_eval_ok!(
            "TEST=/tmp/foobar.txt; \"${TEST/(.txt)/\\\\1.tmp}\"",
            Value::from_str("/tmp/foobar.txt.tmp").unwrap()
        );

        assert_eval_ok!(
            "NAME=\"John Doe\"; \"${NAME}\"",
            Value::from_str("John Doe").unwrap()
        );
        assert_eval_ok!(
            "GREETING=\"Hello, World!\"; \"$GREETING\"",
            Value::from_str("Hello, World!").unwrap()
        );
        assert_eval_ok!(
            "NAME=\"John Doe\"; \"${NAME/John/Jane}\"",
            Value::from_str("Jane Doe").unwrap()
        );
        assert_eval_ok!(
            "GREETING=\"Hello, World!\"; \"${GREETING/World/Universe}\"",
            Value::from_str("Hello, Universe!").unwrap()
        );
        assert_eval_ok!(
            "NAME=\"John Doe\"; \"${NAME/[aeiou]/X}\"",
            Value::from_str("JXhn DXX").unwrap()
        );
        assert_eval_ok!(
            "NAME=\"John Doe\"; \"${NAME/(\\\\w+) (\\\\w+)/\\\\2, \\\\1}\"",
            Value::from_str("Doe, John").unwrap()
        );
        assert_eval_ok!(
            "GREETING=\"Hello, World!\"; \"${GREETING/(Hello), (World)!/\\\\2 says \\\\1}\"",
            Value::from_str("World says Hello").unwrap()
        );
        assert_eval_ok!(
            "\"${UNDEFINED_VAR}\"",
            Value::from_str("${UNDEFINED_VAR}").unwrap()
        );
        assert_eval_ok!(
            "\"${UNDEFINED_VAR/foo/bar}\"",
            Value::from_str("${UNDEFINED_VAR/foo/bar}").unwrap()
        );
    }

    #[test]
    fn test_command_error_handling() {
        assert_eval_err!("cp", "Missing source and destination");
        assert_eval_ok!("if (cp)()", Value::Int(0));
        assert_eval_ok!("if (cp)() else (-1)", Value::Int(-1));
        assert_eval_ok!("if ((cp))()", Value::Int(0));
        assert_eval_ok!("if (!(cp))(123)", Value::Int(123));
        assert_eval_ok!("if ((echo Hello; cp x))() else (-1)", Value::Int(-1));
        assert_eval_err!(
            "if (cp; echo Ok)() else ()",
            "Missing source and destination"
        );
        assert_eval_ok!("if (cp)() else (fail)", Value::from_str("fail").unwrap());
        assert_eval_err!("for i in (cp); ()", "Missing source and destination");
        assert_eval_err!("for i in (cp); (echo $i)", "Missing source and destination");
        assert_eval_err!("for i in (cp); ()", "Missing source and destination");
        assert_eval_err!(
            "for i in (cp; foo); (echo $i)",
            "Missing source and destination"
        );
        assert_eval_ok!("for i in (if(cp)(); foo); (echo $i)", Value::Int(0));
        assert_eval_err!("while (1) (cp x; break)", "Missing destination");
        assert_eval_ok!("while (1) (if (cp)() else (-1); break)", Value::Int(-1));
    }

    #[test]
    fn test_mul() {
        assert_eval_err!("x = 2; y = 3; x * y", "Cannot multiply strings");
        assert_eval_ok!("x = 2; y = 3; $x * $y", Value::Int(6));
    }

    #[test]
    fn test_error() {
        assert_eval_ok!(
            "if (echo Hello && cp x) () else ($__errors)",
            Value::from_str("cp x: Missing destination").unwrap()
        );
        assert_eval_ok!(
            "if (!(0 || cp -x || cp)) ($__errors)",
            Value::from_str("cp -x: Unknown flag: -x\ncp: Missing source and destination").unwrap()
        );
    }
}
