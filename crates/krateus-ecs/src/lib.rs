//! `krateus-ecs` — Entity Component System da KraTeus Engine.
//!
//! Implementacao propria, baseada em archetypes, conforme a decisao D02 em
//! `docs/DECISOES.md`. Entidades com o mesmo conjunto de componentes ficam
//! juntas no mesmo archetype, com um vetor por tipo de componente — layout SoA.
//! Um sistema que le apenas `Posicao` percorre memoria contigua, sem trazer
//! para o cache campos que nao vai usar.
//!
//! ```
//! use krateus_ecs::World;
//!
//! #[derive(Debug, PartialEq)]
//! struct Posicao(f32, f32);
//! struct Velocidade(f32, f32);
//!
//! let mut world = World::new();
//! let e = world.spawn((Posicao(0.0, 0.0), Velocidade(1.0, 2.0)));
//!
//! let v = world.get::<Velocidade>(e).map(|v| (v.0, v.1)).unwrap();
//! let p = world.get_mut::<Posicao>(e).unwrap();
//! p.0 += v.0;
//! p.1 += v.1;
//!
//! assert_eq!(world.get::<Posicao>(e), Some(&Posicao(1.0, 2.0)));
//! ```
//!
//! Estado: armazenamento completo — entidades, componentes, archetypes e
//! `World`. Queries, command buffer, job system e scheduler entram nos passos
//! seguintes da Fase 2. Ver `docs/ROADMAP.md`.

pub mod archetype;
pub mod bundle;
pub(crate) mod column;
pub mod component;
pub mod entity;
pub mod world;

pub use archetype::{Archetype, ArchetypeId, Archetypes};
pub use bundle::Bundle;
pub use component::{Component, ComponentId, ComponentInfo, Components};
pub use entity::{Entities, Entity, EntityLocation};
pub use world::World;
