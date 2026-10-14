//! Estado compartilhado entre o backend (Bluetooth/áudio) e a interface, e os
//! canais de comando entre eles.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use crate::medidor::Medidas;
use crate::sbc;

pub const APP_ID: &str = "io.github._4snt.VitrolaBT";
pub const ALIAS: &str = "Vitrola PC";
const MAX_LOG: usize = 400;

#[derive(Clone, Debug, Default)]
pub struct Aparelho {
    pub nome: String,
    pub mac: String,
    pub chip: Option<String>,
    pub versao_bt: Option<String>,
}

#[derive(Default)]
pub struct Estado {
    pub bt_disponivel: bool,
    pub bt_ligado: bool,
    pub disfarce_caixa: bool,
    pub visivel: bool,
    pub alias: String,
    /// Interruptor BLUETOOTH do deck: receptor no ar.
    pub receptor_ligado: bool,
    pub aparelho: Option<Aparelho>,
    pub fluxo: Option<sbc::Config>,
    pub tocando: bool,
    pub gravando: Option<(PathBuf, Instant)>,
    pub inicio_play: Option<Instant>,
    /// Entrada (o que chega da vitrola) e saída (depois do EQ, filtros e volume).
    pub medidas: Medidas,
    pub medidas_saida: Medidas,
    pub ultima_medida: Option<Instant>,
    pub log: VecDeque<String>,
    /// Mensagem fixa (ex.: sistema sem suporte ao receptor).
    pub aviso: Option<String>,
    /// Trabalhador de áudio atual (só ele atualiza tocando/gravando).
    pub trabalhador_ativo: u64,
    /// Saída de som padrão do sistema (nome) e o volume dela (0..1, mudo).
    pub saida_dispositivo: Option<String>,
    pub saida_volume: Option<(f32, bool)>,
    /// Equalizador e funções de escuta; `som_versao` muda a cada ajuste.
    pub som: crate::dsp::Som,
    pub som_versao: u64,
}

impl Estado {
    pub fn registrar(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        eprintln!("{msg}");
        self.log.push_back(format!("{}  {msg}", hora_local()));
        while self.log.len() > MAX_LOG {
            self.log.pop_front();
        }
    }
}

/// Pede um novo quadro à interface (o backend não conhece o egui diretamente).
#[derive(Clone, Default)]
pub struct Repintador(Arc<Mutex<Option<egui::Context>>>);

impl Repintador {
    pub fn definir(&self, ctx: egui::Context) {
        *self.0.lock().unwrap() = Some(ctx);
    }
    pub fn pedir(&self) {
        if let Some(ctx) = self.0.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }
}

#[derive(Clone)]
pub struct Compartilhado {
    estado: Arc<Mutex<Estado>>,
    pub repintar: Repintador,
}

impl Compartilhado {
    pub fn novo() -> Self {
        Compartilhado { estado: Arc::new(Mutex::new(Estado::default())), repintar: Repintador::default() }
    }
    pub fn trava(&self) -> MutexGuard<'_, Estado> {
        self.estado.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn log(&self, msg: impl Into<String>) {
        self.trava().registrar(msg);
        self.repintar.pedir();
    }
}

/// Interface -> backend.
pub enum Comando {
    LigarBluetooth,
    DesligarBluetooth,
    Play,
    Stop,
    Rec,
    /// Botão HI-FI: renegociar dual channel agora.
    HiFi,
    /// Preferência "tocar ao conectar" mudou.
    TocarAuto(bool),
    /// Encerra tudo (grava o FLAC, solta a vitrola) e avisa pelo canal.
    Sair(std::sync::mpsc::Sender<()>),
}

/// Bandeja/segunda instância -> interface.
#[derive(Debug)]
pub enum EventoUi {
    Mostrar,
    /// Abrir a janela de ajustes (menu da bandeja).
    Ajustes,
    Sair,
}

pub fn pasta_gravacoes() -> PathBuf {
    if let Some(p) = std::env::var_os("VITROLA_REC_DIR") {
        return PathBuf::from(p);
    }
    dirs::audio_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Music")))
        .unwrap_or_else(std::env::temp_dir)
        .join("Vitrola")
}

#[cfg(unix)]
pub fn hora_local() -> String {
    // SAFETY: localtime_r escreve na struct tm que fornecemos.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

#[cfg(not(unix))]
pub fn hora_local() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
}

pub fn carimbo_arquivo() -> String {
    #[cfg(unix)]
    // SAFETY: idem hora_local.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
    #[cfg(not(unix))]
    {
        hora_local().replace(':', "")
    }
}
