# AGENTS.md

This repository is prepared for agent-oriented work. Every agent or developer should follow the sequence below:

1. Read `standard.md` first.
2. Then read `docs/IMPLEMENTATION_PROMPT.md`.
3. Do not try to present the work as "fully finished" in one pass; progress in stages.
4. Keep the application in a runnable state at every stage.

## Workflow

Before starting work, read this file and follow this flow:

1. Inspect the repository first and understand the current structure.
2. Then make the required code changes.
3. After your changes, run `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo build` in that order.
4. If any of those commands fail, do not consider the task finished until the errors are fixed.
5. When the work is done, briefly summarize what you changed and which files you modified.

## Mandatory principles

- Use the native window chrome for V1.
- Do not attempt a custom title bar, custom close button, or frameless window in the first version.
- Do not build a fragile layout based on fixed `x/y/width/height` positioning.
- All UI sections must be built with responsive containers and a layout system.
- Turkish text must remain true UTF-8. Do not replace characters with plain `c`, `g`, or `u`.
- The export process must not block the UI thread.
- Error messages, informational messages, and status text must be user-friendly.
- The application must be packageable as a single `.exe`.

## Work sequence

1. Clarify the project module structure.
2. Build the theme and layout skeleton.
3. Add the Event Log reading services.
4. Complete the XML export flow.
5. Connect file operations and the preview area.
6. Then finish tests, polish, and packaging.

## Unacceptable approaches

- Fixed-pixel UI that can overflow
- String conversions that break encoding
- Putting all business logic into a single file
- Embedding Windows API business logic directly inside UI code
