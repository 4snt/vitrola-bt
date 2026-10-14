//! Agente de pareamento BlueZ que aceita tudo sem perguntar. A vitrola não tem
//! tela nem teclado: ela só pareia com "caixas" que aceitam sozinhas (Just Works)
//! ou com PIN 0000. O agente também marca o aparelho como confiável.

use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};
use zbus::{interface, Connection};

use crate::bluez;
use crate::estado::Compartilhado;

pub const CAMINHO: &str = "/vitrola/agente";
const PIN: &str = "0000";

struct Agente {
    conn: Connection,
    c: Compartilhado,
}

impl Agente {
    fn confiar(&self, dispositivo: &OwnedObjectPath, motivo: &str) {
        let conn = self.conn.clone();
        let c = self.c.clone();
        let caminho = dispositivo.as_str().to_string();
        let motivo = motivo.to_string();
        tokio::spawn(async move {
            match bluez::definir(&conn, &caminho, "org.bluez.Device1", "Trusted", Value::Bool(true)).await {
                Ok(()) => c.log(format!("Pareamento aceito ({motivo}): {}", bluez::mac_do_caminho(&caminho))),
                Err(e) => c.log(format!("Não consegui marcar como confiável: {e}")),
            }
        });
    }
}

#[interface(name = "org.bluez.Agent1")]
impl Agente {
    fn release(&self) {}

    fn request_pin_code(&self, device: OwnedObjectPath) -> String {
        self.confiar(&device, "PIN 0000");
        PIN.into()
    }

    fn display_pin_code(&self, _device: OwnedObjectPath, _pincode: String) {}

    fn request_passkey(&self, device: OwnedObjectPath) -> u32 {
        self.confiar(&device, "passkey 0000");
        0
    }

    fn display_passkey(&self, _device: OwnedObjectPath, _passkey: u32, _entered: u16) {}

    fn request_confirmation(&self, device: OwnedObjectPath, _passkey: u32) {
        self.confiar(&device, "confirmação");
    }

    fn request_authorization(&self, device: OwnedObjectPath) {
        self.confiar(&device, "autorização");
    }

    fn authorize_service(&self, device: OwnedObjectPath, _uuid: String) {
        self.confiar(&device, "serviço");
    }

    fn cancel(&self) {}
}

pub async fn registrar(conn: &Connection, c: &Compartilhado) -> zbus::Result<()> {
    conn.object_server().at(CAMINHO, Agente { conn: conn.clone(), c: c.clone() }).await?;
    let caminho = ObjectPath::try_from(CAMINHO)?;
    bluez::chamar(conn, "/org/bluez", "org.bluez.AgentManager1", "RegisterAgent", &(&caminho, "NoInputNoOutput"))
        .await?;
    bluez::chamar(conn, "/org/bluez", "org.bluez.AgentManager1", "RequestDefaultAgent", &(&caminho,)).await?;
    Ok(())
}

pub async fn desregistrar(conn: &Connection) {
    if let Ok(caminho) = ObjectPath::try_from(CAMINHO) {
        let _ = bluez::chamar(conn, "/org/bluez", "org.bluez.AgentManager1", "UnregisterAgent", &(&caminho,)).await;
    }
    let _ = conn.object_server().remove::<Agente, _>(CAMINHO).await;
}
