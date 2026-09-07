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

### Revisão (Fase 2): fila compartilhada em vez de work-stealing, por ora

O pool implementado usa uma **fila compartilhada com threads trabalhadoras**, e
não deques com roubo de trabalho.

Work-stealing paga em árvores de tarefas recursivas e desbalanceadas, onde uma
thread termina cedo e precisa achar trabalho sozinha. A carga da Fase 2 é
outra: a cada etapa, um conjunto **conhecido** de sistemas que não conflitam
roda junto e o passo espera todos. Para fork-join sobre um lote conhecido, a
fila compartilhada entrega o mesmo resultado.

O que pesa contra antecipar o work-stealing: um deque lock-free correto é
difícil de escrever e, pior, difícil de *testar* — bugs de ordenação de memória
aparecem sob carga e em máquinas específicas. Trocar isso por complexidade
verificável, sem benchmark que mostre ganho, contraria o §19.

**Gatilho de reavaliação.** Quando existir paralelismo aninhado — um sistema que
divide o próprio trabalho em pedaços, como `par_iter` sobre chunks de um
archetype nas Fases 4 e 5. Aí o desbalanceamento vira real e a comparação passa
a ter o que medir.

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

---

## D11 — `Commands::spawn` não devolve `Entity` (ainda)

**Decisão.** O command buffer da Fase 2 enfileira `spawn` sem devolver o
identificador da entidade criada. A reserva de identificadores fica para quando
o scheduler existir.

### Por que não agora

Para `spawn` devolver uma `Entity` utilizável na hora, o identificador precisa
ser reservado antes da aplicação. Reservar sem `&mut World` — que a fila não
tem, e não pode ter, já que é preenchida durante a iteração — exige um contador
compartilhado.

Um contador atômico é o caminho óbvio e reintroduz exatamente o problema da
[D10](#d10--change-detection-modelo-de-ticks): com sistemas em paralelo, a ordem
de reserva varia entre execuções, os identificadores gravados variam junto, e o
estado deixa de ser reproduzível. Isso contradiz a D09 e o §15.

### O desenho quando entrar

O mesmo mecanismo que a D10 usa para os ticks: **faixas disjuntas atribuídas
pela ordem determinística do schedule.**

- Cada sistema recebe sua fila e uma faixa própria de índices de entidade,
  atribuída a partir da posição do sistema na ordem total estável que o
  scheduler já precisa calcular para decidir paralelismo.
- Dentro da faixa, a reserva é um incremento local — sem átomo, sem
  contenção entre threads.
- `Entities` ganha um passo de materialização, que transforma os índices
  reservados em entidades vivas na ordem das faixas. Índices reservados e não
  usados são devolvidos.

Custo zero em determinismo, porque a ordem das faixas é a ordem do schedule.

### O que fazer enquanto isso

Os casos que não precisam do identificador — despawn condicional, adicionar ou
remover componente, criar entidade que nada referencia — são a maioria e já
funcionam. Quando o identificador for necessário, `World::spawn` fora da
iteração continua disponível.

**Custo de reversão.** Baixo: acrescentar o retorno é aditivo, e nenhuma
assinatura existente muda.

---

## D12 — A RHI é interna e experimental até o primeiro triângulo

**Decisão.** `krateus-rhi` existe e compila, mas **não é contrato público**.
Assinaturas podem mudar sem aviso até que exista um backend real e o critério
de aceite da Fase 3 — triângulo na tela, com resize e alt-tab — tenha sido
verificado.

### Por quê

A [D01](#d01--backend-gráfico-rhi-própria-primeiro-backend-sobre-wgpu) diz que
o objetivo da RHI é não contaminar a engine com uma API única, e que isso só
fica provado quando houver um segundo backend. Enquanto houver **zero**, o
risco é maior ainda: uma interface desenhada sem nenhuma implementação é uma
hipótese, não um projeto.

Congelar agora seria o pior dos mundos — pagaria o custo de estabilidade sem
ter a evidência que a justifica.

### O que já está fechado, e o que não está

Fechado: a **forma**. Comandos são gravados num encoder e submetidos, como em
`VkCommandBuffer` e `ID3D12GraphicsCommandList`. Identificadores são opacos, a
interface é objeto-segura, e o `krateus-render` vai segurar `Box<dyn Rhi>`.

Não fechado: praticamente todo o resto. Em particular, **sincronização não
aparece na interface** — barreiras de memória, transições de layout de imagem,
semáforos entre aquisição e apresentação. Um backend sobre wgpu cumpre sem
esforço, porque o wgpu já resolve. Um backend Vulkan nativo pode descobrir que
não há onde colocar um `VkFence`. É exatamente o [R04](RISCOS.md), e é a
primeira coisa que o segundo backend vai cobrar.

### Escopo deliberadamente pequeno

Só o vocabulário que a primeira fatia vertical exige. Sem bind groups,
samplers, profundidade, compute ou múltiplos alvos — não por serem difíceis,
mas porque modelar recurso sem um caso de uso que o exerça produz abstração que
ninguém testou (§19).

### O backend nulo

`NullRhi` não desenha nada e valida tudo: passe fechado antes de submeter,
pipeline fixado antes de desenhar, quadro apresentado antes de adquirir outro,
formato do pipeline igual ao do passe. Isso torna as regras que a interface
enuncia em prosa verificáveis no CI, sem GPU — e dá ao primeiro backend real
uma bateria de testes que ele precisa passar.

**Custo de reversão.** Baixo por construção: é essa a razão de a decisão
existir.

---

## D13 — Integrador semi-implícito, e uma `Posicao` própria da física

**Decisão.** O integrador de `krateus-physics` é **semi-implícito** (velocidade
primeiro, posição com a velocidade nova), e a física define seus próprios
componentes `Posicao` e `Velocidade` em vez de reusar o `Transform` de
`krateus-render`.

### Por que semi-implícito

A diferença para o Euler explícito é a ordem de duas linhas, e muda o
comportamento qualitativo. O semi-implícito é simplético: a energia de um sistema
oscilante fica **limitada**, oscilando em torno do valor correto. O explícito
injeta energia a cada passo.

Isso não é sutileza acadêmica. Quase tudo em física de jogo oscila — mola,
suspensão, contato com penetração —, e com o explícito uma pilha de caixas ganha
energia sozinha e explode. Há dois testes lado a lado em `integrador.rs`: um
verifica que o semi-implícito mantém a energia dentro de 5% ao longo de mil
passos, o outro verifica que o explícito, no mesmo sistema, a infla. O segundo
existe para que a escolha não pareça arbitrária a quem ler depois.

O custo é conhecido e aceito: o semi-implícito é de primeira ordem, então erra a
*fase* — o período sai levemente errado. Errar a fase é aceitável; ganhar energia
não é.

### Por que a massa é guardada invertida

`MassaInversa`, não `Massa`. A divisão por massa aparece em todo passo de
integração e em toda resolução de contato, e guardar `1/m` a transforma em
multiplicação. Mais importante: massa infinita — um corpo estático, o chão — vira
`0.0`, que é um número comum, em vez de um caso especial espalhado pelo código.

### Por que `Posicao` própria, e não o `Transform` do renderer

Duas razões, e a segunda é a que importa.

A primeira é a direção da dependência: `krateus-physics` puxar `krateus-render`
inverteria o grafo — a física é produtora de estado, o renderer é consumidor.

A segunda é que **são conceitos diferentes**. O renderer precisa de translação,
rotação *e escala*; a física não simula escala, e a orientação de um corpo rígido
está ligada ao tensor de inércia, não à apresentação. Unificar os dois hoje
economizaria uma struct e cobraria depois, quando o corpo rígido chegar.

A ponte entre eles é um sistema de sincronização, e ele ainda **não existe** — de
propósito. Não há consumidor: sem backend gráfico, não há o que sincronizar.
Escrevê-lo agora seria adivinhar a forma (§19).

**Custo de reversão.** Baixo. O integrador é uma função de seis parâmetros, e
trocar `Posicao` por um transform compartilhado é uma migração mecânica enquanto
não houver corpo rígido nem contato.

---

## D14 — Como as formas de colisão são representadas

**Decisão.** Três escolhas tomadas ao implementar `krateus-physics::forma`, e
aprovadas em 2026-09-07. O escopo é exatamente este — nada aqui se estende a
broadphase, contato ou consultas, que ainda não existem.

### 1. `Forma` é um `enum`, não um trait

O conjunto de formas é fechado e pequeno, e é percorrido no laço mais interno da
colisão. `dyn Forma` colocaria despacho indireto ali dentro e, mais grave,
tiraria do código o controle da ordem de despacho, que é o que a
[D09](#d09--determinismo-f32-com-ordem-determinística-e-timestep-fixo) exige
poder garantir.

Isso **não revoga** a [D04](#d04--física-própria-no-núcleo-com-costura-para-backend-externo).
A trait que a D04 pede continua valendo, e no lugar onde ela paga: a costura de
backend — passo de simulação e consultas —, por onde o Rapier entraria. Uma
forma individual não é um ponto de extensão; o mundo de física é.

### 2. AABB e OBB são uma `Caixa` só, distinguidas pela rotação

O roadmap lista "esfera, AABB, OBB, cápsula, plano". AABB e OBB não são duas
formas: são a mesma caixa, uma com rotação identidade e outra sem. Modelá-las
separadamente duplicaria todo teste de colisão que envolvesse caixa, para
distinguir um caso que a própria rotação já distingue.

Um tipo separado para a caixa alinhada só se justifica se aparecer um consumidor
que ganhe com isso — um caminho rápido medido, não presumido.

### 3. `Plano` é semiespaço, e `aabb()` devolve `Option<Aabb>`

O plano é o chão, e não uma folha infinitamente fina: um corpo que atravessa a
superfície está *dentro* do sólido, e o contato tem para onde empurrar. Uma folha
sem espessura deixaria o solver sem direção de saída.

E o semiespaço não tem envelope finito. `None` é a resposta honesta: uma caixa
infinita se sobreporia a todos os pares e transformaria a broadphase em força
bruta. O `Option` obriga quem chama a tratar o semiespaço à parte — que é o que
todo motor de física faz com o chão de qualquer maneira.

### Custo de reversão

**Baixo, hoje.** Nada depende ainda dessas escolhas: não há broadphase, contato
nem consulta. As três podem ser revistas se um consumidor real ou um backend
externo demonstrar necessidade — um caminho rápido medido para caixas alinhadas,
uma forma que precise ser definida fora da crate, ou um backend cuja API exija
envelope para todas as formas.

O custo cresce com o que se apoiar nelas. A revisão barata é agora; depois da
broadphase e do solver, não é mais.
