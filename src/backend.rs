//! Backend Linux: conecta ao BlueZ, liga/desliga o receptor (interruptor
//! BLUETOOTH do deck), disfarça o PC de caixa de som, mantém o estado
//! atualizado para a interface e atende PLAY / STOP / REC.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::sync::mpsc::UnboundedReceiver;
use zbus::zvariant::Value;
use zbus::Connection;

use crate::bluez::{self, Adaptador};
use crate::estado::{Aparelho, Comando, Compartilhado, ALIAS};
use crate::receptor::Receptor;
use crate::{agente, sbc};

const CLASSE_MAIOR_AUDIO: &str = "4";
const CLASSE_MENOR_CAIXA: &str = "20"; // 0x14 = Loudspeaker
const AJUDANTE_CLASSE: &str = "/usr/lib/vitrola-bt/classe-caixa";
/// Vitrola conectada sem abrir o áudio por este tempo: o PC abre.
const ESPERA_AUDIO: Duration = Duration::from_secs(4);
/// Intervalo entre tentativas do PC de reconectar a vitrola.
const INTERVALO_RECONEXAO: Duration = Duration::from_secs(20);

type Chips = Arc<Mutex<HashMap<String, Option<(String, String)>>>>;

struct Backend {
    conn: Connection,
    c: Compartilhado,
    tocar_auto: bool,
    adaptador: Adaptador,
    receptor: Option<Arc<Receptor>>,
    agente: bool,
    alias_original: Option<String>,
    chips: Chips,
    /// Alavanca BLUETOOTH em ON (intenção do usuário, sobrevive a reinícios do bluetoothd).
    desejado: bool,
    /// Dono atual do nome org.bluez: muda quando o bluetoothd reinicia.
    dono_bluez: Option<String>,
    ultima_tentativa: Option<Instant>,
    /// Aparelhos que já mandaram áudio para o app (só esses o PC tenta reconectar).
    vitrolas: Vec<String>,
    conectada_desde: HashMap<String, Instant>,
    ultima_reconexao: Option<Instant>,
    reconectando: Arc<AtomicBool>,
    disfarcando: Arc<AtomicBool>,
    avisou_reconexao: bool,
}

fn arquivo_vitrolas() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("vitrola-bt").join("vitrolas.json"))
}

fn ler_vitrolas() -> Vec<String> {
    arquivo_vitrolas()
        .and_then(|c| std::fs::read_to_string(c).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn salvar_vitrolas(v: &[String]) {
    if let Some(c) = arquivo_vitrolas() {
        if let Some(d) = c.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(c, serde_json::to_string_pretty(v).unwrap_or_default());
    }
}

/// `ligar_ao_iniciar`: Some(true) força ligar (modo sem interface); None decide
/// pelo estado (vitrola já conectada ou PC já visível).
pub async fn rodar(
    c: Compartilhado,
    mut rx: UnboundedReceiver<Comando>,
    tocar_auto: bool,
    ligar_ao_iniciar: Option<bool>,
) {
    let conn = match Connection::system().await {
        Ok(conn) => conn,
        Err(e) => {
            c.trava().aviso = Some(format!("Sem acesso ao D-Bus do sistema: {e}"));
            c.log("Sem acesso ao D-Bus do sistema (BlueZ indisponível)");
            return esperar_saida(&mut rx).await;
        }
    };
    let objs = bluez::objetos(&conn).await.unwrap_or_default();
    let Some(adaptador) = bluez::adaptador(&objs) else {
        c.trava().aviso = Some("Nenhum adaptador Bluetooth encontrado".into());
        c.log("Nenhum adaptador Bluetooth encontrado (o serviço bluetooth está rodando?)");
        return esperar_saida(&mut rx).await;
    };
    let mut b = Backend {
        conn,
        c,
        tocar_auto,
        adaptador,
        receptor: None,
        agente: false,
        alias_original: None,
        chips: Arc::default(),
        desejado: false,
        dono_bluez: None,
        ultima_tentativa: None,
        vitrolas: ler_vitrolas(),
        conectada_desde: HashMap::new(),
        ultima_reconexao: None,
        reconectando: Arc::default(),
        disfarcando: Arc::default(),
        avisou_reconexao: false,
    };
    b.dono_bluez = b.dono_atual().await;
    b.atualizar().await;
    let ligar = ligar_ao_iniciar.unwrap_or_else(|| {
        let e = b.c.trava();
        b.adaptador.ligado && (b.adaptador.visivel || e.aparelho.is_some())
    });
    if ligar {
        b.ligar().await;
    }
    let mut relogio = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                None => { b.desligar().await; break; }
                Some(Comando::Sair(ack)) => { b.desligar().await; let _ = ack.send(()); break; }
                Some(cmd) => b.tratar(cmd).await,
            },
            _ = relogio.tick() => b.atualizar().await,
        }
    }
}

async fn esperar_saida(rx: &mut UnboundedReceiver<Comando>) {
    while let Some(cmd) = rx.recv().await {
        if let Comando::Sair(ack) = cmd {
            let _ = ack.send(());
            break;
        }
    }
}

impl Backend {
    async fn tratar(&mut self, cmd: Comando) {
        match cmd {
            Comando::LigarBluetooth => self.ligar().await,
            Comando::DesligarBluetooth => self.desligar().await,
            Comando::Play => match self.receptor.clone() {
                Some(r) => {
                    r.definir_tocar(true);
                    if r.tem_audio() {
                        self.c.log("PLAY: tocando nos alto-falantes");
                    } else {
                        // vitrola conectada mas parada: pede o START (em paralelo)
                        let c = self.c.clone();
                        tokio::spawn(async move {
                            r.acordar().await;
                            r.definir_tocar(true);
                            c.log(if r.tem_audio() {
                                "PLAY: tocando nos alto-falantes"
                            } else {
                                "PLAY: a vitrola não está mandando áudio"
                            });
                        });
                    }
                }
                None => self.c.log("PLAY: ligue o BLUETOOTH primeiro"),
            },
            Comando::Stop => {
                if let Some(r) = &self.receptor {
                    r.parar_gravacao();
                    r.definir_tocar(false);
                    self.c.log("STOP (a vitrola segue conectada)");
                }
            }
            Comando::Rec => match &self.receptor {
                Some(r) => r.alternar_gravacao(),
                None => self.c.log("REC: ligue o BLUETOOTH primeiro"),
            },
            Comando::TocarAuto(auto) => {
                self.tocar_auto = auto;
                if let Some(r) = &self.receptor {
                    r.definir_tocar_auto(auto);
                }
            }
            Comando::Sair(_) => {}
        }
        self.atualizar().await;
    }

    async fn dono_atual(&self) -> Option<String> {
        let dbus = zbus::fdo::DBusProxy::new(&self.conn).await.ok()?;
        let nome = zbus::names::BusName::try_from(bluez::BLUEZ).ok()?;
        dbus.get_name_owner(nome).await.ok().map(|n| n.to_string())
    }

    /// Se o bluetoothd reiniciou (crash, atualização), nosso receptor e o agente
    /// sumiram com ele: esquece os antigos e religa se a alavanca estiver em ON.
    async fn vigiar_bluez(&mut self) {
        let dono = self.dono_atual().await;
        if dono != self.dono_bluez {
            let reiniciou = self.dono_bluez.is_some() && dono.is_some();
            self.dono_bluez = dono.clone();
            self.receptor = None;
            self.agente = false;
            {
                let mut e = self.c.trava();
                e.fluxo = None;
                e.tocando = false;
                e.gravando = None;
            }
            if dono.is_none() {
                self.c.log("O serviço Bluetooth parou; aguardando ele voltar");
            } else if reiniciou {
                self.c.log("O serviço Bluetooth reiniciou");
            }
        }
        let pode_tentar = self.ultima_tentativa.is_none_or(|t| t.elapsed() > Duration::from_secs(5));
        if self.desejado && self.receptor.is_none() && self.dono_bluez.is_some() && pode_tentar {
            self.c.log("Religando o receptor");
            self.ligar().await;
        }
    }

    async fn atualizar(&mut self) {
        self.vigiar_bluez().await;
        let Ok(objs) = bluez::objetos(&self.conn).await else { return };
        if let Some(a) = bluez::adaptador(&objs) {
            self.adaptador = a;
        }
        let fontes = bluez::fontes_conectadas(&objs);
        let aparelho = fontes.first().map(|d| {
            let chip = self.chip(&d.mac);
            Aparelho {
                nome: d.nome.clone(),
                mac: d.mac.clone(),
                chip: chip.as_ref().map(|c| c.0.clone()),
                versao_bt: chip.map(|c| c.1),
            }
        });
        {
            let mut e = self.c.trava();
            e.bt_disponivel = true;
            e.bt_ligado = self.adaptador.ligado;
            e.visivel = self.adaptador.visivel;
            e.alias = self.adaptador.alias.clone();
            e.disfarce_caixa = self.adaptador.disfarcado();
            e.receptor_ligado = self.desejado;
            if aparelho.is_none() && e.fluxo.is_none() {
                e.medidas = Default::default();
            }
            e.aparelho = aparelho;
        }
        if self.desejado && self.receptor.is_some() {
            self.manter_conexao(&objs, &fontes).await;
        }
        self.c.repintar.pedir();
    }

    /// Com a alavanca em ON: invisível com a vitrola conectada (ninguém mais
    /// pareia), visível e disfarçado sem ela (para ela reconectar), abre o áudio
    /// se ela conectar calada e tenta reconectá-la a partir do PC.
    async fn manter_conexao(&mut self, objs: &bluez::Objetos, fontes: &[bluez::Dispositivo]) {
        let agora = Instant::now();
        let conectada = !fontes.is_empty();
        if self.c.trava().fluxo.is_some() {
            let mut mudou = false;
            for d in fontes {
                if !self.vitrolas.contains(&d.mac) {
                    self.vitrolas.push(d.mac.clone());
                    mudou = true;
                }
            }
            if mudou {
                salvar_vitrolas(&self.vitrolas);
            }
        }
        // Guardião único: com uma operação de conexão em andamento, não mexe em nada
        if self.receptor.as_ref().is_some_and(|r| r.ocupado()) {
            return;
        }
        let caminho = self.adaptador.caminho.clone();
        if conectada && self.adaptador.visivel {
            let _ =
                bluez::definir(&self.conn, &caminho, "org.bluez.Adapter1", "Discoverable", Value::Bool(false)).await;
        } else if !conectada && !self.adaptador.visivel {
            self.c.log("Sem vitrola conectada: PC visível para ela reconectar");
            self.anunciar().await;
        }
        if conectada {
            self.avisou_reconexao = false;
        }
        // Conectou mas não abriu o áudio: o PC abre (e escolhe dual channel)
        self.conectada_desde.retain(|k, _| fontes.iter().any(|d| &d.caminho == k));
        if let Some(r) = self.receptor.clone() {
            for d in fontes {
                let desde = *self.conectada_desde.entry(d.caminho.clone()).or_insert(agora);
                if agora.duration_since(desde) > ESPERA_AUDIO && !r.tem_fluxo_de(&d.caminho) {
                    // em paralelo: o ConnectProfile pode levar até ~30 s se a vitrola não responder
                    let (r, caminho) = (r.clone(), d.caminho.clone());
                    tokio::spawn(async move { r.garantir_audio(&caminho).await });
                }
            }
        }
        // Reconexão ativa: o PC chama as vitrolas conhecidas
        if !conectada && self.ultima_reconexao.is_none_or(|t| t.elapsed() > INTERVALO_RECONEXAO) {
            self.ultima_reconexao = Some(agora);
            self.tentar_reconectar(objs);
        }
    }

    fn tentar_reconectar(&mut self, objs: &bluez::Objetos) {
        if self.reconectando.load(Ordering::SeqCst) {
            return;
        }
        // a reconexão também passa pelo guardião único da conexão
        let Some(guarda) = self.receptor.as_ref().and_then(|r| r.guarda.clone().try_lock_owned().ok()) else {
            return;
        };
        let alvo = bluez::conhecidos(objs).into_iter().find(|d| self.vitrolas.contains(&d.mac));
        let Some(d) = alvo else { return };
        if !self.avisou_reconexao {
            self.c.log(format!("Tentando reconectar {} (a cada {} s)", d.nome, INTERVALO_RECONEXAO.as_secs()));
            self.avisou_reconexao = true;
        }
        let (conn, c, flag) = (self.conn.clone(), self.c.clone(), self.reconectando.clone());
        flag.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            let _guarda = guarda;
            let r = tokio::time::timeout(
                Duration::from_secs(15),
                bluez::chamar(&conn, &d.caminho, "org.bluez.Device1", "Connect", &()),
            )
            .await;
            if matches!(r, Ok(Ok(_))) {
                c.log(format!("{} reconectada pelo PC", d.nome));
            }
            flag.store(false, Ordering::SeqCst);
        });
    }

    /// Fabricante do chip e versão do Bluetooth via `hcitool info` (uma vez por aparelho).
    fn chip(&self, mac: &str) -> Option<(String, String)> {
        let mut chips = self.chips.lock().unwrap();
        if let Some(v) = chips.get(mac) {
            return v.clone();
        }
        chips.insert(mac.to_string(), None);
        let (mac, cache, c) = (mac.to_string(), self.chips.clone(), self.c.clone());
        tokio::spawn(async move {
            let saida =
                tokio::time::timeout(Duration::from_secs(20), Command::new("hcitool").args(["info", &mac]).output())
                    .await;
            let texto = match saida {
                Ok(Ok(o)) => String::from_utf8_lossy(&o.stdout).to_string(),
                _ => String::new(),
            };
            let fab = texto
                .lines()
                .find_map(|l| l.trim().strip_prefix("Manufacturer:"))
                .map(|s| s.rsplit_once(" (").map(|(n, _)| n).unwrap_or(s).trim().to_string());
            let ver = texto
                .lines()
                .find_map(|l| l.trim().strip_prefix("LMP Version:"))
                .and_then(|s| s.split_whitespace().next())
                .map(str::to_string);
            let v = Some((fab.unwrap_or_else(|| "desconhecido".into()), ver.unwrap_or_else(|| "?".into())));
            cache.lock().unwrap().insert(mac, v);
            c.repintar.pedir();
        });
        None
    }

    async fn ligar(&mut self) {
        self.desejado = true;
        self.ultima_tentativa = Some(Instant::now());
        if let Some(a) = bluez::objetos(&self.conn).await.ok().as_ref().and_then(bluez::adaptador) {
            self.adaptador = a;
        }
        if self.receptor.is_none() {
            match Receptor::registrar(&self.conn, &self.c, &self.adaptador.caminho, self.tocar_auto).await {
                Ok(r) => {
                    self.receptor = Some(r);
                    self.c.log(format!("Receptor pronto: SBC dual channel, bitpool até {}", sbc::BITPOOL_MAX_DUAL));
                }
                Err(e) => {
                    self.c.log(format!("Não consegui registrar o receptor no BlueZ: {}", bluez::nome_erro(&e)));
                    return;
                }
            }
        }
        self.c.trava().receptor_ligado = true;
        let _ = Command::new("rfkill").args(["unblock", "bluetooth"]).status().await;
        let caminho = self.adaptador.caminho.clone();
        if !self.adaptador.ligado {
            match bluez::definir(&self.conn, &caminho, "org.bluez.Adapter1", "Powered", Value::Bool(true)).await {
                Ok(()) => self.c.log("Bluetooth ligado"),
                Err(e) => return self.c.log(format!("Não consegui ligar o Bluetooth: {e}")),
            }
        }
        if !self.agente {
            match agente::registrar(&self.conn, &self.c).await {
                Ok(()) => self.agente = true,
                Err(e) => self.c.log(format!("Agente de pareamento indisponível: {}", bluez::nome_erro(&e))),
            }
        }
        let objs = bluez::objetos(&self.conn).await.unwrap_or_default();
        if bluez::fontes_conectadas(&objs).is_empty() {
            self.anunciar().await;
        } else {
            self.c.log("Vitrola já conectada: receptor ligado");
        }
    }

    /// Disfarça o PC de caixa de som (classe Audio/Video · Loudspeaker). Roda em
    /// paralelo e com limite de tempo: a senha pode demorar e o btmgmt pode travar,
    /// e nada disso pode parar a lógica do Bluetooth. Com o pacote instalado, a
    /// regra do polkit libera o ajudante sem senha.
    fn disfarcar(&mut self) {
        if self.adaptador.disfarcado() || self.disfarcando.load(Ordering::SeqCst) {
            return;
        }
        self.disfarcando.store(true, Ordering::SeqCst);
        self.c.log("Disfarçando o PC de caixa de som");
        let (c, flag) = (self.c.clone(), self.disfarcando.clone());
        tokio::spawn(async move {
            let mut cmd = Command::new("pkexec");
            if std::path::Path::new(AJUDANTE_CLASSE).exists() {
                cmd.arg(AJUDANTE_CLASSE);
            } else {
                // Desenvolvimento (sem o pacote): btmgmt num pseudo-terminal, pois
                // sem terminal ele não termina (BlueZ 5.72)
                cmd.args([
                    "script",
                    "-qec",
                    &format!("btmgmt class {CLASSE_MAIOR_AUDIO} {CLASSE_MENOR_CAIXA}"),
                    "/dev/null",
                ]);
            }
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            let ok = match cmd.spawn() {
                Ok(mut filho) => matches!(
                    tokio::time::timeout(Duration::from_secs(120), filho.wait()).await,
                    Ok(Ok(s)) if s.success()
                ),
                Err(_) => false,
            };
            c.log(if ok {
                "O PC agora se apresenta como caixa de som"
            } else {
                "Classe não alterada (senha cancelada?): a vitrola pode não achar o PC"
            });
            flag.store(false, Ordering::SeqCst);
        });
    }

    /// Visível como "Vitrola PC", sem prazo, aceitando pareamento.
    async fn anunciar(&mut self) {
        self.disfarcar();
        if self.alias_original.is_none() {
            let atual = self.adaptador.alias.clone();
            self.alias_original = Some(if atual.is_empty() || atual == ALIAS { hostname() } else { atual });
        }
        let (caminho, ad) = (self.adaptador.caminho.clone(), "org.bluez.Adapter1");
        let r = async {
            bluez::definir(&self.conn, &caminho, ad, "Alias", Value::from(ALIAS)).await?;
            bluez::definir(&self.conn, &caminho, ad, "DiscoverableTimeout", Value::U32(0)).await?;
            bluez::definir(&self.conn, &caminho, ad, "Pairable", Value::Bool(true)).await?;
            bluez::definir(&self.conn, &caminho, ad, "Discoverable", Value::Bool(true)).await
        }
        .await;
        match r {
            Ok(()) => {
                self.adaptador.visivel = true;
                self.c.log(format!("Visível como \"{ALIAS}\". Procure por ele na vitrola."));
            }
            Err(e) => self.c.log(format!("Não consegui ficar visível: {e}")),
        }
    }

    /// Desliga o BLUETOOTH do deck. A ordem importa: o BlueZ 5.72 cai (segfault)
    /// se o receptor sair do ar com um fluxo A2DP aberto. Então: para o áudio,
    /// solta a vitrola, espera o BlueZ fechar os fluxos e só então desregistra.
    async fn desligar(&mut self) {
        self.desejado = false;
        let caminho = self.adaptador.caminho.clone();
        if let Some(r) = &self.receptor {
            r.parar_gravacao();
            r.parar_tudo().await;
        }
        let objs = bluez::objetos(&self.conn).await.unwrap_or_default();
        for d in bluez::fontes_conectadas(&objs) {
            match bluez::chamar(&self.conn, &d.caminho, "org.bluez.Device1", "Disconnect", &()).await {
                Ok(_) => self.c.log(format!("{} desconectada", d.nome)),
                Err(e) => self.c.log(format!("Não consegui desconectar {}: {}", d.nome, bluez::nome_erro(&e))),
            }
        }
        if let Some(r) = self.receptor.take() {
            if !r.esperar_sem_fluxos(Duration::from_secs(4)).await {
                self.c.log("O BlueZ não fechou o fluxo a tempo; tirando o receptor do ar mesmo assim");
            }
            r.desregistrar(&caminho).await;
        }
        let ad = "org.bluez.Adapter1";
        let _ = bluez::definir(&self.conn, &caminho, ad, "Discoverable", Value::Bool(false)).await;
        if let Some(alias) = self.alias_original.take() {
            let _ = bluez::definir(&self.conn, &caminho, ad, "Alias", Value::from(alias.as_str())).await;
        }
        if self.agente {
            agente::desregistrar(&self.conn).await;
            self.agente = false;
        }
        {
            let mut e = self.c.trava();
            e.receptor_ligado = false;
            e.fluxo = None;
            e.tocando = false;
            e.gravando = None;
        }
        self.c.log("BLUETOOTH desligado: receptor fora do ar");
    }
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "linux".into())
}
