use super::{Prefab, PrefabEntity};
use bevy::reflect::{
    serde::{TypedReflectDeserializer, TypedReflectSerializer},
    Reflect, TypeRegistryInternal,
};
use serde::{
    de::{DeserializeSeed, Error, MapAccess, Visitor},
    ser::SerializeMap,
};

pub struct PrefabSerializer<'a> {
    prefab: &'a Prefab,
    registry: &'a TypeRegistryInternal,
}

impl<'a> PrefabSerializer<'a> {
    pub fn new(prefab: &'a Prefab, registry: &'a TypeRegistryInternal) -> Self {
        Self { prefab, registry }
    }
}

impl<'a> serde::Serialize for PrefabSerializer<'a> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let registry = self.registry;
        let mut state = serializer.serialize_map(Some(self.prefab.entities.len()))?;
        for PrefabEntity { entity, components } in &self.prefab.entities {
            let value = ComponentsSerializer {
                components,
                registry,
            };
            state.serialize_entry(entity, &value)?;
        }
        state.end()
    }
}

pub struct ComponentsSerializer<'a> {
    components: &'a [Box<dyn Reflect>],
    registry: &'a TypeRegistryInternal,
}

impl<'a> serde::Serialize for ComponentsSerializer<'a> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_map(Some(self.components.len()))?;

        for component in self.components.iter().map(AsRef::as_ref) {
            let value = TypedReflectSerializer::new(component, self.registry);
            state.serialize_entry(component.type_name(), &value)?;
        }

        state.end()
    }
}

pub struct PrefabDeserializer<'a> {
    registry: &'a TypeRegistryInternal,
}

impl<'a> PrefabDeserializer<'a> {
    pub fn new(registry: &'a TypeRegistryInternal) -> Self {
        Self { registry }
    }
}

impl<'a, 'de> DeserializeSeed<'de> for PrefabDeserializer<'a> {
    type Value = Prefab;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        Ok(Prefab {
            entities: deserializer.deserialize_map(self)?,
        })
    }
}

impl<'a, 'de> Visitor<'de> for PrefabDeserializer<'a> {
    type Value = Vec<PrefabEntity>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("list of entities")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut entities = Vec::new();

        let kseed = std::marker::PhantomData;
        let vseed = ComponentsDeserializer {
            registry: self.registry,
        };

        while let Some((entity, components)) = map.next_entry_seed(kseed, vseed)? {
            entities.push(PrefabEntity { entity, components })
        }

        Ok(entities)
    }
}

#[derive(Clone, Copy)]
pub struct ComponentsDeserializer<'a> {
    pub registry: &'a TypeRegistryInternal,
}

impl<'a, 'de> DeserializeSeed<'de> for ComponentsDeserializer<'a> {
    type Value = Vec<Box<dyn Reflect>>;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'a, 'de> Visitor<'de> for ComponentsDeserializer<'a> {
    type Value = Vec<Box<dyn Reflect>>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("entity")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut components = Vec::new();

        while let Some(type_name) = map.next_key::<&str>()? {
            let registration = self.registry.get_with_name(type_name).ok_or_else(|| {
                Error::custom(format_args!("No registration found for `{}`", type_name))
            })?;
            let seed = TypedReflectDeserializer::new(registration, self.registry);
            components.push(map.next_value_seed(seed)?);
        }

        Ok(components)
    }
}
