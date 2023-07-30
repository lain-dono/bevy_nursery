

```rust
use bevy_nursery::prefab::{PrefabComponent, ReflectPrefabComponent};
use bevy::{
    asset::{Asset, AssetPath, AssetServer, Handle},
    ecs::component::Component,
    ecs::entity::Entity,
    ecs::reflect::ReflectComponent,
    ecs::world::{World, EntityMut},
    reflect::Reflect,
};

#[derive(Component, Reflect)]
#[reflect(Component, PrefabComponent)]
pub struct PrefabHandle<T: Asset> {
    pub path: String,

    #[reflect(ignore)]
    marker: std::marker::PhantomData<fn() -> T>,
}

impl<T: Asset> PrefabHandle<T> {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            marker: std::marker::PhantomData,
        }
    }
}

impl<T: Asset> Default for PrefabHandle<T> {
    fn default() -> Self {
        Self {
            path: Default::default(),
            marker: std::marker::PhantomData,
        }
    }
}

impl<T: Asset> PrefabComponent for PrefabHandle<T> {
    fn insert(self, entity: &mut EntityMut) {
        if let Some(asset_server) = entity.world().get_resource::<AssetServer>() {
            let path = AssetPath::from(&self.path);
            let bundle: Handle<T> = asset_server.load(path);
            entity.insert(bundle);
        }
    }
}
```