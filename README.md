# A Command Line Interpreter in Rust

This is a simple, lightweight command line interpreter with a few Unix-like built-in commands, that I wrote to familiarize myself with Rust, and to deal with those odd moments when the muscle memory goes for the quick Unix command only to realize that I am on Windows; (WSL is a solution of course, but the file system is under /mnt/c, and everything that's relative to $HOME is different from the native environment). And yeah, I wanted to see what's the deal with this rusty thing all the cool kids are so excited about.

I also wanted to address the bad habit of writing quick-and-dirty scripts in which I execute a bunch of commands but "forget" to handle the errors. In this command interpreter, when a command fails and its status is not checked with an IF expression, the script stops and the error is reported, like an unhandled exception - sort of.

The interpreter works in interactive mode or can consume script files passed in the command line. In interactive mode history and TAB expansion are supported via rustyline.

## Expression Evaluation Random Notes

### 1. Variable Assignment and Evaluation
You can assign variables and use them in commands:
```shell
i = 5; echo $i
```
#### Variable Scope
Variable scope is limited to the `( )` block. If a variable `$VAR` is not found in the current scope, the interpreter searches the parent (enclosing) scope and continues up to the top.

### 2. Expressions
#### a) Conditional Statements
Use the following structure for if/else:
```shell
if ( <condition> ) ( <body> ) else ( <body> )
```
##### Example:
```shell
i = 3; if ($i > 0) (echo "i is positive") else (echo "i is not positive")
```

#### b) Loops
**While Loop:**
```shell
while ( <condition> ) ( <body> )
```
##### Example:
```shell
i = 3;
while ($i > 0) (echo $i; $i = $i - 1)
```

**For Loop:**
```shell
for VAR in <list>; ( <body> )
```
```shell
for f in *.rs; ( echo $f; ls -l $f )
```

#### c) Arithmetic Operations
Supported operations: `+`, `-`, `/`, `*`

#### d) Logical Operations
Supported operations: `||`, `&&`

### 3. Command Execution Evaluation
- A command evaluates to true if it succeeds (exit code 0) and false otherwise.
- Errors from failed commands are stored in the special variable `$__errors` if their status was evaluated.
- If a command fails and its status is not evaluated or assigned, execution stops.

#### Example Error Handling:
```shell
if (cp source/path dest/path) (echo Ok) else (echo $__errors)
```

### 4. Redirects
Use the following operators for redirects:
- Output to file: `command => file`
- Append to file: `command =>> file`

### 5. Pipes
Pipe output between commands:
```shell
<expression> | <expression>
```
##### Example:
```shell
ls -al | (echo "\t\tHEADER"; cat; echo "\t\tFOOTER")
```
Example for using pipe operator for variable assignment:
```shell
realpath .. | x; basename $x
```

### 6. Special Variables
Use the following special variables for redirecting output:
- Redirect stdout: `$__stdout`
- Redirect stderr: `$__stderr`

##### Examples:
```shell
__stderr=null; ls;
__stderr=log.txt; ls -al;
__stderr=__stdout; ls -al /
__stdout=some/path/file.txt ls -al;
```

### 7. Gotchas and Pitfalls
- **Variable Expansion in Arithmetic:**
  - `2*3` evaluates to `6`, but `x=2; y=3; $x*$y` evaluates to `2*3`. This is because the interpreter tries to determine the meaning of `/` and `*` from the context; they can act as path delimiters, glob wildcards, or arithmetic operators. This distinction is made at parsing time, while variable assignment occurs at evaluation time. When using spaces like this: ```$x * $y``` the evaluation works as expected.

- **Operation Precedence:**
  - ```echo 2 + 2``` is evaluated as ```(echo 2) + 2```, because the low priority of the addition operator. It is recommended to always use parentheses as in ```echo (2 + 2)```. 

### 8. Variable Parsing and Expansion
This section details the parsing and expanding of shell-like variable expressions in a given string.

- **Grouping and Escaping:** Groups in substitution must be enclosed in quotes, and captures need to be double escaped.
- **Examples of Variable Expansion:**
  - Basic variable expansion:
    ```shell
    "${NAME}"         -> "John Doe"
    "$GREETING"       -> "Hello, World!"
    ```
  - Variable substitution:
    ```shell
    "${NAME/John/Jane}"            -> "Jane Doe"
    "${GREETING/World/Universe}"   -> "Hello, Universe!"
    ```
  - Capture groups in substitution:
    ```shell
    "${NAME/(\\w+) (\\w+)/\\2, \\1}"   -> "Doe, John"
    "${GREETING/(Hello), (World)!/\\2 says \\1}" -> "World says Hello"
    ```
  - Handling non-existent variables:
    ```shell
    "${UNDEFINED_VAR}"             -> "$UNDEFINED_VAR"
    "${UNDEFINED_VAR/foo/bar}"     -> "$UNDEFINED_VAR"
    ```

### 9. Escaping and Globbing
```
# Count lines in the current project
find src .*rs | srcs; echo "${srcs/\n/ }" | args; wc -l $args;

# There is a subtle "bug" in the line above, having to do with the implementation of the 'wc' command:
# if no arguments are given, wc will read from stdin. So, if no files ending in 'rs' are found, wc will
# wait for user input.

# The workaround is:
find src ".*rs" | srcs; echo "${srcs/\n/ }" | args; if ($args) (wc -l $args);

# And, the more correct regular expression in the find command argument should be ".*\\.rs"
# The first backslash escapes the next one so the Rust Regex being compiled is: .*\.rs
# NOTE that in mysh backslashes only work as such inside quotes. This is to avoid complications
# with TAB-completion of Windows file paths in interactive mode.

find src ".*\\.rs" | src; if ($src) (wc -l $src)
```

Without quotes the backslashes are passed verbatim to the tokenizer, which will attempt globbing.
If the pattern is globbed successfully, the expanded tokens are passed to the expression evaluator,
otherwise the pattern is treated as a literal. Note the difference in outputs bellow:
```
C:\Users\crist\Projects\rust\mysh> for f in src\*.rs; (echo $f)
src\cmds.rs
src\eval.rs
src\macros.rs
src\main.rs
src\prompt.rs
src\testeval.rs
src\utils.rs
C:\Users\crist\Projects\rust\mysh> for f in "src\*.rs"; (echo $f)
src*.rs
C:\Users\crist\Projects\rust\mysh> for f in "src\\*.rs"; (echo $f)
src\*.rs
```

### 10. Raw Strings
```
echo r"(This is a "raw string")"
```

### 11. Export and Source
The rough equivalents of bash 'eval', 'export' and 'source' are implemented in the eval command,
that supports --export and --source command line options.

Examples:
```
eval --export r"(__prompt = "CuStOm\ Pr0mpT \\u@\\h:\\w\\$ ")"
```

```
eval --source examples/activate.my
```
```
# examples/activate.my
# Activate / deactivate Python virtual environment.
# From within the interactive shell, type:
# eval --source activate.my

# Run again to deactivate the virtual env (i.e. restore saved environment vars.)

if (defined __OLD_PATH) (
    # Restore (deactivate)

    # Use raw string to prevent $__OLD_PATH expansion before being passed to
    # the eval command (and pass backslash path delimiters through as-in)
    eval --export r"(PATH = $__OLD_PATH)";

    # Erase variables that are no longer needed
    eval --export r"($VIRTUAL_ENV = )";
    eval --export r"($__OLD_PATH = )";

) else (
    # Activate

    eval --export "VIRTUAL_ENV = C:\\Users\\crist\\Projects\\venv312-chess";
    eval --export r"(__OLD_PATH = $PATH)";

    # Use raw string r"(...)" to prevent the semicolon from being interpreted as
    # end of statement, and to pass variable names directly to the eval command,
    # without expanding.
    eval --export r"(PATH = "${VIRTUAL_ENV}\\Scripts\;$PATH")";

    echo "VIRTUAL_ENV=${VIRTUAL_ENV}";
);
```
