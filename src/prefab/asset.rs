use super::{
    builder::PrefabBuilder,
    serde::{PrefabDeserializer, PrefabSerializer},
};
use bevy::{
    app::AppTypeRegistry,
    asset::{AssetLoader, BoxedFuture, Error, LoadContext, LoadedAsset},
    ecs::entity::Entity,
    ecs::world::{FromWorld, World},
    reflect::{FromType, Reflect, TypeRegistryArc, TypeUuid},
    utils::{HashMap, HashSet},
};

#[derive(Default)]
pub struct Patch {
    pub path: String,
    pub modify: Vec<PatchEntity>,
    pub ignore: HashSet<u32>,
}

pub struct PatchEntity {
    pub entity: u32,
    pub append: Vec<Box<dyn Reflect>>,
    pub modify: HashMap<String, HashMap<String, Box<dyn Reflect>>>,
    pub remove: HashSet<String>,
}

#[derive(Default, TypeUuid)]
#[uuid = "28dd2ec1-5d0c-41af-b0ea-d6bf557a4279"]
pub struct Prefab {
    pub entities: Vec<PrefabEntity>,
}

impl Prefab {
    /// Create a new prefab from a given world.
    pub fn from_world(world: &World, registry: &AppTypeRegistry) -> Self {
        let mut builder = PrefabBuilder::from_world_with_registry(world, registry.clone());

        builder.extract_entities(world.iter_entities());

        builder.build()
    }

    /// Serialize this prefab into rust object notation (ron).
    pub fn serialize_ron(&self, registry: &AppTypeRegistry) -> Result<String, ron::Error> {
        let registry = &registry.read();
        let value = PrefabSerializer::new(self, registry);
        let config = ron::ser::PrettyConfig::default()
            .indentor(String::from("  "))
            .new_line(String::from("\n"));
        ron::ser::to_string_pretty(&value, config)
    }

    /// Deserialize prefab from rust object notation (ron).
    pub fn deserialize_ron(input: &[u8], registry: &TypeRegistryArc) -> Result<Self, ron::Error> {
        let registry = &registry.read();
        serde::de::DeserializeSeed::deserialize(
            PrefabDeserializer::new(registry),
            &mut ron::de::Deserializer::from_bytes(input)?,
        )
    }
}

pub struct PrefabEntity {
    pub entity: u32,
    pub components: Vec<Box<dyn Reflect>>,
}

pub trait PrefabComponent {
    fn insert(self, world: &mut World, entity: Entity);
}

#[derive(Clone)]
pub struct ReflectPrefabComponent {
    apply_insert: fn(&mut World, Entity, &dyn Reflect),
}

impl ReflectPrefabComponent {
    pub fn apply_insert(&self, world: &mut World, entity: Entity, proxy: &dyn Reflect) {
        (self.apply_insert)(world, entity, proxy);
    }
}

impl<T: PrefabComponent + FromWorld + Reflect> FromType<T> for ReflectPrefabComponent {
    fn from_type() -> Self {
        Self {
            apply_insert: |world, entity, reflect| {
                let mut proxy = T::from_world(world);
                proxy.apply(reflect);
                proxy.insert(world, entity);
            },
        }
    }
}

#[derive(Debug)]
pub struct PrefabLoader {
    registry: TypeRegistryArc,
}

impl FromWorld for PrefabLoader {
    fn from_world(world: &mut World) -> Self {
        let registry = world.resource::<AppTypeRegistry>().0.clone();
        Self { registry }
    }
}

impl AssetLoader for PrefabLoader {
    fn load<'a>(
        &'a self,
        bytes: &'a [u8],
        load_context: &'a mut LoadContext,
    ) -> BoxedFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let prefab = Prefab::deserialize_ron(bytes, &self.registry)?;
            load_context.set_default_asset(LoadedAsset::new(prefab));
            Ok(())
        })
    }

    fn extensions(&self) -> &[&str] {
        &["prefab", "prefab.ron"]
    }
}
