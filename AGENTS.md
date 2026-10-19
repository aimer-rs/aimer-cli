# Repository guidance

## Rust source layout

- Keep every project-owned `.rs` file below 1,000 physical lines, including tests and documentation comments.
- When a file approaches the limit, move cohesive responsibilities into sibling modules and keep the parent module as a small facade.
- Name modules after the responsibility they own. Re-export only the items needed by callers; keep implementation details private.
- Keep tests beside the behavior they exercise, using child test modules or dedicated test files when that keeps source files within the line limit.
- Treat `vendor/` as imported third-party code; do not restructure it unless the task specifically requires a vendor change.

## Changes

- Preserve existing public Rust APIs, CLI behavior, and MCP tool names and argument shapes unless the task explicitly requests a behavior change.
- Prefer a focused module extraction over unrelated cleanup.
- Update or add focused tests when behavior changes. Do not run the test suite unless requested; report when checks were not run.
- Keep module interfaces small and put behavior behind the module that owns the relevant data and rules.
