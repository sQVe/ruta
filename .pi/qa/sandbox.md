# Sandbox

Use it for every run of the Running sandbox section. It needs the parent's sandbox under `$NAME`
and the one under `$NAME-gone`, whose app the parent stopped.

Run every command as `cd $PROJECT && RUTA_NAME=$NAME $RUTA <command>`, unless a step says
otherwise.

## Entry points

- The sandbox under `$NAME`, in `$PROJECT`.
- The same sandbox, from another project folder.
- The sandbox under `$NAME-gone`.

## Driving steps

1. Run `shot`, and run `file` on the path it prints. Then run `shot <file>` with a file under
   `$PROJECT`.
2. Run `RUTA_NAME=$NAME $RUTA log | cmp - /var/tmp/ruta-$NAME/app.log`.
3. Run a `click`, and read the log.
4. Run `/var/tmp/ruta-$NAME/bin/xdg-open https://example.com/qa`. Then run `opened`, and run
   `cat /var/tmp/ruta-$NAME/opened`.
5. Run `mkdir $PROJECT/other`, then `cp $PROJECT/ruta.toml $PROJECT/other/`. Then run
   `cd $PROJECT/other && RUTA_NAME=$NAME $RUTA status`. Then run `status` in `$PROJECT`.
6. Under `RUTA_NAME=$NAME-gone`, run `status`, `shot`, `click 1 2`, and `log`.

## Observable results

- Step 1: `shot` prints `/var/tmp/ruta-$NAME/shot.png`, and `file` reports a 640 x 480 PNG.
  `shot <file>` prints and writes that file instead.
- Step 2: the command prints nothing. `log` prints the app log byte for byte.
- Step 3: the `click` logs a `button-press` with the `x` and `y` of the command.
- Step 4: `opened` prints the links that the stub openers logged. It prints that link, and so
  does the `cat`.
- Step 5: `status` from another project folder is a refusal. It exits 1, prints nothing on
  stdout, and prints `ruta $NAME serves $PROJECT, not $PROJECT/other` on stderr. `status` in
  `$PROJECT` still exits 0 afterwards.
- Step 6, `status`: it exits 1, prints nothing on stdout, and prints `ruta $NAME-gone is not up`
  on stderr.
- Step 6, `shot` and `click 1 2`: each exits 1 and prints
  `ruta $NAME-gone is not up; run 'ruta up' first` on stderr.
- Step 6, `log`: it still exits 0 and prints the app log.
- Step 6: `/var/tmp/ruta-$NAME-gone/shot.png` does not exist afterwards.

## Gotchas

- `shot` without a file writes `/var/tmp/ruta-$NAME/shot.png`. Run it, because that default path
  is under test. Write every other shot under `$PROJECT`.
- Step 6 runs without the `status` check of the `ruta guide` text, because the app under
  `$NAME-gone` exited.
