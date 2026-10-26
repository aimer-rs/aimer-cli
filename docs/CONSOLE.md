[← Back to README](../README.md)

# Interactive Developer Console

When running `aimer run`, Aimer launches an interactive terminal interface (TUI) powered by Ratatui and Crossterm.

## Keyboard Shortcuts

| Shortcut               | Action               | Description                                          |
|:-----------------------|:---------------------|:-----------------------------------------------------|
| `r`                    | **Hot Reload**       | Triggers an in-place hot reload of application code  |
| `R` / `Shift + R`      | **Hot Restart**      | Stops the app, rebuilds, and relaunches fresh        |
| `q` / `Shift + Q`      | **Quit**             | Gracefully stops the running app and exits the CLI   |
| `1`                    | **App Pane**         | Focuses the Application log output pane              |
| `2`                    | **Build Pane**       | Focuses the Compiler / Build output pane             |
| `3`                    | **Inspector Pane**   | Focuses the Widget Inspector pane                    |
| `4`                    | **Resource Monitor** | Shows live CPU, GPU, memory, PID, and thread metrics  |
| `Tab`                  | **Next Pane**        | Cycles focus between available panes                 |
| `F12`                  | **Toggle Inspector** | Opens or hides the Widget Inspector view             |
| `t`                    | **Toggle Tree**      | Toggles the Widget Inspector full-tree view          |
| `e`                    | **Source Locations** | Toggles file & line number locations on log messages |
| `s`                    | **Selection Mode**   | Enables keyboard selection mode                      |
| `y`                    | **Yank**             | Copies the highlighted selection to clipboard        |
| `Shift + C`            | **Clear**            | Clears logs in the currently focused pane            |
| `Ctrl + C` / `Cmd + C` | **Copy**             | Copies the active selection or focused pane content  |
| `↑` / `↓`              | **Scroll**           | Scrolls the active pane up or down                   |
| `PgUp` / `PgDn`        | **Page Scroll**      | Scrolls active pane by full page                     |
