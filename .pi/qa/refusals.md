# Refusals

Use it when command parsing, the usage text, `ruta.toml` reading, error text, or exit codes change.

Run every command as `cd $PROJECT && RUTA_NAME=$NAME-idle $RUTA <command>`, unless a row says
otherwise.

## Entry points

`<toml>` is the path of the `ruta.toml` that ruta found.

| Entry point | Command | stderr |
| ----------- | ------- | ------ |
| No command | none | The usage text, starting with `Usage:` |
| Unknown command or option | `nonsense`, `--help`, `--version` | The usage text |
| Wrong argument count | `log x`, `shot a b`, `type` | The usage text |
| Zero notches | `scroll 1 2 0` | `scroll needs a nonzero number of notches` |
| Not a number | `click a 2` | `x must be a whole number, not "a"` |
| Button out of range | `click 1 2 300` | `button must be a whole number, not "300"` |
| No config | `log`, run in an empty folder under `/tmp` | `No ruta.toml in <folder> or a folder above it.` |
| Unknown config key | `log`, after step 3 below | `Invalid <toml>: unknown key "colour"` |
| Bad name | `RUTA_NAME=Bad_Name $RUTA log` | `Invalid <toml>: "RUTA_NAME" must be one name of lowercase letters, digits, and dashes, at most 32 bytes, not "Bad_Name"` |
| Shot with nothing up | `shot` | `ruta <name> is not up; run 'ruta up' first` |
| Input with nothing up | `click 1 2`, `hold Shift_L`, and the other six input commands | `ruta <name> is not up; run 'ruta up' first` |
| Log with nothing up | `log` | `Could not read the app log: No such file or directory (os error 2)` |
| Opened with nothing up | `opened` | Empty, and the command exits 0 |
| Refusals of `up`, `restart`, `down`, and `capture` | none | Report as not verified |

## Driving steps

1. Check that `ls -d /var/tmp/ruta-$NAME-idle` fails. When the folder exists, stop and ask the
   parent.
2. Run each row above that needs no config change. Record the exit code, stdout, and stderr.
3. Add the line `colour = "red"` at the top of `$PROJECT/ruta.toml`, run the unknown-key row, and
   remove the line again.
4. Run step 1 again.

## Observable results

- Each refusal exits 1, prints the message and a newline on stderr, and prints nothing on stdout.
- A parse error comes before the config is read. The first six rows give the same result in a
  folder with no `ruta.toml`.
- `ls -d /var/tmp/ruta-$NAME-idle` fails before and after: no refusal makes a root.

## Gotchas

- When a parent sandbox runs, a config edit in `$PROJECT` also breaks every command on it until
  the line is removed. Remove the line before you do anything else.
- Never edit a tracked `ruta.toml`, and never test a refusal of `up`, `restart`, `down`, or
  `capture`. When the change under test breaks the order of their checks, they start or stop
  processes.
