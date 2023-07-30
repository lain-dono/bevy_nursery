use super::{Patch, Prefab, PrefabError};
use bevy::{
    asset::{AssetEvent, Assets, Handle},
    ecs::{
        bundle::Bundle,
        component::Component,
        entity::{Entity, EntityMap},
        event::{Events, ManualEventReader},
        query::Changed,
        system::{Command, Commands, Query, ResMut, Resource},
        world::{Mut, World},
    },
    hierarchy::{AddChild, Parent},
    render::view::{ComputedVisibility, Visibility},
    transform::components::{GlobalTransform, Transform},
    utils::HashMap,
};

pub fn prefab_spawner_maintain_system(world: &mut World) {
    world.resource_scope(|world, mut spawner: Mut<PrefabSpawner>| spawner.maintain(world));
}

/// System that will spawn prefabs from [`PrefabBundle`].
#[allow(clippy::type_complexity)]
pub fn prefab_update_system(
    mut commands: Commands,
    mut to_spawn: Query<
        (Entity, &Handle<Prefab>, Option<&mut PrefabInstance>),
        Changed<Handle<Prefab>>,
    >,
    mut spawner: ResMut<PrefabSpawner>,
) {
    for (entity, prefab, instance) in &mut to_spawn {
        let new = spawner.spawn(prefab.clone(), Some(entity));
        if let Some(mut instance) = instance {
            spawner.despawn(&instance);
            *instance = new;
        } else {
            commands.entity(entity).insert(new);
        }
    }
}

type Id = bevy::utils::Uuid;

/// Instance identifier of a spawned prefab.
/// It can be used with the [`PrefabSpawner`] to interact with the spawned prefab.
#[derive(Component, Debug)]
pub struct PrefabInstance(Id);

/// A component bundle for a [`Prefab`] root.
///
/// The prefab from `prefab` will be spawn as a child of the entity with this component.
/// Once it's spawned, the entity will have a [`PrefabInstance`] component.
#[derive(Default, Bundle)]
pub struct PrefabBundle {
    /// Handle to the prefab to spawn
    pub prefab: Handle<Prefab>,
    pub transform: Transform,
    pub global_transform: GlobalTransform,
    pub visibility: Visibility,
    pub computed_visibility: ComputedVisibility,
}

#[derive(Default)]
pub struct PrefabInstanceInfo {
    entity_map: EntityMap,
}

impl PrefabInstanceInfo {
    /// Get an iterator over the entities in an instance
    pub fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entity_map.values()
    }

    fn spawn(&mut self, world: &mut World, handle: &Handle<Prefab>) -> Result<(), PrefabError> {
        world.resource_scope(|world, prefabs: Mut<Assets<Prefab>>| {
            let prefab = prefabs.get(handle);
            let prefab = prefab.ok_or_else(|| PrefabError::NonExistentPrefab {
                handle: handle.clone_weak(),
            })?;

            let patch = Patch::default();

            super::write_to_world(&patch, prefab, world, &mut self.entity_map)
        })
    }

    fn despawn(&mut self, world: &mut World) {
        for entity in self.entity_map.values() {
            let _ = world.despawn(entity);
        }
    }
}

#[derive(Default)]
struct Spawned {
    prefabs: HashMap<Handle<Prefab>, Vec<Id>>,
    instances: HashMap<Id, PrefabInstanceInfo>,
}

impl Spawned {
    fn spawn(&mut self, world: &mut World, handle: &Handle<Prefab>) -> Result<Id, PrefabError> {
        let mut info = PrefabInstanceInfo::default();
        info.spawn(world, handle)?;

        let id = self.generate_id();
        self.instances.insert(id, info);
        self.prefabs.entry(handle.clone()).or_default().push(id);

        Ok(id)
    }

    fn generate_id(&self) -> Id {
        Id::new_v4()
    }

    fn update(&mut self, world: &mut World, handle: &Handle<Prefab>) {
        if let Some(spawned_instances) = self.prefabs.get(handle) {
            for id in spawned_instances {
                if let Some(info) = self.instances.get_mut(id) {
                    info.spawn(world, handle).unwrap();
                }
            }
        }
    }

    fn despawn(&mut self, world: &mut World, id: &Id) {
        if let Some(mut info) = self.instances.remove(id) {
            info.despawn(world);
        }
    }
}

#[derive(Default, Resource)]
pub struct PrefabSpawner {
    asset_event_reader: ManualEventReader<AssetEvent<Prefab>>,

    spawned: Spawned,

    to_spawn: Vec<(Handle<Prefab>, Id)>,
    to_despawn: Vec<Id>,

    with_parent: Vec<(Id, Entity)>,
    updates: Vec<Handle<Prefab>>,
}

impl PrefabSpawner {
    pub fn spawn(&mut self, handle: Handle<Prefab>, parent: Option<Entity>) -> PrefabInstance {
        let id = self.spawned.generate_id();
        self.to_spawn.push((handle, id));
        if let Some(parent) = parent {
            self.with_parent.push((id, parent));
        }
        PrefabInstance(id)
    }

    pub fn despawn(&mut self, id: &PrefabInstance) {
        self.to_despawn.push(id.0);
    }

    /// Check that an prefab instance spawned previously is ready to use
    pub fn is_ready(&self, id: &PrefabInstance) -> bool {
        self.spawned.instances.contains_key(&id.0)
    }

    pub fn info(&self, id: &PrefabInstance) -> Option<&PrefabInstanceInfo> {
        self.spawned.instances.get(&id.0)
    }

    pub fn spawn_sync(
        &mut self,
        world: &mut World,
        handle: &Handle<Prefab>,
    ) -> Result<PrefabInstance, PrefabError> {
        self.spawned.spawn(world, handle).map(PrefabInstance)
    }

    pub fn update_sync(&mut self, world: &mut World, handle: &Handle<Prefab>) {
        self.spawned.update(world, handle);
    }

    pub fn despawn_sync(&mut self, world: &mut World, id: &PrefabInstance) {
        self.spawned.despawn(world, &id.0);
    }

    fn maintain(&mut self, world: &mut World) {
        let asset_events = world.resource::<Events<AssetEvent<Prefab>>>();
        for event in self.asset_event_reader.iter(asset_events) {
            if let AssetEvent::Modified { handle } = event {
                if self.spawned.prefabs.contains_key(handle) {
                    self.updates.push(handle.clone_weak());
                }
            }
        }

        for id in self.to_despawn.drain(..) {
            self.spawned.despawn(world, &id);
        }

        self.to_spawn.retain(|(handle, id)| {
            let mut info = PrefabInstanceInfo::default();

            match info.spawn(world, handle) {
                Ok(_) => {
                    self.spawned.instances.insert(*id, info);
                    let spawned = self.spawned.prefabs.entry(handle.clone()).or_default();
                    spawned.push(*id);
                    false
                }
                Err(PrefabError::NonExistentPrefab { .. }) => true,
                Err(err) => {
                    bevy::log::error!("{}", err);
                    false
                }
            }
        });

        for handle in self.updates.drain(..) {
            self.spawned.update(world, &handle);
        }

        self.with_parent.retain(|&(id, parent)| {
            if let Some(info) = self.spawned.instances.get(&id) {
                for child in info.entities() {
                    // Add the `Parent` component to the prefab root,
                    // and update the `Children` component of the prefab parent
                    if !world
                        .get_entity(child)
                        // This will filter only the prefab root entity,
                        // as all other from the prefab have a parent
                        .map(|entity| entity.contains::<Parent>())
                        // Default is true so that it won't run on an entity that wouldn't exist anymore
                        // this case shouldn't happen anyway
                        .unwrap_or(true)
                    {
                        AddChild { parent, child }.apply(world);
                    }
                }
                false
            } else {
                true
            }
        });
    }
}
