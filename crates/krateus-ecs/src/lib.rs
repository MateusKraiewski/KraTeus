//! `krateus-ecs` — Entity Component System da KraTeus Engine.
//!
//! Implementacao propria, baseada em archetypes, conforme a decisao D02 em
//! `docs/DECISOES.md`. Entidades com o mesmo conjunto de componentes ficam
//! juntas no mesmo archetype, com um vetor por tipo de componente — layout SoA.
//! Um sistema que le apenas `Posicao` percorre memoria contigua, sem trazer
//! para o cache campos que nao vai usar.
//!
//! ```
//! use krateus_ecs::{With, World};
//!
//! struct Posicao(f32, f32);
//! struct Velocidade(f32, f32);
//! struct Ativa;
//!
//! let mut world = World::new();
//! world.spawn((Posicao(0.0, 0.0), Velocidade(1.0, 2.0), Ativa));
//! world.spawn((Posicao(0.0, 0.0), Velocidade(9.0, 9.0)));
//!
//! // Um sistema de integracao: so as entidades ativas avancam.
//! for (p, v) in world.query_filtered::<(&mut Posicao, &Velocidade), With<Ativa>>() {
//!     p.0 += v.0;
//!     p.1 += v.1;
//! }
//!
//! let posicoes: Vec<_> = world.query::<&Posicao>().map(|p| (p.0, p.1)).collect();
//! assert!(posicoes.contains(&(1.0, 2.0)));
//! assert!(posicoes.contains(&(0.0, 0.0)));
//! ```
//!
//! Estado: armazenamento, queries tipadas com filtros, recursos globais e
//! command buffer. Change detection, job system e scheduler entram nos passos
//! seguintes da Fase 2. Ver `docs/ROADMAP.md`.

pub mod access;
pub mod archetype;
pub mod bundle;
pub(crate) mod column;
pub mod commands;
pub mod component;
pub mod entity;
pub mod query;
pub mod resource;
pub mod schedule;
pub mod system;
pub mod tick;
pub mod world;
pub mod world_cell;

pub use access::{Access, Conflict};
pub use archetype::{Archetype, ArchetypeId, Archetypes};
pub use bundle::Bundle;
pub use commands::Commands;
pub use component::{Component, ComponentId, ComponentInfo, Components};
pub use entity::{Entities, Entity, EntityLocation};
pub use query::{
    Added, Changed, Query, QueryData, QueryFilter, QueryIter, QueryState, With, Without,
};
pub use resource::{Resource, ResourceId, Resources};
pub use schedule::Schedule;
pub use system::{
    ExclusiveFunctionSystem, FunctionSystem, IntoExclusiveSystem, IntoSystem, Res, ResMut, System,
    SystemParam, SystemParamFunction,
};
pub use tick::{ComponentTicks, SystemTicks, Tick};
pub use world::World;
pub use world_cell::WorldCell;
