//! M5 end-to-end smoke test for the full gameplay stack.

use std::{
    any::type_name,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bevy_app::App;
use bevy_ecs::{event::Events, reflect::AppTypeRegistry, world::Mut};
use bevy_reflect::Reflect;
use hyge_asset::AssetId;
use hyge_audio::prelude::{AudioPlugin, AudioServer, AudioSource, BusKind, PlaySound};
use hyge_ecs::{prelude::*, schedule::Label};
use hyge_input::prelude::{Action, ActionMap, InputConfig, InputPlugin, Vec2Action};
use hyge_physics::prelude::{
    CharacterController as PhysicsCharacterController, Collider as PhysicsCollider,
    ColliderShape as PhysicsColliderShape, PhysicsPosition, PhysicsVelocity, RapierPhysicsWorld,
    RigidBody as PhysicsRigidBody, RigidBodyKind as PhysicsRigidBodyKind,
};
use hyge_scene::prelude::{
    load_world_document_from_path, reload_loaded_scene_from_disk, Environment, Name, PointLight,
    PostProcessProfile, Prefab, PrefabAssets, PrefabInstance, PrefabLibrary, PrefabNode,
    SerializedComponentOverride, StaticMeshAssetRefs, Transform, WorldDocument,
};
use hyge_script::prelude::{
    ScriptConfig, ScriptError, ScriptInput, ScriptPlugin, ScriptRef, ScriptRuntime, ScriptState,
};
use hyge_window::events::{DeviceEvent, DeviceEventKind};

struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hyge_m5_smoke_{suffix}"));
        fs::create_dir_all(&root).expect("temporary M5 project should be created");
        Self(root)
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture parent should be created");
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .expect("fixture should open");
    file.write_all(contents.as_bytes())
        .expect("fixture should be written");
    file.sync_all().expect("fixture should be flushed");
}

fn override_component<T: Reflect + 'static>(
    value: &T,
    registry: &bevy_reflect::TypeRegistry,
) -> SerializedComponentOverride {
    SerializedComponentOverride::new(type_name::<T>(), value, registry)
        .expect("M5 component should serialize through reflection")
}

fn build_complex_prefab(registry: &bevy_reflect::TypeRegistry) -> Prefab {
    let mesh = AssetId::from(blake3::hash(b"m5-complex-mesh"));
    let material = AssetId::from(blake3::hash(b"m5-complex-material"));
    let mut root = PrefabNode::named("m5-complex-asset");
    root.components
        .push(override_component(&Name::new("m5-complex-asset"), registry));
    root.components.push(override_component(
        &StaticMeshAssetRefs::new(mesh, material),
        registry,
    ));
    root.components.push(override_component(
        &AudioSource {
            clip: "music/m5_theme.ogg".into(),
            bus: BusKind::Music,
            spatial: true,
            volume: 0.6,
            pitch: 1.0,
            range: 30.0,
            rolloff: hyge_audio::prelude::AudioRolloff::Inverse,
        },
        registry,
    ));
    root.components
        .push(override_component(&PhysicsRigidBody::default(), registry));
    root.components.push(override_component(
        &PhysicsCollider {
            shape: PhysicsColliderShape::Ball(0.5),
            ..PhysicsCollider::default()
        },
        registry,
    ));
    root.components.push(override_component(
        &ScriptRef::new("assets/scripts/player.lua"),
        registry,
    ));
    let mut child = PrefabNode::named("m5-point-light");
    child
        .components
        .push(override_component(&PointLight::default(), registry));
    root.children.push(child);
    Prefab::new(
        "m5-complex-prefab",
        root,
        PrefabAssets {
            meshes: vec![mesh],
            materials: vec![material],
            scripts: vec![AssetId::from(blake3::hash(b"assets/scripts/player.lua"))],
        },
    )
}

fn build_world(prefab: &Prefab) -> WorldDocument {
    WorldDocument {
        env: Environment::default(),
        root_prefab_instances: (0..5)
            .map(|index| {
                PrefabInstance::new(
                    prefab.prefab_id,
                    Transform {
                        translation: [index as f32 * 3.0, 4.0, 0.0],
                        ..Transform::identity()
                    },
                )
            })
            .collect(),
        post_process: PostProcessProfile::default(),
        editor_layer: None,
    }
}

fn register_runtime_types(world: &mut bevy_ecs::world::World) {
    let registry = world.resource::<AppTypeRegistry>().0.clone();
    let mut registry = registry.write();
    registry.register::<hyge_audio::prelude::AudioSource>();
    registry.register::<hyge_audio::prelude::AudioRolloff>();
    registry.register::<BusKind>();
    registry.register::<PhysicsRigidBody>();
    registry.register::<PhysicsRigidBodyKind>();
    registry.register::<PhysicsCollider>();
    registry.register::<PhysicsColliderShape>();
    registry.register::<PhysicsCharacterController>();
    registry.register::<PhysicsPosition>();
    registry.register::<PhysicsVelocity>();
}

fn build_headless_app(project: &TempProject, input_path: PathBuf) -> App {
    let mut app = App::new();
    app.init_schedule(Label::PreUpdate);
    app.init_schedule(Label::FixedUpdate);
    app.init_schedule(Label::Update);
    app.init_schedule(Label::Last);
    app.add_event::<DeviceEvent>();
    hyge_scene::prelude::ScenePlugin.build(&mut app);
    register_runtime_types(app.world_mut());
    hyge_physics::prelude::PhysicsPlugin::default().build(&mut app);
    AudioPlugin.build(&mut app);
    InputPlugin::new(InputConfig {
        binding_file: Some(input_path),
        enable_hot_reload: true,
    })
    .build(&mut app);
    ScriptPlugin::new(ScriptConfig::default())
        .with_project_root(&project.0)
        .build(&mut app);
    app
}

#[test]
fn m5_full_systems_scene_loads_and_hot_reloads() {
    let project = TempProject::new();
    let input_path = project.path("assets/input.bind.toml");
    write_file(
        &input_path,
        r#"[actions.move]
type = "vec2"
bindings = [
  { kind = "keyboard", key = "w", axis = "y", scale = 1.0 },
  { kind = "keyboard", key = "s", axis = "y", scale = -1.0 },
  { kind = "keyboard", key = "a", axis = "x", scale = -1.0 },
  { kind = "keyboard", key = "d", axis = "x", scale = 1.0 },
  { kind = "gamepad_axis", id = 0, axis = 0, scale = 1.0 },
]

[actions.look]
type = "vec2"
bindings = [{ kind = "mouse_delta", scale = 0.1 }]

[actions.jump]
type = "button"
bindings = [{ kind = "gamepad_button", id = 0, button = 0 }]
"#,
    );

    let mut app = build_headless_app(&project, input_path.clone());
    let registry = app.world().resource::<AppTypeRegistry>().0.read();
    let prefab = build_complex_prefab(&registry);
    let world_document = build_world(&prefab);
    let prefab_path = project.path("assets/prefabs/m5_complex.hyge-prefab");
    let world_path = project.path("assets/scenes/m5_smoke.hyge-world");
    fs::create_dir_all(prefab_path.parent().expect("prefab parent")).expect("prefab dir");
    fs::create_dir_all(world_path.parent().expect("world parent")).expect("world dir");
    fs::write(&prefab_path, prefab.to_bytes().expect("prefab bytes")).expect("prefab file");
    fs::write(
        &world_path,
        world_document.to_bytes().expect("world document bytes"),
    )
    .expect("world file");
    drop(registry);
    app.world_mut().resource_mut::<PrefabLibrary>().insert(
        Prefab::from_bytes(&fs::read(&prefab_path).expect("prefab read")).expect("prefab load"),
    );

    let roots = load_world_document_from_path(app.world_mut(), &world_path).expect("world loads");
    assert_eq!(
        roots.len(),
        5,
        "M5 scene must instantiate five prefab roots"
    );
    assert_eq!(
        app.world()
            .resource::<hyge_scene::prelude::LoadedSceneState>()
            .root_entities
            .len(),
        5
    );
    let player = roots[0];
    let player_bits = player.to_bits();
    let script_path = project.path("assets/scripts/player.lua");
    write_file(&script_path, "return { state = { version = 1 }, on_update = function(entity, dt) local velocity = hyge.world.get(entity, 'PhysicsVelocity'); velocity.linear[1] = hyge.input.action('move'); hyge.world.set(entity, 'PhysicsVelocity', velocity) end }");
    app.world_mut().entity_mut(player).insert((
        PhysicsPosition::from_translation(hyge_core::prelude::Vec3::new(0.0, 4.0, 0.0)),
        PhysicsVelocity::default(),
        PhysicsCharacterController::default(),
    ));
    let mut dynamic_entities = vec![player];
    for index in 1..100 {
        let entity = app
            .world_mut()
            .spawn((
                PhysicsRigidBody::default(),
                PhysicsCollider {
                    shape: PhysicsColliderShape::Ball(0.5),
                    ..PhysicsCollider::default()
                },
                PhysicsPosition::from_translation(hyge_core::prelude::Vec3::new(
                    (index % 10) as f32 * 2.0 - 9.0,
                    5.0 + (index / 10) as f32,
                    (index / 10) as f32 * 2.0 - 9.0,
                )),
                PhysicsVelocity::default(),
            ))
            .id();
        dynamic_entities.push(entity);
    }
    assert_eq!(dynamic_entities.len(), 100);

    let ground = app
        .world_mut()
        .spawn((
            PhysicsRigidBody {
                kind: PhysicsRigidBodyKind::Fixed,
                ..PhysicsRigidBody::default()
            },
            PhysicsCollider {
                shape: PhysicsColliderShape::Cuboid([20.0, 0.5, 20.0]),
                ..PhysicsCollider::default()
            },
            PhysicsPosition::from_translation(hyge_core::prelude::Vec3::new(0.0, -0.5, 0.0)),
        ))
        .id();
    let ground_data = {
        let world = app.world_mut();
        (
            *world.get::<PhysicsRigidBody>(ground).expect("ground body"),
            world
                .get::<PhysicsCollider>(ground)
                .expect("ground collider")
                .clone(),
            *world
                .get::<PhysicsPosition>(ground)
                .expect("ground position"),
        )
    };
    let dynamic_data: Vec<_> = {
        let world = app.world_mut();
        dynamic_entities
            .iter()
            .map(|entity| {
                (
                    *entity,
                    *world.get::<PhysicsRigidBody>(*entity).expect("body"),
                    world
                        .get::<PhysicsCollider>(*entity)
                        .expect("collider")
                        .clone(),
                    *world.get::<PhysicsPosition>(*entity).expect("position"),
                )
            })
            .collect()
    };
    {
        let world = app.world_mut();
        let mut rapier = world.resource_mut::<RapierPhysicsWorld>();
        rapier.ensure_body(ground, &ground_data.0, &ground_data.1, ground_data.2);
        for (entity, body, collider, position) in &dynamic_data {
            rapier.ensure_body(*entity, body, collider, *position);
        }
    }
    for _ in 0..240 {
        app.world_mut().run_schedule(Label::FixedUpdate);
    }
    assert!(
        !app.world()
            .resource::<Events<hyge_physics::CollisionEvent>>()
            .is_empty(),
        "Rapier collision transitions must reach ECS events"
    );

    app.world_mut()
        .spawn(hyge_audio::prelude::AudioListener::default());
    for index in 0..11 {
        app.world_mut().spawn(AudioSource {
            clip: format!("sfx/emitter_{index}.ogg"),
            spatial: true,
            ..AudioSource::default()
        });
    }
    app.world_mut().spawn(AudioSource {
        clip: "music/m5_theme.ogg".into(),
        bus: BusKind::Music,
        ..AudioSource::default()
    });
    app.world_mut().run_schedule(Label::Update);
    let spatial_count = {
        let world = app.world_mut();
        world
            .query::<&AudioSource>()
            .iter(world)
            .filter(|source| source.spatial)
            .count()
    };
    assert_eq!(spatial_count, 16);
    let music = {
        let world = app.world_mut();
        world
            .query::<(Entity, &AudioSource)>()
            .iter(world)
            .find_map(|(entity, source)| (source.bus == BusKind::Music).then_some(entity))
            .expect("music source")
    };
    app.world_mut().send_event(PlaySound {
        source: music,
        volume: 1.0,
        pitch: 1.0,
    });
    app.world_mut().run_schedule(Label::Update);
    assert_eq!(
        app.world().resource::<AudioServer>().play_request_count(),
        1
    );
    assert_eq!(
        app.world()
            .resource::<AudioServer>()
            .spatial_emitter_count(),
        16
    );

    app.world_mut().send_event(DeviceEvent {
        event: DeviceEventKind::Key {
            scancode: 0,
            key: "w".into(),
            pressed: true,
        },
    });
    app.world_mut().send_event(DeviceEvent {
        event: DeviceEventKind::Key {
            scancode: 0,
            key: "d".into(),
            pressed: true,
        },
    });
    app.world_mut().run_schedule(Label::PreUpdate);
    assert_eq!(
        app.world().resource::<ActionMap>().get("move"),
        Some(&Action::Vec2(Vec2Action {
            value: hyge_core::prelude::Vec2::new(1.0, 1.0),
            raw: hyge_core::prelude::Vec2::new(1.0, 1.0)
        }))
    );

    let move_value = match app
        .world()
        .resource::<ActionMap>()
        .get("move")
        .expect("move action")
    {
        Action::Vec2(action) => action.value.y,
        _ => 0.0,
    };
    app.world_mut()
        .resource_mut::<ScriptInput>()
        .actions
        .insert("move".into(), move_value);
    app.world_mut().run_schedule(Label::Update);
    let source = fs::read_to_string(&script_path).expect("player script");
    app.world_mut()
        .resource_scope(|world, runtime: Mut<ScriptRuntime>| {
            runtime
                .engine
                .as_ref()
                .expect("Lua runtime")
                .execute_module_update(world, &source, player_bits, 1.0 / 60.0)
                .expect("player script executes");
        });
    assert_eq!(
        app.world()
            .get::<PhysicsVelocity>(player)
            .expect("player velocity")
            .linear[0],
        1.0
    );
    let first_script_generation = app
        .world()
        .resource::<ScriptState>()
        .generation(player)
        .expect("script loaded");
    write_file(&script_path, "return { state = { version = 2 }, on_update = function(entity, dt) local velocity = hyge.world.get(entity, 'PhysicsVelocity'); velocity.linear[1] = 2.0; hyge.world.set(entity, 'PhysicsVelocity', velocity) end }");
    let deadline = Instant::now() + Duration::from_millis(800);
    while app.world().resource::<ScriptState>().generation(player) == Some(first_script_generation)
    {
        app.world_mut().run_schedule(Label::Update);
        assert!(Instant::now() < deadline, "player.lua did not hot-reload");
        thread::sleep(Duration::from_millis(15));
    }
    let script_errors: Vec<String> = app
        .world()
        .resource::<Events<ScriptError>>()
        .iter_current_update_events()
        .map(|event| event.error.to_string())
        .collect();
    assert!(script_errors.is_empty(), "script errors: {script_errors:?}");

    let mut reloaded_world = world_document.clone();
    reloaded_world.root_prefab_instances[4]
        .transform
        .translation[0] = 99.0;
    fs::write(
        &world_path,
        reloaded_world.to_bytes().expect("reloaded world bytes"),
    )
    .expect("rewrite world");
    let diff = reload_loaded_scene_from_disk(app.world_mut()).expect("scene hot reload");
    assert_eq!(diff.changed_instances, 1);
    assert_eq!(
        app.world()
            .resource::<hyge_scene::prelude::LoadedSceneState>()
            .root_entities
            .len(),
        5
    );
}
