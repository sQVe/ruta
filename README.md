# ruta

Throwaway app windows on a private headless display, for agents and tests.

ruta is a command-line tool that opens an app in a throwaway window on a private headless Weston
and Xwayland display. Agents and people can take shots of the window and send it input, and nothing
reaches the real screen, pointer, or keyboard. It runs on Linux.

ruta is in early development. The `ruta` binary does nothing yet.

## Install

```sh
cargo install --git https://github.com/sQVe/ruta ruta
```

See [the documentation](docs/README.md) for the development guide and architecture decisions.
