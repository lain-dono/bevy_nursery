#![doc = include_str!("doc.md")]

mod asset;
mod builder;
mod serde;
mod spawner;

use std::any::TypeId;

pub use self::asset::{
    Patch, PatchEntity, Prefab, PrefabComponent, PrefabEntity, PrefabLoader, ReflectPrefabComponent,
};
pub use self::builder::PrefabBuilder;
pub use self::serde::{
    ComponentsDeserializer, ComponentsSerializer, PrefabDeserializer, PrefabSerializer,
};
pub use self::spawner::{
    prefab_spawner_maintain_system, prefab_update_system, PrefabBundle, PrefabInstance,
    PrefabInstanceInfo, PrefabSpawner,
};

use bevy::{
    app::{App, Plugin, PreUpdate, Update},
    asset::{AddAsset, Handle},
    ecs::entity::{Entity, EntityMap},
    ecs::reflect::{AppTypeRegistry, ReflectComponent, ReflectMapEntities},
    ecs::world::World,
    reflect::GetPath,
    utils::{tracing::error, HashMap},
};

pub struct PrefabPlugin;

impl Plugin for PrefabPlugin {
    fn build(&self, app: &mut App) {
        app.add_asset::<Prefab>()
            .init_asset_loader::<PrefabLoader>()
            .init_resource::<PrefabSpawner>()
            .add_systems(PreUpdate, self::prefab_update_system)
            .add_systems(Update, self::prefab_spawner_maintain_system);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrefabError {
    #[error("prefab contains the unregistered component `{type_name}`. consider adding `#[reflect(Component)]` to your type")]
    UnregisteredComponent { type_name: String },
    #[error("prefab contains the unregistered type `{type_name}`. consider registering the type using `app.register_type::<T>()`")]
    UnregisteredType { type_name: String },
    #[error("prefab does not exist")]
    NonExistentPrefab { handle: Handle<Prefab> },
    #[error("prefab patch contains the wrong path")]
    PatchContainsWrongPath { path: String, err: String },
}

pub fn write_to_world(
    patch: &Patch,
    prefab: &Prefab,
    world: &mut World,
    entity_map: &mut EntityMap,
) -> Result<(), PrefabError> {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();

    let mut patch_map: HashMap<_, _> = patch.modify.iter().map(|e| (e.entity, e)).collect();

    // For each component types that reference other entities, we keep track
    // of which entities in the scene use that component.
    // This is so we can update the scene-internal references to references
    // of the actual entities in the world.
    let mut scene_mappings: HashMap<TypeId, Vec<Entity>> = HashMap::default();

    for prefab_entity in &prefab.entities {
        // ignore despawned entities
        if patch.ignore.contains(&prefab_entity.entity) {
            continue;
        }

        // Fetch the entity with the given entity id from the `entity_map`
        let entity = entity_map.entry(Entity::from_raw(prefab_entity.entity));
        // or spawn a new entity with a transiently unique id if there is no corresponding entry.
        let entity = *entity.or_insert_with(|| world.spawn_empty().id());
        let mut entity = world.entity_mut(entity);

        let patch = patch_map.remove(&prefab_entity.entity);

        // Combine components
        let components = patch.map(|p| p.append.iter()).into_iter();
        let components = prefab_entity.components.iter().chain(components.flatten());

        // Apply/ add each component to the given entity.
        for mut component in components.map(AsRef::as_ref) {
            let type_name = component.type_name();

            let mut _clone;
            if let Some(patch) = patch {
                // ignore removed components
                if patch.remove.contains(type_name) {
                    continue;
                }

                // patch component fields
                if let Some(modify) = patch.modify.get(type_name) {
                    _clone = component.clone_value();

                    for (path, value) in modify {
                        let field = _clone.reflect_path_mut(path);
                        let field = field.map_err(|err| PrefabError::PatchContainsWrongPath {
                            path: path.clone(),
                            err: err.to_string(),
                        })?;
                        field.apply(value.as_ref());
                    }

                    component = _clone.as_ref();
                }
            }

            let registration = registry.get_with_name(type_name);
            let registration = registration.ok_or_else(|| PrefabError::UnregisteredType {
                type_name: type_name.to_string(),
            })?;

            if let Some(proxy) = registration.data::<ReflectPrefabComponent>() {
                proxy.apply_insert(&mut entity, component);
                continue;
            }

            let reflect = registration.data::<ReflectComponent>();
            let reflect = reflect.ok_or_else(|| PrefabError::UnregisteredComponent {
                type_name: type_name.to_string(),
            })?;

            // If this component references entities in the scene, track it
            // so we can update it to the entity in the world.
            if registration.data::<ReflectMapEntities>().is_some() {
                scene_mappings
                    .entry(registration.type_id())
                    .or_insert(Vec::new())
                    .push(entity.id());
            }

            // If the entity already has the given component attached,
            // just apply the (possibly) new value,
            // otherwise add the component to the entity.
            reflect.apply_or_insert(&mut entity, component);
        }
    }

    for patch in patch_map.into_values() {
        // Fetch the entity with the given entity id from the `entity_map`
        let entity = entity_map.entry(Entity::from_raw(patch.entity));
        // or spawn a new entity with a transiently unique id if there is no corresponding entry.
        let entity = *entity.or_insert_with(|| world.spawn_empty().id());
        let mut entity = world.entity_mut(entity);

        for component in patch.append.iter().map(AsRef::as_ref) {
            let type_name = component.type_name();

            let registration = registry.get_with_name(type_name);
            let registration = registration.ok_or_else(|| PrefabError::UnregisteredType {
                type_name: type_name.to_string(),
            })?;

            if let Some(proxy) = registration.data::<ReflectPrefabComponent>() {
                proxy.apply_insert(&mut entity, component);
                continue;
            }

            let reflect = registration.data::<ReflectComponent>().ok_or_else(|| {
                PrefabError::UnregisteredComponent {
                    type_name: type_name.to_string(),
                }
            })?;

            // If this component references entities in the scene, track it
            // so we can update it to the entity in the world.
            if registration.data::<ReflectMapEntities>().is_some() {
                scene_mappings
                    .entry(registration.type_id())
                    .or_insert(Vec::new())
                    .push(entity.id());
            }

            // If the entity already has the given component attached,
            // just apply the (possibly) new value,
            // otherwise add the component to the entity.
            reflect.apply_or_insert(&mut entity, component);
        }
    }

    // Updates references to entities in the scene to entities in the world
    for (type_id, entities) in scene_mappings.into_iter() {
        let registration = registry
            .get(type_id)
            .expect("we should be getting TypeId from this TypeRegistration in the first place");
        if let Some(map_entities_reflect) = registration.data::<ReflectMapEntities>() {
            map_entities_reflect.map_entities(world, entity_map, &entities);
        }
    }

    Ok(())
}
