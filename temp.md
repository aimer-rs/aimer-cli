- cargo_progress_lines_filtered: 1 # remove this
- command: cargo clippy --workspace -- -D warnings
- diagnostic_summary:
  - error_count: 6
  - warning_count: 0 # not include if it 0
- diagnostics:
  - [0]
    - code: clippy::unnecessary_map_or. # remove this
    - column: 23  # remove this
    - file: aimer_hot_reload/src/config.rs # became - file: aimer_hot_reload/src/config.rs:212:23
    - line: 212 # remove this
    - message: this `map_or` can be simplified # remove this
    - rendered: # remove this keep the render
      ```
      error: this `map_or` can be simplified
         --> aimer_hot_reload/src/config.rs:212:23
          |
      212 |         Ok(config) => config.mcp.map_or(true, |mcp| mcp.auto_start),
          |                       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
          |
          = help: for further information visit https://rust-lang.github.io/rust-clippy/rust-1.99.0/index.html#unnecessary_map_or
          = note: `-D clippy::unnecessary-map-or` implied by `-D warnings`
          = help: to override `-D warnings` add `#[allow(clippy::unnecessary_map_or)]`
      help: use `is_none_or` instead
          |
      212 -         Ok(config) => config.mcp.map_or(true, |mcp| mcp.auto_start),
      212 +         Ok(config) => config.mcp.is_none_or(|mcp| mcp.auto_start),
          |
      
      ```
    - severity: error # remove this
    - source: clippy # remove this
- diagnostics_truncated: false # not include if it 0
- duration_ms: 977 # not include if it 0
- exit_code: 101
- lockfile_changed: false
- operation: cargo_clippy # not include if it 0
- status: failed
- stderr:
# Keep the error only warning is filter out
error: could not compile `aimer_hot_reload` (lib) due to 6 previous errors