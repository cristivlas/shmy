#[cfg(test)]
pub mod tests {
    use crate::eval::*;
    use std::sync::{Mutex, Once};
    use std::{io, str::FromStr};

    // Initialize a global Mutex to synchronize access
    static INIT: Once = Once::new();
    static mut INTERP_MUTEX: Option<Mutex<()>> = None;

    pub fn eval(input: &str) -> EvalResult<Value> {
        // Initialize the Mutex once
        unsafe {
            INIT.call_once(|| {
                INTERP_MUTEX = Some(Mutex::new(()));
            });
        }

        // Lock the mutex to ensure exclusive access
        let _guard = unsafe { INTERP_MUTEX.as_ref().unwrap().lock().unwrap() };

        // Workaround for cargo test using stdout redirection
        let _stdout = io::stdout().lock();

        let mut interp = Interp::with_env_vars();
        interp
            .global_scope()
            .insert("NO_COLOR".to_string(), Value::Int(1));

        // Debugging
        // interp.global_scope().insert("__dump_ast".into(), Value::Int(1));

        interp.eval_status(input, None)
    }

    #[macro_export]
    macro_rules! assert_eval_cmd_ok {
        ($expr:expr) => {{
            let result = eval($expr);
            assert!(matches!(result, Ok(Value::Stat(_))));
            if let Ok(Value::Stat(stat)) = result {
                assert!(!stat.is_err());
            }
        }};
    }

    #[macro_export]
    macro_rules! assert_eval_ok {
        ($expr:expr, $val:expr) => {{
            assert!(matches!(eval($expr), Ok(ref v) if *v == $val));
        }}
    }

    #[macro_export]
    macro_rules! assert_eval_err {
        ($expr:literal, $message:literal) => {{
            match eval($expr) {
                Err(EvalError {
                    message: ref msg, ..
                }) => {
                    assert_eq!(msg, $message);
                }
                Ok(_) => panic!("Expected an error for expression '{}', but got Ok", $expr),
            }
        }};
    }

    #[test]
    fn test_add() {
        assert_eval_ok!("i = 2; $i + 1", Value::Int(3));
        assert_eval_ok!("hello + 0", Value::from_str("hello0").unwrap());
        assert_eval_ok!(
            "hello + \" world!\"",
            Value::from_str("hello world!").unwrap()
        );
        assert_eval_err!("1 + (echo)", "Cannot add number and command status");
        assert_eval_err!("(echo) + 1", "Cannot add to command status");
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
        assert_eval_ok!("if (42) (My_True) else (My_False);", Value::from("My_True"));
        assert_eval_ok!(
            "TEST_VAR = 1; if ($TEST_VAR) (My_True) else (My_False);",
            Value::from("My_True")
        );
        assert_eval_ok!(
            "TEST_VAR = 0; if ($TEST_VAR == 0) (My_True) else (My_False);",
            Value::from("My_True")
        );
        assert_eval_ok!(
            "TEST_VAR = 1; if ($TEST_VAR > 0) (My_True) else (My_False);",
            Value::from("My_True")
        );
        assert_eval_ok!(
            "TEST_VAR = 1; if ($TEST_VAR >= 0) (My_True) else (My_False);",
            Value::from("My_True")
        );
        assert_eval_ok!(
            "TEST_VAR = -1; if ($TEST_VAR < 0) (My_True) else (My_False);",
            Value::from("My_True")
        );
        assert_eval_ok!(
            "TEST_VAR = -1; if ($TEST_VAR <= 0) (My_True) else (My_False);",
            Value::from("My_True")
        );
    }

    #[test]
    fn test_if_no_group() {
        assert_eval_err!(
            "i = 1; if $i true",
            "Parentheses are required around IF body"
        )
    }

    #[test]
    fn test_else() {
        assert_eval_ok!(
            "i = 1; if ($i < 0) (Apple) else (Orange)",
            Value::from("Orange")
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
            "Parentheses are required around ELSE body"
        )
    }

    #[test]
    fn test_for() {
        assert_eval_ok!(
            "i = \"\"; for j in a b c d; ($i = $i + $j);",
            Value::from("abcd")
        );
    }

    #[test]
    fn test_for_tilde() {
        let mut interp = Interp::with_env_vars();
        interp
            .global_scope()
            .insert("HOME".to_string(), Value::from("abc"));
        let result = interp.eval_status("for i in ~/foo; ($i)", None);
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
    fn test_for_with_expr_args() {
        assert_eval_ok!(
            "acc = \"\"; x = 3; for i in x ($x + 2) (2 - $x * 2) y; ($acc = $acc + _ + $i)",
            "_x_5_-4_y".parse::<Value>().unwrap()
        );
    }

    #[test]
    fn test_for_slash() {
        assert_eval_ok!("for i in /; ($i)", "/".parse::<Value>().unwrap());
    }

    // #[test]
    // fn test_for_pipe() {
    //     assert_eval_ok!("echo 123 | for x in -; (echo $x) | y; $y", Value::Int(123));
    // }

    #[test]
    fn test_break_for() {
        assert_eval_ok!(
            "for i in 0 1 2 3 4 5 6 8 9; ($i; if ($i >= 5) (break))",
            Value::Int(5)
        );
    }

    #[test]
    fn test_continue_for() {
        assert_eval_ok!(
            "for i in 0 1 2 3 4 5 6 8 9; (echo $i; if ($i < 5) (continue); $i)",
            Value::Int(9)
        );
    }

    #[test]
    fn test_break_while() {
        assert_eval_ok!(
            "i = 0; while ($i < 10) ($i = $i + 1; if ($i >= 5) (break))",
            Value::Int(5)
        );
    }

    #[test]
    fn test_continue_while() {
        assert_eval_ok!("i = 0; j = 0; while ($i < 10) ($i = $i + 1; if ($i > 5) (continue); $j = $j + 1); $i - $j", Value::Int(5));
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
            "TESTVAR=/tmp/foobar/baz/bam; $TESTVAR",
            Value::from("/tmp/foobar/baz/bam")
        );
        assert_eval_ok!(
            "TESTVAR=/tmp/foobar/baz/bam; ${TESTVAR}",
            Value::from("/tmp/foobar/baz/bam")
        );
        assert_eval_ok!(
            "TESTVAR=/tmp/foobar/baz/bam; aaa${TESTVAR}bbb",
            Value::from("aaa/tmp/foobar/baz/bambbb")
        );
        assert_eval_ok!(
            "TESTVAR=/tmp/foobar/baz/bam; aaa${TESTVAR/.a/}",
            Value::from("aaa/tmp/foor/z/m")
        );
        assert_eval_ok!(
            "TESTVAR=\"/tmp/f  bar/baz/bam\"; \"${TESTVAR/ +/_}\"",
            Value::from("/tmp/f_bar/baz/bam")
        );
        assert_eval_ok!(
            "TESTVAR=/tmp/foobar.txt; \"${TESTVAR/(.txt)/\\\\1.tmp}\"",
            Value::from("/tmp/foobar.txt.tmp")
        );

        assert_eval_ok!("NAME=\"John Doe\"; \"${NAME}\"", Value::from("John Doe"));
        assert_eval_ok!(
            "GREETING=\"Hello, World!\"; \"$GREETING\"",
            Value::from("Hello, World!")
        );
        assert_eval_ok!(
            "NAME=\"John Doe\"; \"${NAME/John/Jane}\"",
            Value::from("Jane Doe")
        );
        assert_eval_ok!(
            "GREETING=\"Hello, World!\"; \"${GREETING/World/Universe}\"",
            Value::from("Hello, Universe!")
        );
        assert_eval_ok!(
            "NAME=\"John Doe\"; \"${NAME/[aeiou]/X}\"",
            Value::from("JXhn DXX")
        );
        assert_eval_ok!(
            "NAME=\"John Doe\"; \"${NAME/(\\\\w+) (\\\\w+)/\\\\2, \\\\1}\"",
            Value::from("Doe, John")
        );
        assert_eval_ok!(
            "GREETING=\"Hello, World!\"; \"${GREETING/(Hello), (World)!/\\\\2 says \\\\1}\"",
            Value::from("World says Hello")
        );
        assert_eval_ok!("\"${UNDEFINED_VAR}\"", Value::from("$UNDEFINED_VAR"));
        assert_eval_ok!(
            "\"${UNDEFINED_VAR/foo/bar}\"",
            Value::from("$UNDEFINED_VAR")
        );
        assert_eval_ok!("$UNDEFINED", Value::from("$UNDEFINED"));

        assert_eval_ok!(
            "foo=\"blah blah\"; bar = hu; ${foo/bla/$bar}",
            Value::from("huh huh")
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
        assert_eval_ok!("if (cp)() else (fail)", Value::from("fail"));
        assert_eval_cmd_ok!("for i in (if(cp)(); foo); (echo $i)");
        assert_eval_err!("while (1) (cp x; break)", "Missing destination");
        assert_eval_ok!("while (1) (if (cp)() else (-1); break)", Value::Int(-1));
    }

    #[test]
    fn test_status_as_arg() {
        assert_eval_err!("for i in (cp); ()", "Missing source and destination");
        assert_eval_err!("for i in (cp); (echo $i)", "Missing source and destination");
        assert_eval_err!("for i in (cp --bug); ()", "Unknown flag: --bug");
        assert_eval_err!(
            "for i in (echo ok) foo; (echo $i)",
            "Command status argument is not allowed"
        );
    }

    #[test]
    fn test_mul() {
        assert_eval_err!("x = 2; y = 3; x * y", "Cannot multiply strings");
        assert_eval_ok!("x = 2; y = 3; $x * $y", Value::Int(6));

        assert_eval_err!("x * 2", "Cannot multiply string by number");
        assert_eval_err!("2 * x", "Cannot multiply number by string");
        assert_eval_err!("(echo) * 2", "Cannot multiply command statuses");
    }

    #[test]
    fn test_arithmetic() {
        assert_eval_ok!("2+2", Value::Int(4));
        assert_eval_ok!("1 - 2 * 2 + 3", Value::Int(0));
    }

    #[test]
    fn test_error() {
        assert_eval_ok!(
            "if (echo Hello && cp x) () else ($__errors)",
            Value::from("cp x: Missing destination")
        );
        assert_eval_ok!(
            "if (!(0 || cp -x || cp)) ($__errors)",
            Value::from("cp -x: Unknown flag: -x\ncp: Missing source and destination")
        );
    }

    #[test]
    fn test_erase() {
        assert_eval_ok!("x = 123; $x = ", Value::Int(123));
        assert_eval_err!("x = 123; $x = ; $x = 0", "Variable not found: $x");
    }

    #[test]
    fn test_logical_or_error() {
        assert_eval_ok!(
            "(basename . || echo $__errors) | x; $x",
            Value::from("basename .: Failed to get file name")
        );
    }

    #[test]
    fn test_pipeline_rewrite() {
        assert_eval_ok!(
            "echo World | (echo Hello; cat) | cat | x; $x",
            Value::from("Hello\nWorld")
        );
    }

    #[test]
    fn test_power() {
        assert_eval_ok!("2 ^ 14", Value::Int(16384));
        assert_eval_ok!("2 ^ (-2)", Value::Real(0.25));
        assert_eval_err!("x ^ 10", "Invalid base type");
        assert_eval_err!("10 ^ x", "Exponent cannot be a string");
        assert_eval_err!("(echo) ^ x", "Invalid base type");
        assert_eval_err!("2 ^ (echo) ^ x", "Exponent cannot be a command status");
    }

    #[test]
    fn test_sub() {
        assert_eval_ok!("10000 - 2 ^ 14", Value::Int(-6384));
        assert_eval_ok!("1 - 2 * 2 - 1", Value::Int(-4));
        assert_eval_err!(
            "x - y",
            "Cannot subtract strings, x is not a recognized command"
        );
        assert_eval_err!("0 - y", "Cannot subtract string from number");
        assert_eval_err!("x - 2", "Cannot subtract number from string");
        assert_eval_err!("1 - (echo)", "Cannot subtract command status from number");
        assert_eval_err!("(echo) -1", "Cannot subtract from command status");
        assert_eval_err!("100 - (echo)", "Cannot subtract command status from number");
        assert_eval_err!(
            "hello - (echo)",
            "Cannot subtract command status from string"
        );
    }

    #[test]
    fn test_status_and() {
        assert_eval_err!("(echo Hello && cp x && ls .)", "Missing destination");
    }

    #[test]
    fn test_status_or() {
        // Expect the error of the last of any that failed
        assert_eval_err!("(0 || cp -x || cp)", "Missing source and destination");

        assert_eval_ok!(
            "if (0 || cp -x || cp) (ok) else ($__errors)",
            Value::from("cp -x: Unknown flag: -x\ncp: Missing source and destination")
        );
    }

    #[test]
    fn test_negated_status() {
        assert_eval_ok!(
            "if (!(0 || cp -x || cp)) ($__errors)",
            Value::from("cp -x: Unknown flag: -x\ncp: Missing source and destination")
        );
    }

    #[test]
    fn test_dash_parse() {
        assert_eval_ok!("echo ---Hello--- | x; $x", Value::from("---Hello---"));
    }

    #[test]
    fn test_pass_vars_thru_pipes() {
        assert_eval_ok!("i = 2; echo hello | echo $i | x; $x", Value::Int(2));
    }

    #[test]
    fn test_hash_tag() {
        assert_eval_ok!("x = hey#world; $x", Value::from("hey"));
        assert_eval_ok!("x = \"hey#world\"; $x", Value::from("hey#world"));
    }

    #[test]
    fn test_raw_strings() {
        assert_eval_ok!("r\"(_;)( \" )\"", Value::from("_;)( \" "));
    }

    #[test]
    fn test_trailing_equals() {
        assert_eval_err!("FOO=", "Variable expected on left hand-side of assignment");
    }

    #[test]
    fn test_export() {
        assert_eval_ok!("eval --export \"FOO=123\"; $FOO", Value::Int(123));
        // Expect value to be preserved across evals.
        assert_eval_ok!("$FOO", Value::Int(123));
        // Expect to find it in the environment
        assert_eval_ok!("env | grep FOO | bar; $bar", Value::from("FOO=123"));
        // Erase it
        assert_eval_ok!("$FOO =", Value::Int(123));
        // Should be gone from the env.
        assert_eval_ok!("env | grep FOO | bar; $bar", Value::from(""));
        // Should not be found (not expanded)
        assert_eval_ok!("$FOO", Value::from("$FOO"));
    }

    #[test]
    fn test_escape_unicode() {
        assert_eval_ok!("\"\\u{1b}\"", Value::from("\x1b"));
    }

    #[test]
    fn test_invalid_escapes() {
        assert_eval_err!("\"\\u{1b\"", "Invalid unicode escape sequence");
        assert_eval_err!("\"\\uac\"", "Invalid unicode escape sequence");
        assert_eval_err!("\"\\u{x}\"", "Invalid unicode escape sequence");
        assert_eval_err!("\"\\xyz\"", "Invalid hex escape sequence");
        assert_eval_err!("\"\\xabc", "Unbalanced quotes");
    }
}
