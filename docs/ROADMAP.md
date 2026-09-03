# Roadmap de Criação — KraTeus Engine

Plano de execução derivado do documento de visão v2. Detalha o §20 em fases com
entregáveis concretos e **critérios de aceite mensuráveis**, seguindo o §19
("não otimizar apenas por opinião: medir com benchmarks").

## Duas regras que governam o plano

**1. Toda fase termina executável.** Nenhuma fase entrega "infraestrutura para o
futuro". Se não roda e não pode ser medido, não fechou.

**2. Fatia vertical antes de largura.** As Fases 1–4 atravessam a pilha inteira
(core → ECS → RHI → pixel na tela) com o mínimo de cada camada, em vez de
completar o Core antes de tocar em render. O atrito real da arquitetura está nas
costuras (ECS ↔ Render World ↔ RHI), e essas costuras precisam ser descobertas
enquanto ainda são baratas de mudar.

---

## Fase 0 — Ambiente ✅

- ✅ `rustup` + toolchain `stable-x86_64-pc-windows-msvc` (`rustc 1.98.1`)
- ✅ Visual Studio Build Tools + Windows SDK — já estavam instalados
- ✅ `rust-toolchain.toml` fixando canal e componentes
- ✅ **Smart App Control:** decidido manter ligado. Bloqueia DLL de proc-macro,
  build scripts e, de forma imprevisível, binários gerados. Mitigado em parte
  redirecionando o `target/` para dentro do perfil do usuário. O que não roda
  localmente passa a rodar no CI Linux. Risco [R01](RISCOS.md), com gatilho de
  reavaliação no início da Fase 3.
- ✅ `cargo-deny` configurado em `deny.toml`, executado no CI em vez de local.
- ⏸️ `cargo-nextest` dispensado: `cargo test` cobre a necessidade e evita
  instalar mais um binário sujeito ao R01.

**Aceite.** `cargo build --workspace` e `cargo test --workspace` completam.

---

## Fase 1 — Fundação: workspace e Core ✅

**Objetivo.** O esqueleto do §21 existindo de verdade, com as convenções de
qualidade travadas antes de haver código para consertar.

**Entregáveis**

- ✅ Workspace Cargo com as crates do §21, mais `krateus-rhi-wgpu` (backend da
  D01) e `krateus-assets` (pipeline do §13)
- ✅ `krateus-core`: `Handle<T>` com geração, `Pool<T>` com slots reciclados,
  `Clock` de timestep fixo, logging estruturado (`tracing`) com filtro por
  subsistema, configuração (`serde` + TOML) em camadas, tipo de erro
  (`thiserror`)
- ⏸️ Arena de bytes adiada: sem consumidor real ainda, entraria como feature
  antes do caso de uso — exatamente o que o §19 proíbe. O `Pool` já cobre a
  necessidade da Fase 2.
- ✅ `rustfmt.toml` e lints centralizados em `[workspace.lints]` (preferido a
  `clippy.toml`: fica junto do resto da configuração do workspace)
- ✅ CI (GitHub Actions): fmt → clippy → build → test em Windows **e** Linux,
  mais um job que verifica as regras de arquitetura do §19
- ⚠️ Harness de benchmark (`criterion`): `benches/pool.rs` escrito e job de CI
  configurado, mas **não compila localmente** — o SAC bloqueia o build script de
  `num-traits` ([R01](RISCOS.md)). A medição passa a viver no CI Linux, que é
  ambiente mais estável para comparar execuções do que a máquina de
  desenvolvimento. Não bloqueia a Fase 2.

**Aceite**

- ✅ `cargo test --workspace` verde localmente: 19 testes de unidade + 1 doc test
  (CI nas duas plataformas ainda não executado — falta o remote)
- ✅ Exemplo `hello_core` inicializa logging, lê config em camadas, povoa um
  `Pool` com 1024 corpos e roda 10.000 ticks de timestep fixo
- ✅ `krateus-core` é folha no grafo de dependências, verificado por job de CI

---

## Fase 2 — ECS, jobs e loop de simulação 🚧

**Objetivo.** O coração do §5. É a fase mais cara de errar (ver D02).

**Entregáveis**

- ✅ **Armazenamento:** entidade `(index, generation)` com reciclagem LIFO
  determinística, registro de componentes, coluna type-erased (SoA), archetypes
  indexados por assinatura ordenada, `World` com spawn/despawn/get/insert/remove
  e migração entre archetypes sem cópia pela pilha. 48 testes.
- ✅ **Queries tipadas:** `QueryData` para `Entity`, `&T`, `&mut T` e tuplas de
  até 8; filtros `With` / `Without` combináveis. Seleção por archetype, não por
  entidade. `Access` rejeita na construção uma query que conflitaria consigo
  mesma — `(&T, &mut T)` vira panic, não comportamento indefinido.
- ✅ **Benchmarks** `benches/ecs.rs` cobrindo o critério de 1M de entidades,
  spawn/despawn em regime permanente e iteração fragmentada em dois archetypes.
  Rodam no CI Linux ([R01](RISCOS.md)).
- ⬜ Change detection (`Changed<T>`)
- ✅ **Recursos globais:** dados únicos do mundo (tempo, gravidade, entrada),
  com espaço de identificadores próprio. `Access` passou a ter dois espaços
  separados — um componente `Posicao` e um recurso `Posicao` não conflitam.
- ⬜ Mudanças estruturais diferidas por command buffer
- Job system: thread pool work-stealing, sem alocação no caminho quente
- Scheduler: sistemas declaram acessos de leitura e escrita; o grafo de conflito
  é derivado dos tipos; executa em paralelo o que não colide, com ordem estável
  e determinística
- Loop de simulação: timestep fixo com acumulador, `alpha` de interpolação
  exposto para o renderer

**Aceite** (os números viram baseline no primeiro commit da fase)

Enquanto o [R01](RISCOS.md) estiver aberto, toda medição de `criterion` roda no
CI Linux. Localmente valem `cargo build`, `cargo test` e `cargo run --example` —
suficientes para desenvolver ECS e scheduler, que não tocam a GPU.

- Iteração sobre 1M de entidades com 3 componentes, medida em `criterion`
- Spawn e despawn de 100k entidades sem picos de alocação
- Escalabilidade do scheduler medida de 1 a N threads
- **Teste de determinismo:** mesma seed produz o mesmo hash de estado após
  10.000 ticks, com resultado idêntico entre Windows e Linux no CI

---

## Fase 3 — RHI e primeiro backend

**Objetivo.** §7. A costura que impede o resto da engine de conhecer uma API
gráfica.

**Entregáveis**

- `krateus-rhi`: traits Device, Queue, Swapchain, Buffer, Texture, Sampler,
  ShaderModule, Pipeline, BindGroup, CommandBuffer, RenderPass
- Backend `rhi-wgpu` implementando essas traits (ver D01)
- Janela e input via `winit`; surface, present, resize, troca de modo de exibição

**Aceite**

- Triângulo colorido na tela, com resize e alt-tab estáveis
- **Isolamento verificado no CI:** zero ocorrências de `wgpu::` fora da crate de
  backend — a regra do §19 virando teste, não intenção

---

## Fase 4 — Render World e renderer 2D/3D básico

**Objetivo.** §8 e §10. Separar mundo lógico de mundo de renderização e provar
que a separação paga.

**Entregáveis**

- Render World com estágios explícitos: `extract` (ECS → dados de render),
  `prepare`, `queue`, `render`
- Sprites em lote, mesh estática, câmera 2D e 3D, material simples, texturas
- GPU instancing, frustum culling, ordenação por estado para reduzir draw calls

**Aceite**

- 100k sprites com número de draw calls constante, não proporcional à contagem
- 10k cubos instanciados a 60 fps
- **Zero alocações de heap por frame no caminho de render**, verificado por um
  allocator instrumentado que falha o teste ao contar alocação durante o frame

---

## Fase 5 — Física e ferramentas de visualização

**Objetivo.** §6, incluindo explicitamente o exemplo de balística.

**Entregáveis**

- Integrador semi-implícito; gravidade, forças, impulsos
- Shapes: esfera, AABB, OBB, cápsula, plano. Broadphase (grid ou SAP) e
  narrowphase; resolução de contato
- Raycast, sweep, triggers, character controller, projéteis
- **Ferramenta de trajetória:** dada altura e distância do salto, retorna
  velocidade inicial, ângulo, altura máxima, tempo de subida, tempo de descida e
  tempo total de voo
- Níveis de fidelidade selecionáveis — analítico, integrado, integrado com
  arrasto — para que o custo acompanhe o uso (§18)
- Gizmos de debug desenhados pelo renderer: linhas, shapes, contatos, normais

**Aceite**

- Teste de conservação de energia dentro de tolerância definida
- Pilha de 100 caixas estável por 60 s sem afundar
- Solver determinístico: mesmo input, mesmo resultado, cross-platform
- Trajetória analítica e integrada convergem dentro de tolerância

---

## Fase 6 — World, pipeline de assets e editor inicial

**Objetivo.** §12, §13 e §14.

**Entregáveis**

- Pipeline: importer (glTF, PNG, WAV) → processor → `.ktz` → loader de runtime
  (ver D06), com cache de build por hash da fonte
- World em chunks, streaming assíncrono por distância, carga e descarga dinâmica
- Editor (`egui`, ver D05): viewport, hierarquia, inspector, asset browser,
  console e logs

**Aceite**

- Mundo grande com chunks entrando e saindo sem estouro de frame time, medido
  por **p99 de frame time**, não por média
- Editor abre uma cena, edita um transform e salva; o runtime carrega o
  resultado **sem o editor no binário** — §4 verificado por tamanho de build

---

## Fase 7 — Profiler, save/replay e robustez

**Objetivo.** §16.

**Entregáveis**

- Profiler de CPU (spans sobre `tracing`), de GPU (timestamp queries) e de
  memória; timings por sistema e por entidade
- Save/load do estado do World; replay determinístico a partir de log de input
- Stress tests e benchmarks no CI com detecção de regressão

**Aceite**

- Replay de 10 minutos reproduz hash de estado idêntico à execução original
- Regressão de performance acima de um limiar definido quebra o CI

---

## Fase 8 — Backends adicionais, networking e recursos avançados

**Objetivo.** §15 e §8, agora que o núcleo está validado.

- Backend **Vulkan nativo** (`ash`) — sua função principal é provar a RHI (D01)
- Networking: lockstep determinístico e/ou snapshot com interpolação
- Lua embutido (`mlua`) para scripting, regras e modding (§2)
- Render avançado: sombras, PBR, pós-processamento, partículas

---

## Regras de arquitetura que viram teste, não intenção

O §19 lista o que não fazer. Cada item vira uma verificação automática:

| Regra do §19 | Verificação automática |
|---|---|
| Não criar engine monolítica | Grafo de dependência entre crates validado no CI |
| Não obrigar jogos simples a carregar sistemas avançados | `default = []` em todas as crates; tamanho do build mínimo medido |
| Não prender o renderer a uma API gráfica | Ocorrências de `wgpu::` / `ash::` fora do backend = 0 |
| Não misturar lógica de jogo com render | `krateus-render` não pode depender de `krateus-simulation` |
| Não otimizar apenas por opinião | Benchmark obrigatório antes de qualquer otimização |
| Não começar por centenas de features | Toda fase entrega executável com aceite medido |

---

## Riscos principais

**Escopo.** Uma engine é trabalho de equipe. Mitigação: a fatia vertical entrega
algo funcional cedo, e nenhuma feature entra sem um caso de uso que a exija.

**Determinismo em ponto flutuante cross-platform.** Frágil por natureza (D09).
Mitigação: o teste existe desde a Fase 2, quando ainda é barato consertar.

**A RHI virar um clone da API do wgpu.** Mitigação descrita em D01.

**O ECS próprio ser mais lento que os existentes.** Mitigação: `bevy_ecs` e
`hecs` entram como baseline nos benchmarks da Fase 2. Se a implementação própria
perder de forma consistente, isso é dado para revisitar D02 — não para ignorar.
