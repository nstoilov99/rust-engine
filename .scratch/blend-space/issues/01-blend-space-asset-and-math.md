# 01 — Blend space asset document and pure blend math

**What to build:** The `.blendspace` RON document (version 1: one or two axes `{name, param, min, max, grid_divisions}`, samples `{x, y, clip, clip_name, rate_scale}`, `input_smoothing`), its load/save helpers on the engine's content-path conventions, and a pure compiled `BlendSpace` with weight evaluation: 1D bracketing lerp with clamping; 2D Delaunay triangulation (Bowyer–Watson, deterministic), barycentric weights inside the hull, projection to the nearest hull edge/vertex outside it; degenerate sets (1 sample, 2 samples, all collinear → projected 1D; 0 samples → refusal). Weights under epsilon dropped, at most three contributors. Lives in the engine animation module (engine-side per ADR 0001). No runtime or editor wiring yet. Also `AssetType::BlendSpace` for the `blendspace` extension so the browser classifies the file. Ship the demo `content/blendspaces/locomotion.blendspace` (1D Speed, Defeated clip at three rate scales).

**Blocked by:** None — can start immediately.

**Status:** done

- [x] `.blendspace` document round-trips through RON (1D and 2D, samples, smoothing)
- [x] Weights on a sample are exactly (1.0 on it, nothing else); weights always sum to 1
- [x] 2D input inside a triangle gives barycentric weights of that triangle's samples; outside the hull it equals the weights at the nearest hull point
- [x] Collinear / two-sample / one-sample sets evaluate as projected 1D / single clip; zero samples refuse with a message
- [x] 1D evaluation brackets and clamps like the existing `anim_blend1d` rule
- [x] Triangulation result (and therefore weights) is independent of sample order
- [x] `AssetType::from_extension("blendspace")` classifies as BlendSpace with the animation category
