//! `krateus-render` — a separacao entre mundo logico e mundo de renderizacao.
//!
//! A secao 10 do documento de visao pede que os dois mundos existam separados:
//! a simulacao pode ter muitas entidades enquanto o renderer recebe apenas o
//! necessario para desenhar o estado atual. Esta crate e essa costura, em quatro
//! estagios.
//!
//! ```text
//! World  ──extract──▶  RenderSnapshot  ──prepare──▶  PreparedFrame
//!                                                          │
//!                                                        queue
//!                                                          ▼
//!                          Rhi  ◀──render──  RenderWork
//! ```
//!
//! | Estagio | Recebe | Produz | Decide |
//! |---|---|---|---|
//! | [`extract`] | `&mut World` | [`RenderSnapshot`] | o que existe |
//! | [`prepare`] | snapshot + proporcao | [`PreparedFrame`] | o que aparece |
//! | [`queue`] | quadro preparado | [`RenderWork`] | como agrupar |
//! | [`render`] | trabalho + RHI | comandos | nada |
//!
//! # O corte que importa
//!
//! O [`RenderSnapshot`] **nao referencia o mundo**: nem `&World`, nem tempo de
//! vida, nem identificador a resolver depois. Depois da extracao, a simulacao
//! pode avancar, criar e destruir entidades sem invalidar o quadro em desenho.
//!
//! Cada estagio tambem sabe menos que o anterior. O [`render`] nao escolhe
//! nada: recebe lotes prontos e os traduz em chamadas de RHI. Por isso o caminho
//! inteiro e testavel sem GPU — o backend nulo registra o que recebeu, e o teste
//! afirma sobre isso.
//!
//! # Estado
//!
//! Escrito enquanto o aceite da Fase 3 segue bloqueado pelo ambiente
//! ([R01](../../../docs/RISCOS.md)). E arquitetura de CPU pura, verificavel por
//! CI; nao substitui o triangulo na tela, e nao declara fase nenhuma concluida.
//!
//! # Como amarrar ao scheduler
//!
//! A extracao precisa do `World` inteiro, e a forma certa de expressar isso e
//! um **sistema exclusivo** — nao um parametro comum:
//!
//! ```
//! use krateus_ecs::{Schedule, World};
//! use krateus_render::extract;
//!
//! # let mut world = World::new();
//! let mut schedule = Schedule::new();
//! schedule.add_stage("simulacao").add_stage("extracao");
//! schedule.add_exclusive_system("extracao", |world: &mut World| {
//!     let _snapshot = extract(world);
//! });
//! schedule.initialize(&mut world);
//! ```
//!
//! Um `&mut World` como parametro comum esconderia o custo: o scheduler nao
//! teria como saber que aquele sistema alcanca tudo, e o `Access` passaria a
//! mentir. Como sistema exclusivo, a extracao vira fronteira de etapa — nada
//! roda em paralelo com ela — e essa limitacao fica visivel no agrupamento, que
//! e onde ela e paga.

pub mod componentes;
pub mod extract;
pub mod prepare;
pub mod queue;
pub mod render;

pub use componentes::{
    Bounds, Camera, CameraAtiva, MaterialId, MeshId, Renderable, Transform, Visibility,
};
pub use extract::{CameraSnapshot, InstanceSnapshot, RenderSnapshot, extract};
pub use prepare::{Frustum, PreparedFrame, prepare};
pub use queue::{DrawBatch, InstanceData, RenderWork, queue};
pub use render::{FUNDO, RenderResources, render};
