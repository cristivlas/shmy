# A Command Line Interpreter in Rust
[![Rust CI](https://github.com/cristivlas/shmy/actions/workflows/rust.yml/badge.svg)](https://github.com/cristivlas/shmy/actions/workflows/rust.yml)

## Introduction

A lightweight command line interpreter implemented in Rust, featuring Unix-like built-in commands.

Project purpose and goals:

1. An exercise in Rust programming.
2. Provide am alternative, light-weight solution for executing Unix-like commands in a Windows environment.
3. Experiment with design ideas that enforce robust error handling practices in shell scripting.

The interpreter operates in both interactive mode and script execution mode, with the latter activated by passing script files as command line arguments. In interactive mode, the interpreter leverages rustyline to provide history functionality and TAB completion.

## Key Features

1. WSL (Windows Subsystem for Linux) symbolic link support (read, traversal).
2. Strict enforcement of command result error checking.
3. Dual-mode operation: interactive and script execution.

## Command-Line Autocompletion Notes

This shell provides autocompletion when the `TAB` key is pressed.

### 1. **History Expansion**

- **When**: The input starts with `!` followed by some text.
- **Function**: Autocompletes entries from the command history that match the provided text.
- **Example**:
  - **Input**: `!git sta`
  - **Completion on TAB**: `git status` (if `git status` is in the history)

### 2. **Environment Variable Expansion**

- **When**: The input includes `$` followed by a partial environment variable name.
- **Function**: Autocompletes environment variable names but **does not** substitute their values.
- **Examples**:
  - **Input**: `$HO`
    - **Completion on TAB**: `$HOMEDRIVE  $HOMEPATH  $HOME`
  - **Input**: `cat $HIST`
    - **Completion on TAB**: `cat $HISTORY`

> The autocompletion provides matching environment variables, but does **not** replace them with their actual values.

### 3. **Custom Command Completions**

- **When**: Other completions do not apply.
- **Function**: Uses a custom YAML configuration file to autocomplete commands, subcommands, and options.
- **Setup Instructions**:
  - **Add a File**: Create a file named `completions.yaml` in `~\.shmy\`.
  - **Edit the File**: Define commands, subcommands, and options in YAML format. Example:
```yaml
  commands:
  - name: cargo
    subcommands:
      - name: build
        options:
          - --release
          - --verbose
          - --target
          - --features
          - --jobs
          - --example
      - name: test
        options:
          - --release
          - --verbose
  - name: git
    subcommands:
      - name: commit
        options:
          - --amend
          - --message
      - name: clone
        options:
          - --recursive
      - name: diff
        options:
```
### 4. **File Completion**

- **When**: Other completions do not apply, and the input involves file paths.
- **Function**: Completes file names using built-in file completion features.

## Expression Evaluation and Language Features

### 1. Variable Assignment and Evaluation

Variables can be assigned and utilized within commands:

```shell
i = 5; echo $i
```

#### Variable Scope

Variable scope is constrained to blocks defined by parentheses `( )`. When a variable `$VAR` is not found in the current scope, the interpreter recursively searches parent (enclosing) scopes up to the global level. The global level "shadows" the environment variables.

Variable names are case insensitive (but case-preserving) in Windows.

### 2. Control Structures

#### a) Conditional Statements

The interpreter supports if/else constructs with the following syntax:

```shell
if ( <condition> ) ( <body> ) else ( <body> )
```

Example:
```shell
i = 3; if ($i > 0) (echo "i is positive") else (echo "i is not positive")
```

#### b) Loops

The interpreter implements both while and for loops:

**While Loop:**
```shell
while ( <condition> ) ( <body> )
```

Example:
```shell
i = 3;
while ($i > 0) (echo $i; $i = $i - 1)
```

**For Loop:**
```shell
for VAR in <list>; ( <body> )
```

Example:
```shell
for f in *.rs; ( echo $f; ls -l $f )
```

#### c) Arithmetic Operations

Supported arithmetic operations include: `+`, `-`, `/`, `*`, `//`, `%`

Note: The `//` operator performs integer division, and `%` is the modulo operator.

#### d) Logical Operations

Supported logical operations: `||`, `&&`

### 3. Command Execution Evaluation

- Commands evaluate to true upon successful execution (exit code 0) and false otherwise.
- Errors from failed commands are stored in the special variable `$__errors` if their status was evaluated.
- If a command fails and its status is not evaluated or assigned, execution halts and an error is reported.

Example of error handling:
```shell
if (cp source/path dest/path) (echo Ok) else (echo $__errors)
```

### 4. Input/Output Redirection and Pipes

#### Redirects

The interpreter uses the following operators for redirects:
- Output to file: `command => file`
- Append to file: `command =>> file`

Note: The redirect operators '=>', '=>>' differ from '>' and '>>' used by other shells. This design choice avoids complications with inferring the meaning of '>' (which could be interpreted as greater-than) based on context.

#### Pipes

Output can be piped between commands using the following syntax:
```shell
<expression> | <expression>
```

Example:
```shell
ls -al | (echo "\t\tHEADER"; cat; echo "\t\tFOOTER")
```

Example of using pipe operator for variable assignment:
```shell
realpath .. | x; basename $x
```

##### Limitations
Output and redirects are not allowed with 'sudo', because on Windows the semantics of 'sudo' is not
"substitute-user-do", but "runas Administrator" (UAC).
```
ls -al | sudo cat
```
The above expression does not work on Windows under this shell. Workaround:

```
sudo $SHELL
```
Then evaluate any expression, including pipes, in the elevated shell window.

### 5. Special Variables

The interpreter provides special variables for output redirection:
- Redirect stdout: `$__stdout`
- Redirect stderr: `$__stderr`

Examples:
```shell
__stderr=null; ls;
__stderr=log.txt; ls -al;
__stderr=__stdout; ls -al /
__stdout=some/path/file.txt ls -al;
```

#### Color.
Some commands have default color output (ls, grep).
The NO_COLOR variable, if defined (value does not matter) in the environment or the current evaluation
scope, suppresses color output.
E.g.
```
NO_COLOR = _; ls -al
```

### 6. Variable Parsing and Expansion

This section details the parsing and expansion of shell-like variable expressions within a given string.

- **Grouping and Escaping:** Groups in substitution must be enclosed in quotes, and captures require double escaping.
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

### 7. Escaping and Globbing

```shell
# Count lines in the current project
find src .*rs | srcs; echo "${srcs/\n/ }" | args; wc -l $args;

# Note: There is a subtle issue in the line above related to the implementation of the 'wc' command:
# If no arguments are provided, wc will read from stdin. Consequently, if no files ending in 'rs' are found, wc will
# wait for user input.

# A workaround for this issue:
find src ".*rs" | srcs; echo "${srcs/\n/ }" | args; if ($args) (wc -l $args);

# A more precise regular expression for the find command argument would be ".*\\.rs"
# The first backslash escapes the subsequent one, resulting in the Rust Regex being compiled as: .*\.rs
# Note that in this interpreter, backslashes only function as escape characters inside quotes. This design choice
# avoids complications with TAB-completion of Windows file paths in interactive mode.

find src ".*\\.rs" | src; if ($src) (wc -l $src)
```

Without quotes, backslashes are passed verbatim to the tokenizer, which attempts globbing.
If the pattern is successfully globbed, the expanded tokens are passed to the expression evaluator;
otherwise, the pattern is treated as a literal. Note the differences in outputs below:

```shell
C:~\Projects\shmy> for f in src\*.rs; (echo $f)
src\cmds.rs
src\eval.rs
src\macros.rs
src\main.rs
src\prompt.rs
src\testeval.rs
src\utils.rs
C:~\Projects\shmy> for f in "src\*.rs"; (echo $f)
src*.rs
C:~\Projects\shmy> for f in "src\\*.rs"; (echo $f)
src\*.rs
```

### 8. Raw Strings

The interpreter supports raw strings to avoid escaping:

```shell
echo r"(This is a "raw string")"
```

### 9. Export and Source

The interpreter implements rough equivalents of bash 'eval', 'export', and 'source' commands via the eval command,
which supports --export and --source command line options.

Examples:
```shell
eval --export r"(__prompt = "CuStOm\ Pr0mpT \\u@\\h:\\w\\$ ")"
```

```shell
eval --source examples/activate.my
```

```shell
# examples/activate.my
# Activate / deactivate Python virtual environment.
# From within the interactive shell, type:
# eval --source activate.my

# Run again to deactivate the virtual env (i.e., restore saved environment vars.)

if (defined __OLD_PATH) (
    # Restore (deactivate)

    # Use raw string to prevent $__OLD_PATH expansion before being passed to
    # the eval command (and pass backslash path delimiters through as-is)
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

## Implementation Considerations and Potential Issues

### Variable Expansion in Arithmetic

The expression `2*3` evaluates to `6`, but `x=2; y=3; $x*$y` evaluates to `2*3`. This behavior stems from the interpreter's attempt to determine the contextual meaning of `/` and `*`, which can function as path delimiters, glob wildcards, or arithmetic operators. This distinction is made at parsing time, while variable assignment occurs at evaluation time. When using spaces, as in ```$x * $y```, the evaluation proceeds as expected.

### Operation Precedence

The expression ```echo 2 + 2``` is evaluated as ```(echo 2) + 2```, due to the low precedence of the addition operator. It is recommended to always use parentheses, as in ```echo (2 + 2)```, to ensure correct evaluation.

For more detailed information or specific use cases, refer to the source code and examples in the repo at: https://github.com/cristivlas/shmy
