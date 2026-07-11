//! Transactional editor commands over the engine-owned ECS world.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::ReflectComponent;
use bevy_ecs::world::World;
use bevy_reflect::serde::{ReflectDeserializer, ReflectSerializer};
use hyge_core::result::HygeError;
use hyge_scene::{
    assign_new_scene_node_ids, Children, Parent, PrefabId, PrefabLibrary, SceneManagedEntity,
    SceneNodeId, Transform,
};
use serde::de::DeserializeSeed;
use serde::{Deserialize, Serialize};

use crate::snapshots::EntityId;

/// Structured failure returned by an editor command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandFailure {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable diagnostic.
    pub message: String,
}

impl CommandFailure {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn invalid_entity(entity: EntityId) -> Self {
        Self::new("invalid_entity", format!("entity {entity} is not alive"))
    }
}

/// Result metadata emitted after a command changes the world.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEffect {
    /// Entities touched by the operation.
    pub affected_entities: Vec<EntityId>,
    /// Entity IDs changed while restoring or duplicating state.
    pub entity_remappings: BTreeMap<EntityId, EntityId>,
}

/// Command interface implemented by every reversible editor operation.
pub trait Command {
    /// Applies the command, or returns a failure before mutation completes.
    fn apply(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure>;
    /// Reverts the command, or returns a failure before mutation completes.
    fn revert(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure>;
}

/// A reflected component edit. An empty field path replaces the whole value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditComponentCommand {
    /// Target entity.
    pub entity: EntityId,
    /// Reflected component type path.
    pub type_path: String,
    /// Optional dot-separated reflected field path.
    pub field_path: Option<String>,
    /// New complete component value or field value.
    pub value: serde_json::Value,
    #[serde(skip)]
    old_value: Option<serde_json::Value>,
}

/// Adds a reflected component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AddComponentCommand {
    /// Target entity.
    pub entity: EntityId,
    /// Reflected component type path.
    pub type_path: String,
    /// Serialized reflected value.
    pub value: serde_json::Value,
}

/// Removes a reflected component and retains it for undo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoveComponentCommand {
    /// Target entity.
    pub entity: EntityId,
    /// Reflected component type path.
    pub type_path: String,
    #[serde(skip)]
    old_value: Option<serde_json::Value>,
}

/// Changes an entity's parent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReparentCommand {
    /// Entity being moved.
    pub entity: EntityId,
    /// New parent, or `None` for a root.
    pub new_parent: Option<EntityId>,
    #[serde(skip)]
    old_parent: Option<EntityId>,
}

/// Instantiates a prefab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstantiateCommand {
    /// Prefab to instantiate.
    pub prefab: PrefabId,
    /// Root transform.
    pub transform: Transform,
    /// Optional parent entity.
    pub parent: Option<EntityId>,
    #[serde(skip)]
    spawned_root: Option<EntityId>,
}

/// Duplicates an entity subtree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateCommand {
    /// Entity subtree to copy.
    pub entity: EntityId,
    /// Created root after apply.
    #[serde(skip)]
    duplicated_root: Option<EntityId>,
}

/// Destroys an entity subtree and retains its complete state for undo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DestroyCommand {
    /// Root entity to destroy.
    pub entity: EntityId,
    #[serde(skip)]
    captured: Option<Subtree>,
}

impl EditComponentCommand {
    /// Creates a complete-component edit.
    #[must_use]
    pub fn new(entity: EntityId, type_path: impl Into<String>, value: serde_json::Value) -> Self {
        Self {
            entity,
            type_path: type_path.into(),
            field_path: None,
            value,
            old_value: None,
        }
    }
}

impl AddComponentCommand {
    /// Creates an add-component command.
    #[must_use]
    pub fn new(entity: EntityId, type_path: impl Into<String>, value: serde_json::Value) -> Self {
        Self {
            entity,
            type_path: type_path.into(),
            value,
        }
    }
}

impl RemoveComponentCommand {
    /// Creates a remove-component command.
    #[must_use]
    pub fn new(entity: EntityId, type_path: impl Into<String>) -> Self {
        Self {
            entity,
            type_path: type_path.into(),
            old_value: None,
        }
    }
}

impl ReparentCommand {
    /// Creates a reparent command.
    #[must_use]
    pub fn new(entity: EntityId, new_parent: Option<EntityId>) -> Self {
        Self {
            entity,
            new_parent,
            old_parent: None,
        }
    }
}

impl InstantiateCommand {
    /// Creates a prefab-instantiation command.
    #[must_use]
    pub fn new(prefab: PrefabId, transform: Transform, parent: Option<EntityId>) -> Self {
        Self {
            prefab,
            transform,
            parent,
            spawned_root: None,
        }
    }
}

impl DuplicateCommand {
    /// Creates a subtree-duplication command.
    #[must_use]
    pub fn new(entity: EntityId) -> Self {
        Self {
            entity,
            duplicated_root: None,
        }
    }
}

impl DestroyCommand {
    /// Creates a subtree-destruction command.
    #[must_use]
    pub fn new(entity: EntityId) -> Self {
        Self {
            entity,
            captured: None,
        }
    }
}

/// All supported editor commands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EditorCommand {
    /// Edit a reflected component.
    EditComponent(EditComponentCommand),
    /// Reparent an entity.
    Reparent(ReparentCommand),
    /// Instantiate a prefab.
    Instantiate(InstantiateCommand),
    /// Destroy an entity subtree.
    Destroy(DestroyCommand),
    /// Duplicate an entity subtree.
    Duplicate(DuplicateCommand),
    /// Add a component.
    AddComponent(AddComponentCommand),
    /// Remove a component.
    RemoveComponent(RemoveComponentCommand),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Subtree {
    root: EntityId,
    nodes: Vec<CapturedEntity>,
    parent: Option<EntityId>,
    parent_children_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CapturedEntity {
    entity: EntityId,
    parent: Option<EntityId>,
    children: Vec<EntityId>,
    components: Vec<CapturedComponent>,
    scene_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CapturedComponent {
    type_path: String,
    value: serde_json::Value,
}

impl Command for EditorCommand {
    fn apply(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        match self {
            Self::EditComponent(command) => command.apply(world),
            Self::Reparent(command) => command.apply(world),
            Self::Instantiate(command) => command.apply(world),
            Self::Destroy(command) => command.apply(world),
            Self::Duplicate(command) => command.apply(world),
            Self::AddComponent(command) => command.apply(world),
            Self::RemoveComponent(command) => command.apply(world),
        }
    }

    fn revert(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        match self {
            Self::EditComponent(command) => command.revert(world),
            Self::Reparent(command) => command.revert(world),
            Self::Instantiate(command) => command.revert(world),
            Self::Destroy(command) => command.revert(world),
            Self::Duplicate(command) => command.revert(world),
            Self::AddComponent(command) => command.revert(world),
            Self::RemoveComponent(command) => command.revert(world),
        }
    }
}

impl Command for EditComponentCommand {
    fn apply(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let entity = entity(world, self.entity)?;
        let current = reflected_value(world, entity, &self.type_path)?;
        if self.old_value.is_none() {
            self.old_value = Some(current.clone());
        }
        let value = if let Some(path) = self.field_path.as_deref().filter(|p| !p.is_empty()) {
            replace_field(current, path, self.value.clone())?
        } else {
            self.value.clone()
        };
        insert_reflected(world, entity, &self.type_path, value)?;
        Ok(effect([self.entity]))
    }

    fn revert(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let entity = entity(world, self.entity)?;
        let value = self
            .old_value
            .clone()
            .ok_or_else(|| CommandFailure::new("command_failed", "edit has no captured value"))?;
        insert_reflected(world, entity, &self.type_path, value)?;
        Ok(effect([self.entity]))
    }
}

impl Command for AddComponentCommand {
    fn apply(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let entity = entity(world, self.entity)?;
        if reflected_value(world, entity, &self.type_path).is_ok() {
            return Err(CommandFailure::new(
                "invalid_component",
                "component already exists",
            ));
        }
        insert_reflected(world, entity, &self.type_path, self.value.clone())?;
        Ok(effect([self.entity]))
    }

    fn revert(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let entity = entity(world, self.entity)?;
        remove_reflected(world, entity, &self.type_path)?;
        Ok(effect([self.entity]))
    }
}

impl Command for RemoveComponentCommand {
    fn apply(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let entity = entity(world, self.entity)?;
        let value = reflected_value(world, entity, &self.type_path)?;
        self.old_value = Some(value);
        remove_reflected(world, entity, &self.type_path)?;
        Ok(effect([self.entity]))
    }

    fn revert(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let entity = entity(world, self.entity)?;
        let value = self
            .old_value
            .clone()
            .ok_or_else(|| CommandFailure::new("command_failed", "remove has no captured value"))?;
        insert_reflected(world, entity, &self.type_path, value)?;
        Ok(effect([self.entity]))
    }
}

impl Command for ReparentCommand {
    fn apply(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let entity = entity(world, self.entity)?;
        validate_parent(world, entity, self.new_parent)?;
        self.old_parent = world.get::<Parent>(entity).map(|p| p.0.to_bits());
        set_parent(world, entity, self.new_parent)?;
        let mut affected = vec![self.entity];
        if let Some(parent) = self.old_parent {
            affected.push(parent);
        }
        if let Some(parent) = self.new_parent {
            affected.push(parent);
        }
        Ok(effect_vec(affected))
    }

    fn revert(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let entity = entity(world, self.entity)?;
        validate_parent(world, entity, self.old_parent)?;
        set_parent(world, entity, self.old_parent)?;
        Ok(effect([self.entity]))
    }
}

impl Command for InstantiateCommand {
    fn apply(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let library = world
            .get_resource::<PrefabLibrary>()
            .cloned()
            .ok_or_else(|| CommandFailure::new("scene_unavailable", "PrefabLibrary is missing"))?;
        let prefab = library
            .get(&self.prefab)
            .ok_or_else(|| CommandFailure::new("invalid_component", "prefab is not registered"))?
            .clone();
        {
            let type_registry = world
                .get_resource::<bevy_ecs::reflect::AppTypeRegistry>()
                .ok_or_else(|| {
                    CommandFailure::new("reflection_error", "AppTypeRegistry is missing")
                })?
                .0
                .read();
            prefab.hydrate(&type_registry).map_err(map_error)?;
        }
        let parent = self.parent.map(|id| entity(world, id)).transpose()?;
        let root = prefab
            .instantiate(world, self.transform, parent)
            .map_err(map_error)?;
        assign_new_scene_node_ids(world, root).map_err(map_error)?;
        self.spawned_root = Some(root.to_bits());
        let mut affected = vec![root.to_bits()];
        if let Some(parent) = parent {
            affected.push(parent.to_bits());
        }
        Ok(effect_vec(affected))
    }

    fn revert(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let root = self.spawned_root.ok_or_else(|| {
            CommandFailure::new("command_failed", "instantiate has no spawned entity")
        })?;
        destroy_subtree(world, entity(world, root)?)?;
        Ok(effect([root]))
    }
}

impl Command for DestroyCommand {
    fn apply(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let root = entity(world, self.entity)?;
        let captured = capture_subtree(world, root)?;
        destroy_subtree(world, root)?;
        let mut affected: Vec<_> = captured.nodes.iter().map(|node| node.entity).collect();
        if let Some(parent) = captured.parent {
            affected.push(parent);
        }
        self.captured = Some(captured);
        Ok(effect_vec(affected))
    }

    fn revert(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let captured = self.captured.clone().ok_or_else(|| {
            CommandFailure::new("command_failed", "destroy has no captured subtree")
        })?;
        restore_subtree(world, &captured, true)?;
        Ok(effect_vec(captured.nodes.iter().map(|n| n.entity)))
    }
}

impl Command for DuplicateCommand {
    fn apply(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let source = entity(world, self.entity)?;
        let captured = capture_subtree(world, source)?;
        let mapping = restore_subtree(world, &captured, false)?;
        let root = *mapping.get(&self.entity).ok_or_else(|| {
            CommandFailure::new("command_failed", "duplicate root was not restored")
        })?;
        self.duplicated_root = Some(root);
        Ok(CommandEffect {
            affected_entities: mapping.values().copied().collect(),
            entity_remappings: mapping,
        })
    }

    fn revert(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let root = self.duplicated_root.ok_or_else(|| {
            CommandFailure::new("command_failed", "duplicate has no created entity")
        })?;
        destroy_subtree(world, entity(world, root)?)?;
        Ok(effect([root]))
    }
}

fn entity(world: &World, bits: EntityId) -> Result<Entity, CommandFailure> {
    let entity = Entity::try_from_bits(bits).map_err(|_| CommandFailure::invalid_entity(bits))?;
    world
        .get_entity(entity)
        .map(|_| entity)
        .ok_or_else(|| CommandFailure::invalid_entity(bits))
}

fn effect<const N: usize>(entities: [EntityId; N]) -> CommandEffect {
    CommandEffect {
        affected_entities: entities.into_iter().collect(),
        entity_remappings: BTreeMap::new(),
    }
}

fn effect_vec(entities: impl IntoIterator<Item = EntityId>) -> CommandEffect {
    CommandEffect {
        affected_entities: entities.into_iter().collect(),
        entity_remappings: BTreeMap::new(),
    }
}

fn registry(world: &World) -> Result<bevy_reflect::TypeRegistryArc, CommandFailure> {
    world
        .get_resource::<bevy_ecs::reflect::AppTypeRegistry>()
        .map(|r| r.0.clone())
        .ok_or_else(|| CommandFailure::new("reflection_error", "AppTypeRegistry is missing"))
}

fn registration(
    world: &World,
    type_path: &str,
) -> Result<(bevy_reflect::TypeRegistryArc, ReflectComponent), CommandFailure> {
    let registry = registry(world)?;
    let read = registry.read();
    let registration = read.get_with_type_path(type_path).ok_or_else(|| {
        CommandFailure::new(
            "invalid_component",
            format!("component type '{type_path}' is not registered"),
        )
    })?;
    let component = registration
        .data::<ReflectComponent>()
        .ok_or_else(|| {
            CommandFailure::new(
                "invalid_component",
                format!("'{type_path}' is not an ECS component"),
            )
        })?
        .clone();
    drop(read);
    Ok((registry, component))
}

fn reflected_value(
    world: &World,
    entity: Entity,
    type_path: &str,
) -> Result<serde_json::Value, CommandFailure> {
    let (registry, component) = registration(world, type_path)?;
    let value = component.reflect(world.entity(entity)).ok_or_else(|| {
        CommandFailure::new("invalid_component", "component is not present on entity")
    })?;
    let serialized = serde_json::to_value(ReflectSerializer::new(value, &registry.read()))
        .map_err(|e| CommandFailure::new("reflection_error", e.to_string()));
    serialized
}

fn insert_reflected(
    world: &mut World,
    entity: Entity,
    type_path: &str,
    value: serde_json::Value,
) -> Result<(), CommandFailure> {
    let (registry, component) = registration(world, type_path)?;
    let value_text = value.to_string();
    let mut deserializer = serde_json::Deserializer::from_str(&value_text);
    let reflect = DeserializeSeed::deserialize(
        ReflectDeserializer::new(&registry.read()),
        &mut deserializer,
    )
    .map_err(|e| CommandFailure::new("reflection_error", e.to_string()))?;
    component.insert(
        &mut world.entity_mut(entity),
        reflect.as_ref(),
        &registry.read(),
    );
    Ok(())
}

fn remove_reflected(
    world: &mut World,
    entity: Entity,
    type_path: &str,
) -> Result<(), CommandFailure> {
    let (_, component) = registration(world, type_path)?;
    component.remove(&mut world.entity_mut(entity));
    Ok(())
}

fn replace_field(
    current: serde_json::Value,
    path: &str,
    value: serde_json::Value,
) -> Result<serde_json::Value, CommandFailure> {
    let mut root = current;
    let mut target = &mut root;
    let parts: Vec<&str> = path.split('.').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        target = target.get_mut(*part).ok_or_else(|| {
            CommandFailure::new(
                "reflection_error",
                format!("field path '{path}' does not exist"),
            )
        })?;
    }
    let Some(last) = parts.last() else {
        return Err(CommandFailure::new(
            "reflection_error",
            "field path is empty",
        ));
    };
    let object = target.as_object_mut().ok_or_else(|| {
        CommandFailure::new("reflection_error", "field path does not address an object")
    })?;
    if !object.contains_key(*last) {
        return Err(CommandFailure::new(
            "reflection_error",
            format!("field '{last}' does not exist"),
        ));
    }
    object.insert((*last).to_owned(), value);
    Ok(root)
}

fn validate_parent(
    world: &World,
    child: Entity,
    parent: Option<EntityId>,
) -> Result<(), CommandFailure> {
    let Some(parent_bits) = parent else {
        return Ok(());
    };
    let parent = entity(world, parent_bits)?;
    if child == parent {
        return Err(CommandFailure::new(
            "cycle_detected",
            "an entity cannot parent itself",
        ));
    }
    let mut current = Some(parent);
    let mut seen = BTreeSet::new();
    while let Some(candidate) = current {
        if !seen.insert(candidate.to_bits()) {
            return Err(CommandFailure::new(
                "cycle_detected",
                "existing hierarchy contains a cycle",
            ));
        }
        if candidate == child {
            return Err(CommandFailure::new(
                "cycle_detected",
                "parent would create a cycle",
            ));
        }
        current = world.get::<Parent>(candidate).map(|p| p.0);
    }
    Ok(())
}

fn set_parent(
    world: &mut World,
    child: Entity,
    parent: Option<EntityId>,
) -> Result<(), CommandFailure> {
    if let Some(old) = world.get::<Parent>(child).map(|p| p.0) {
        if let Some(mut children) = world.get_mut::<Children>(old) {
            children.0.retain(|candidate| *candidate != child);
            if children.0.is_empty() {
                world.entity_mut(old).remove::<Children>();
            }
        }
    }
    if let Some(parent_bits) = parent {
        let parent = entity(world, parent_bits)?;
        world.entity_mut(child).insert(Parent(parent));
        if let Some(mut children) = world.get_mut::<Children>(parent) {
            if !children.0.contains(&child) {
                children.0.push(child);
            }
        } else {
            world.entity_mut(parent).insert(Children(vec![child]));
        }
    } else {
        world.entity_mut(child).remove::<Parent>();
    }
    Ok(())
}

fn capture_subtree(world: &World, root: Entity) -> Result<Subtree, CommandFailure> {
    let mut ids = Vec::new();
    collect_subtree(world, root, &mut ids)?;
    let parent = world.get::<Parent>(root).map(|p| p.0.to_bits());
    let parent_children_index = parent.and_then(|bits| {
        world
            .get::<Children>(Entity::from_bits(bits))
            .and_then(|c| c.0.iter().position(|e| *e == root))
    });
    let nodes = ids
        .into_iter()
        .map(|entity| capture_entity(world, entity))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Subtree {
        root: root.to_bits(),
        nodes,
        parent,
        parent_children_index,
    })
}

fn collect_subtree(
    world: &World,
    current: Entity,
    ids: &mut Vec<Entity>,
) -> Result<(), CommandFailure> {
    entity(world, current.to_bits())?;
    ids.push(current);
    let children = world
        .get::<Children>(current)
        .map(|c| c.0.clone())
        .unwrap_or_default();
    for child in children {
        collect_subtree(world, child, ids)?;
    }
    Ok(())
}

fn capture_entity(world: &World, entity: Entity) -> Result<CapturedEntity, CommandFailure> {
    let registry = registry(world)?;
    let read = registry.read();
    let mut components = Vec::new();
    for registration in read.iter_with_data::<ReflectComponent>() {
        let type_path = registration.0.type_info().type_path().to_owned();
        if type_path.ends_with("::Parent")
            || type_path.ends_with("::Children")
            || type_path.ends_with("::SceneNodeId")
        {
            continue;
        }
        let Some(value) = registration.1.reflect(world.entity(entity)) else {
            continue;
        };
        let value = serde_json::to_value(ReflectSerializer::new(value, &read))
            .map_err(|e| CommandFailure::new("reflection_error", e.to_string()))?;
        components.push(CapturedComponent { type_path, value });
    }
    components.sort_by(|a, b| a.type_path.cmp(&b.type_path));
    Ok(CapturedEntity {
        entity: entity.to_bits(),
        parent: world.get::<Parent>(entity).map(|p| p.0.to_bits()),
        children: world
            .get::<Children>(entity)
            .map(|c| c.0.iter().map(|e| e.to_bits()).collect())
            .unwrap_or_default(),
        components,
        scene_id: world.get::<SceneNodeId>(entity).map(|id| id.0.clone()),
    })
}

fn restore_subtree(
    world: &mut World,
    subtree: &Subtree,
    preserve_ids: bool,
) -> Result<BTreeMap<EntityId, EntityId>, CommandFailure> {
    let mut mapping = BTreeMap::new();
    for node in &subtree.nodes {
        let target = if preserve_ids {
            world
                .get_or_spawn(Entity::from_bits(node.entity))
                .ok_or_else(|| {
                    CommandFailure::new("command_failed", "could not restore entity ID")
                })?
                .id()
        } else {
            world.spawn_empty().id()
        };
        mapping.insert(node.entity, target.to_bits());
    }
    for node in &subtree.nodes {
        let target = Entity::from_bits(mapping[&node.entity]);
        if preserve_ids {
            if let Some(scene_id) = &node.scene_id {
                world
                    .entity_mut(target)
                    .insert(SceneNodeId::new(scene_id.clone()));
            }
        } else {
            world.entity_mut(target).insert(SceneManagedEntity);
        }
        for component in &node.components {
            insert_reflected(world, target, &component.type_path, component.value.clone())?;
        }
    }
    for node in &subtree.nodes {
        let target = Entity::from_bits(mapping[&node.entity]);
        let parent = node.parent.and_then(|p| mapping.get(&p).copied()).or({
            if node.entity == subtree.root {
                subtree.parent
            } else {
                None
            }
        });
        set_parent(world, target, parent)?;
    }
    if !preserve_ids {
        assign_new_scene_node_ids(world, Entity::from_bits(mapping[&subtree.root]))
            .map_err(map_error)?;
    }
    if let Some(index) = subtree.parent_children_index {
        if let Some(parent) = subtree.parent {
            let parent = Entity::from_bits(mapping.get(&parent).copied().unwrap_or(parent));
            if let Some(mut children) = world.get_mut::<Children>(parent) {
                if let Some(pos) = children
                    .0
                    .iter()
                    .position(|e| *e == Entity::from_bits(mapping[&subtree.root]))
                {
                    let child = children.0.remove(pos);
                    let desired = if preserve_ids {
                        index
                    } else {
                        index.saturating_add(1)
                    };
                    let insert_at = desired.min(children.0.len());
                    children.0.insert(insert_at, child);
                }
            }
        }
    }
    Ok(mapping)
}

fn destroy_subtree(world: &mut World, root: Entity) -> Result<(), CommandFailure> {
    let captured = capture_subtree(world, root)?;
    if let Some(parent) = captured.parent {
        if let Some(mut children) = world.get_mut::<Children>(Entity::from_bits(parent)) {
            children.0.retain(|e| *e != root);
            if children.0.is_empty() {
                world
                    .entity_mut(Entity::from_bits(parent))
                    .remove::<Children>();
            }
        }
    }
    for node in captured.nodes.iter().rev() {
        if !world.despawn(Entity::from_bits(node.entity)) {
            return Err(CommandFailure::invalid_entity(node.entity));
        }
    }
    Ok(())
}

fn map_error(error: HygeError) -> CommandFailure {
    CommandFailure::new("command_failed", error.to_string())
}
