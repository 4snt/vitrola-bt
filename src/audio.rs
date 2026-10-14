//! Trabalhador de áudio: uma thread por transporte A2DP. Lê os pacotes RTP do
//! socket L2CAP, decodifica o SBC e entrega o PCM para a saída de som, os
//! medidores e o gravador FLAC. Nada aqui toca na thread da interface.

use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::dsp::Processador;
use crate::estado::Compartilhado;
use crate::flac::Flac;
use crate::medidor::Medidor;
use crate::saida::Saida;
use crate::sbc::{self, Libsbc, Sbc};

pub enum Controle {
    Tocar(bool),
    Gravar(Option<PathBuf>),
    Parar,
}

pub struct Trabalhador {
    tx: mpsc::Sender<Controle>,
    fio: Option<JoinHandle<()>>,
}

impl Trabalhador {
    pub fn enviar(&self, c: Controle) {
        let _ = self.tx.send(c);
    }

    /// Para a thread e espera ela fechar o FLAC e a saída (bloqueia até ~200 ms).
    pub fn parar_e_esperar(mut self) {
        let _ = self.tx.send(Controle::Parar);
        if let Some(f) = self.fio.take() {
            let _ = f.join();
        }
    }
}

impl Drop for Trabalhador {
    fn drop(&mut self) {
        let _ = self.tx.send(Controle::Parar);
    }
}

/// Sem pacote nenhum por este tempo = fluxo travado (com o disco parado a
/// vitrola ainda manda quadros de silêncio).
const SEM_DADOS: Duration = Duration::from_secs(3);

/// `sem_dados`: chamado (uma vez por travada) quando a vitrola para de mandar pacotes.
pub fn iniciar(
    fd: OwnedFd,
    mtu: usize,
    cfg: sbc::Config,
    mac: String,
    tocar: bool,
    c: Compartilhado,
    sem_dados: Box<dyn Fn() + Send>,
) -> Trabalhador {
    static PROXIMO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = PROXIMO.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    c.trava().trabalhador_ativo = id;
    let (tx, rx) = mpsc::channel();
    let fio = std::thread::Builder::new()
        .name("vitrola-audio".into())
        .spawn(move || laco(id, fd, mtu, cfg, mac, tocar, rx, c, sem_dados))
        .expect("thread de áudio");
    Trabalhador { tx, fio: Some(fio) }
}

#[allow(clippy::too_many_arguments)]
fn laco(
    id: u64,
    fd: OwnedFd,
    mtu: usize,
    cfg: sbc::Config,
    mac: String,
    tocar: bool,
    rx: mpsc::Receiver<Controle>,
    c: Compartilhado,
    sem_dados: Box<dyn Fn() + Send>,
) {
    let lib = match Libsbc::carregar() {
        Ok(l) => l,
        Err(e) => return c.log(e),
    };
    let mut dec = match Sbc::novo(&lib) {
        Ok(d) => d,
        Err(e) => return c.log(e),
    };
    let raw = fd.as_raw_fd();
    let mut sock = File::from(fd);
    let mut pacote = vec![0u8; mtu.max(1024)];
    let mut pcm = Vec::with_capacity(16 * 1024);
    let mut medidor = Medidor::novo(cfg.rate);
    let mut medidor_saida = Medidor::novo(cfg.rate);
    let (som, mut versao_som) = {
        let e = c.trava();
        (e.som.clone(), e.som_versao)
    };
    let mut dsp = Processador::novo(cfg.rate, &som);
    let mut escuta: Vec<u8> = Vec::with_capacity(16 * 1024);
    let mut saida: Option<Saida> = None;
    let mut gravacao: Option<(Flac, PathBuf)> = None;

    let abrir_saida = |c: &Compartilhado| match Saida::abrir(&cfg, &mac) {
        Ok(s) => Some(s),
        Err(e) => {
            c.log(format!("Não consegui abrir a saída de som (pw-cat): {e}"));
            None
        }
    };
    if tocar {
        saida = abrir_saida(&c);
    }
    atualizar(&c, id, saida.is_some(), &gravacao);

    let mut ultimo_pacote = Instant::now();
    let mut avisou = false;
    'principal: loop {
        while let Ok(ctl) = rx.try_recv() {
            match ctl {
                Controle::Tocar(true) if saida.is_none() => saida = abrir_saida(&c),
                Controle::Tocar(false) => saida = None,
                Controle::Gravar(Some(caminho)) if gravacao.is_none() => match Flac::criar(&caminho, cfg.rate) {
                    Ok(f) => {
                        c.log(format!("REC: gravando em {}", caminho.display()));
                        gravacao = Some((f, caminho));
                    }
                    Err(e) => c.log(format!("Não consegui criar {}: {e}", caminho.display())),
                },
                Controle::Gravar(None) => fechar_gravacao(&c, gravacao.take()),
                Controle::Parar => break 'principal,
                _ => {}
            }
            atualizar(&c, id, saida.is_some(), &gravacao);
        }

        let mut pfd = libc::pollfd { fd: raw, events: libc::POLLIN, revents: 0 };
        // SAFETY: pfd aponta para um pollfd válido.
        let r = unsafe { libc::poll(&mut pfd, 1, 100) };
        if r == 0 {
            if !avisou && ultimo_pacote.elapsed() > SEM_DADOS {
                avisou = true;
                sem_dados();
            }
            continue;
        }
        if r < 0 || pfd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            break;
        }
        let n = match sock.read(&mut pacote) {
            Ok(0) => break,
            Ok(n) => {
                ultimo_pacote = Instant::now();
                avisou = false;
                n
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => break,
        };
        let Some(payload) = sbc::payload_rtp(&pacote[..n]) else { continue };
        pcm.clear();
        dec.decodificar(payload, &mut pcm);
        if pcm.is_empty() {
            continue;
        }
        if let Some(m) = medidor.alimentar(&pcm) {
            {
                let mut e = c.trava();
                e.medidas = m;
                e.ultima_medida = Some(Instant::now());
            }
            c.repintar.pedir();
        }
        {
            // ajustes de som mudaram na interface?
            let e = c.trava();
            if e.som_versao != versao_som {
                versao_som = e.som_versao;
                dsp.definir(&e.som);
            }
        }
        // o EQ só afeta o que se ouve; o REC grava o sinal original
        dsp.processar_f32(&pcm, &mut escuta);
        if let Some(m) = medidor_saida.alimentar_f32(&escuta) {
            c.trava().medidas_saida = m;
        }
        if let Some(s) = saida.as_mut() {
            if s.escrever(&escuta).is_err() {
                c.log("A saída de som fechou (pw-cat)");
                saida = None;
                atualizar(&c, id, false, &gravacao);
            }
        }
        if let Some((f, caminho)) = gravacao.as_mut() {
            if let Err(e) = f.escrever(&pcm) {
                c.log(format!("Erro gravando {}: {e}", caminho.display()));
                gravacao = None;
                atualizar(&c, id, saida.is_some(), &gravacao);
            }
        }
    }
    fechar_gravacao(&c, gravacao.take());
    drop(saida);
    atualizar(&c, id, false, &None);
    std::thread::sleep(Duration::from_millis(10));
}

fn fechar_gravacao(c: &Compartilhado, g: Option<(Flac, PathBuf)>) {
    if let Some((f, caminho)) = g {
        match f.finalizar() {
            Ok(n) => c.log(format!("Gravação salva: {} ({:.0} s)", caminho.display(), n as f64 / 48_000.0)),
            Err(e) => c.log(format!("Erro ao fechar {}: {e}", caminho.display())),
        }
    }
}

fn atualizar(c: &Compartilhado, id: u64, tocando: bool, gravacao: &Option<(Flac, PathBuf)>) {
    {
        let mut e = c.trava();
        // Só o trabalhador atual manda no PLAY/REC: numa reconexão rápida o
        // antigo termina depois que o novo começou e não pode apagar o estado dele
        if e.trabalhador_ativo != id {
            return;
        }
        if tocando && !e.tocando {
            e.inicio_play = Some(Instant::now());
        } else if !tocando {
            e.inicio_play = None;
        }
        e.tocando = tocando;
        match gravacao {
            Some((_, caminho)) => {
                if e.gravando.as_ref().map(|g| &g.0) != Some(caminho) {
                    e.gravando = Some((caminho.clone(), Instant::now()));
                }
            }
            None => e.gravando = None,
        }
    }
    c.repintar.pedir();
}
