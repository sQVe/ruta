# Input

Use it when an input command, its pacing, or the environment that `xdotool` gets changes. It needs
the parent's running sandbox.

Run every command as `cd $PROJECT && RUTA_NAME=$NAME $RUTA <command>`.

## Entry points

The test app logs one line per event, `ruta-test-app: <event>`. Each row lists the fields to find
in the new lines. Every command except `hold` and `release` first logs `motion x=0 y=0`, as the
gotchas describe. A line holds more
fields than a row names: a key line also has `keycode`, and every line has `state`.

| Entry point | Command | New log lines |
| ----------- | ------- | ------------- |
| `move` | `move 10 20 200 210` | `motion` lines from `x=10 y=20` to `x=200 y=210` |
| `click` | `click 100 120` | `motion x=100 y=120`, `button-press x=100 y=120 button=1 state=0x0`, `button-release x=100 y=120 button=1 state=0x100` |
| `click` with a button | `click 30 40 3` | `button-press x=30 y=40 button=3 state=0x0` and its `button-release` |
| `drag` | `drag 50 60 200 210` | `button-press x=50 y=60 button=1`, `motion` lines with `state=0x100`, `button-release x=200 y=210 button=1 state=0x100` |
| `type` | `type hi` | `key-press` and `key-release` with `keysym=0x68`, then with `keysym=0x69` |
| `key` | `key Return` | `key-press` and `key-release` with `keysym=0xff0d` |
| `key` with a modifier | `key s ctrl` | `key-press keysym=0xffe3`, then `key-press keysym=0x73 state=0x4`, then both releases |
| `scroll` down | `scroll 300 200 3` | Three `button-press x=300 y=200 button=5` and their releases |
| `scroll` up | `scroll 300 200 -2` | Two `button-press x=300 y=200 button=4` and their releases |
| `hold` | `hold Shift_L` | One `key-press keysym=0xffe1`, and no `key-release` |
| `release` | `release Shift_L` | One `key-release keysym=0xffe1` |

## Driving steps

1. Record the log size: `SIZE=$(wc -c < /var/tmp/ruta-$NAME/app.log)`.
2. Run one command from the table.
3. Wait one second, then print the new lines:
   `tail -c +$((SIZE + 1)) /var/tmp/ruta-$NAME/app.log`.
4. Repeat steps 1 to 3 for each row except `hold` and `release`.
5. With steps 1 to 3, run `hold Shift_L`, then `click 150 160`.
6. With steps 1 to 3, run `release Shift_L`, then `click 170 180`.
7. Run `shot`, and read the file it prints.

## Observable results

- Each input command prints nothing and exits 0. `shot` prints the path of its file.
- The log is the second view: the output of `RUTA_NAME=$NAME $RUTA log` ends with the same lines
  as the `tail`.
- A press or release has the `x` and `y` of its command. The last `motion` line of `move` and
  `drag` has the end point.
- Step 5: the `button-press x=150 y=160` line has `state=0x1`. Shift stayed down across commands.
- Step 6: the `button-press x=170 y=180` line has `state=0x0`.
- Step 7: the shot is a 640 x 480 PNG.

## Gotchas

- Read only the lines after your command. The parent's `up` leaves one focus click in the log.
- Each command except `hold` and `release` first moves the pointer to `0 0`, so its new lines
  start with `motion x=0 y=0`.
- `state` is the modifier and button mask before the event, so a `button-release` shows its own
  button as down.
- The guide's `release` rule also holds when a step after `hold` fails. Run `release Shift_L`
  before you report.
- The app logs asynchronously. When lines are missing, wait and read again before you report.
