//! Preferências do usuário em ~/.config/vitrola-bt/prefs.json.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Tocar nos alto-falantes assim que a vitrola começar a transmitir.
    pub tocar_auto: bool,
    /// Fechar a janela minimiza (e a bandeja mantém o app) em vez de sair.
    pub minimizar_bandeja: bool,
    /// Equalizador e funções de escuta.
    pub som: crate::dsp::Som,
    /// Iluminação dos VU.
    pub tema_vu: crate::desenho::TemaVu,
    /// Posição da alavanca BLUETOOTH: o app abre como foi deixado.
    pub bluetooth_ligado: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            tocar_auto: true,
            minimizar_bandeja: false,
            som: Default::default(),
            tema_vu: Default::default(),
            bluetooth_ligado: true,
        }
    }
}

fn caminho() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("vitrola-bt").join("prefs.json"))
}

impl Prefs {
    pub fn carregar() -> Self {
        caminho()
            .and_then(|c| std::fs::read_to_string(c).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn salvar(&self) {
        if let Some(c) = caminho() {
            if let Some(dir) = c.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(t) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(c, t);
            }
        }
    }
}
