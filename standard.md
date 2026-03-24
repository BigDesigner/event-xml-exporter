# standard.md

## Product standard

This project is not a "demo window"; it is a distributable Windows utility. The interface should feel clean, fast, and natural. The design language should be modern but calm.

## UI standard

- Main sections:
  - `Event Source Select`
  - `Export Parameters`
  - `System Analytics`
  - `Live Output Preview`
- Titles should use the same typography system.
- Cards and panels should share a consistent spacing rhythm.
- The left sidebar buttons should have equal height and full width.
- The `Destination Output Path` row must never be clipped at any window size.
- The footer should stay simple; a single centered copyright line is enough.
- Native window controls must be preserved.

## Technical standard

- Rust edition: `2024`
- Error handling: use `anyhow` with context
- UI: `eframe/egui`
- Platform dependency may remain Windows-only
- Event Log access must live in a service layer separate from the UI
- XML writing must live in a separate module

## Architecture standard

The following separation is the target:

- `src/main.rs`
  - application entry point
- `src/app/`
  - UI state and screen composition
- `src/platform/`
  - Windows Event Log access
- `src/export/`
  - XML generation and file writing
- `src/domain/`
  - data models and export request/response types

## Text and language standard

- The default UI language will be Turkish
- Technical field names may remain in English when needed
- All source files must be UTF-8
- Turkish characters must be preserved: `ç`, `ğ`, `ı`, `İ`, `ö`, `ş`, `ü`

## Performance standard

- Event scanning must run asynchronously or as a background task
- Large preview text must not freeze the UI
- Unnecessary clones and large intermediate buffers should be minimized

## Packaging standard

- Reproducible build steps for Windows release outputs must be documented
- The application icon must be defined with `app.ico`
- Release notes and installation steps must be kept in `README.md`

## Test standard

- At minimum, write unit tests for the domain and export layers
- Make the Windows Event Log API testable through a trait or adapter
- Validate XML output with snapshots or fixtures

## UX standard

- Empty states must be clear
- Invalid numeric input must produce clear errors
- When export completes, provide actions to open the file and its folder
- The default output file name must include a timestamp for every export
