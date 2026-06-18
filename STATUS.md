# STATUS -- Diagram View

Last updated: 2026-06-18

## drawio-parity P2 COMPLETE (2026-06-18)
Properties side panel + styled rendering. Builds clean, 6 diagram tests pass.
- mod.rs: primary_selected(), mutate_selected() (one undo step over the whole
  selection), setters set_selected_{stroke_color,background,fill_style,
  stroke_style,stroke_width}.
- view.rs render: stroke_polyline_dashed() (pattern carries across vertices),
  dash_pattern() (dashed/dotted scaled by stroke width), stroke_styled()
  wrapper. Shapes + lines/arrows honor el.stroke_style. Arrowhead stays solid.
  hatch_dir() + fill_shape(): hachure / cross-hatch fills via scanline lines
  clipped to the (convex) shape polygon; "solid" = fill_polygon as before.
- view.rs render_panel(): right-side panel, shown only when selection
  non-empty. Stroke + Background color swatches (excalidraw palette,
  transparent included), Width (1/2/4 with bar glyph), Stroke style
  (solid/dash/dot), Fill style (solid/hach/cross). Active = accent border/bg.
  Each button calls the matching setter; panel swallows Left clicks so they
  don't hit the canvas. Wired via .children(render_panel) in render().

NEXT: P3 (connectors -- shape snap points + arrow start/end binding +
re-route on move/resize). See PLAN.md.

## drawio-parity P1 COMPLETE (2026-06-18)
Builds clean (no new warnings); 6 diagram tests pass.
- Arrowhead: now a filled triangle (view.rs arrowhead()) -- fixes the
  "two lines / gap at tip" look. Used by arrows + the palette glyph.
- Grid: paint_grid() draws world-space grid lines in the canvas closure;
  spacing = DiagramState::grid_size() (appState.gridSize, default 20);
  auto-skipped when step < 4px. show_grid field (default true).
- Snap: DiagramState::snap_coord() + snap_enabled field (default true).
  Applied to move (anchor origin -> grid, group keeps relative layout),
  create (origin corner at mouse-down + dragged corner), resize (dragged
  corner). Alt at drag time inverts snap (snap_enabled ^ alt).
- Freehand: Tool::Draw removed from palette TOOLS (now 7); enum + model +
  import support retained.
- Multi-select: toggle_select (Ctrl+click), is_selected, select_in_rect
  (bbox-overlap, additive union onto base) in mod.rs. view.rs: Drag::Marquee
  variant + Act::Marquee; left-drag empty canvas rubber-bands (Ctrl additive),
  marquee rect painted (accent outline + faint fill). PAN moved to
  MIDDLE-mouse drag (on_mouse_down/up Middle); move guard accepts Left|Middle.
- Clicking an already-selected element keeps the group (moves all, no collapse).

NEXT: P2 (properties side panel: stroke/bg/fill-style/stroke-style/width;
dashed/dotted + hachure render). See PLAN.md "drawio feature-parity track".

## --- prior status (Phases 0-4) below ---

Last updated: 2026-06-16

## Where we are
Phase 3 (Editing) COMPLETE (rotate deferred -- see below).
  3a: tool state + palette, click-select, drag-move, delete, save.
  3b: creation -- drag-create rect/ellipse/diamond/line/arrow, freedraw,
    text via minibuffer. gen_id (fastrand). Degenerate clicks dropped.
  3d: snapshot-based undo/redo.
  3c: resize -- 8 handles on a single box selection (rect/ellipse/diamond/
    text/image); drag a handle to resize; handles drawn as accent squares.

ROTATE is intentionally NOT done here: the renderer ignores element `angle`
(Phase 1 deferral), so a rotate handle would change data invisibly. Rotate
is bundled with angle-rendering in the polish pass.

Phases 0/1/2 COMPLETE. 6 diagram tests pass. Builds clean (13 warnings, all
pre-existing non-diagram dead code).

## The editor is now fully usable
Create (all tools) / select / move / resize / delete / text / undo-redo /
save / import (drawio+excalidraw) / render + pan/zoom.

## DONE in Phase 3a
- mod.rs: Tool enum (Select/Rect/Ellipse/Diamond/Arrow/Line/Draw/Text);
  DiagramState fields tool/selected/dirty/origin; screen_to_world +
  set_viewport_origin (origin stashed in canvas prepaint); hit_test (bbox +
  tolerance); select_only/clear_selection/selected_origins/set_element_position;
  delete_selected (isDeleted); save(); set_tool. Commands: diagram-save,
  diagram-delete, diagram-tool-*.
- view.rs: Drag enum (Pan/Move); tool-aware mouse_down (select+move vs pan);
  mouse_move drag; mouse_up sets dirty; selection outline in paint; floating
  tool palette (clickable, active highlighted).

## Phase 3a notes / limits
- Save is explicit only (":diagram-save" or palette "Diagram: Save"); no
  autosave. ":w" still saves the markdown editor, not the focused diagram.
- Single-key tool shortcuts (binding fields v/r/o/...) NOT wired into keymap
  yet; use palette or command palette. Wire in polish.
- Move works for any selection; resize/rotate not yet (3c).

## DONE in Phase 2
- Cargo.toml: roxmltree 0.20.
- model.rs: Element::base constructor (defaults; used by importers).
- src/diagram/import/{mod,drawio,excalidraw}.rs:
  - import_file(path) dispatches by extension.
  - drawio: roxmltree mxGraph parse -> vertices (rect/ellipse/diamond/text),
    edges -> arrows between vertex centers + waypoints; style colors mapped;
    HTML labels stripped. Compressed payloads rejected with clear error.
  - excalidraw: parse + normalise (passthrough; native format).
- workspace_controller: import_diagram -> writes <stem>.excalidraw into
  diagrams/ (unique), links it, opens it.
- app.rs: ":diagram-import <path>" command. command_registry entry.

## DONE in Phase 1 (src/diagram/view.rs + mod.rs)
- Camera: DiagramState::content_bounds + fit_to_content (frames content on
  open, called from create_diagram_item with assumed 800x600 pane).
- canvas() paint closure draws every element type via paint_path:
  rectangle/frame, diamond, ellipse (48-seg polygon), line, arrow (+arrowhead),
  freedraw, text (multi-line via shape_line), image (placeholder box).
- Stroke = filled quad per segment (width scales with zoom); fill = closed
  polygon. Colors via elem_rgba (hex + element opacity -> Rgba).
- Pan (drag) + zoom (wheel) live; "Empty diagram" hint when no elements.

## Phase 1 known limitations (intentional, later phases)
- Element rotation (angle) ignored -- axis-aligned only.
- Rectangle corner rounding (roundness) not drawn -- sharp corners.
- rough.js hand-drawn style not applied -- clean strokes (Phase 4).
- Zoom is about canvas origin, not cursor.
- No element images decoded (placeholder box only).

Pre-existing unrelated failure: pdf::...native_highlight_roundtrips_through_
atomic_save -> "Access is denied (os error 5)" on Windows. Confirmed failing
WITHOUT the diagram changes (stash test). Not ours; do not chase here.

## DONE in Phase 0
- src/diagram/model.rs  -- excalidraw schema, lossless round-trip (extra map).
- src/diagram/mod.rs    -- DiagramState + method-set + commands (zoom/center).
- src/diagram/view.rs   -- DiagramView skeleton render + pan/zoom + key dispatch.
- src/main.rs           -- `mod diagram;`.
- src/vault/layout.rs   -- diagrams/ dir + diagram_path() + ensure + tests.
- src/app/resource.rs   -- ResourceKey::Diagram, BufferContent::Diagram,
                           is_diagram_path, is_diagram_link.
- src/pane.rs           -- ActiveItem::Diagram + all dispatch arms.
- src/app/workspace_controller.rs -- create/prepare/open/new_diagram; route
                           .excalidraw in open_note_by_path.
- src/app.rs            -- follow_wikilink routes [[*.excalidraw]]; "diagram-new".
- src/app/command_registry.rs -- "New Diagram" palette command.

## Phase 4 -- Polish (IN PROGRESS)
DONE:
- Inline text editing: Text tool click creates an empty text element and
  enters edit mode; key_char feeds chars, Enter=newline, Backspace, Escape or
  click-away commits (empty discarded). Click existing text to re-edit. Caret
  drawn at the end of the text. Minibuffer text path removed entirely.
- Tool palette: drawn glyph icons (per-tool mini-canvas) in rounded buttons,
  centered floating bar, surface bg + border + shadow, accent-on-active +
  hover. No more text labels.

## Remaining Phase 4 backlog, per user's plan
Renderer:
- Element rotation: honor `angle` in paint (rotate the shape's points/quad
  about its center) -- THEN add the rotate handle (couples to this).
- Rounded rectangle corners (roundness).
- rough.js-style hand-drawn strokes (optional toggle).
- True stroke width on shapes already done via stroke_segment; revisit joins.
- Zoom toward cursor instead of origin.
- Decode embedded images (files map) instead of placeholder box.
- Polyline-distance hit-testing for lines/arrows/freedraw (currently bbox).
Editor:
- Edit existing text (double-click / command, prefilled).
- Wire single-key tool shortcuts + Ctrl+Z/Ctrl+Shift+Z into the keymap.
- Autosave or wire ":w" to save the focused diagram (currently saves the
  markdown editor; diagram uses ":diagram-save").
- Multi-select (rubber-band) + group move/resize.
- Style controls (stroke/fill/width/color pickers), grid + snap.
Importers:
- Compressed drawio payloads (base64+deflate) -> add flate2+base64.

## Phase 2 known limitations (later)
- Compressed drawio payloads (base64+deflate <diagram>) rejected; need
  flate2+base64 to inflate.
- drawio labels are standalone text near top-left of shape, not bound/centered.
- drawio shape coverage is the common set (rect/ellipse/rhombus/text/edge);
  exotic stencils fall back to rectangle.

## Decisions locked
- Full editor. Excalidraw JSON native format. Native GPUI rendering.
- Diagrams in `diagrams/` vault folder. Linked via `[[name.excalidraw]]`.
  Opens in secondary pane (PDF-style).

## Architecture confirmed (source-verified)
- Views = `ActiveItem` enum, `src/pane.rs:85`. No trait; dispatch arms only.
- PDF is the template: state+view entity pair; opened into secondary pane.
- ResourceKey/BufferContent: `src/app/resource.rs:8`.
- Vault layout + ensure(): `src/vault/layout.rs:18-52`.
- create_pdf_item/prepare_pdf: `src/app/workspace_controller.rs`.
- Wikilink routing: `src/app.rs` follow_wikilink (parse_pdf_link branch).

## GPUI painting API (CONFIRMED via src/graph/view.rs)
- `canvas(prepaint, paint).size_full()` -- custom-paint element.
- `window.paint_path(gpui::Path::new(pt).line_to(pt), color)` -- lines/polylines.
- `window.paint_quad(PaintQuad { bounds, corner_radii, background,
  border_widths, border_color, border_style })` -- rects; full corner_radii =>
  circle. Use for rectangle/ellipse-circle.
- `window.text_system().shape_line(text, px(size), &[TextRun], None)` then
  `shaped.paint(point, px(size), window, cx)` -- text.
- Mouse/keys via on_mouse_down/move/up/on_scroll_wheel + on_key_down on the
  parent div; hit-test in state (cf. GraphState::node_at). Pan/zoom = state
  fields applied in the paint closure.
Risk RESOLVED: native rendering + interaction fully expressible.

## Notes / risks
- rough.js hand-drawn style deferred to Phase 4.
- Compressed drawio payloads deferred (need flate2+base64).
- No XML crate currently in tree (only tree-sitter-xml).
