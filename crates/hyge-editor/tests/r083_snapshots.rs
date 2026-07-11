//! R-083 evidence for real ECS hierarchy, reflection metadata and revisions.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use hyge_editor::EditorSessionRuntime;
use hyge_scene::{
    Environment, PostProcessProfile, Prefab, PrefabAssets, PrefabInstance, PrefabNode, Transform,
    WorldDocument,
};

static NEXT_PROJECT: AtomicU64 = AtomicU64::new(1);

struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hyge-r083-{}-{}",
            std::process::id(),
            NEXT_PROJECT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("create project");
        Self(path)
    }

    fn write_fixture(&self) -> PathBuf {
        let mut root = PrefabNode::named("Loaded Root");
        root.children.push(PrefabNode::named("Loaded Child"));
        let prefab = Prefab::new("r083-root", root, PrefabAssets::default());
        fs::write(
            self.0.join("root.hyge-prefab"),
            prefab.to_bytes().expect("prefab bytes"),
        )
        .expect("write prefab");
        let document = WorldDocument {
            env: Environment::empty(),
            root_prefab_instances: vec![PrefabInstance::new(
                prefab.prefab_id,
                Transform::identity(),
            )],
            post_process: PostProcessProfile::default(),
        };
        let scene = self.0.join("main.hyge-world");
        fs::write(&scene, document.to_bytes().expect("scene bytes")).expect("write scene");
        scene
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn loaded_scene_snapshot_contains_hierarchy_values_catalog_and_stable_revision() {
    let project = TempProject::new();
    let scene = project.write_fixture();
    let mut runtime = EditorSessionRuntime::new();
    runtime.open_project(&project.0).expect("open project");
    runtime.open_scene(&scene).expect("open scene");

    let first = runtime.editor_snapshot().expect("editor snapshot");
    let second = runtime.editor_snapshot().expect("editor snapshot");
    assert_eq!(first, second, "unchanged snapshots must be deterministic");
    assert_eq!(first.selection, Vec::<u64>::new());
    assert!(first
        .hierarchy
        .iter()
        .any(|node| node.name == "Loaded Root"));
    let child = first
        .hierarchy
        .iter()
        .find(|node| node.name == "Loaded Child")
        .expect("loaded child");
    let root = first
        .hierarchy
        .iter()
        .find(|node| node.name == "Loaded Root")
        .expect("loaded root");
    assert_eq!(child.parent, Some(root.entity));
    assert_eq!(root.children, vec![child.entity]);

    let transform = first
        .component_catalog
        .iter()
        .find(|component| component.type_path.ends_with("::Transform"))
        .expect("Transform catalog entry");
    assert!(transform.has_reflect_component);
    assert!(transform.can_serialize);
    assert!(transform.can_deserialize);
    assert!(transform
        .fields
        .iter()
        .any(|field| field.field_path == "translation"));
    assert!(first.entities.iter().any(|entity| {
        entity.entity == child.entity
            && entity.components.iter().any(|component| {
                component.type_id == transform.type_id && component.value.is_some()
            })
    }));

    let selected = runtime
        .select_entities(vec![child.entity, u64::MAX, child.entity])
        .expect("select entities");
    assert!(selected.revision > first.revision);
    assert_eq!(selected.selection, vec![child.entity]);
    let cleared = runtime
        .select_entities(vec![u64::MAX])
        .expect("clear stale selection");
    assert!(cleared.revision > selected.revision);
    assert!(cleared.selection.is_empty());

    let non_component = first
        .component_catalog
        .iter()
        .find(|component| component.type_path.ends_with("::RigidBodyKind"))
        .expect("registered non-component reflected type");
    assert!(!non_component.has_reflect_component);
    assert!(!non_component.editable);
}
