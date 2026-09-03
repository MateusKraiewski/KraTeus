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

---

## D10 — Change detection: modelo de ticks

**Decisão.** Definida antes de implementar `Changed<T>`, porque a escolha
atravessa armazenamento, queries e scheduler ao mesmo tempo. Chegar nela depois
significaria redesenhar os três.

### Os três ticks

| Tick | Onde vive | Quando é escrito |
|---|---|---|
| `added_tick` | por instância de componente | na inserção do componente |
| `changed_tick` | por instância de componente | na inserção e a cada `&mut T` entregue |
| `last_run_tick` | por sistema | ao fim de cada execução do sistema |

`Changed<T>` passa quando `changed_tick` é mais recente que o `last_run_tick` do
sistema que pergunta. `Added<T>`, quando `added_tick` é.

**`&mut T` marca alterado mesmo sem alteração real.** Detectar mutação de fato
exigiria comparar o valor antes e depois, o que custa mais do que o filtro
economiza. O contrato é "foi exposto para escrita", não "mudou de valor", e a
documentação da API precisa dizer isso — senão vira bug de expectativa.

### Onde os ticks são guardados

**Array paralelo por coluna, no `Archetype`** — não intercalados com os dados e
não dentro de `Column`.

Duas razões:

- Intercalar `(valor, ticks)` destruiria o ganho de SoA. Um sistema que não
  filtra por mudança passaria a arrastar 8 bytes de tick por componente para o
  cache sem usar.
- `Column` concentra todo o `unsafe` do ECS. Manter os ticks fora dela deixa
  esse módulo intocado, e change detection vira código seguro.

### Quando o tick global avança

**Uma vez por execução de sistema**, não uma vez por passo de simulação.

Por passo, dois sistemas que rodam no mesmo frame não conseguiriam se distinguir:
um sistema não saberia dizer se a alteração veio do sistema anterior deste frame
ou do frame passado. É justamente o caso de uso mais comum — reagir ao que outro
sistema acabou de escrever.

### O ponto que interage com o scheduler

**O tick de cada sistema é atribuído pela ordem determinística do schedule, não
por um contador atômico incrementado na hora em que o sistema começa.**

Um contador atômico é o caminho óbvio e está errado para esta engine: com
sistemas rodando em paralelo, a ordem de incremento varia entre execuções, os
ticks gravados nos componentes variam junto, e o resultado de `Changed<T>` deixa
de ser reproduzível — contradizendo a D09 e o §15.

Como o scheduler já precisa de uma ordem total estável entre sistemas (para
decidir paralelismo a partir do [`Access`]), essa mesma ordem serve para
pré-atribuir os ticks. Custo zero, e o determinismo se mantém.

### `Changed<T>` declara leitura de `T`

O filtro lê a coluna de ticks de `T`. Um sistema que escreve `T` escreve esses
ticks. Se o `Access` não registrasse a leitura, o scheduler poderia rodar os dois
em paralelo e produzir uma corrida sobre os ticks.

Portanto `Changed<T>` e `Added<T>` acrescentam `T` às leituras do `Access`,
mesmo sem devolver o componente — ao contrário de `With<T>` e `Without<T>`, que
só olham a assinatura do archetype e não declaram acesso nenhum.

### Overflow

**`u32` com comparação por diferença circular, mais varredura periódica de
saneamento.**

Comparar ticks com `>` quebra no wraparound. A comparação correta mede idade
relativa ao tick atual:

```text
idade(t) = agora.wrapping_sub(t).min(IDADE_MAXIMA)
mais_novo(t, last_run) = idade(last_run) > idade(t)
```

com `IDADE_MAXIMA = u32::MAX / 2`. Isso funciona enquanto nenhum tick guardado
for mais velho que `IDADE_MAXIMA` — um componente que nunca é tocado acabaria
violando isso.

A varredura de saneamento resolve: periodicamente percorre os ticks armazenados
e fixa em `agora - IDADE_MAXIMA` qualquer um mais velho que isso. Custo O(total
de componentes), amortizado ao longo de centenas de milhares de ticks.

**O gatilho da varredura é o contador de ticks, nunca tempo de relógio.** Um
gatilho temporal faria a mesma simulação produzir estados diferentes em máquinas
de velocidades diferentes, que é exatamente o que a D09 proíbe.

**Alternativa considerada:** `u64`, que nunca estoura na prática e dispensa toda
essa maquinaria. Descartada pelo custo de memória: com 1M de entidades e três
componentes, os ticks passariam de 24 MB para 48 MB. A complexidade de `u32` fica
contida em uma função de comparação e uma varredura, ambas testáveis.

**Custo de reversão.** Baixo para a escolha de largura — trocar `u32` por `u64`
mexe em um tipo e apaga a varredura. Alto para a atribuição de ticks pelo
schedule, que é por isso que está decidida agora.
