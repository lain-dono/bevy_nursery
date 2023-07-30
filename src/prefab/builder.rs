use super::{Prefab, PrefabEntity};
use bevy::ecs::{
    entity::Entity,
    reflect::{AppTypeRegistry, ReflectComponent},
    world::World,
};
use bevy::utils::{default, HashMap};

/// A [`Prefab`] builder, used to build a scene from a [`World`] by extracting some entities.
pub struct PrefabBuilder<'w> {
    entities: HashMap<u32, PrefabEntity>,
    registry: AppTypeRegistry,
    world: &'w World,
}

impl<'w> PrefabBuilder<'w> {
    /// Prepare a builder that will extract entities and their component from the given [`World`].
    /// All components registered in that world's [`AppTypeRegistry`] resource will be extracted.
    pub fn from_world(world: &'w World) -> Self {
        let registry = world.resource::<AppTypeRegistry>().clone();
        Self::from_world_with_registry(world, registry)
    }

    /// Prepare a builder that will extract entities and their component from the given [`World`].
    /// Only components registered in the given [`AppTypeRegistry`] will be extracted.
    pub fn from_world_with_registry(world: &'w World, registry: AppTypeRegistry) -> Self {
        Self {
            entities: default(),
            registry,
            world,
        }
    }

    /// Consume the builder, producing a [`Prefab`].
    pub fn build(self) -> Prefab {
        Prefab {
            entities: self.entities.into_values().collect(),
        }
    }

    /// Extract one entity from the builder's [`World`].
    ///
    /// Re-extracting an entity that was already extracted will have no effect.
    pub fn extract_entity(&mut self, entity: Entity) -> &mut Self {
        self.extract_entities(std::iter::once(entity))
    }

    /// Extract entities from the builder's [`World`].
    ///
    /// Re-extracting an entity that was already extracted will have no effect.
    ///
    /// Extracting entities can be used to extract entities from a query.
    pub fn extract_entities(&mut self, entities: impl Iterator<Item = Entity>) -> &mut Self {
        let registry = self.registry.read();

        for entity in entities {
            if self.entities.contains_key(&entity.index()) {
                continue;
            }

            let mut entry = PrefabEntity {
                entity: entity.index(),
                components: Vec::new(),
            };

            for component_id in self.world.entity(entity).archetype().components() {
                let reflect_component = self
                    .world
                    .components()
                    .get_info(component_id)
                    .and_then(|info| registry.get(info.type_id().unwrap()))
                    .and_then(|registration| registration.data::<ReflectComponent>());

                if let Some(reflect_component) = reflect_component {
                    let entity = self.world.entity(entity);
                    if let Some(component) = reflect_component.reflect(entity) {
                        entry.components.push(component.clone_value());
                    }
                }
            }

            self.entities.insert(entity.index(), entry);
        }

        drop(registry);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::PrefabBuilder;
    use bevy::ecs::{
        component::Component,
        prelude::Entity,
        query::With,
        reflect::{AppTypeRegistry, ReflectComponent},
        world::World,
    };
    use bevy::reflect::Reflect;

    #[derive(Component, Reflect, Default, Eq, PartialEq, Debug)]
    #[reflect(Component)]
    struct ComponentA;

    #[derive(Component, Reflect, Default, Eq, PartialEq, Debug)]
    #[reflect(Component)]
    struct ComponentB;

    #[test]
    fn extract_one_entity() {
        let mut world = World::default();

        let atr = AppTypeRegistry::default();
        atr.write().register::<ComponentA>();
        world.insert_resource(atr);

        let entity = world.spawn((ComponentA, ComponentB)).id();

        let mut builder = PrefabBuilder::from_world(&world);
        builder.extract_entity(entity);
        let scene = builder.build();

        assert_eq!(scene.entities.len(), 1);
        assert_eq!(scene.entities[0].entity, entity.index());
        assert_eq!(scene.entities[0].components.len(), 1);
        assert!(scene.entities[0].components[0].represents::<ComponentA>());
    }

    #[test]
    fn extract_one_entity_twice() {
        let mut world = World::default();

        let atr = AppTypeRegistry::default();
        atr.write().register::<ComponentA>();
        world.insert_resource(atr);

        let entity = world.spawn((ComponentA, ComponentB)).id();

        let mut builder = PrefabBuilder::from_world(&world);
        builder.extract_entity(entity);
        builder.extract_entity(entity);
        let scene = builder.build();

        assert_eq!(scene.entities.len(), 1);
        assert_eq!(scene.entities[0].entity, entity.index());
        assert_eq!(scene.entities[0].components.len(), 1);
        assert!(scene.entities[0].components[0].represents::<ComponentA>());
    }

    #[test]
    fn extract_one_entity_two_components() {
        let mut world = World::default();

        let atr = AppTypeRegistry::default();
        {
            let mut register = atr.write();
            register.register::<ComponentA>();
            register.register::<ComponentB>();
        }
        world.insert_resource(atr);

        let entity = world.spawn((ComponentA, ComponentB)).id();

        let mut builder = PrefabBuilder::from_world(&world);
        builder.extract_entity(entity);
        let scene = builder.build();

        assert_eq!(scene.entities.len(), 1);
        assert_eq!(scene.entities[0].entity, entity.index());
        assert_eq!(scene.entities[0].components.len(), 2);
        assert!(scene.entities[0].components[0].represents::<ComponentA>());
        assert!(scene.entities[0].components[1].represents::<ComponentB>());
    }

    #[test]
    fn extract_query() {
        let mut world = World::default();

        let atr = AppTypeRegistry::default();
        {
            let mut register = atr.write();
            register.register::<ComponentA>();
            register.register::<ComponentB>();
        }
        world.insert_resource(atr);

        let entity_a_b = world.spawn((ComponentA, ComponentB)).id();
        let entity_a = world.spawn(ComponentA).id();
        let _entity_b = world.spawn(ComponentB).id();

        let mut query = world.query_filtered::<Entity, With<ComponentA>>();
        let mut builder = PrefabBuilder::from_world(&world);
        builder.extract_entities(query.iter(&world));
        let scene = builder.build();

        assert_eq!(scene.entities.len(), 2);
        let mut scene_entities = vec![scene.entities[0].entity, scene.entities[1].entity];
        scene_entities.sort();
        assert_eq!(scene_entities, [entity_a_b.index(), entity_a.index()]);
    }
}
