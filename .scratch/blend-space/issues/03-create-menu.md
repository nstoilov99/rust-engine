# 03 — Create ▸ menu in the asset browser

**What to build:** Right-clicking the asset-grid background (empty space) opens a context menu with `Create ▸ Folder / Scene / Material / Script Graph / Animation Graph / Blend Space / Curve` (plus Reveal in Explorer); the folder-tree context menu gets the same `Create ▸` submenu replacing its ad-hoc "New Folder / New Animation Graph" items. The host's `CreateAsset` handler writes a minimal valid template per type (empty scene, default material definition, empty script `GraphDoc`, the existing animgraph template, an empty 1D blend space, the curve crate's default document), uniquifies `New<Type>.<ext>` → `_1`, `_2`…, rescans, selects the new row and enters inline rename — Unreal behaviour; creation no longer auto-opens (Animation Graph changes to match). `.blendspace` gets the animation icon/colour and is a first-class row. Open dispatch for `.blendspace` is stubbed to the tab that ticket 04 adds (until then it may no-op).

**Blocked by:** 01 (BlendSpace asset type + template)

**Status:** done

- [x] Right-click on empty grid space shows Create ▸ with all seven entries; right-click on a folder node shows the same submenu
- [x] Each created asset parses back through its own loader (scene, material, graph, animgraph, blendspace, curve)
- [x] Names uniquify (`NewBlendSpace.blendspace`, `NewBlendSpace_1.blendspace`, …)
- [x] After creation the new row is selected and in rename mode; Enter commits, Esc keeps the default name
- [x] Created assets appear without a manual refresh

**Where things landed:**
- Templates + uniquify: `engine/src/engine/editor/asset_browser/templates.rs` (`unique_asset_name`, `template_text`, `create_asset_file`).
- Panel: `AssetBrowserPanel::select_and_rename_after_rescan(path)` — resolved by `process_rescan` on the next draw (selects, emits `AssetSelected`, sets `RenameTarget::Asset`).
- Menus: `create_submenu` / `background_context_menu` in `asset_browser_crusty.rs`; grid/list report "pointer over a row" so the background menu only opens on empty space.
- Host: `CreateAsset` in `game_client/src/app.rs` routes every template type through `templates::create_asset_file`; Input Action / Mapping Context keep their own writers but get the same select+rename. `AssetOpened` has a `// ticket 04: open blend space tab` no-op arm for `.blendspace`.
