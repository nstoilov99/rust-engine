# Editor UI Style Guide

## Icon System

Icons are classified into three rendering modes:

- **Chrome:** Monochrome UI glyphs (folder, chevron, play/pause, etc.), tinted per interaction
  state (Default, Hovered, Active, Disabled, Accent). Used for UI affordances.
- **Typed:** Category-colored icons for asset/entity types. Each icon belongs to one of 10
  categories (Geometry, Lights, Cameras, VFX, Audio, Animation, Materials, Scripting, Physics, UI).
  Used for "what kind of thing is this."
- **Severity:** Semantic state badges (Error, Warning, Success, Info). Used for status indicators.

All icons render as **white source pixels tinted at draw time**. The tint color is resolved from
the `IconPalette` based on the icon's class. Per-icon overrides can customize individual icons.

### Default Palette

| Category   | Color   | Hex       |
|------------|---------|-----------|
| Geometry   | Grey    | `#A8B0BA` |
| Lights     | Amber   | `#FFC857` |
| Cameras    | Blue    | `#5B9BD5` |
| VFX        | Orange  | `#E07856` |
| Audio      | Green   | `#62C370` |
| Animation  | Pink    | `#E66BB8` |
| Materials  | Purple  | `#A47AE8` |
| Scripting  | Teal    | `#4FC1B6` |
| Physics    | Lime    | `#9DCC4D` |
| UI         | Salmon  | `#F08C7E` |

| Severity | Color  | Hex       |
|----------|--------|-----------|
| Error    | Red    | `#E56B6B` |
| Warning  | Yellow | `#E6C04F` |
| Success  | Green  | `#6ECB7C` |
| Info     | Blue   | `#6EA8E8` |

| Chrome State | Color   | Hex       |
|--------------|---------|-----------|
| Default      | Light   | `#C0C4CC` |
| Hovered      | Bright  | `#E6E8EC` |
| Active       | White   | `#FFFFFF` |
| Disabled     | Dim     | `#60646C` |
| Accent       | Blue    | `#4FA3E8` |

### Icon Inspector (Dev Tool)

The Icon Inspector (`Debug > Icon Inspector`, requires `--features editor-debug`) is the
canonical tool for verifying palette changes before they land.

**Workflow:**
1. Open the Icon Inspector from the Debug menu
2. Experiment with category/severity/chrome colors using the color pickers
3. Click individual icons to access per-icon tint overrides and tint mode toggles
4. Click **Export Palette** to copy a drop-in `default_dark()` Rust snippet to the clipboard
5. Paste over `IconPalette::default_dark()` in `icon_classes.rs` and commit through code review

The inspector does **not** auto-save. Closing the window or restarting the editor returns to
the palette defined in `IconPalette::default_dark()`. This is intentional: palette decisions
go through normal code review.

### Adding a New Icon

1. Decide its class (Chrome / Typed / Severity)
2. Add the variant to `IconKind` in `engine/src/engine/editor/widgets/mod.rs`
3. Implement `class()`, `category()`, `severity()`, `display_name()`, `fallback_text()`
4. Add the variant to `IconKind::ALL`
5. Add a PNG to `engine/icons/` and map it in `IconRegistry::load()`
6. Verify in the Icon Inspector

### Key Files

| File | Contents |
|------|----------|
| `engine/src/engine/editor/icon_classes.rs` | `IconClass`, `TypeCategory`, `Severity`, `ChromeState`, `TintMode`, `IconPalette` |
| `engine/src/engine/editor/widgets/mod.rs` | `IconKind` enum, `IconRegistry`, `UiExt` trait |
| `engine/src/engine/editor/icon_inspector/mod.rs` | Icon Inspector window (editor-debug only) |
