feat(task-30): implement skeletal animation & GPU skinning pipeline

Add complete skeletal animation system with GPU bone palette skinning,
keyframe sampling, forward kinematics, crossfade blending, and editor
integration.

## Phase 1: GPU Skinning Pipeline
- Extend Vertex3D with `joint_indices` ([u32;4]) and `joint_weights` ([f32;4])
- Add BonePaletteData UBO (256 bones max) to gbuffer.vert, shadow_vs, thumbnail_vs
- Identity skinning pattern: static meshes use weights=[1,0,0,0], bones[0]=I
- SkinningBackend: manages UBO allocation, descriptor sets, identity binding
- Bind bone palette at set 0 in geometry pass with last-bound tracking
- Identity palette for thumbnail_renderer and mesh_editor pipelines
- Multi-submesh rendering via `indices_for_path()` with per-submesh culling

## Phase 2: Animation System (src/engine/animation/)
- SkeletonInstance: bone hierarchy, local SQT transforms, world-space palette
- AnimationPlayer: clip playback with speed, looping, play/stop/reset
- Keyframe sampling: binary search + lerp (Vec3) / slerp (Quat)
- Forward kinematics: parent-before-child world transform propagation
- AnimationUpdateSystem registered in Stage::PreUpdate
- CrossfadeState: smooth blending between clips over configurable duration
- 154 tests passing (11 new animation/crossfade tests)

## Phase 3: Editor & Polish
- Inspector panel: Skeleton section (bone count, bone list, debug draw toggle)
- Inspector panel: Animation Player controls (play/stop/reset, time scrubber,
  speed slider, looping toggle)
- Bone debug visualization: parent→child lines + joint crosses via debug draw
- glTF skeleton extraction: bones, inverse bind matrices, skinning weights,
  animation channels (translation/rotation/scale keyframes)

## Files Changed (25 modified/created)
- New: src/engine/animation/{mod,components,sampling,system,debug_draw}.rs
- New: src/engine/rendering/3d/skinning.rs
- Modified: 6 shaders (gbuffer.vert, shadow_vs, thumbnail_vs, mesh_vs,
  lit_mesh_vs, pbr_vs)
- Modified: deferred_renderer, pipeline_3d, mesh, render_loop, app,
  standalone, inspector_panel, thumbnail_renderer, mesh_editor,
  model_loader_gltf, model_loader_fbx, mesh_import
