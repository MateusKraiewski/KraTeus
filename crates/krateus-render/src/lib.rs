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
//! Falta o laco que amarra os quatro estagios ao `Schedule` como sistemas. A
//! extracao precisa do `World` inteiro, e os parametros de sistema hoje so
//! oferecem query e recurso — a peca que falta e um parametro de acesso
//! exclusivo, e ela entra quando houver um consumidor de verdade.

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
