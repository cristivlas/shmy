# My First Rust Project

This is a simple, lightweight command line interpreter with Unix-like built-in commands, created for familiarizing myself with Rust.

The interpreter works in interactive mode or can consume script files passed in the command line.
In interactive mode history and TAB expansion are supported via rustyline.

## Command Interpreter Notes

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
__stderr=$__stdout; ls -al /
__stdout=some/path/file.txt ls -al;
```

### 7. Gotchas and Pitfalls
- **Variable Expansion in Arithmetic:**
  - `2*3` evaluates to `6`, but `x=2; y=3; $x*$y` evaluates to `2*3`. This is because the interpreter tries to determine the meaning of `/` and `*` from the context; they can act as path delimiters, glob wildcards, or arithmetic operators. This distinction is made at parsing time, while variable assignment occurs at evaluation time.
When using spaces like this: ```$x * $y``` the evaluation works as expected.

### 8. Variable Parsing and Expansion
This section details the parsing and expanding of shell-like variable expressions in a given string.

### Notes:
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
    "${UNDEFINED_VAR}"             -> ""
    "${UNDEFINED_VAR/foo/bar}"     -> ""
    ```
