//! Create-menu templates: the minimal valid document per asset type, written
//! under a uniquified `New<Type>.<ext>` name (blend space ticket 03).

use crate::engine::animation::blend_space::{serialize_blend_space, BlendSpaceDoc};
use crate::engine::animation::graph::new_animgraph_doc;
use crate::engine::assets::mesh_import::MaterialDefinition;
use crate::engine::assets::AssetType;
use crate::engine::scene::scene_format::SceneFile;
use node_graph_types::GraphDoc;
use std::path::{Path, PathBuf};

/// `New<Type>.<ext>`, then `_1`, `_2`, … — the first name not yet taken in `dir`.
pub fn unique_asset_name(dir: &Path, base: &str, ext: &str) -> String {
    let mut name = format!("{base}.{ext}");
    let mut counter = 1;
    while dir.join(&name).exists() {
        name = format!("{base}_{counter}.{ext}");
        counter += 1;
    }
    name
}

/// `(base file name, extension)` for the types the Create menu offers.
pub fn template_name(asset_type: AssetType) -> Option<(&'static str, &'static str)> {
    Some(match asset_type {
        AssetType::Scene => ("NewScene", "scene"),
        AssetType::Material => ("NewMaterial", "material"),
        AssetType::Graph => ("NewGraph", "graph"),
        AssetType::AnimGraph => ("NewAnimGraph", "animgraph"),
        AssetType::BlendSpace => ("NewBlendSpace", "blendspace"),
        AssetType::Curve => ("NewCurve", "curve"),
        _ => return None,
    })
}

fn pretty() -> ron::ser::PrettyConfig {
    ron::ser::PrettyConfig::default()
}

/// The template text for `asset_type`; `name` is the asset's stem for the
/// formats that carry a name field.
pub fn template_text(asset_type: AssetType, name: &str) -> Result<String, String> {
    let ron_err = |e: ron::Error| e.to_string();
    match asset_type {
        AssetType::Scene => {
            let scene = SceneFile { version: "1.0".into(), name: name.into(), entities: Vec::new() };
            ron::ser::to_string_pretty(&scene, pretty()).map_err(ron_err)
        }
        AssetType::Material => {
            let def = MaterialDefinition {
                name: name.into(),
                base_color_factor: [1.0; 4],
                metallic_factor: 0.0,
                roughness_factor: 1.0,
                emissive_factor: [0.0; 3],
                albedo_texture: String::new(),
                normal_texture: String::new(),
                metallic_roughness_texture: String::new(),
                ao_texture: String::new(),
            };
            ron::ser::to_string_pretty(&def, pretty()).map_err(ron_err)
        }
        AssetType::Graph => node_graph_types::serialize_graph(&GraphDoc::default())
            .map_err(|e| e.to_string()),
        AssetType::AnimGraph => {
            node_graph_types::serialize_graph(&new_animgraph_doc()).map_err(|e| e.to_string())
        }
        AssetType::BlendSpace => serialize_blend_space(&BlendSpaceDoc::default()),
        AssetType::Curve => {
            curve_asset::serialize_curve(&curve_asset::CurveDoc::default()).map_err(|e| e.to_string())
        }
        other => Err(format!("no template for {}", other.display_name())),
    }
}

/// Write a fresh `asset_type` document into `dir` under a uniquified name and
/// return that file name. `Ok(None)` when the type has no template.
pub fn create_asset_file(dir: &Path, asset_type: AssetType) -> Result<Option<PathBuf>, String> {
    let Some((base, ext)) = template_name(asset_type) else {
        return Ok(None);
    };
    let file_name = unique_asset_name(dir, base, ext);
    let stem = file_name.trim_end_matches(&format!(".{ext}"));
    let text = template_text(asset_type, stem)?;
    std::fs::write(dir.join(&file_name), text).map_err(|e| e.to_string())?;
    Ok(Some(PathBuf::from(file_name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ab_templates_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn names_uniquify_with_numeric_suffixes() {
        let dir = scratch("uniq");
        assert_eq!(unique_asset_name(&dir, "NewBlendSpace", "blendspace"), "NewBlendSpace.blendspace");
        std::fs::write(dir.join("NewBlendSpace.blendspace"), "").expect("write");
        assert_eq!(unique_asset_name(&dir, "NewBlendSpace", "blendspace"), "NewBlendSpace_1.blendspace");
        std::fs::write(dir.join("NewBlendSpace_1.blendspace"), "").expect("write");
        assert_eq!(unique_asset_name(&dir, "NewBlendSpace", "blendspace"), "NewBlendSpace_2.blendspace");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every template parses back through its own loader.
    #[test]
    fn every_template_parses_through_its_loader() {
        use crate::engine::animation::blend_space::parse_blend_space;
        use crate::engine::assets::mesh_import::load_material_ron;
        let dir = scratch("parse");
        let created = |t: AssetType| -> PathBuf {
            let name = create_asset_file(&dir, t).expect("create").expect("template");
            dir.join(name)
        };

        let scene: SceneFile =
            ron::from_str(&std::fs::read_to_string(created(AssetType::Scene)).expect("read"))
                .expect("scene parses");
        assert_eq!((scene.name.as_str(), scene.entities.len()), ("NewScene", 0));

        let mat = load_material_ron(&created(AssetType::Material)).expect("material parses");
        assert_eq!((mat.name.as_str(), mat.roughness_factor), ("NewMaterial", 1.0));

        let graph = node_graph_types::load_graph(&created(AssetType::Graph)).expect("graph parses");
        assert!(graph.nodes.is_empty());

        let anim = node_graph_types::load_graph(&created(AssetType::AnimGraph)).expect("animgraph parses");
        assert_eq!(anim.nodes.len(), 2);

        let bs = parse_blend_space(&std::fs::read_to_string(created(AssetType::BlendSpace)).expect("read"))
            .expect("blend space parses");
        assert_eq!(bs, BlendSpaceDoc::default());

        let curve = curve_asset::parse_curve(&std::fs::read_to_string(created(AssetType::Curve)).expect("read"))
            .expect("curve parses");
        assert!(curve.tracks.is_empty());

        assert_eq!(create_asset_file(&dir, AssetType::Texture).expect("ok"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
