//! Ícone na bandeja do sistema (StatusNotifierItem + menu com.canonical.dbusmenu),
//! implementado direto no D-Bus da sessão: funciona no COSMIC, KDE, XFCE,
//! Budgie e no GNOME com a extensão AppIndicator.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, StructureBuilder, Value};
use zbus::{interface, Connection, SignalContext};

use crate::estado::{Comando, Compartilhado, EventoUi};

const ITEM: &str = "/StatusNotifierItem";
type PropsMenu = HashMap<String, OwnedValue>;
type Layout = (u32, (i32, PropsMenu, Vec<OwnedValue>));
type Dica = (String, Vec<(i32, i32, Vec<u8>)>, String, String);
type Pixmaps = Vec<(i32, i32, Vec<u8>)>;

/// O ícone embutido (PNG 128 px) em ARGB32 big-endian, como o StatusNotifier
/// espera: a bandeja mostra o ícone mesmo se o cache de ícones do painel falhar.
fn icone_pixmap() -> Pixmaps {
    let png = include_bytes!("../data/vitrola-bt-128.png");
    let Ok(mut leitor) = png::Decoder::new(&png[..]).read_info() else { return vec![] };
    let mut buf = vec![0; leitor.output_buffer_size()];
    let Ok(info) = leitor.next_frame(&mut buf) else { return vec![] };
    if info.color_type != png::ColorType::Rgba {
        return vec![];
    }
    let argb = buf[..info.buffer_size()].chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
    vec![(info.width as i32, info.height as i32, argb)]
}
const MENU: &str = "/MenuBar";

fn dica(c: &Compartilhado) -> String {
    let e = c.trava();
    match (&e.fluxo, e.tocando, &e.aparelho) {
        (Some(f), true, _) => format!("Tocando: SBC {} · {} kbps", f.modo.nome(), f.kbps),
        (Some(_), false, _) => "Vitrola conectada (parado)".into(),
        (None, _, Some(a)) => format!("{} conectada", a.nome),
        _ => "Aguardando a vitrola".into(),
    }
}

struct Item {
    c: Compartilhado,
    tx_ui: Sender<EventoUi>,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "Hardware"
    }
    #[zbus(property)]
    fn id(&self) -> &str {
        "vitrola-bt"
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "Vitrola BT"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        "vitrola-bt"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        "/usr/share/icons"
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Pixmaps {
        icone_pixmap()
    }
    #[zbus(property)]
    fn tool_tip(&self) -> Dica {
        (String::new(), vec![], "Vitrola BT".into(), dica(&self.c))
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        ObjectPath::from_static_str_unchecked(MENU).into()
    }
    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }

    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.tx_ui.send(EventoUi::Mostrar);
        self.c.repintar.pedir();
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {
        let _ = self.tx_ui.send(EventoUi::Mostrar);
        self.c.repintar.pedir();
    }
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: String) {}

    #[zbus(signal)]
    async fn new_tool_tip(ctxt: &SignalContext<'_>) -> zbus::Result<()>;
}

struct Menu {
    c: Compartilhado,
    tx_cmd: UnboundedSender<Comando>,
    tx_ui: Sender<EventoUi>,
    revisao: Arc<AtomicU32>,
}

fn valor(v: Value<'_>) -> OwnedValue {
    OwnedValue::try_from(v).expect("valor sem fd")
}

fn item(id: i32, rotulo: Option<&str>) -> OwnedValue {
    let mut props: HashMap<String, Value> = HashMap::new();
    match rotulo {
        None => {
            props.insert("type".into(), Value::from("separator"));
        }
        Some(r) => {
            props.insert("label".into(), Value::from(r.to_string()));
            props.insert("enabled".into(), Value::Bool(true));
            props.insert("visible".into(), Value::Bool(true));
        }
    }
    let filhos: Vec<Value> = vec![];
    let s = StructureBuilder::new().add_field(id).add_field(props).add_field(filhos).build();
    valor(Value::Structure(s))
}

impl Menu {
    fn itens(&self) -> Vec<(i32, Option<&'static str>)> {
        let (tem_audio, tocando) = {
            let e = self.c.trava();
            (e.fluxo.is_some(), e.tocando)
        };
        let mut v = vec![(1, Some("Abrir Vitrola BT")), (5, Some("Ajustes"))];
        if tem_audio {
            v.push((2, Some(if tocando { "Parar" } else { "Tocar" })));
        }
        v.push((3, None));
        v.push((4, Some("Desligar")));
        v
    }

    fn propriedades(rotulo: Option<&str>) -> HashMap<String, OwnedValue> {
        let mut p = HashMap::new();
        match rotulo {
            None => {
                p.insert("type".into(), valor(Value::from("separator")));
            }
            Some(r) => {
                p.insert("label".into(), valor(Value::from(r.to_string())));
                p.insert("enabled".into(), valor(Value::Bool(true)));
                p.insert("visible".into(), valor(Value::Bool(true)));
            }
        }
        p
    }
}

#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn get_layout(&self, _parent_id: i32, _recursion_depth: i32, _property_names: Vec<String>) -> Layout {
        let filhos = self.itens().into_iter().map(|(id, r)| item(id, r)).collect();
        let raiz = HashMap::from([("children-display".to_string(), valor(Value::from("submenu")))]);
        (self.revisao.load(Ordering::SeqCst), (0, raiz, filhos))
    }

    fn get_group_properties(&self, ids: Vec<i32>, _property_names: Vec<String>) -> Vec<(i32, PropsMenu)> {
        let itens = self.itens();
        ids.into_iter()
            .map(|id| {
                let r = itens.iter().find(|(i, _)| *i == id).and_then(|(_, r)| *r);
                (id, Menu::propriedades(r))
            })
            .collect()
    }

    fn get_property(&self, id: i32, name: String) -> OwnedValue {
        let itens = self.itens();
        let r = itens.iter().find(|(i, _)| *i == id).and_then(|(_, r)| *r);
        Menu::propriedades(r).remove(&name).unwrap_or_else(|| valor(Value::from("")))
    }

    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        match id {
            1 => {
                let _ = self.tx_ui.send(EventoUi::Mostrar);
            }
            2 => {
                let tocando = self.c.trava().tocando;
                let _ = self.tx_cmd.send(if tocando { Comando::Stop } else { Comando::Play });
            }
            4 => {
                let _ = self.tx_ui.send(EventoUi::Sair);
            }
            5 => {
                let _ = self.tx_ui.send(EventoUi::Ajustes);
            }
            _ => {}
        }
        self.c.repintar.pedir();
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        for (id, ev, data, ts) in events {
            self.event(id, ev, data, ts);
        }
        vec![]
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (vec![], vec![])
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        vec![]
    }

    #[zbus(signal)]
    async fn layout_updated(ctxt: &SignalContext<'_>, revision: u32, parent: i32) -> zbus::Result<()>;
}

/// Publica o ícone. Falha (Err) se o desktop não tiver bandeja.
pub async fn iniciar(
    conn: &Connection,
    c: Compartilhado,
    tx_cmd: UnboundedSender<Comando>,
    tx_ui: Sender<EventoUi>,
) -> zbus::Result<()> {
    let watcher_existe =
        zbus::fdo::DBusProxy::new(conn).await?.name_has_owner("org.kde.StatusNotifierWatcher".try_into()?).await?;
    if !watcher_existe {
        return Err(zbus::Error::Failure("sem StatusNotifierWatcher".into()));
    }
    let revisao = Arc::new(AtomicU32::new(1));
    conn.object_server().at(ITEM, Item { c: c.clone(), tx_ui: tx_ui.clone() }).await?;
    conn.object_server().at(MENU, Menu { c: c.clone(), tx_cmd, tx_ui, revisao: revisao.clone() }).await?;
    let nome = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    conn.request_name(nome.as_str()).await?;
    conn.call_method(
        Some("org.kde.StatusNotifierWatcher"),
        "/StatusNotifierWatcher",
        Some("org.kde.StatusNotifierWatcher"),
        "RegisterStatusNotifierItem",
        &(nome.as_str(),),
    )
    .await?;

    // Atualiza menu e dica quando o estado muda (tocando / áudio chegando)
    let conn = conn.clone();
    tokio::spawn(async move {
        let mut anterior = None;
        loop {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let agora = {
                let e = c.trava();
                (e.fluxo.is_some(), e.tocando, e.aparelho.is_some())
            };
            if Some(agora) == anterior {
                continue;
            }
            anterior = Some(agora);
            let rev = revisao.fetch_add(1, Ordering::SeqCst) + 1;
            if let Ok(m) = conn.object_server().interface::<_, Menu>(MENU).await {
                let _ = Menu::layout_updated(m.signal_context(), rev, 0).await;
            }
            if let Ok(i) = conn.object_server().interface::<_, Item>(ITEM).await {
                let _ = Item::new_tool_tip(i.signal_context()).await;
            }
        }
    });
    Ok(())
}
