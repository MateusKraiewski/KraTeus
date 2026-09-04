//! Vocabulario de recursos e descritores.
//!
//! Cada tipo aqui foi escolhido olhando para o que Vulkan e DX12 nomeiam, e nao
//! para o que uma API especifica oferece pronto. Onde os dois divergem, o
//! comentario diz como o conceito se chama em cada um — e o proximo backend vai
//! cobrar isso.

use std::num::NonZeroU32;
use std::ops::BitOr;

use krateus_core::handle::Handle;

/// Marcadores de tipo para os identificadores de recurso.
///
/// Nao existem em tempo de execucao: servem para que um `BufferId` nao possa ser
/// usado onde se espera um `TextureId`.
pub mod marcadores {
    /// Marcador de buffer.
    #[derive(Debug)]
    pub struct Buffer;
    /// Marcador de textura.
    #[derive(Debug)]
    pub struct Texture;
    /// Marcador de vista de textura.
    #[derive(Debug)]
    pub struct TextureView;
    /// Marcador de modulo de shader.
    #[derive(Debug)]
    pub struct Shader;
    /// Marcador de pipeline grafico.
    #[derive(Debug)]
    pub struct RenderPipeline;
    /// Marcador de gravador de comandos.
    #[derive(Debug)]
    pub struct CommandEncoder;
    /// Marcador de lista de comandos pronta para submissao.
    #[derive(Debug)]
    pub struct CommandBuffer;
    /// Marcador de passe de renderizacao em gravacao.
    #[derive(Debug)]
    pub struct RenderPass;
}

/// Identificador de buffer. `VkBuffer` no Vulkan, `ID3D12Resource` no DX12.
pub type BufferId = Handle<marcadores::Buffer>;
/// Identificador de textura. `VkImage` / `ID3D12Resource`.
pub type TextureId = Handle<marcadores::Texture>;
/// Identificador de vista de textura. `VkImageView` / descriptor de RTV.
pub type TextureViewId = Handle<marcadores::TextureView>;
/// Identificador de modulo de shader. `VkShaderModule` / blob compilado.
pub type ShaderId = Handle<marcadores::Shader>;
/// Identificador de pipeline grafico. `VkPipeline` / `ID3D12PipelineState`.
pub type RenderPipelineId = Handle<marcadores::RenderPipeline>;
/// Identificador de gravador de comandos. `VkCommandBuffer` em gravacao.
pub type CommandEncoderId = Handle<marcadores::CommandEncoder>;
/// Identificador de lista pronta. `VkCommandBuffer` fechado.
pub type CommandBufferId = Handle<marcadores::CommandBuffer>;
/// Identificador de passe em gravacao.
pub type RenderPassId = Handle<marcadores::RenderPass>;

/// Constroi um identificador de RHI a partir de um handle de outra marca.
///
/// Existe porque um backend guarda seus recursos em colecoes tipadas pelo
/// proprio tipo interno, e precisa devolver o identificador publico
/// correspondente. Indice e geracao sao preservados, entao a checagem de
/// geracao continua valendo.
#[must_use]
pub fn reetiqueta<A, B>(h: Handle<A>) -> Handle<B> {
    Handle::from_raw(h.index(), h.generation())
}

// ------------------------------------------------------------------ formatos --

/// Formato de pixel.
///
/// Deliberadamente curto: so o que a primeira fatia vertical usa. Acrescentar
/// formato e barato; remover um que virou publico, nao.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextureFormat {
    /// BGRA de 8 bits por canal, com correcao sRGB. Formato tipico de
    /// swapchain no Windows.
    Bgra8UnormSrgb,
    /// RGBA de 8 bits por canal, com correcao sRGB.
    Rgba8UnormSrgb,
    /// Profundidade de 32 bits em ponto flutuante.
    Depth32Float,
}

impl TextureFormat {
    /// Indica se o formato serve como anexo de profundidade.
    #[must_use]
    pub const fn e_profundidade(self) -> bool {
        matches!(self, Self::Depth32Float)
    }
}

/// Formato de um atributo de vertice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VertexFormat {
    /// Dois floats de 32 bits.
    Float32x2,
    /// Tres floats de 32 bits.
    Float32x3,
    /// Quatro floats de 32 bits.
    Float32x4,
}

impl VertexFormat {
    /// Tamanho em bytes.
    #[must_use]
    pub const fn tamanho(self) -> u64 {
        match self {
            Self::Float32x2 => 8,
            Self::Float32x3 => 12,
            Self::Float32x4 => 16,
        }
    }
}

/// Como os vertices formam primitivas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PrimitiveTopology {
    /// Cada tres vertices formam um triangulo.
    #[default]
    TriangleList,
    /// Cada dois vertices formam uma linha.
    LineList,
}

/// Quando a superficie troca de quadro.
///
/// `Fifo` corresponde a `VK_PRESENT_MODE_FIFO_KHR` e e o unico que a
/// especificacao do Vulkan garante existir em todo dispositivo — por isso e o
/// padrao.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PresentMode {
    /// Espera o vsync. Sempre suportado.
    #[default]
    Fifo,
    /// Apresenta assim que pronto, com risco de tearing.
    Immediate,
}

// -------------------------------------------------------------------- cor --

/// Cor em ponto flutuante, no espaco linear.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Componente vermelho.
    pub r: f64,
    /// Componente verde.
    pub g: f64,
    /// Componente azul.
    pub b: f64,
    /// Componente alfa.
    pub a: f64,
}

impl Color {
    /// Preto opaco.
    pub const PRETO: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

    /// Cria uma cor opaca.
    #[must_use]
    pub const fn rgb(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b, a: 1.0 }
    }
}

// ------------------------------------------------------------------- usos --

/// Para que um buffer pode ser usado.
///
/// Declarar o uso na criacao nao e burocracia: Vulkan e DX12 escolhem o tipo de
/// memoria e o estado inicial do recurso a partir disso, e mudar depois exige
/// recriar. Um conjunto de bits, e nao um enum, porque um mesmo buffer costuma
/// ter mais de um uso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferUsage(u32);

impl BufferUsage {
    /// Fonte de atributos de vertice.
    pub const VERTEX: Self = Self(1 << 0);
    /// Fonte de indices.
    pub const INDEX: Self = Self(1 << 1);
    /// Bloco de constantes lido por shader.
    pub const UNIFORM: Self = Self(1 << 2);
    /// Destino de escrita vinda da CPU.
    pub const COPY_DST: Self = Self(1 << 3);

    /// Conjunto vazio.
    pub const NENHUM: Self = Self(0);

    /// Indica se todos os bits de `outro` estao presentes.
    #[inline]
    #[must_use]
    pub const fn contem(self, outro: Self) -> bool {
        self.0 & outro.0 == outro.0
    }

    /// Indica se nenhum uso foi declarado.
    #[inline]
    #[must_use]
    pub const fn vazio(self) -> bool {
        self.0 == 0
    }

    /// Valor bruto, para diagnostico.
    #[inline]
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl BitOr for BufferUsage {
    type Output = Self;

    fn bitor(self, outro: Self) -> Self {
        Self(self.0 | outro.0)
    }
}

/// Em que estagio um shader roda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShaderStage {
    /// Estagio de vertice.
    Vertex,
    /// Estagio de fragmento. `Pixel shader` no vocabulario da Microsoft.
    Fragment,
}

/// Como o codigo do shader chega ao backend.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShaderSource<'a> {
    /// Codigo-fonte WGSL.
    ///
    /// Escolhido por ser o que o primeiro backend consome direto. Um backend
    /// Vulkan nativo precisaria de SPIR-V, e e por isso que este enum existe em
    /// vez de um `&str` solto: a variante nova entra sem quebrar assinatura.
    Wgsl(&'a str),
    /// SPIR-V ja compilado.
    SpirV(&'a [u32]),
}

// ------------------------------------------------------------- descritores --

/// O que se pede ao criar um buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferDesc<'a> {
    /// Nome para diagnostico e para ferramentas de captura.
    pub rotulo: &'a str,
    /// Tamanho em bytes.
    pub tamanho: u64,
    /// Usos pretendidos.
    pub uso: BufferUsage,
}

/// O que se pede ao criar um modulo de shader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderDesc<'a> {
    /// Nome para diagnostico.
    pub rotulo: &'a str,
    /// Codigo do shader.
    pub fonte: ShaderSource<'a>,
}

/// Um atributo dentro de um vertice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexAttribute {
    /// Posicao do atributo no shader.
    pub local: u32,
    /// Deslocamento em bytes dentro do vertice.
    pub deslocamento: u64,
    /// Formato do atributo.
    pub formato: VertexFormat,
}

/// Como os bytes de um buffer de vertices sao interpretados.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexLayout<'a> {
    /// Distancia em bytes entre dois vertices consecutivos.
    pub passo: u64,
    /// Atributos, na ordem em que aparecem.
    pub atributos: &'a [VertexAttribute],
}

/// O que se pede ao criar um pipeline grafico.
///
/// Corresponde ao `VkGraphicsPipelineCreateInfo` e ao
/// `D3D12_GRAPHICS_PIPELINE_STATE_DESC`: um objeto imutavel que fixa shaders,
/// formato de entrada e formato de saida de uma vez. As duas APIs exigem isso
/// porque o driver compila o estado inteiro junto — trocar peca a peca em tempo
/// de desenho e o modelo antigo, que nenhuma das duas oferece.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPipelineDesc<'a> {
    /// Nome para diagnostico.
    pub rotulo: &'a str,
    /// Modulo do estagio de vertice.
    pub vertex: ShaderId,
    /// Ponto de entrada no modulo de vertice.
    pub vertex_entrada: &'a str,
    /// Modulo do estagio de fragmento.
    pub fragment: ShaderId,
    /// Ponto de entrada no modulo de fragmento.
    pub fragment_entrada: &'a str,
    /// Formato dos buffers de vertice.
    pub vertices: &'a [VertexLayout<'a>],
    /// Como os vertices formam primitivas.
    pub topologia: PrimitiveTopology,
    /// Formato do alvo de cor. Precisa casar com o do passe.
    pub formato_alvo: TextureFormat,
}

/// O que fazer com o conteudo anterior do anexo ao comecar o passe.
///
/// `Limpar` costuma ser mais barato que `Carregar` em GPUs de renderizacao por
/// tiles, e essa diferenca e o motivo de Vulkan e DX12 exigirem a escolha
/// explicita em vez de deduzi-la.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoadOp {
    /// Substitui o conteudo pela cor dada.
    Limpar(Color),
    /// Preserva o que ja estava la.
    Carregar,
}

/// O que fazer com o resultado ao terminar o passe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StoreOp {
    /// Guarda o resultado.
    #[default]
    Guardar,
    /// Descarta. Util para anexos intermediarios que ninguem le depois.
    Descartar,
}

/// Um alvo de cor de um passe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorAttachment {
    /// Vista onde o passe desenha.
    pub vista: TextureViewId,
    /// O que fazer com o conteudo anterior.
    pub load: LoadOp,
    /// O que fazer com o resultado.
    pub store: StoreOp,
}

/// O que se pede ao abrir um passe de renderizacao.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPassDesc<'a> {
    /// Nome para diagnostico.
    pub rotulo: &'a str,
    /// Alvos de cor. A primeira fatia usa exatamente um.
    pub cores: &'a [ColorAttachment],
}

/// Como a superficie deve ser configurada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceConfig {
    /// Largura em pixels.
    pub largura: NonZeroU32,
    /// Altura em pixels.
    pub altura: NonZeroU32,
    /// Formato dos quadros.
    pub formato: TextureFormat,
    /// Politica de apresentacao.
    pub modo: PresentMode,
}

/// Um quadro adquirido da superficie.
///
/// Corresponde ao par `vkAcquireNextImageKHR` + `vkQueuePresentKHR`. O quadro
/// precisa ser apresentado ou descartado; segura-lo indefinidamente trava a
/// swapchain, que tem um numero fixo de imagens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// Vista onde desenhar.
    pub vista: TextureViewId,
    /// Indice da imagem dentro da swapchain, para diagnostico.
    pub indice: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usos_combinam_e_sao_consultaveis() {
        let uso = BufferUsage::VERTEX | BufferUsage::COPY_DST;

        assert!(uso.contem(BufferUsage::VERTEX));
        assert!(uso.contem(BufferUsage::COPY_DST));
        assert!(!uso.contem(BufferUsage::INDEX));
        assert!(!uso.vazio());
        assert!(BufferUsage::NENHUM.vazio());
    }

    #[test]
    fn conter_o_conjunto_vazio_e_sempre_verdade() {
        assert!(BufferUsage::NENHUM.contem(BufferUsage::NENHUM));
        assert!(BufferUsage::VERTEX.contem(BufferUsage::NENHUM));
    }

    #[test]
    fn tamanhos_de_formato_de_vertice() {
        assert_eq!(VertexFormat::Float32x2.tamanho(), 8);
        assert_eq!(VertexFormat::Float32x3.tamanho(), 12);
        assert_eq!(VertexFormat::Float32x4.tamanho(), 16);
    }

    #[test]
    fn so_profundidade_e_profundidade() {
        assert!(TextureFormat::Depth32Float.e_profundidade());
        assert!(!TextureFormat::Bgra8UnormSrgb.e_profundidade());
    }

    #[test]
    fn reetiquetar_preserva_indice_e_geracao() {
        let origem: Handle<marcadores::Buffer> =
            Handle::from_raw(7, NonZeroU32::new(3).expect("nao nulo"));
        let destino: Handle<marcadores::Texture> = reetiqueta(origem);

        assert_eq!(destino.index(), 7);
        assert_eq!(destino.generation().get(), 3);
    }

    #[test]
    fn padroes_sao_os_universalmente_suportados() {
        // Fifo e o unico modo que o Vulkan garante em todo dispositivo.
        assert_eq!(PresentMode::default(), PresentMode::Fifo);
        assert_eq!(PrimitiveTopology::default(), PrimitiveTopology::TriangleList);
        assert_eq!(StoreOp::default(), StoreOp::Guardar);
    }
}
