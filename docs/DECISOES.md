# Decisões Técnicas — KraTeus Engine

Fecha os itens em aberto do §22 do documento de visão. Cada decisão tem
justificativa e custo de reversão. Nada aqui é definitivo: o formato existe
para que uma mudança futura seja consciente, não acidental.

Status: **proposto** (aguardando validação)

---

## D01 — Backend gráfico: RHI própria, primeiro backend sobre wgpu

**Decisão.** `krateus-rhi` define traits próprias (Device, Queue, Swapchain,
Buffer, Texture, Pipeline, BindGroup, CommandBuffer, RenderPass). O primeiro
backend implementa essas traits em cima de **wgpu** (que resolve para DX12 no
Windows, Vulkan no Linux, Metal no macOS).

**Por quê.** O objetivo do §7 é *não contaminar a engine com uma API única* —
esse objetivo é atendido pela RHI, não pelo backend. Escrever Vulkan puro
(`ash`) como primeiro backend adiciona meses de trabalho (sincronização,
memória, descriptor pools, layout transitions) antes do primeiro triângulo, e
ainda assim não valida se a RHI é boa, porque só teria um backend.

**Custo de reversão.** Baixo — é exatamente o ponto da RHI. A Fase 8 prevê um
backend Vulkan nativo, cuja função principal é *provar* que a abstração aguenta.

**Risco.** A RHI pode acidentalmente virar um clone da API do wgpu. Mitigação:
desenhar cada trait olhando também para o vocabulário de Vulkan e DX12, e
manter uma nota de "como isso mapearia em Vulkan" nos tipos não óbvios.

---

## D02 — ECS: implementação própria, baseada em archetypes

**Decisão.** `krateus-ecs` é código próprio. Storage archetype (SoA), entidade
como `(index, generation)`, queries tipadas, mudanças estruturais diferidas via
command buffer.

**Por quê.** O ECS é a identidade técnica da engine e a principal alavanca dos
§5, §11 e §18 (layout de cache, paralelismo, custo proporcional). Já consta como
crate própria no §21. Usar `hecs`/`bevy_ecs` entregaria mais rápido, mas
transformaria a KraTeus em uma camada sobre outra engine.

**Custo de reversão.** Alto. É a decisão mais cara da lista — por isso vem cedo
(Fase 2) e com benchmarks desde o primeiro dia.

---

## D03 — Scheduler e jobs: próprios, com paralelismo derivado de acesso

**Decisão.** Sistemas declaram seus acessos (`&T` / `&mut T`); o scheduler
constrói o grafo de conflitos e roda em paralelo o que não colide. Thread pool
work-stealing próprio. `rayon` entra apenas como baseline de comparação em
benchmark, não como dependência do runtime.

**Por quê.** §5 pede paralelismo por ausência de dependência. Derivar isso dos
tipos evita que o usuário da engine tenha de ordenar sistemas à mão, e mantém a
ordem de execução determinística (§15).

---

## D04 — Física: própria no núcleo, com costura para backend externo

**Decisão.** `krateus-physics` implementa integrador, shapes, broadphase,
narrowphase, raycast/sweep, character controller e balística. A API pública é
desenhada como trait de forma que Rapier possa ser plugado como backend
alternativo, sem que isso seja feito agora.

**Por quê.** O exemplo do §6 (dado altura e distância de um salto, calcular v0,
trajetória, tempo de subida/descida) é uma ferramenta analítica própria de
qualquer maneira — não é o que uma lib de física entrega. E os níveis de
fidelidade configuráveis são um requisito de arquitetura, não uma feature.

**Custo de reversão.** Médio, desde que a costura seja um trait desde o início.

---

## D05 — UI do editor: egui

**Decisão.** Editor construído com **egui** sobre a mesma superfície wgpu do
renderer.

**Por quê.** O editor é ferramenta, não produto (§4, §12). egui é imediato,
Rust puro, dark-first por padrão, e roda dentro da janela da engine — o que
permite viewport nativo sem interop. Alternativas (Qt, Slint, Dear ImGui via
FFI) custam build complexo ou binding.

**Custo de reversão.** Baixo se a lógica do editor ficar separada do desenho.

---

## D06 — Formato de asset: container próprio `.ktz`, payload zero-copy

**Decisão.** Header versionado + tabela de chunks + payloads alinhados, lidos
por `mmap`/slice sem desserialização (`bytemuck` para POD; `rkyv` onde houver
estrutura). Formatos de autoria (glTF, PNG, WAV) nunca chegam ao runtime.

**Por quê.** §13. O ganho real do pipeline próprio é carregar sem parse.

---

## D07 — Plataformas iniciais: Windows x64 primeiro, Linux no CI desde a Fase 1

**Decisão.** Alvo de desenvolvimento é Windows x64 (MSVC). Linux entra no CI
desde o primeiro commit, mesmo sem ser alvo de release.

**Por quê.** CI em duas plataformas desde cedo é o que impede dependências
acidentais de Windows, e é o mecanismo dos testes de determinismo cross-platform
(§16). macOS fica fora até haver máquina.

---

## D08 — Matemática: `glam`

**Decisão.** Usar `glam` para vetores, matrizes e quaternions. Não reescrever.

**Por quê.** SIMD, testado, padrão de facto no ecossistema Rust gráfico. Não há
diferencial competitivo em reimplementar `Mat4`.

---

## D09 — Determinismo: `f32` com ordem determinística e timestep fixo

**Decisão.** Não usar fixed-point. Garantir determinismo por: timestep fixo,
ordem de iteração estável (nunca iterar `HashMap`), evitar funções
transcendentais de libm do sistema (usar implementações próprias/`libm` crate
onde o resultado entra na simulação), e proibir reduções em paralelo com ordem
não determinística.

**Por quê.** §15 e §16. Fixed-point custa ergonomia e performance em toda a
engine para resolver um problema que só aparece em lockstep cross-platform.

**Risco.** Determinismo bit-exato entre Windows e Linux é frágil. Mitigação: o
teste de determinismo cross-platform existe desde a Fase 2, quando ainda é
barato consertar.
