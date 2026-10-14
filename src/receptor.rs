//! Receptor A2DP próprio: um endpoint SBC Sink registrado no BlueZ que força o
//! maior bitrate que o transmissor aceita (dual channel) e entrega o áudio ao
//! trabalhador de áudio.
//!
//! O chip da vitrola (JieLi) ignora as capacidades que anunciamos quando é ele
//! quem configura (escolhe joint stereo, ~219 kbps). Então o PC abre o A2DP ou
//! pede a reconfiguração ao endpoint dele, e aí escolhemos dual channel (~396 kbps).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedObjectPath, OwnedValue, Value};
use zbus::{fdo, interface, Connection};

use crate::audio::{self, Controle, Trabalhador};
use crate::bluez::{self, BLUEZ, UUID_A2DP_SINK, UUID_A2DP_SOURCE};
use crate::estado::{carimbo_arquivo, pasta_gravacoes, Compartilhado};
use crate::sbc::{self, Modo};

pub const CAMINHO: &str = "/vitrola/a2dp_sink_sbc";
/// Se o chip refizer a própria config depois da nossa, insistimos em dual
/// channel, mas no máximo TENTATIVAS vezes por JANELA.
const TENTATIVAS: usize = 3;
const JANELA: Duration = Duration::from_secs(120);
/// Tempo tocando sem travar antes de tentar o upgrade para dual channel.
const ESTAVEL: Duration = Duration::from_secs(20);
/// Depois que o dual trava, fica em joint stereo por este tempo.
const BLOQUEIO_DUAL: Duration = Duration::from_secs(2 * 60);
/// Em joint stereo, de quanto em quanto tempo tenta de novo o dual channel.
const INTERVALO_UPGRADE: Duration = Duration::from_secs(60);

struct Fluxo {
    cfg: sbc::Config,
    mac: String,
    trabalhador: Option<Trabalhador>,
    vigia: Option<tokio::task::JoinHandle<()>>,
    /// Um Acquire em andamento: impede duas chamadas de pegar o mesmo transporte.
    iniciando: bool,
}

pub struct Receptor {
    conn: Connection,
    c: Compartilhado,
    tocar_auto: AtomicBool,
    tocar: AtomicBool,
    fluxos: Mutex<HashMap<String, Fluxo>>,
    tentativas: Mutex<HashMap<String, Vec<Instant>>>,
    travadas: Mutex<HashMap<String, Vec<Instant>>>,
    /// Guardião único da conexão: só uma operação (reconectar, abrir o áudio,
    /// reconfigurar, reanimar) por vez. Quem chega com outra em andamento desiste
    /// e tenta no próximo ciclo; nada é enfileirado.
    pub guarda: Arc<tokio::sync::Mutex<()>>,
    /// O dual channel travou: fica em joint stereo até este instante.
    dual_bloqueado_ate: Mutex<Option<Instant>>,
}

struct Endpoint {
    r: Arc<Receptor>,
}

#[interface(name = "org.bluez.MediaEndpoint1")]
impl Endpoint {
    fn set_configuration(
        &self,
        transport: OwnedObjectPath,
        properties: HashMap<String, OwnedValue>,
    ) -> fdo::Result<()> {
        self.r.configurado(transport.as_str(), &properties)
    }

    fn select_configuration(&self, capabilities: Vec<u8>) -> fdo::Result<Vec<u8>> {
        let cfg = sbc::escolher(&capabilities, self.r.dual_permitido())
            .ok_or_else(|| fdo::Error::InvalidArgs("capacidades SBC sem opção em comum".into()))?;
        if let Some(d) = sbc::descrever(&cfg) {
            self.r.c.log(format!("PC escolheu: SBC {}, bitpool {}, {} kbps", d.modo.nome(), d.bitpool, d.kbps));
        }
        Ok(cfg.to_vec())
    }

    async fn clear_configuration(&self, transport: OwnedObjectPath) {
        self.r.encerrar_fluxo(transport.as_str()).await;
    }

    async fn release(&self) {
        let caminhos: Vec<String> = self.r.fluxos.lock().unwrap().keys().cloned().collect();
        for t in caminhos {
            self.r.encerrar_fluxo(&t).await;
        }
    }
}

impl Receptor {
    pub async fn registrar(
        conn: &Connection,
        c: &Compartilhado,
        adaptador: &str,
        tocar_auto: bool,
    ) -> zbus::Result<Arc<Self>> {
        let r = Arc::new(Receptor {
            conn: conn.clone(),
            c: c.clone(),
            tocar_auto: AtomicBool::new(tocar_auto),
            tocar: AtomicBool::new(tocar_auto),
            fluxos: Mutex::new(HashMap::new()),
            tentativas: Mutex::new(HashMap::new()),
            travadas: Mutex::new(HashMap::new()),
            guarda: Arc::default(),
            dual_bloqueado_ate: Mutex::new(None),
        });
        conn.object_server().at(CAMINHO, Endpoint { r: r.clone() }).await?;
        let props: HashMap<&str, Value> = HashMap::from([
            ("UUID", Value::from(UUID_A2DP_SINK)),
            ("Codec", Value::U8(sbc::CODEC_SBC)),
            ("Capabilities", Value::from(sbc::CAPACIDADES.to_vec())),
        ]);
        let caminho = ObjectPath::try_from(CAMINHO)?;
        if let Err(e) = bluez::chamar(conn, adaptador, "org.bluez.Media1", "RegisterEndpoint", &(&caminho, props)).await
        {
            let _ = conn.object_server().remove::<Endpoint, _>(CAMINHO).await;
            return Err(e);
        }
        // Vitrola que já estava conectada: o PC abre o áudio (e escolhe dual channel)
        let r2 = r.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(800)).await;
            r2.configurar_conectados().await;
        });
        Ok(r)
    }

    /// Para o áudio e devolve os transportes, sem tirar o endpoint do ar.
    pub async fn parar_tudo(&self) {
        let caminhos: Vec<String> = self.fluxos.lock().unwrap().keys().cloned().collect();
        for t in caminhos {
            self.parar_trabalhador(&t).await;
        }
    }

    /// Espera o BlueZ fechar os fluxos (ClearConfiguration). O BlueZ 5.72 cai
    /// (segfault) se o endpoint sair do ar com um fluxo A2DP ainda aberto.
    pub async fn esperar_sem_fluxos(&self, limite: Duration) -> bool {
        let fim = Instant::now() + limite;
        while Instant::now() < fim {
            if self.fluxos.lock().unwrap().is_empty() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.fluxos.lock().unwrap().is_empty()
    }

    pub fn tem_fluxo_de(&self, dispositivo: &str) -> bool {
        let prefixo = format!("{dispositivo}/");
        self.fluxos.lock().unwrap().keys().any(|t| t.starts_with(&prefixo))
    }

    /// Vitrola conectada sem áudio conosco: o PC abre o A2DP (com limite de tentativas).
    pub async fn garantir_audio(self: &Arc<Self>, dispositivo: &str) {
        if !self.tem_fluxo_de(dispositivo) && self.pode_tentar(dispositivo) {
            self.conectar_audio(dispositivo).await;
        }
    }

    pub async fn desregistrar(self: &Arc<Self>, adaptador: &str) {
        let caminhos: Vec<String> = self.fluxos.lock().unwrap().keys().cloned().collect();
        for t in caminhos {
            self.encerrar_fluxo(&t).await;
        }
        if let Ok(caminho) = ObjectPath::try_from(CAMINHO) {
            let _ = bluez::chamar(&self.conn, adaptador, "org.bluez.Media1", "UnregisterEndpoint", &(&caminho,)).await;
        }
        let _ = self.conn.object_server().remove::<Endpoint, _>(CAMINHO).await;
    }

    // ---------- configuração (chamado pelo BlueZ) ----------

    fn configurado(self: &Arc<Self>, transporte: &str, props: &HashMap<String, OwnedValue>) -> fdo::Result<()> {
        let bytes = props.get("Configuration").and_then(|v| bluez::bytes_de(v)).unwrap_or_default();
        let cfg = sbc::descrever(&bytes).ok_or_else(|| fdo::Error::InvalidArgs("configuração SBC inválida".into()))?;
        let dispositivo = props
            .get("Device")
            .and_then(|v| if let Value::ObjectPath(p) = &**v { Some(p.as_str().to_string()) } else { None })
            .unwrap_or_else(|| transporte.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default());
        let estado_ini = props
            .get("State")
            .and_then(|v| if let Value::Str(s) = &**v { Some(s.as_str().to_string()) } else { None })
            .unwrap_or_default();
        self.c.log(format!("Vitrola configurou: SBC {}, bitpool {}, {} kbps", cfg.modo.nome(), cfg.bitpool, cfg.kbps));
        {
            let mut e = self.c.trava();
            e.fluxo = Some(cfg.clone());
        }
        let r = self.clone();
        let t = transporte.to_string();
        let vigia = tokio::spawn(async move { r.vigiar(t, estado_ini).await });
        self.fluxos.lock().unwrap().insert(
            transporte.to_string(),
            Fluxo {
                cfg: cfg.clone(),
                mac: bluez::mac_do_caminho(&dispositivo),
                trabalhador: None,
                vigia: Some(vigia),
                iniciando: false,
            },
        );
        if cfg.modo != Modo::Dual && self.dual_permitido() {
            // Toca já no modo da vitrola; o dual é um upgrade só com o link estável
            let (r, t) = (self.clone(), transporte.to_string());
            tokio::spawn(async move { r.upgrade_quando_estavel(t, dispositivo).await });
        }
        self.c.repintar.pedir();
        Ok(())
    }

    /// Acompanha o estado do transporte: "pending" = a vitrola quer mandar áudio.
    async fn vigiar(self: Arc<Self>, transporte: String, estado_ini: String) {
        let proxy = match fdo::PropertiesProxy::builder(&self.conn)
            .destination(BLUEZ)
            .and_then(|b| b.path(transporte.as_str()))
        {
            Ok(b) => match b.build().await {
                Ok(p) => p,
                Err(_) => return,
            },
            Err(_) => return,
        };
        let Ok(mut sinais) = proxy.receive_properties_changed().await else { return };
        let atual = match proxy
            .get(zbus::names::InterfaceName::from_static_str_unchecked("org.bluez.MediaTransport1"), "State")
            .await
        {
            Ok(v) => match &*v {
                Value::Str(s) => s.as_str().to_string(),
                _ => estado_ini,
            },
            Err(_) => estado_ini,
        };
        if atual == "pending" || atual == "active" {
            self.iniciar_fluxo(&transporte).await;
        } else {
            // Depois de uma config feita pelo PC (dual channel) o transmissor pode
            // ficar parado esperando: o Acquire faz o BlueZ pedir o START a ele.
            let r = self.clone();
            let t = transporte.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(700)).await;
                r.iniciar_fluxo(&t).await;
            });
        }
        while let Some(sinal) = sinais.next().await {
            let Ok(args) = sinal.args() else { continue };
            if args.interface_name().as_str() != "org.bluez.MediaTransport1" {
                continue;
            }
            if let Some(Value::Str(s)) = args.changed_properties().get("State") {
                match s.as_str() {
                    "pending" => self.iniciar_fluxo(&transporte).await,
                    "idle" => self.parar_trabalhador(&transporte).await,
                    _ => {}
                }
            }
        }
    }

    async fn iniciar_fluxo(self: &Arc<Self>, transporte: &str) {
        let (cfg, mac) = {
            let mut fluxos = self.fluxos.lock().unwrap();
            match fluxos.get_mut(transporte) {
                Some(f) if f.trabalhador.is_none() && !f.iniciando => {
                    f.iniciando = true;
                    (f.cfg.clone(), f.mac.clone())
                }
                _ => return,
            }
        };
        let iface = "org.bluez.MediaTransport1";
        // TryAcquire: a vitrola já quer mandar áudio. Acquire: pede o START a ela.
        let resposta = match bluez::chamar(&self.conn, transporte, iface, "TryAcquire", &()).await {
            Ok(m) => Ok(m),
            Err(_) => bluez::chamar(&self.conn, transporte, iface, "Acquire", &()).await,
        };
        if let Some(f) = self.fluxos.lock().unwrap().get_mut(transporte) {
            f.iniciando = false;
        } else {
            return; // o fluxo foi encerrado enquanto esperávamos
        }
        let msg = match resposta {
            Ok(m) => m,
            Err(e) => return self.c.log(format!("Não consegui pegar o áudio: {}", bluez::nome_erro(&e))),
        };
        let (fd, mtu, _mtu_escrita): (OwnedFd, u16, u16) = match msg.body().deserialize() {
            Ok(v) => v,
            Err(e) => return self.c.log(format!("Resposta inesperada do Acquire: {e}")),
        };
        // Cada fluxo novo (inclusive reconexão) segue a preferência "tocar ao conectar"
        let auto = self.tocar_auto.load(Ordering::SeqCst);
        self.tocar.store(auto, Ordering::SeqCst);
        // Cão de guarda: fluxo aberto mas sem pacotes -> reanimar (em paralelo)
        let (fraco, t, rt) = (Arc::downgrade(self), transporte.to_string(), tokio::runtime::Handle::current());
        let sem_dados = Box::new(move || {
            if let Some(r) = fraco.upgrade() {
                let t = t.clone();
                rt.spawn(Receptor::reanimar_em_caixa(r, t));
            }
        });
        let trabalhador = audio::iniciar(fd.into(), mtu as usize, cfg.clone(), mac, auto, self.c.clone(), sem_dados);
        if let Some(f) = self.fluxos.lock().unwrap().get_mut(transporte) {
            f.trabalhador = Some(trabalhador);
        }
        self.c.log(format!(
            "Áudio chegando: SBC {}, bitpool {}, {} kbps, {} Hz",
            cfg.modo.nome(),
            cfg.bitpool,
            cfg.kbps,
            cfg.rate
        ));
    }

    /// Futuro encaixotado: quebra o ciclo reanimar -> iniciar_fluxo -> cão de guarda.
    fn reanimar_em_caixa(r: Arc<Self>, t: String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move { r.reanimar(&t).await })
    }

    /// Fluxo aberto mas a vitrola parou de mandar pacotes (reconexão atropelada):
    /// devolve o transporte e pede o START de novo; se travar 3 vezes em 1 min,
    /// derruba e reabre o perfil A2DP (de novo em dual channel).
    async fn reanimar(self: &Arc<Self>, transporte: &str) {
        let _guarda = self.guarda.lock().await; // espera: reanimar não pode ser pulado
        let Some(em_dual) = self.fluxos.lock().unwrap().get(transporte).map(|f| f.cfg.modo == Modo::Dual) else {
            return; // o fluxo já foi substituído enquanto esperávamos
        };
        let dispositivo = transporte.split("/sep").next().unwrap_or(transporte);
        let dispositivo = dispositivo.rsplit_once("/fd").map(|(d, _)| d).unwrap_or(dispositivo).to_string();
        if em_dual {
            // O dual travou: volta já para joint stereo, sem insistir
            *self.dual_bloqueado_ate.lock().unwrap() = Some(Instant::now() + BLOQUEIO_DUAL);
            self.c.log(format!(
                "O dual channel travou: voltando para joint stereo (sem dual por {} min)",
                BLOQUEIO_DUAL.as_secs() / 60
            ));
            return self.reabrir_a2dp(&dispositivo).await;
        }
        let agora = Instant::now();
        let n = {
            let mut t = self.travadas.lock().unwrap();
            let v = t.entry(dispositivo.clone()).or_default();
            v.retain(|i| agora.duration_since(*i) < Duration::from_secs(60));
            v.push(agora);
            v.len()
        };
        if n < 3 {
            self.c.log("A vitrola parou de mandar áudio: pedindo o fluxo de novo");
            let tocar = self.tocar.load(Ordering::SeqCst);
            self.parar_trabalhador(transporte).await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            self.tocar.store(tocar, Ordering::SeqCst);
            self.iniciar_fluxo(transporte).await;
            return;
        }
        self.c.log("O fluxo travou de novo: reabrindo a conexão de áudio");
        self.travadas.lock().unwrap().remove(&dispositivo);
        self.reabrir_a2dp(&dispositivo).await;
    }

    /// Derruba e reabre o perfil A2DP (o PC escolhe a config). Exige o guardião.
    async fn reabrir_a2dp(self: &Arc<Self>, dispositivo: &str) {
        let _ = bluez::chamar(&self.conn, dispositivo, "org.bluez.Device1", "DisconnectProfile", &(UUID_A2DP_SOURCE,))
            .await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        self.tentativas.lock().unwrap().remove(dispositivo);
        self.abrir_a2dp(dispositivo).await;
    }

    fn dual_permitido(&self) -> bool {
        self.dual_bloqueado_ate.lock().unwrap().is_none_or(|t| Instant::now() >= t)
    }

    /// Upgrade para dual channel só depois de ESTAVEL sem travar no modo atual.
    /// Sempre busca a melhor qualidade: enquanto este fluxo em joint stereo existir,
    /// tenta o dual channel com o link estável (a primeira vez após ESTAVEL, depois
    /// a cada INTERVALO_UPGRADE). Um fluxo novo (o dual) encerra este laço.
    async fn upgrade_quando_estavel(self: Arc<Self>, transporte: String, dispositivo: String) {
        let mut espera = ESTAVEL;
        loop {
            tokio::time::sleep(espera).await;
            espera = INTERVALO_UPGRADE;
            let Some(tocando_aqui) = self.fluxos.lock().unwrap().get(&transporte).map(|f| f.trabalhador.is_some())
            else {
                return; // fluxo substituído (upgrade feito ou reconexão)
            };
            let travou = self
                .travadas
                .lock()
                .unwrap()
                .get(&dispositivo)
                .is_some_and(|v| v.last().is_some_and(|t| t.elapsed() < ESTAVEL));
            if !tocando_aqui || travou || !self.dual_permitido() || !self.pode_tentar(&dispositivo) {
                continue;
            }
            self.c.log("Link estável: tentando o upgrade para dual channel");
            self.forcar_dual(&dispositivo).await;
        }
    }

    /// Botão HI-FI: renegocia o dual channel agora, ignorando bloqueio e limites.
    pub async fn hifi(self: &Arc<Self>) {
        *self.dual_bloqueado_ate.lock().unwrap() = None;
        let Ok(objs) = bluez::objetos(&self.conn).await else { return };
        let Some(d) = bluez::fontes_conectadas(&objs).into_iter().next() else {
            return self.c.log("HI-FI: nenhuma vitrola conectada");
        };
        let em_dual = self.fluxos.lock().unwrap().values().any(|f| f.cfg.modo == Modo::Dual && f.trabalhador.is_some());
        if em_dual {
            return self.c.log("HI-FI: já em dual channel (~396 kbps)");
        }
        self.tentativas.lock().unwrap().remove(&d.caminho);
        self.travadas.lock().unwrap().remove(&d.caminho);
        self.c.log("HI-FI: renegociando dual channel");
        if self.tem_fluxo_de(&d.caminho) {
            self.forcar_dual(&d.caminho).await;
        } else {
            self.conectar_audio(&d.caminho).await;
        }
    }

    async fn parar_trabalhador(&self, transporte: &str) {
        let t = self.fluxos.lock().unwrap().get_mut(transporte).and_then(|f| f.trabalhador.take());
        if let Some(t) = t {
            let _ = tokio::task::spawn_blocking(move || t.parar_e_esperar()).await;
            let _ = bluez::chamar(&self.conn, transporte, "org.bluez.MediaTransport1", "Release", &()).await;
        }
    }

    async fn encerrar_fluxo(&self, transporte: &str) {
        let fluxo = self.fluxos.lock().unwrap().remove(transporte);
        let Some(mut f) = fluxo else { return };
        if let Some(v) = f.vigia.take() {
            v.abort();
        }
        if let Some(t) = f.trabalhador.take() {
            let _ = tokio::task::spawn_blocking(move || t.parar_e_esperar()).await;
        }
        let vazio = self.fluxos.lock().unwrap().is_empty();
        if vazio {
            let mut e = self.c.trava();
            e.fluxo = None;
            e.tocando = false;
            e.inicio_play = None;
        }
        self.c.repintar.pedir();
    }

    // ---------- transporte: PLAY / STOP / REC ----------

    fn enviar(&self, c: impl Fn() -> Controle) {
        for f in self.fluxos.lock().unwrap().values() {
            if let Some(t) = &f.trabalhador {
                t.enviar(c());
            }
        }
    }

    /// PLAY com a vitrola conectada mas parada: pede o START nos transportes.
    pub async fn acordar(self: &Arc<Self>) {
        let parados: Vec<String> = self
            .fluxos
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, f)| f.trabalhador.is_none())
            .map(|(t, _)| t.clone())
            .collect();
        for t in parados {
            self.iniciar_fluxo(&t).await;
        }
    }

    pub fn definir_tocar(&self, tocar: bool) {
        self.tocar.store(tocar, Ordering::SeqCst);
        self.enviar(|| Controle::Tocar(tocar));
    }

    pub fn definir_tocar_auto(&self, auto: bool) {
        self.tocar_auto.store(auto, Ordering::SeqCst);
    }

    pub fn tem_audio(&self) -> bool {
        self.fluxos.lock().unwrap().values().any(|f| f.trabalhador.is_some())
    }

    pub fn alternar_gravacao(&self) {
        if self.c.trava().gravando.is_some() {
            return self.enviar(|| Controle::Gravar(None));
        }
        if !self.tem_audio() {
            return self.c.log("REC: não há áudio chegando da vitrola");
        }
        let pasta = pasta_gravacoes();
        if let Err(e) = std::fs::create_dir_all(&pasta) {
            return self.c.log(format!("Não consegui criar {}: {e}", pasta.display()));
        }
        let caminho = pasta.join(format!("disco-{}.flac", carimbo_arquivo()));
        self.enviar(|| Controle::Gravar(Some(caminho.clone())));
    }

    pub fn parar_gravacao(&self) {
        self.enviar(|| Controle::Gravar(None));
    }

    // ---------- forçar dual channel ----------

    fn pode_tentar(&self, dispositivo: &str) -> bool {
        let agora = Instant::now();
        let mut t = self.tentativas.lock().unwrap();
        let recentes: Vec<Instant> = t
            .get(dispositivo)
            .map(|v| v.iter().copied().filter(|i| agora.duration_since(*i) < JANELA).collect())
            .unwrap_or_default();
        if recentes.len() >= TENTATIVAS {
            // avisa uma vez por janela (o backend chama isto a cada segundo)
            let avisar = recentes.len() == TENTATIVAS;
            let mut cheio = recentes;
            if avisar {
                cheio.push(agora);
            }
            t.insert(dispositivo.to_string(), cheio);
            drop(t);
            if avisar {
                self.c.log("Muitas tentativas seguidas: esperando a vitrola se acalmar antes de tentar de novo");
            }
            return false;
        }
        let mut novas = recentes;
        novas.push(agora);
        t.insert(dispositivo.to_string(), novas);
        true
    }

    /// Vitrolas conectadas sem áudio conosco: o PC abre o A2DP.
    pub async fn configurar_conectados(self: &Arc<Self>) {
        let Ok(objs) = bluez::objetos(&self.conn).await else { return };
        for d in bluez::fontes_conectadas(&objs) {
            self.garantir_audio(&d.caminho).await;
        }
    }

    /// Há uma operação de conexão em andamento?
    pub fn ocupado(&self) -> bool {
        self.guarda.try_lock().is_err()
    }

    async fn conectar_audio(self: &Arc<Self>, dispositivo: &str) {
        let Ok(_guarda) = self.guarda.try_lock() else { return };
        self.abrir_a2dp(dispositivo).await;
    }

    /// O PC abre o A2DP (o BlueZ chama nosso SelectConfiguration). Exige o guardião.
    async fn abrir_a2dp(self: &Arc<Self>, dispositivo: &str) {
        // Conectada pelo applet do sistema, o BlueZ pode abrir o A2DP ao contrário
        // (PC -> vitrola, tratando o lado receptor do chip como caixa de som); isso
        // ocupa a conexão de áudio e o sentido certo dá "busy". Desfaz primeiro.
        if let Ok(objs) = bluez::objetos(&self.conn).await {
            if !bluez::transportes_no_sentido_errado(&objs, dispositivo).is_empty() {
                // Desfazer só o perfil deixa o AVDTP num estado em que a vitrola recusa
                // a conexão nova; desconectando tudo, ela volta limpa
                self.c.log("O sistema conectou o áudio ao contrário (PC → vitrola): reconectando no sentido certo");
                let _ = bluez::chamar(&self.conn, dispositivo, "org.bluez.Device1", "Disconnect", &()).await;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        let r =
            bluez::chamar(&self.conn, dispositivo, "org.bluez.Device1", "ConnectProfile", &(UUID_A2DP_SOURCE,)).await;
        if let Err(e) = r {
            let nome = bluez::nome_erro(&e);
            if nome.contains("AlreadyConnected") || nome.contains("InProgress") {
                if self.dual_permitido() {
                    self.reconfigurar_dual(dispositivo).await;
                }
            } else {
                self.c.log(format!("Não consegui abrir o áudio: {nome}"));
            }
        }
    }

    async fn forcar_dual(self: &Arc<Self>, dispositivo: &str) {
        // espera a vez (ex.: a reconexão do PC ainda segurando o guardião)
        let Ok(_guarda) = tokio::time::timeout(Duration::from_secs(10), self.guarda.lock()).await else { return };
        if !self.dual_permitido() {
            return;
        }
        self.reconfigurar_dual(dispositivo).await;
    }

    /// Pede ao endpoint do transmissor a config dual channel. Exige o guardião.
    async fn reconfigurar_dual(self: &Arc<Self>, dispositivo: &str) {
        let Ok(objs) = bluez::objetos(&self.conn).await else { return };
        let Some((sep, caps)) = bluez::seps_sbc(&objs, dispositivo).into_iter().next() else {
            return self.c.log("Endpoint SBC do transmissor não encontrado");
        };
        let Some(cfg) = sbc::escolher(&caps, true) else { return };
        let Some(d) = sbc::descrever(&cfg) else { return };
        if d.modo != Modo::Dual {
            return self.c.log("O transmissor não anuncia dual channel; mantendo a configuração dele");
        }
        self.c.log(format!("Pedindo reconfiguração: SBC {}, bitpool {}, {} kbps", d.modo.nome(), d.bitpool, d.kbps));
        let Ok(caminho) = ObjectPath::try_from(CAMINHO) else { return };
        let props: HashMap<&str, Value> = HashMap::from([("Capabilities", Value::from(cfg.to_vec()))]);
        match bluez::chamar(&self.conn, &sep, "org.bluez.MediaEndpoint1", "SetConfiguration", &(&caminho, props)).await
        {
            Ok(_) => self.c.log("Reconfiguração aceita pelo transmissor"),
            Err(e) => self.c.log(format!("O transmissor recusou a reconfiguração: {}", bluez::nome_erro(&e))),
        }
    }
}
