//! Pool de threads para execucao paralela de tarefas curtas.
//!
//! O consumidor imediato e o scheduler de sistemas: a cada etapa, um conjunto
//! conhecido de sistemas que nao conflitam roda junto e o passo so avanca quando
//! todos terminam. E fork-join sobre um lote conhecido, nao uma arvore
//! recursiva.
//!
//! ```
//! use krateus_core::jobs::JobPool;
//!
//! let pool = JobPool::new(4);
//! let mut a = 0_u64;
//! let mut b = 0_u64;
//!
//! pool.scope(|s| {
//!     s.spawn(|| a = (1..=1_000_u64).sum());
//!     s.spawn(|| b = (1..=1_000_u64).filter(|n| n % 3 == 0).sum());
//! });
//!
//! // Ao sair do escopo, as duas tarefas terminaram.
//! assert_eq!(a, 500_500);
//! assert_eq!(b, 166_833);
//! ```
//!
//! # Determinismo
//!
//! A **ordem de execucao** das tarefas varia entre execucoes — e disso que vem o
//! paralelismo. O determinismo da simulacao nao depende dela: quem usa este pool
//! e responsavel por so paralelizar trabalho cujo efeito comuta. No scheduler,
//! isso e garantido pelo `Access`, que so deixa rodar junto o que nao colide.
//!
//! Ver a secao 15 do documento de visao e a D09 em `docs/DECISOES.md`.
//!
//! # Emprestimo da pilha
//!
//! [`Scope::spawn`] aceita closures que emprestam variaveis locais. A garantia e
//! estrutural: [`JobPool::scope`] nao retorna antes de toda tarefa daquele
//! escopo ter terminado, inclusive quando alguma entra em panico.

use std::any::Any;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Tempo maximo que a thread dona de um escopo dorme antes de reconferir.
///
/// Existe apenas para fechar a janela entre "conferi o contador" e "esperei o
/// aviso": sem o limite, um aviso emitido nessa fresta seria perdido e o escopo
/// dormiria para sempre. Com ele, o pior caso e um atraso desta ordem.
const ESPERA_MAXIMA: Duration = Duration::from_micros(200);

type Trabalho = Box<dyn FnOnce() + Send + 'static>;

/// Estado compartilhado de um escopo: quantas tarefas faltam e o primeiro
/// panico ocorrido.
struct EstadoEscopo {
    pendentes: AtomicUsize,
    guarda: Mutex<()>,
    concluiu: Condvar,
    panico: Mutex<Option<Box<dyn Any + Send + 'static>>>,
}

impl EstadoEscopo {
    fn novo() -> Self {
        Self {
            pendentes: AtomicUsize::new(0),
            guarda: Mutex::new(()),
            concluiu: Condvar::new(),
            panico: Mutex::new(None),
        }
    }

    fn concluir_uma(&self) {
        if self.pendentes.fetch_sub(1, Ordering::AcqRel) == 1 {
            // O `lock` sincroniza com quem esta prestes a dormir, garantindo que
            // o aviso nao caia na fresta entre conferir o contador e esperar.
            let _g = self.guarda.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            self.concluiu.notify_all();
        }
    }

    fn registrar_panico(&self, carga: Box<dyn Any + Send + 'static>) {
        let mut slot = self.panico.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // So o primeiro interessa; os demais sao consequencia provavel dele.
        if slot.is_none() {
            *slot = Some(carga);
        }
    }
}

struct Tarefa {
    executar: Trabalho,
    escopo: Arc<EstadoEscopo>,
}

impl Tarefa {
    fn executar(self) {
        let Self { executar, escopo } = self;

        // `catch_unwind` impede que o panico de uma tarefa mate a thread
        // trabalhadora e encolha o pool. A carga e guardada e relancada na
        // thread dona do escopo, onde ha contexto para trata-la.
        let resultado = catch_unwind(AssertUnwindSafe(executar));
        if let Err(carga) = resultado {
            escopo.registrar_panico(carga);
        }

        escopo.concluir_uma();
    }
}

/// Fila de tarefas e sinalizacao, compartilhadas com as threads trabalhadoras.
struct Compartilhado {
    fila: Mutex<VecDeque<Tarefa>>,
    ha_trabalho: Condvar,
    encerrando: AtomicBool,
}

impl Compartilhado {
    fn empurrar(&self, tarefa: Tarefa) {
        {
            let mut fila = self.fila.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            fila.push_back(tarefa);
        }
        self.ha_trabalho.notify_one();
    }

    fn tentar_pegar(&self) -> Option<Tarefa> {
        let mut fila = self.fila.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        fila.pop_front()
    }

    /// Laco de uma thread trabalhadora.
    fn trabalhar(&self) {
        loop {
            let tarefa = {
                let mut fila = self.fila.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                loop {
                    if let Some(t) = fila.pop_front() {
                        break Some(t);
                    }
                    if self.encerrando.load(Ordering::Acquire) {
                        break None;
                    }
                    fila = self
                        .ha_trabalho
                        .wait(fila)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            };

            match tarefa {
                Some(t) => t.executar(),
                None => return,
            }
        }
    }
}

/// Conjunto fixo de threads que consomem uma fila de tarefas.
///
/// Criar o pool custa caro; usa-lo, nao. A intencao e um pool vivo durante toda
/// a execucao, e nao um por passo de simulacao.
pub struct JobPool {
    compartilhado: Arc<Compartilhado>,
    threads: Vec<JoinHandle<()>>,
}

impl JobPool {
    /// Cria um pool com `threads` threads trabalhadoras.
    ///
    /// A thread que chama [`scope`](Self::scope) tambem executa tarefas, entao
    /// `threads` e o paralelismo adicional, nao o total. `0` e valido e produz
    /// um pool onde tudo roda na thread chamadora — util para reproduzir um bug
    /// sem concorrencia, ou para medir o baseline sequencial.
    #[must_use]
    pub fn new(threads: usize) -> Self {
        let compartilhado = Arc::new(Compartilhado {
            fila: Mutex::new(VecDeque::new()),
            ha_trabalho: Condvar::new(),
            encerrando: AtomicBool::new(false),
        });

        let threads = (0..threads)
            .map(|i| {
                let compartilhado = Arc::clone(&compartilhado);
                thread::Builder::new()
                    .name(format!("krateus-job-{i}"))
                    .spawn(move || compartilhado.trabalhar())
                    .expect("falha ao criar thread trabalhadora")
            })
            .collect();

        Self { compartilhado, threads }
    }

    /// Cria um pool dimensionado pela maquina.
    ///
    /// Usa um a menos que o paralelismo disponivel, ja que a thread chamadora
    /// participa. A secao 17 do documento de visao pede que a engine se adapte
    /// as capacidades locais.
    #[must_use]
    pub fn com_paralelismo_do_sistema() -> Self {
        let total = thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Self::new(total.saturating_sub(1))
    }

    /// Numero de threads trabalhadoras, sem contar a chamadora.
    #[inline]
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }

    /// Executa um lote de tarefas que podem emprestar da pilha, e espera todas.
    ///
    /// # Panics
    ///
    /// Se alguma tarefa entrar em panico, o panico e relancado aqui, depois que
    /// todas as outras terminarem. Isso preserva a garantia de que nenhuma
    /// tarefa sobrevive ao escopo.
    pub fn scope<'env, F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Scope<'_, 'env>) -> R,
    {
        let estado = Arc::new(EstadoEscopo::novo());
        let escopo = Scope { pool: self, estado: Arc::clone(&estado), _env: PhantomData };

        // Mesmo que `f` entre em panico, as tarefas ja enfileiradas precisam ser
        // esperadas antes de liberar a pilha de onde elas emprestam.
        let resultado = catch_unwind(AssertUnwindSafe(|| f(&escopo)));
        self.aguardar(&estado);

        if let Some(carga) =
            estado.panico.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
        {
            resume_unwind(carga);
        }

        match resultado {
            Ok(v) => v,
            Err(carga) => resume_unwind(carga),
        }
    }

    /// Espera o escopo esvaziar, executando tarefas enquanto isso.
    ///
    /// A thread chamadora nao fica ociosa: com o pool de tamanho zero, e ela
    /// quem executa tudo.
    fn aguardar(&self, estado: &EstadoEscopo) {
        loop {
            if estado.pendentes.load(Ordering::Acquire) == 0 {
                return;
            }

            if let Some(tarefa) = self.compartilhado.tentar_pegar() {
                tarefa.executar();
                continue;
            }

            // Fila vazia mas ainda ha tarefas em voo noutras threads.
            let guarda = estado.guarda.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if estado.pendentes.load(Ordering::Acquire) != 0 {
                let _ = estado.concluiu.wait_timeout(guarda, ESPERA_MAXIMA);
            }
        }
    }
}

impl Drop for JobPool {
    fn drop(&mut self) {
        self.compartilhado.encerrando.store(true, Ordering::Release);
        {
            let _g =
                self.compartilhado.fila.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            self.compartilhado.ha_trabalho.notify_all();
        }

        for t in self.threads.drain(..) {
            // Uma thread trabalhadora nao propaga panico: `Tarefa::executar` ja
            // os captura. Ignorar aqui e seguro e evita panico dentro de `drop`.
            let _ = t.join();
        }
    }
}

impl std::fmt::Debug for JobPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobPool").field("threads", &self.threads.len()).finish()
    }
}

/// Lote de tarefas que compartilham um tempo de vida.
///
/// O parametro `'env` amarra o que as tarefas podem emprestar; `'scope` amarra o
/// proprio lote.
pub struct Scope<'scope, 'env> {
    pool: &'scope JobPool,
    estado: Arc<EstadoEscopo>,
    /// Invariante em `'env`: impede que o compilador aceite emprestimos de vida
    /// mais curta que a do escopo.
    _env: PhantomData<&'env mut &'env ()>,
}

impl<'env> Scope<'_, 'env> {
    /// Enfileira uma tarefa.
    ///
    /// A closure pode emprestar de `'env`. Nenhuma tarefa sobrevive ao retorno
    /// de [`JobPool::scope`].
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'env,
    {
        self.estado.pendentes.fetch_add(1, Ordering::Relaxed);

        let boxed: Box<dyn FnOnce() + Send + 'env> = Box::new(f);
        // SAFETY: a fila exige `'static`, mas a tarefa so empresta de `'env`.
        // O que torna isto correto e a garantia estrutural de `JobPool::scope`:
        // ele nao retorna antes de `aguardar` ver o contador de pendentes zerar,
        // e o contador so zera depois de cada tarefa ter sido executada ate o
        // fim — inclusive as que entram em panico, capturadas por
        // `Tarefa::executar`. Logo nenhuma tarefa observa `'env` ja encerrado.
        let executar: Trabalho = unsafe {
            std::mem::transmute::<Box<dyn FnOnce() + Send + 'env>, Box<dyn FnOnce() + Send>>(boxed)
        };

        self.pool.compartilhado.empurrar(Tarefa { executar, escopo: Arc::clone(&self.estado) });
    }
}

impl std::fmt::Debug for Scope<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scope")
            .field("pendentes", &self.estado.pendentes.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn executa_todas_as_tarefas() {
        let pool = JobPool::new(4);
        let contador = AtomicU32::new(0);

        pool.scope(|s| {
            for _ in 0..100 {
                s.spawn(|| {
                    contador.fetch_add(1, Ordering::Relaxed);
                });
            }
        });

        assert_eq!(contador.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn tarefas_podem_emprestar_da_pilha() {
        let pool = JobPool::new(2);
        let mut a = 0_u64;
        let mut b = 0_u64;

        pool.scope(|s| {
            s.spawn(|| a = 1);
            s.spawn(|| b = 2);
        });

        assert_eq!((a, b), (1, 2));
    }

    #[test]
    fn escreve_em_fatias_disjuntas() {
        let pool = JobPool::new(4);
        let mut dados = vec![0_u64; 1000];

        pool.scope(|s| {
            for (i, pedaco) in dados.chunks_mut(100).enumerate() {
                s.spawn(move || {
                    for v in pedaco.iter_mut() {
                        *v = i as u64;
                    }
                });
            }
        });

        assert_eq!(dados[0], 0);
        assert_eq!(dados[999], 9);
    }

    #[test]
    fn pool_sem_threads_executa_na_chamadora() {
        let pool = JobPool::new(0);
        assert_eq!(pool.thread_count(), 0);

        let contador = AtomicU32::new(0);
        pool.scope(|s| {
            for _ in 0..10 {
                s.spawn(|| {
                    contador.fetch_add(1, Ordering::Relaxed);
                });
            }
        });

        assert_eq!(contador.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn escopo_vazio_retorna_imediatamente() {
        let pool = JobPool::new(2);
        pool.scope(|_| {});
    }

    #[test]
    fn scope_devolve_o_valor_da_closure() {
        let pool = JobPool::new(2);
        let n = pool.scope(|s| {
            s.spawn(|| {});
            42
        });
        assert_eq!(n, 42);
    }

    #[test]
    fn escopos_sucessivos_reaproveitam_o_pool() {
        let pool = JobPool::new(3);
        let contador = AtomicU32::new(0);

        for _ in 0..50 {
            pool.scope(|s| {
                for _ in 0..10 {
                    s.spawn(|| {
                        contador.fetch_add(1, Ordering::Relaxed);
                    });
                }
            });
        }

        assert_eq!(contador.load(Ordering::Relaxed), 500);
    }

    #[test]
    #[should_panic(expected = "tarefa quebrada")]
    fn panico_em_tarefa_e_relancado_na_chamadora() {
        let pool = JobPool::new(2);
        pool.scope(|s| {
            s.spawn(|| panic!("tarefa quebrada"));
        });
    }

    #[test]
    fn panico_em_uma_tarefa_nao_impede_as_outras_de_terminar() {
        let pool = JobPool::new(3);
        let concluidas = Arc::new(AtomicU32::new(0));

        let copia = Arc::clone(&concluidas);
        let resultado = std::panic::catch_unwind(AssertUnwindSafe(|| {
            pool.scope(|s| {
                s.spawn(|| panic!("uma falhou"));
                for _ in 0..20 {
                    let c = Arc::clone(&copia);
                    s.spawn(move || {
                        c.fetch_add(1, Ordering::Relaxed);
                    });
                }
            });
        }));

        assert!(resultado.is_err(), "o panico precisa chegar a chamadora");
        assert_eq!(
            concluidas.load(Ordering::Relaxed),
            20,
            "as demais tarefas do escopo precisam ter terminado mesmo assim"
        );
    }

    #[test]
    fn pool_continua_utilizavel_apos_um_panico() {
        let pool = JobPool::new(2);

        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            pool.scope(|s| s.spawn(|| panic!("boom")));
        }));

        // As threads trabalhadoras sobreviveram: `catch_unwind` na tarefa
        // impede que o panico as mate.
        let contador = AtomicU32::new(0);
        pool.scope(|s| {
            for _ in 0..10 {
                s.spawn(|| {
                    contador.fetch_add(1, Ordering::Relaxed);
                });
            }
        });
        assert_eq!(contador.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn resultado_agregado_independe_da_ordem_de_execucao() {
        // O paralelismo torna a ordem imprevisivel; a soma nao pode depender
        // dela. E o contrato que o scheduler vai honrar via Access.
        let pool = JobPool::new(4);
        let soma = AtomicU32::new(0);

        for _ in 0..20 {
            soma.store(0, Ordering::Relaxed);
            pool.scope(|s| {
                for i in 1..=100_u32 {
                    let soma = &soma;
                    s.spawn(move || {
                        soma.fetch_add(i, Ordering::Relaxed);
                    });
                }
            });
            assert_eq!(soma.load(Ordering::Relaxed), 5050);
        }
    }
}
