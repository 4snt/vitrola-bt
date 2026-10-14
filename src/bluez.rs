//! Utilitários para conversar com o BlueZ pelo D-Bus do sistema.

use std::collections::HashMap;

use zbus::fdo;
use zbus::names::InterfaceName;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, Message};

pub const BLUEZ: &str = "org.bluez";
pub const UUID_A2DP_SOURCE: &str = "0000110a-0000-1000-8000-00805f9b34fb";
pub const UUID_A2DP_SINK: &str = "0000110b-0000-1000-8000-00805f9b34fb";

pub type Props = HashMap<String, OwnedValue>;
pub type Objetos = fdo::ManagedObjects;

pub async fn objetos(conn: &Connection) -> zbus::Result<Objetos> {
    let om = fdo::ObjectManagerProxy::builder(conn).destination(BLUEZ)?.path("/")?.build().await?;
    Ok(om.get_managed_objects().await?)
}

fn valor<'a>(p: &'a Props, k: &str) -> Option<&'a Value<'static>> {
    p.get(k).map(|v| &**v)
}

pub fn como_bool(p: &Props, k: &str) -> bool {
    matches!(valor(p, k), Some(Value::Bool(true)))
}

pub fn como_str(p: &Props, k: &str) -> Option<String> {
    match valor(p, k)? {
        Value::Str(s) => Some(s.as_str().to_string()),
        Value::ObjectPath(o) => Some(o.as_str().to_string()),
        _ => None,
    }
}

pub fn como_u32(p: &Props, k: &str) -> Option<u32> {
    match valor(p, k)? {
        Value::U32(x) => Some(*x),
        _ => None,
    }
}

pub fn como_u8(p: &Props, k: &str) -> Option<u8> {
    match valor(p, k)? {
        Value::U8(x) => Some(*x),
        _ => None,
    }
}

pub fn bytes_de(v: &Value<'_>) -> Option<Vec<u8>> {
    match v {
        Value::Array(a) => a.iter().map(|x| if let Value::U8(b) = x { Some(*b) } else { None }).collect(),
        _ => None,
    }
}

pub fn como_bytes(p: &Props, k: &str) -> Option<Vec<u8>> {
    bytes_de(valor(p, k)?)
}

pub fn como_strs(p: &Props, k: &str) -> Vec<String> {
    match valor(p, k) {
        Some(Value::Array(a)) => {
            a.iter().filter_map(|x| if let Value::Str(s) = x { Some(s.as_str().to_string()) } else { None }).collect()
        }
        _ => vec![],
    }
}

#[derive(Clone, Debug, Default)]
pub struct Adaptador {
    pub caminho: String,
    pub ligado: bool,
    pub visivel: bool,
    pub alias: String,
    pub classe: u32,
}

impl Adaptador {
    /// Classe maior 0x04 = Audio/Video (o disfarce de caixa de som).
    pub fn disfarcado(&self) -> bool {
        (self.classe >> 8) & 0x1F == 0x04
    }
}

pub fn adaptador(objs: &Objetos) -> Option<Adaptador> {
    let mut lista: Vec<_> = objs
        .iter()
        .filter_map(|(p, ifs)| ifs.iter().find(|(n, _)| n.as_str() == "org.bluez.Adapter1").map(|(_, a)| (p, a)))
        .collect();
    lista.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    let (p, a) = lista.first()?;
    Some(Adaptador {
        caminho: p.as_str().to_string(),
        ligado: como_bool(a, "Powered"),
        visivel: como_bool(a, "Discoverable"),
        alias: como_str(a, "Alias").unwrap_or_default(),
        classe: como_u32(a, "Class").unwrap_or(0),
    })
}

#[derive(Clone, Debug)]
pub struct Dispositivo {
    pub caminho: String,
    pub nome: String,
    pub mac: String,
}

/// Aparelhos conectados que transmitem áudio (anunciam A2DP Source).
pub fn fontes_conectadas(objs: &Objetos) -> Vec<Dispositivo> {
    let mut v: Vec<_> = objs
        .iter()
        .filter_map(|(p, ifs)| {
            let d = ifs.iter().find(|(n, _)| n.as_str() == "org.bluez.Device1")?.1;
            (como_bool(d, "Connected") && como_strs(d, "UUIDs").iter().any(|u| u == UUID_A2DP_SOURCE)).then(|| {
                Dispositivo {
                    caminho: p.as_str().to_string(),
                    nome: como_str(d, "Alias").unwrap_or_else(|| "?".into()),
                    mac: como_str(d, "Address").unwrap_or_default(),
                }
            })
        })
        .collect();
    v.sort_by(|a, b| a.caminho.cmp(&b.caminho));
    v
}

/// Aparelhos pareados e desconectados que transmitem áudio (candidatos a reconectar).
pub fn conhecidos(objs: &Objetos) -> Vec<Dispositivo> {
    objs.iter()
        .filter_map(|(p, ifs)| {
            let d = ifs.iter().find(|(n, _)| n.as_str() == "org.bluez.Device1")?.1;
            (como_bool(d, "Paired")
                && !como_bool(d, "Connected")
                && como_strs(d, "UUIDs").iter().any(|u| u == UUID_A2DP_SOURCE))
            .then(|| Dispositivo {
                caminho: p.as_str().to_string(),
                nome: como_str(d, "Alias").unwrap_or_else(|| "?".into()),
                mac: como_str(d, "Address").unwrap_or_default(),
            })
        })
        .collect()
}

/// Endpoints SBC remotos (do transmissor) sob um dispositivo: (caminho, capacidades).
pub fn seps_sbc(objs: &Objetos, dispositivo: &str) -> Vec<(String, Vec<u8>)> {
    let prefixo = format!("{dispositivo}/");
    objs.iter()
        .filter(|(p, _)| p.as_str().starts_with(&prefixo))
        .filter_map(|(p, ifs)| {
            let e = ifs.iter().find(|(n, _)| n.as_str() == "org.bluez.MediaEndpoint1")?.1;
            (como_u8(e, "Codec") == Some(crate::sbc::CODEC_SBC))
                .then(|| (p.as_str().to_string(), como_bytes(e, "Capabilities").unwrap_or_default()))
        })
        .collect()
}

pub fn mac_do_caminho(caminho: &str) -> String {
    caminho.rsplit("/dev_").next().unwrap_or("").split('/').next().unwrap_or("").replace('_', ":")
}

pub async fn definir(conn: &Connection, caminho: &str, iface: &str, prop: &str, v: Value<'_>) -> zbus::Result<()> {
    let p = fdo::PropertiesProxy::builder(conn).destination(BLUEZ)?.path(caminho)?.build().await?;
    p.set(InterfaceName::try_from(iface)?, prop, &v).await?;
    Ok(())
}

pub async fn chamar<B>(conn: &Connection, caminho: &str, iface: &str, metodo: &str, corpo: &B) -> zbus::Result<Message>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    conn.call_method(Some(BLUEZ), caminho, Some(iface), metodo, corpo).await
}

/// Nome do erro D-Bus (ex.: org.bluez.Error.AlreadyConnected), se houver.
pub fn nome_erro(e: &zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(nome, desc, _) => format!("{} {}", nome.as_str(), desc.clone().unwrap_or_default()),
        outro => outro.to_string(),
    }
}
