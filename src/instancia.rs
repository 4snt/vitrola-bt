//! Instância única: o app registra um nome no D-Bus da sessão. Se já existir
//! um Vitrola BT aberto, a segunda cópia só pede para ele se mostrar e sai —
//! nunca dois receptores brigando pela vitrola.

use std::sync::mpsc::Sender;

use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::{interface, Connection};

use crate::estado::{EventoUi, Repintador, APP_ID};

const CAMINHO: &str = "/io/github/_4snt/VitrolaBT";

struct Unica {
    tx_ui: Sender<EventoUi>,
    rep: Repintador,
}

#[interface(name = "io.github._4snt.VitrolaBT")]
impl Unica {
    fn mostrar(&self) {
        let _ = self.tx_ui.send(EventoUi::Mostrar);
        self.rep.pedir();
    }
}

pub enum Instancia {
    Primeira(Connection),
    JaAberta,
    SemSessao,
}

pub async fn garantir(tx_ui: Sender<EventoUi>, rep: Repintador) -> Instancia {
    let Ok(conn) = Connection::session().await else {
        return Instancia::SemSessao;
    };
    if conn.object_server().at(CAMINHO, Unica { tx_ui, rep }).await.is_err() {
        return Instancia::SemSessao;
    }
    match conn.request_name_with_flags(APP_ID, RequestNameFlags::DoNotQueue.into()).await {
        Ok(RequestNameReply::PrimaryOwner) | Ok(RequestNameReply::AlreadyOwner) => Instancia::Primeira(conn),
        _ => {
            let _ = conn.call_method(Some(APP_ID), CAMINHO, Some(APP_ID), "Mostrar", &()).await;
            Instancia::JaAberta
        }
    }
}
