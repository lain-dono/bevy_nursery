#![doc = include_str!("doc.md")]

mod asset;
mod builder;
mod serde;
mod spawner;

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
    app::{App, AppTypeRegistry, CoreSet, Plugin},
    asset::{AddAsset, Handle},
    ecs::entity::{Entity, EntityMap},
    ecs::reflect::{ReflectComponent, ReflectMapEntities},
    ecs::schedule::IntoSystemConfig,
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
            .add_system(self::prefab_update_system.in_base_set(CoreSet::PreUpdate))
            .add_system(self::prefab_spawner_maintain_system.in_base_set(CoreSet::Update));
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

            // If the entity already has the given component attached,
            // just apply the (possibly) new value,
            // otherwise add the component to the entity.
            reflect.apply_or_insert(&mut entity, component);
        }
    }

    for registration in registry.iter() {
        if let Some(reflect) = registration.data::<ReflectMapEntities>() {
            reflect.map_entities(world, entity_map).unwrap();
        }
    }

    Ok(())
}
