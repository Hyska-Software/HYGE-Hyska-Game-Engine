//! Immutable ECS snapshots and reflection metadata for the editor frontend.

use std::collections::{BTreeSet, HashSet};

use bevy_ecs::reflect::ReflectComponent;
use bevy_ecs::world::World;
use bevy_reflect::serde::ReflectSerializer;
use bevy_reflect::{TypeInfo, TypeRegistry};
use serde::{Deserialize, Serialize};

use hyge_core::result::{HygeError, HygeResult};
use hyge_scene::{Children, Name, Parent};

/// Opaque, process-stable representation of an ECS entity for IPC.
pub type EntityId = u64;

/// Complete immutable editor view of the current ECS world.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditorSnapshot {
    /// Monotonic in-memory revision used for stale edit detection.
    pub revision: u64,
    /// Persisted scene revision, kept separate from the in-memory revision.
    pub scene_revision: u64,
    /// Hierarchy nodes in deterministic order.
    pub hierarchy: Vec<HierarchyNode>,
    /// Reflected entity values in deterministic order.
    pub entities: Vec<EntitySnapshot>,
    /// All registered reflected components available to the editor.
    pub component_catalog: Vec<ComponentDescriptor>,
    /// Current engine-owned selection.
    pub selection: Vec<EntityId>,
    /// Non-fatal problems encountered while extracting values.
    pub diagnostics: Vec<SnapshotDiagnostic>,
}

/// One entity in the editor hierarchy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HierarchyNode {
    /// Entity identifier.
    pub entity: EntityId,
    /// Display name, falling back to the entity identifier.
    pub name: String,
    /// Parent entity, if present and alive.
    pub parent: Option<EntityId>,
    /// Children in the order stored by the ECS `Children` component.
    pub children: Vec<EntityId>,
}

/// Reflected components present on one entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntitySnapshot {
    /// Entity identifier.
    pub entity: EntityId,
    /// Components present on this entity.
    pub components: Vec<ReflectedComponent>,
}

/// One reflected component value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReflectedComponent {
    /// Stable catalog identifier.
    pub type_id: String,
    /// Full Bevy reflection type path.
    pub type_path: String,
    /// Serialized reflected value, when serialization succeeded.
    pub value: Option<serde_json::Value>,
    /// Serialization diagnostic, when the value could not be encoded.
    pub error: Option<String>,
}

/// Metadata for a reflected component exposed to the inspector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentDescriptor {
    /// Stable identifier derived from `type_path`.
    pub type_id: String,
    /// Full reflection type path.
    pub type_path: String,
    /// Short display name.
    pub short_name: String,
    /// Reflection shape (`struct`, `tuple`, `enum`, etc.).
    pub reflection_kind: String,
    /// Recursive field metadata.
    pub fields: Vec<FieldDescriptor>,
    /// Whether the registry has ECS reflection access for this type.
    pub has_reflect_component: bool,
    /// Whether the registry advertises reflection serialization.
    pub can_serialize: bool,
    /// Whether the registry advertises reflection deserialization.
    pub can_deserialize: bool,
    /// Whether the type can be edited through the future command model.
    pub editable: bool,
}

/// Reflected field metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDescriptor {
    /// Stable identifier derived from the complete field path.
    pub field_id: String,
    /// Dot-separated path relative to the component.
    pub field_path: String,
    /// Display name.
    pub name: String,
    /// Full reflected field type path.
    pub type_path: String,
    /// Nested reflected fields, when available from the registry.
    pub fields: Vec<FieldDescriptor>,
}

/// Non-fatal extraction diagnostic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotDiagnostic {
    /// Entity involved, if applicable.
    pub entity: Option<EntityId>,
    /// Component type involved, if applicable.
    pub type_path: Option<String>,
    /// Human-readable diagnostic.
    pub message: String,
}

/// Builds an immutable editor snapshot from an exclusive world borrow.
pub fn build_snapshot(
    world: &World,
    revision: u64,
    scene_revision: u64,
    selection: &[EntityId],
) -> HygeResult<EditorSnapshot> {
    let registry = world
        .get_resource::<bevy_ecs::reflect::AppTypeRegistry>()
        .ok_or_else(|| HygeError::invalid_argument("AppTypeRegistry resource not found"))?
        .0
        .read();

    let mut entity_ids: Vec<EntityId> = world
        .iter_entities()
        .map(|entity| entity.id().to_bits())
        .collect();
    entity_ids.sort_unstable();
    let alive: BTreeSet<EntityId> = entity_ids.iter().copied().collect();

    let (hierarchy, mut diagnostics) = build_hierarchy(world, &alive, &entity_ids);
    let component_catalog = build_catalog(&registry);
    let mut entities = Vec::new();

    for entity_bits in entity_ids {
        let entity = bevy_ecs::entity::Entity::from_bits(entity_bits);
        let mut components = Vec::new();
        for registration in registry.iter_with_data::<ReflectComponent>() {
            let type_path = registration.0.type_info().type_path().to_owned();
            let reflect_component = registration.1;
            let Some(value) = reflect_component.reflect(world.entity(entity)) else {
                continue;
            };
            let type_id = stable_id(&type_path);
            match serde_json::to_value(ReflectSerializer::new(value, &registry)) {
                Ok(value) => components.push(ReflectedComponent {
                    type_id,
                    type_path,
                    value: Some(value),
                    error: None,
                }),
                Err(error) => {
                    let message = format!("failed to serialize reflected component: {error}");
                    diagnostics.push(SnapshotDiagnostic {
                        entity: Some(entity_bits),
                        type_path: Some(type_path.clone()),
                        message: message.clone(),
                    });
                    components.push(ReflectedComponent {
                        type_id,
                        type_path,
                        value: None,
                        error: Some(message),
                    });
                }
            }
        }
        components.sort_by(|left, right| left.type_path.cmp(&right.type_path));
        entities.push(EntitySnapshot {
            entity: entity_bits,
            components,
        });
    }

    let mut filtered_selection: Vec<EntityId> = selection
        .iter()
        .copied()
        .filter(|entity| alive.contains(entity))
        .collect();
    filtered_selection.sort_unstable();

    Ok(EditorSnapshot {
        revision,
        scene_revision,
        hierarchy,
        entities,
        component_catalog,
        selection: filtered_selection,
        diagnostics,
    })
}

fn build_hierarchy(
    world: &World,
    alive: &BTreeSet<EntityId>,
    ids: &[EntityId],
) -> (Vec<HierarchyNode>, Vec<SnapshotDiagnostic>) {
    let mut nodes = Vec::with_capacity(ids.len());
    let mut diagnostics = Vec::new();
    for bits in ids {
        let entity = bevy_ecs::entity::Entity::from_bits(*bits);
        let name = world
            .get::<Name>(entity)
            .map(|name| name.0.clone())
            .unwrap_or_else(|| format!("Entity {bits}"));
        let parent = world.get::<Parent>(entity).map(|parent| parent.0.to_bits());
        let parent = parent.and_then(|parent| {
            if alive.contains(&parent) {
                Some(parent)
            } else {
                diagnostics.push(SnapshotDiagnostic {
                    entity: Some(*bits),
                    type_path: Some(std::any::type_name::<Parent>().to_owned()),
                    message: format!("parent entity {parent} is not alive"),
                });
                None
            }
        });
        let children = world
            .get::<Children>(entity)
            .map(|children| {
                children
                    .0
                    .iter()
                    .filter_map(|child| {
                        let child_bits = child.to_bits();
                        if alive.contains(&child_bits) {
                            Some(child_bits)
                        } else {
                            diagnostics.push(SnapshotDiagnostic {
                                entity: Some(*bits),
                                type_path: Some(std::any::type_name::<Children>().to_owned()),
                                message: format!("child entity {child_bits} is not alive"),
                            });
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        nodes.push(HierarchyNode {
            entity: *bits,
            name,
            parent,
            children,
        });
    }
    for node in &nodes {
        let mut seen = HashSet::new();
        let mut current = Some(node.entity);
        while let Some(entity) = current {
            if !seen.insert(entity) {
                diagnostics.push(SnapshotDiagnostic {
                    entity: Some(node.entity),
                    type_path: Some(std::any::type_name::<Parent>().to_owned()),
                    message: "cycle detected in Parent hierarchy".to_owned(),
                });
                break;
            }
            current = nodes
                .iter()
                .find(|candidate| candidate.entity == entity)
                .and_then(|candidate| candidate.parent);
        }
    }
    (nodes, diagnostics)
}

fn build_catalog(registry: &TypeRegistry) -> Vec<ComponentDescriptor> {
    let mut descriptors = registry
        .iter()
        .map(|registration| {
            let info = registration.type_info();
            let type_path = info.type_path().to_owned();
            let fields = describe_fields(registry, info, "", &type_path);
            let has_reflect_component = registration.data::<ReflectComponent>().is_some();
            let can_serialize = registration
                .data::<bevy_reflect::ReflectSerialize>()
                .is_some()
                || has_reflect_component;
            let can_deserialize = registration
                .data::<bevy_reflect::ReflectDeserialize>()
                .is_some()
                || has_reflect_component;
            ComponentDescriptor {
                type_id: stable_id(&type_path),
                short_name: info
                    .type_path_table()
                    .ident()
                    .unwrap_or(info.type_path())
                    .to_owned(),
                reflection_kind: reflection_kind(info).to_owned(),
                fields,
                has_reflect_component,
                can_serialize,
                can_deserialize,
                editable: can_deserialize && has_reflect_component,
                type_path,
            }
        })
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.type_path.cmp(&right.type_path));
    descriptors
}

fn describe_fields(
    registry: &TypeRegistry,
    info: &TypeInfo,
    prefix: &str,
    root_type_path: &str,
) -> Vec<FieldDescriptor> {
    let mut fields = Vec::new();
    if let TypeInfo::Struct(struct_info) = info {
        for field in struct_info.iter() {
            let path = if prefix.is_empty() {
                field.name().to_owned()
            } else {
                format!("{prefix}.{}", field.name())
            };
            let nested = registry
                .get_with_type_path(field.type_path())
                .map(|registration| {
                    describe_fields(registry, registration.type_info(), &path, root_type_path)
                })
                .unwrap_or_default();
            fields.push(FieldDescriptor {
                field_id: stable_id(&format!("{root_type_path}::{path}")),
                field_path: path,
                name: field.name().to_owned(),
                type_path: field.type_path().to_owned(),
                fields: nested,
            });
        }
    }
    fields.sort_by(|left, right| left.field_path.cmp(&right.field_path));
    fields
}

fn reflection_kind(info: &TypeInfo) -> &'static str {
    match info {
        TypeInfo::Struct(_) => "struct",
        TypeInfo::TupleStruct(_) => "tuple_struct",
        TypeInfo::Tuple(_) => "tuple",
        TypeInfo::List(_) => "list",
        TypeInfo::Array(_) => "array",
        TypeInfo::Map(_) => "map",
        TypeInfo::Enum(_) => "enum",
        TypeInfo::Value(_) => "value",
    }
}

fn stable_id(path: &str) -> String {
    blake3::hash(path.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::App;
    use hyge_ecs::plugin::HygePlugin;
    use hyge_scene::{build_scene_type_registry, ScenePlugin, Transform};

    #[test]
    fn snapshot_contains_real_hierarchy_and_reflected_fields() {
        let mut app = App::new();
        ScenePlugin.build(&mut app);
        let world = app.world_mut();
        let parent = world
            .spawn((Name("Root".into()), Transform::default()))
            .id();
        let child = world
            .spawn((Name("Child".into()), Transform::default()))
            .id();
        world.entity_mut(child).insert(Parent(parent));
        world.entity_mut(parent).insert(Children(vec![child]));

        let snapshot = build_snapshot(world, 7, 3, &[]).expect("snapshot");
        assert_eq!(snapshot.revision, 7);
        assert_eq!(snapshot.hierarchy.len(), 2);
        assert_eq!(snapshot.hierarchy[0].name, "Root");
        assert_eq!(snapshot.hierarchy[0].children, vec![child.to_bits()]);
        let transform = snapshot
            .component_catalog
            .iter()
            .find(|component| component.type_path.ends_with("::Transform"))
            .expect("registered Transform");
        assert!(transform
            .fields
            .iter()
            .any(|field| field.field_path == "translation"));
        assert_eq!(transform.type_id, stable_id(&transform.type_path));
        assert_eq!(
            transform.fields[0].field_id,
            stable_id(&format!(
                "{}::{}",
                transform.type_path, transform.fields[0].field_path
            ))
        );
        assert!(snapshot
            .entities
            .iter()
            .any(|entity| entity.entity == child.to_bits()));
        let _ = build_scene_type_registry;
    }
}
