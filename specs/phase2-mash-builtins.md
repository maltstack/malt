# Phase 2: `mash` Sub-Project 5 — POSIX Core Builtins

## Goal

Implement 15 POSIX core builtins needed for conformance suite compliance: export, unset, readonly, cd, pwd, read, printf, test/[, trap, type, hash, alias/unalias, command, getopts, umask.

## Architecture

All builtins added to the existing `try_execute_builtin()` inline match in `executor.rs`. No new files — follows the pattern established for echo, true, false, exit, etc. Each builtin is a match arm that takes `argv: &[String]` and `env: &mut Env`, returns `Option<ExecResult>`.

## Reference

`C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\builtins.rs` (5,820 lines). Port the POSIX logic for each builtin.

---

## Already Implemented (in executor)

echo, true, false, :, exit, return, break, continue, eval, local, set, shift, source

---

## Builtins to Add

### Variable Builtins

**`export`**
- `export VAR=value` — set variable and mark exported
- `export VAR` — mark existing variable exported (or create empty exported)
- `export -p` — print all exported variables as `export VAR=value`
- `export -n VAR` — remove export flag
- Multiple args: `export A=1 B=2 C`

**`unset`**
- `unset VAR` — unset variable (default)
- `unset -v VAR` — explicitly unset variable
- `unset -f NAME` — unset function
- Returns error if readonly
- Multiple args: `unset A B C`

**`readonly`**
- `readonly VAR=value` — set and mark readonly
- `readonly VAR` — mark existing readonly
- `readonly -p` — print all readonly variables as `readonly VAR=value`
- Multiple args: `readonly A=1 B`

### Directory Builtins

**`cd`**
- `cd dir` — change to directory
- `cd` — change to `$HOME`
- `cd -` — change to `$OLDPWD`, print new directory
- `cd -P` — resolve symlinks (physical path)
- Update `$OLDPWD` to previous `$PWD`, update `$PWD` to new directory
- Use `std::env::set_current_dir()` for actual directory change
- Error if directory doesn't exist or no permission

**`pwd`**
- `pwd` — print current working directory
- `pwd -P` — print physical path (resolve symlinks)
- Default: print `$PWD` if set, else `std::env::current_dir()`

### I/O Builtins

**`read`**
- `read VAR` — read one line from stdin, assign to VAR
- `read VAR1 VAR2 ... VARN` — split on IFS, assign to variables. Last var gets remainder.
- `read -r` — don't treat backslash as escape
- `read -p PROMPT` — display prompt (interactive)
- `read -n N` — read at most N characters
- `read -t TIMEOUT` — timeout in seconds (return 1 if timeout)
- Strips trailing newline
- Returns 1 on EOF

**`printf`**
- `printf FORMAT [ARGS...]`
- Format specifiers: `%s` (string), `%d` (decimal), `%i` (integer), `%o` (octal), `%x`/`%X` (hex), `%c` (char), `%%` (literal %)
- Escape sequences: `\n`, `\t`, `\r`, `\\`, `\0NNN` (octal), `\xNN` (hex)
- Reuse format string if more args than specifiers (POSIX behavior)
- `%b` — interpret backslash escapes in argument
- Returns 0 on success

### Test Builtin

**`test` / `[`**
- `[` requires closing `]`
- File tests: `-e` (exists), `-f` (regular file), `-d` (directory), `-r` (readable), `-w` (writable), `-x` (executable), `-s` (non-empty), `-L`/`-h` (symlink), `-p` (pipe), `-S` (socket), `-b` (block device), `-c` (char device)
- String tests: `-z` (empty), `-n` (non-empty), `str1 = str2`, `str1 != str2`, `str1 < str2`, `str1 > str2`
- Arithmetic tests: `-eq`, `-ne`, `-lt`, `-le`, `-gt`, `-ge`
- Logic: `!` (negation), `-a` (AND), `-o` (OR), `(` `)` (grouping)
- Returns 0 for true, 1 for false, 2 for error

### Signal Builtin

**`trap`**
- `trap 'command' SIGNAL [SIGNAL...]` — set trap handler
- `trap '' SIGNAL` — ignore signal
- `trap - SIGNAL` — reset to default
- `trap` — list all active traps
- `trap -l` — list available signal names
- `trap -p [SIGNAL]` — print trap for specific signal
- Signals: EXIT, ERR, DEBUG, RETURN, INT, TERM, HUP, etc.
- Trap commands stored in `env.traps`, fired by executor

### Info Builtins

**`type`**
- `type cmd` — identify command: "cmd is /path/to/cmd", "cmd is a shell builtin", "cmd is a shell function", "cmd is aliased to 'expansion'"
- `type -t cmd` — print type word only: `file`, `builtin`, `function`, `alias`, `keyword`
- Returns 1 if command not found

**`hash`**
- `hash` — list cached PATH lookups
- `hash cmd` — cache PATH lookup for cmd
- `hash -r` — clear cache
- `hash -d cmd` — remove specific entry
- Uses `env.hash_table` for storage

**`command`**
- `command cmd args` — execute cmd bypassing functions and aliases
- `command -v cmd` — print path or identify type (like `which`)
- `command -V cmd` — verbose identification (like `type`)

### Alias Builtins

**`alias`**
- `alias` — list all aliases
- `alias name` — show specific alias
- `alias name='value'` — set alias
- Multiple: `alias ll='ls -la' la='ls -a'`

**`unalias`**
- `unalias name` — remove alias
- `unalias -a` — remove all aliases

### Other Builtins

**`getopts`**
- `getopts OPTSTRING VAR [ARGS]`
- POSIX option parsing for scripts
- Uses `$OPTIND` (next argument index) and `$OPTARG` (option argument)
- Returns 0 while options remain, 1 when done
- `:` prefix in optstring = silent error mode

**`umask`**
- `umask` — display current mask (octal)
- `umask NNN` — set mask (octal)
- `umask -S` — display symbolic
- Uses platform-specific calls (libc::umask on Unix, no-op on Windows)

---

## Dependencies

No new dependencies. All builtins use existing:
- `crate::env::Env` for variable access
- `crate::expander` for word expansion in trap commands
- `crate::parser` for eval/trap command parsing
- `malt_platform::signals` for trap signal name resolution
- `std::fs` for file tests in `test`/`[`
- `std::env` for cd/pwd

---

## Testing Strategy

~40 tests via `run()` / `run_stdout()`:

### Variable (6 tests)
- export set+export, export existing, export -p lists
- unset removes variable, unset -f removes function
- readonly prevents modification

### Directory (5 tests)
- cd to tempdir, pwd matches
- cd - returns to OLDPWD
- cd (no args) goes to HOME
- PWD/OLDPWD updated after cd
- cd nonexistent returns error

### I/O (5 tests)
- read single var from heredoc/pipe
- read multiple vars splits on IFS
- read -r preserves backslashes
- printf %s %d formatting
- printf format reuse with extra args

### Test (8 tests)
- test -f with existing file
- test -d with directory
- test -z empty string
- test -n non-empty string
- test str1 = str2
- test N -eq N
- test ! negation
- [ requires closing ]

### Trap (3 tests)
- trap EXIT fires on subshell exit
- trap '' SIGNAL ignores
- trap -l lists signal names

### Info (4 tests)
- type identifies builtin
- type identifies external
- type -t returns type word
- command -v finds external path

### Alias (3 tests)
- alias sets, alias lists, unalias removes
- alias expansion in execution
- unalias -a clears all

### Other (3 tests)
- getopts basic option loop
- umask display
- command bypasses function

### Integration (3 tests)
- export + child process sees variable
- trap EXIT + function scope
- read in while loop (read line; do ...; done)
