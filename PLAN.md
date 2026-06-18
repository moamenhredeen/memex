# PLAN -- Diagram View

## Goal

Add a fourth view type `Diagram` alongside `editor`, `graph`, `pdf`. A full
diagram editor (excalidraw-class) rendered natively in GPUI.

Locked decisions (user, 2026-06-16):
- Scope: FULL EDITOR (draw, select, move, resize, text, undo/redo, save).
- Native storage format: Excalidraw JSON (`.excalidraw`).
- Rendering: native GPUI primitives (quad/path/text). No webview, no SVG layer.

Requirements:
- Diagrams stored under a new `diagrams/` vault folder.
- A diagram is linked in a note via wikilink `[[name.excalidraw]]`.
- Clicking the link opens the diagram in the embedded viewer (secondary pane,
  same slot the PDF viewer uses).
- Import from drawio (`.drawio`, mxGraph XML) and excalidraw (`.excalidraw` JSON).

## Architecture (mirrors PDF view)

New module `src/diagram/`:
- `model.rs`   -- serde structs for the excalidraw file schema (this IS the
                  in-memory model; round-trips to disk verbatim).
- `mod.rs`     -- `DiagramState`: owns elements, selection, tool, camera
                  (pan/zoom), undo stack, focus_handle, path. Implements the
                  item method-set (new/focus/commands/execute_command/
                  get_candidates/handle_confirm/on_input_changed).
- `view.rs`    -- `DiagramView`: Render impl drawing elements with GPUI; emits
                  `DiagramViewEvent::Command`; resolve_command via keymap;
                  pointer interaction (create/select/move/resize).
- `import/drawio.rs`     -- mxGraph XML -> excalidraw elements.
- `import/excalidraw.rs` -- parse/normalise `.excalidraw` JSON.

Integration points (verified against source):
1. `src/app/resource.rs`  -- add `ResourceKey::Diagram(PathBuf)` +
   `BufferContent::Diagram(PathBuf)`. Add `is_diagram_path` + diagram link
   parse helper.
2. `src/pane.rs`          -- add `ActiveItem::Diagram { state, view }` and the
   dispatch arms (set_theme, commands, execute_command, get_candidates,
   handle_confirm, on_input_changed, highlight_input, is_diagram, view_element,
   focus, position_text, mode_badge, display_name). pane.rs:8-12 doc says
   adding a variant + arms is the full contract.
3. `src/vault/layout.rs`  -- add `DIAGRAMS_DIR = "diagrams"`, field on
   `VaultLayout`, create in `ensure()`, `diagram_path(name)` helper. Update
   tests.
4. `src/app/workspace_controller.rs` -- `create_diagram_item`, `prepare_diagram`,
   `open_diagram` (mirror create_pdf_item / prepare_pdf / open_pdf_target).
5. `src/app.rs` follow_wikilink -- route `*.excalidraw` targets to open_diagram
   (mirrors the parse_pdf_link branch at follow_wikilink).
6. Commands: `diagram-new`, `diagram-import` (drawio/excalidraw file picker via
   minibuffer delegate), plus editor command to create+link a new diagram.

## Dependencies to add (Cargo.toml)

- `roxmltree` (read-only XML DOM) for drawio parsing.
- `flate2` + `base64` ONLY IF we must support compressed drawio `<diagram>`
  payloads. Decision: implement uncompressed mxGraph first; add inflate path as
  a tracked sub-task. Compressed drawio = base64( raw-deflate( url-encoded xml )).

## Phases

### Phase 0 -- Plumbing + skeleton (compiles, opens empty diagram)
- Cargo: add roxmltree.
- model.rs: excalidraw schema structs (rectangle, ellipse, diamond, line,
  arrow, text, freedraw, image) + ExcalidrawFile load/save.
- mod.rs: DiagramState minimal (load from path, focus, empty commands).
- view.rs: DiagramView with a placeholder Render (blank canvas + bg).
- resource.rs / pane.rs / layout.rs / workspace_controller.rs / app.rs wiring.
- New-diagram command writes empty `.excalidraw` to diagrams/, inserts link,
  opens viewer.
DONE WHEN: `cargo build` clean; new + open round-trips an empty diagram file.

### Phase 1 -- Renderer (view-only, clean style)
- Camera (pan/zoom) -> screen transform.
- Draw each element type with GPUI primitives. Clean (non-rough) strokes first.
- Text rendering with fontSize/family/align.
- Arrow heads, bound text on shapes.
DONE WHEN: an imported sample renders faithfully (modulo hand-drawn jitter).

### Phase 2 -- Importers
- excalidraw: deserialize, version-normalise, copy into diagrams/.
- drawio: roxmltree parse mxGraph; map vertex/edge geometry+style to elements.
- `diagram-import` command + file picker; writes a `.excalidraw` into diagrams/.
DONE WHEN: a real .drawio and a real .excalidraw import and render.

### Phase 3 -- Editing
- Tool palette (select/rect/ellipse/diamond/arrow/line/draw/text).
- Pointer: create-on-drag, hit-test select, multi-select, move, resize handles,
  rotate.
- Text editing in shapes.
- Undo/redo stack.
- Save to disk (autosave or explicit) preserving excalidraw schema.
DONE WHEN: can draw + edit + save and reopen losslessly.

### Phase 4 -- Polish
- Hand-drawn (rough.js-style) stroke option.
- Inline embed `![[name.excalidraw]]` rendered in the editor.
- Style controls (stroke/fill/width/color), grid/snap.

## drawio feature-parity track (user, 2026-06-18)

Goal: bring the editor toward drawio parity. Locked decisions (user):
- Connectors: STRAIGHT lines, endpoints bound to shapes (not orthogonal).
- Hand-drawn / rough.js style: SKIPPED entirely. Clean strokes only.
- Properties UI: SIDE PANEL, shown when selection non-empty.
- Freehand (Tool::Draw): removed from palette (model + import keep it).

### P1 -- quick wins  [DONE]
- Filled-triangle arrowhead (was two strokes / V-shape with a gap).
- Background grid: world-space lines, gridSize from appState (default 20),
  skipped when the on-screen step < 4px.
- Snap-to-grid on move/create/resize; Alt inverts snap. snap_enabled field.
- Freehand removed from tool palette.
- Multi-select: Ctrl+click toggles; left-drag empty canvas = rubber-band
  marquee (Ctrl = additive); pan moved to MIDDLE-mouse drag (+ wheel zoom).

### P2 -- properties side panel
- Panel visible when selection non-empty.
- Stroke color, background color, fill style (solid/hachure/cross-hatch),
  stroke style (solid/dashed/dotted), stroke width. Fields already in model.rs.
- Renderer additions: dashed/dotted strokes (segment gaps), hachure fill.

### P3 -- connectors (hardest)
- Snap points per shape: edge mid-points + center, shown on hover.
- Add start_binding/end_binding to Element; maintain boundElements on shapes.
- Bind arrow endpoint on create/drag-near; re-route endpoints when the bound
  shape moves/resizes. Straight lines only.

### P4 -- parity polish
- Copy/paste/duplicate, arrow-key nudge (grid step), z-order (reorder vec),
  right-click context menu, bound/centered text inside shapes.

Still deferred: rotation+angle render, rounded corners, zoom-to-cursor,
polyline hit-test for lines/arrows, image decode, orthogonal edges, export.

## Open questions / risks
- rough.js fidelity is large; Phase 1-3 ship clean strokes, rough is Phase 4.
- Compressed drawio payloads need flate2+base64 (deferred sub-task).
- GPUI custom path painting API surface must be confirmed against gpui 0.2
  (PdfView only uses divs/img; diagram needs real path/quad painting).
