//! `krateus-rhi` — Rendering Hardware Interface.
//!
//! A costura da secao 7 do documento de visao: o renderer fala com esta
//! interface, e nunca com uma API grafica. Trocar de backend nao deve tocar
//! nada acima daqui.
//!
//! # ⚠ Interface interna e experimental
//!
//! **Nada aqui e contrato publico ainda.** A forma so estara validada quando
//! existir o primeiro backend de verdade e um triangulo na tela — o criterio de
//! aceite da Fase 3, que hoje esta bloqueado pelo
//! [R01](../../../docs/RISCOS.md).
//!
//! Ate la, espere que assinaturas mudem sem aviso. Um segundo backend vai
//! cobrar o que este desenho ainda nao sabe: sincronizacao explicita, layouts de
//! recurso, e o que mais aparecer. Ver a decisao
//! [D12](../../../docs/DECISOES.md).
//!
//! # Escopo
//!
//! Apenas o vocabulario da primeira fatia vertical — desenhar um triangulo,
//! redimensionar, apresentar:
//!
//! | Conceito | Vulkan | DX12 |
//! |---|---|---|
//! | [`BufferId`] | `VkBuffer` | `ID3D12Resource` |
//! | [`TextureId`] | `VkImage` | `ID3D12Resource` |
//! | [`ShaderId`] | `VkShaderModule` | blob compilado |
//! | [`RenderPipelineId`] | `VkPipeline` | `ID3D12PipelineState` |
//! | [`CommandEncoderId`] | `VkCommandBuffer` gravando | `ID3D12GraphicsCommandList` |
//! | [`Frame`] | `vkAcquireNextImageKHR` | `IDXGISwapChain::Present` |
//!
//! Nao ha bind groups, samplers, profundidade, compute nem multiplos alvos.
//! Nao por serem dificeis: modelar recurso sem um caso de uso que o exerca
//! produz abstracao que ninguem testou, e a secao 19 e explicita sobre nao
//! comecar por features antes de validar o nucleo.
//!
//! # Backends
//!
//! [`NullRhi`] nao desenha nada e valida tudo. E o que permite ao CI verificar,
//! sem GPU, que as regras de ordem que a [`Rhi`] enuncia em prosa sao regras de
//! verdade — passe fechado antes de submeter, pipeline fixado antes de
//! desenhar, quadro apresentado antes de adquirir outro.
//!
//! ```
//! use krateus_rhi::{
//!     BufferDesc, BufferUsage, NullRhi, Rhi, ShaderDesc, ShaderSource,
//! };
//!
//! let mut rhi = NullRhi::new();
//!
//! let vertices = rhi.create_buffer(&BufferDesc {
//!     rotulo: "triangulo",
//!     tamanho: 3 * 24,
//!     uso: BufferUsage::VERTEX | BufferUsage::COPY_DST,
//! })?;
//!
//! rhi.write_buffer(vertices, 0, &[0u8; 72])?;
//!
//! let shader = rhi.create_shader(&ShaderDesc {
//!     rotulo: "triangulo",
//!     fonte: ShaderSource::Wgsl("// ..."),
//! })?;
//!
//! rhi.destroy_shader(shader);
//! rhi.destroy_buffer(vertices);
//! assert_eq!(rhi.recursos_vivos(), 0);
//! # Ok::<(), krateus_rhi::RhiError>(())
//! ```

pub mod error;
pub mod null;
pub mod rhi;
pub mod types;

pub use error::{Result, RhiError};
pub use null::{NullRhi, Operacao};
pub use rhi::Rhi;
pub use types::{
    BufferDesc, BufferId, BufferUsage, Color, ColorAttachment, CommandBufferId, CommandEncoderId,
    Frame, LoadOp, PresentMode, PrimitiveTopology, RenderPassDesc, RenderPassId,
    RenderPipelineDesc, RenderPipelineId, ShaderDesc, ShaderId, ShaderSource, ShaderStage, StoreOp,
    SurfaceConfig, TextureFormat, TextureId, TextureViewId, VertexAttribute, VertexFormat,
    VertexLayout,
};
