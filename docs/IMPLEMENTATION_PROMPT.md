# Implementation Prompt

You can give the prompt below directly to a new agent or a fresh Codex session.

## Prompt

I want you to build a modern Windows desktop application from scratch in Rust.

Application name: `Event XML Exporter`

Goal:
Build a modern utility application that filters selected Event ID records from the Windows Event Log and exports them as an XML file.

Technology expectations:
- Rust
- Native desktop UI built with `eframe/egui`
- Windows Event Log integration through the `windows` crate
- XML generation with `quick-xml`
- File selection dialog with `rfd`

Must read:
- `agents.md`
- `standard.md`

Application requirements:

1. Window behavior
- Use the native Windows title bar.
- Do not build a frameless window or a custom close/minimize system.
- Build a layout that does not break under DPI scaling.

2. Main UI sections
- Left sidebar: `Event Source Select`
- Upper middle panel: `Export Parameters`
- Upper right panel: `System Analytics`
- Large middle panel: `Live Output Preview`
- Bottom action bar: `Open File`, `Open Folder`, `Close`, `Export as XML`
- Bottom footer: centered copyright

3. Sidebar behavior
- Default event presets:
  - `41 Kernel Power`
  - `55 NTFS Error`
  - `6008 Unexpected Shutdown`
  - `6005 Log Started`
  - `6006 Log Stopped`
  - `1001 BugCheck`
- Buttons:
  - `Add ID`
  - `Remove`
  - `All`
  - `Clear`
- Buttons must be full width and equal height.

4. Export Parameters behavior
- Log source selection: `System`, `Application`, `Security`
- `Max Events` numeric input
- Checkbox option: `Export all matching records without limit`
- `Destination Output Path` field and `Browse` button
- The layout must never clip or overflow

5. System Analytics behavior
- `Total Logs Found`
- `Queue Size`
- Status label: `READY`, `WORK`, `DONE`, `ERROR`
- Analytics cards should have proper padding

6. Preview behavior
- Show a live XML preview based on the selected settings
- The preview must be scrollable
- Use a monospace font

7. Export behavior
- Scan Windows Event Log records according to the selected Event IDs
- Write them to an XML file
- The root element must include metadata:
  - source
  - exported_at
- The default file name must include a timestamp:
  - example: `GNN_Export_20260324_012736.xml`
- If the same file already exists, make the output unique by appending a timestamp instead of overwriting

8. File actions
- `Open File` should open the most recently generated file
- `Open Folder` should open the file's containing folder

9. Language and encoding
- The UI language will be Turkish
- Turkish characters must not break
- All source files must be UTF-8

10. Design expectations
- Flat, dark, calm, and professional appearance
- Do not build a fragile layout with fixed pixel coordinates
- Use a shared spacing system
- All panel titles should use the same typography

11. Architecture expectations
- Separate the UI, domain, export, and Windows platform layers
- Do not pile all business logic into a single file
- Use contextual error handling

12. Delivery order
- First create the project structure
- Then finish the static UI skeleton
- Then add the Event Log service layer
- Then complete the export and preview connections
- Then clean up tests and the README

13. Acceptance criteria
- `cargo fmt`, `cargo check`, `cargo clippy`, and `cargo test` must pass cleanly
- The layout must not overflow
- Native window controls must remain visible
- The export flow must actually produce an XML file
- Turkish text must render correctly

Work in stages. At the end of each stage, briefly state which files you changed and which risk you reduced.
