//! A interface que um backend grafico precisa cumprir.
//!
//! # O que esta aqui, e o que nao esta
//!
//! Apenas o vocabulario que a primeira fatia vertical exige: desenhar um
//! triangulo numa janela, redimensionar, e apresentar. Nao ha bind groups,
//! samplers, anexo de profundidade, compute nem multiplos alvos — nao porque
//! sejam dificeis, mas porque modelar recurso sem um caso de uso que o exerca
//! produz abstracao que ninguem testou (secao 19 do documento de visao).
//!
//! # A forma vem de Vulkan e DX12
//!
//! Comandos sao **gravados** num encoder e depois **submetidos**, em vez de
//! executados na chamada. E o modelo das duas APIs — `VkCommandBuffer`,
//! `ID3D12GraphicsCommandList` — e o unico que permite gravar em paralelo e
//! submeter uma vez. Uma interface de maquina de estados, no estilo antigo,
//! seria mais curta de escrever e impossivel de mapear de volta.
//!
//! # O que o backend esconde, e por que isso e um risco
//!
//! Sincronizacao — barreiras de memoria, transicoes de layout de imagem,
//! semaforos entre aquisicao e apresentacao — nao aparece nesta interface. Em
//! Vulkan e DX12 nada disso e opcional; e trabalho que o backend precisa fazer
//! por conta.
//!
//! Isso e uma aposta consciente e o coracao do [R04](../../../docs/RISCOS.md):
//! um backend sobre wgpu cumpre sem esforco, porque o wgpu ja resolve, e um
//! backend Vulkan nativo talvez descubra que a interface nao lhe da onde colocar
//! um `VkFence`. Quando esse dia chegar, e a interface que muda.

use crate::error::Result;
use crate::types::{
    BufferDesc, BufferId, CommandBufferId, CommandEncoderId, Frame, RenderPassDesc, RenderPassId,
    RenderPipelineDesc, RenderPipelineId, ShaderDesc, ShaderId, SurfaceConfig, TextureFormat,
};

/// Backend grafico.
///
/// Um unico trait, e nao um por objeto, de proposito: com identificadores
/// opacos em vez de tipos associados, a interface fica objeto-segura, o
/// `krateus-render` pode segurar um `Box<dyn Rhi>` e trocar de backend em tempo
/// de execucao, e nao ha cascata de genericos atravessando a engine.
///
/// O custo e uma chamada virtual por comando. Isso importa em laco quente de
/// desenho, e a resposta certa quando importar sera agrupar comandos — nao
/// trocar o desenho por genericos antes de medir.
pub trait Rhi: Send + Sync {
    /// Nome do backend, para diagnostico.
    fn nome(&self) -> &str;

    // ------------------------------------------------------------ recursos --

    /// Cria um buffer.
    ///
    /// # Errors
    ///
    /// Se o tamanho for zero, se nenhum uso for declarado, ou se a alocacao
    /// falhar.
    fn create_buffer(&mut self, desc: &BufferDesc<'_>) -> Result<BufferId>;

    /// Escreve bytes num buffer criado com [`BufferUsage::COPY_DST`].
    ///
    /// [`BufferUsage::COPY_DST`]: crate::types::BufferUsage::COPY_DST
    ///
    /// Em Vulkan isto normalmente vira uma copia por buffer de staging; a
    /// interface nao promete que a escrita e imediata, apenas que o dado estara
    /// visivel para comandos submetidos depois.
    ///
    /// # Errors
    ///
    /// Se o identificador nao existir, se o buffer nao aceitar escrita, ou se
    /// os dados nao couberem a partir de `deslocamento`.
    fn write_buffer(&mut self, buffer: BufferId, deslocamento: u64, dados: &[u8]) -> Result<()>;

    /// Destroi um buffer.
    fn destroy_buffer(&mut self, buffer: BufferId);

    /// Compila um modulo de shader.
    ///
    /// # Errors
    ///
    /// Se a fonte nao for aceita pelo backend ou nao compilar.
    fn create_shader(&mut self, desc: &ShaderDesc<'_>) -> Result<ShaderId>;

    /// Destroi um modulo de shader.
    fn destroy_shader(&mut self, shader: ShaderId);

    /// Cria um pipeline grafico.
    ///
    /// # Errors
    ///
    /// Se algum shader referenciado nao existir, ou se o descritor for
    /// inconsistente.
    fn create_render_pipeline(&mut self, desc: &RenderPipelineDesc<'_>)
    -> Result<RenderPipelineId>;

    /// Destroi um pipeline.
    fn destroy_render_pipeline(&mut self, pipeline: RenderPipelineId);

    // ----------------------------------------------------------- superficie --

    /// Configura ou reconfigura a superficie.
    ///
    /// E o que o redimensionamento chama. Em Vulkan corresponde a recriar a
    /// swapchain; quadros adquiridos antes deixam de valer.
    ///
    /// # Errors
    ///
    /// Se o formato nao for suportado pela superficie.
    fn configure_surface(&mut self, config: &SurfaceConfig) -> Result<()>;

    /// Formato com que a superficie esta configurada.
    ///
    /// # Errors
    ///
    /// Se a superficie ainda nao foi configurada.
    fn surface_format(&self) -> Result<TextureFormat>;

    /// Adquire o proximo quadro.
    ///
    /// # Errors
    ///
    /// Se a superficie nao estiver configurada, ou se ja houver um quadro
    /// adquirido e nao apresentado.
    fn acquire_frame(&mut self) -> Result<Frame>;

    /// Apresenta o quadro adquirido.
    ///
    /// # Errors
    ///
    /// Se nao houver quadro adquirido, ou se o quadro nao for o corrente.
    fn present_frame(&mut self, frame: Frame) -> Result<()>;

    // -------------------------------------------------------------- comandos --

    /// Abre um gravador de comandos.
    ///
    /// # Errors
    ///
    /// Se o backend nao conseguir alocar.
    fn create_command_encoder(&mut self, rotulo: &str) -> Result<CommandEncoderId>;

    /// Abre um passe de renderizacao dentro do gravador.
    ///
    /// # Errors
    ///
    /// Se o gravador nao existir, se ja houver um passe aberto nele, ou se
    /// algum anexo for invalido.
    fn begin_render_pass(
        &mut self,
        encoder: CommandEncoderId,
        desc: &RenderPassDesc<'_>,
    ) -> Result<RenderPassId>;

    /// Fixa o pipeline do passe.
    ///
    /// # Errors
    ///
    /// Se o passe ou o pipeline nao existirem.
    fn set_pipeline(&mut self, passe: RenderPassId, pipeline: RenderPipelineId) -> Result<()>;

    /// Liga um buffer de vertices a um slot.
    ///
    /// # Errors
    ///
    /// Se o passe ou o buffer nao existirem, ou se o buffer nao tiver uso de
    /// vertice.
    fn set_vertex_buffer(&mut self, passe: RenderPassId, slot: u32, buffer: BufferId)
    -> Result<()>;

    /// Emite um desenho.
    ///
    /// # Errors
    ///
    /// Se o passe nao existir, se nenhum pipeline tiver sido fixado, ou se a
    /// contagem de vertices ou instancias for zero.
    fn draw(&mut self, passe: RenderPassId, vertices: u32, instancias: u32) -> Result<()>;

    /// Fecha o passe.
    ///
    /// # Errors
    ///
    /// Se o passe nao existir ou ja tiver sido fechado.
    fn end_render_pass(&mut self, passe: RenderPassId) -> Result<()>;

    /// Fecha o gravador e produz uma lista submetivel.
    ///
    /// # Errors
    ///
    /// Se o gravador nao existir ou tiver um passe aberto.
    fn finish_encoder(&mut self, encoder: CommandEncoderId) -> Result<CommandBufferId>;

    /// Submete listas para execucao, na ordem dada.
    ///
    /// # Errors
    ///
    /// Se alguma lista nao existir ou ja tiver sido submetida.
    fn submit(&mut self, listas: &[CommandBufferId]) -> Result<()>;
}
