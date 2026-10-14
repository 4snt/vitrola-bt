//! SBC no A2DP: configuração (4 bytes), escolha da config de maior bitrate e
//! decodificador via libsbc (a biblioteca do BlueZ), carregada em tempo de
//! execução para não exigir cabeçalhos de desenvolvimento na compilação.
//!
//! Transmissores baratos (ex.: JieLi das vitrolas) anunciam bitpool máximo baixo
//! (30) e, quando escolhem sozinhos, usam joint stereo: ~219 kbps. No modo
//! *dual channel* o bitpool vale POR CANAL, então o mesmo bitpool 30 rende
//! ~396 kbps (o truque do SBC-XQ), dentro da especificação e do que o chip anuncia.

use std::ffi::{c_int, c_ulong, c_void};

pub const CODEC_SBC: u8 = 0x00;
pub const BITPOOL_MAX_DUAL: u8 = 64;

// Byte 0: frequência (16k 0x80, 32k 0x40, 44.1k 0x20, 48k 0x10) | modo
// (mono 0x08, dual 0x04, stereo 0x02, joint 0x01). Byte 1: blocos (4 0x80,
// 8 0x40, 12 0x20, 16 0x10) | sub-bandas (4 0x08, 8 0x04) | alocação
// (SNR 0x02, loudness 0x01). Bytes 2-3: bitpool mínimo e máximo.
/// O que o nosso receptor anuncia: qualquer frequência, SÓ dual channel.
pub const CAPACIDADES: [u8; 4] = [0xF0 | 0x04, 0xF0 | 0x04 | 0x03, 2, BITPOOL_MAX_DUAL];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modo {
    Mono,
    Dual,
    Stereo,
    Joint,
}

impl Modo {
    pub fn nome(self) -> &'static str {
        match self {
            Modo::Mono => "mono",
            Modo::Dual => "dual channel",
            Modo::Stereo => "stereo",
            Modo::Joint => "joint stereo",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub rate: u32,
    pub modo: Modo,
    pub blocos: u32,
    pub subbandas: u32,
    pub loudness: bool,
    pub bitpool: u8,
    pub kbps: u32,
}

fn bit(mascara: u8, ordem: &[u8]) -> Option<u8> {
    ordem.iter().copied().find(|b| mascara & b != 0)
}

/// Config SBC (4 bytes) -> parâmetros legíveis com bitrate estimado.
pub fn descrever(cfg: &[u8]) -> Option<Config> {
    if cfg.len() < 4 {
        return None;
    }
    let rate = match bit(cfg[0] & 0xF0, &[0x10, 0x20, 0x40, 0x80])? {
        0x10 => 48_000,
        0x20 => 44_100,
        0x40 => 32_000,
        _ => 16_000,
    };
    let modo = match bit(cfg[0] & 0x0F, &[0x04, 0x01, 0x02, 0x08])? {
        0x04 => Modo::Dual,
        0x01 => Modo::Joint,
        0x02 => Modo::Stereo,
        _ => Modo::Mono,
    };
    let blocos = match bit(cfg[1] & 0xF0, &[0x10, 0x20, 0x40, 0x80])? {
        0x10 => 16,
        0x20 => 12,
        0x40 => 8,
        _ => 4,
    };
    let subbandas = if cfg[1] & 0x04 != 0 { 8 } else { 4 };
    let loudness = cfg[1] & 0x01 != 0;
    let bitpool = cfg[3];
    let nch = if modo == Modo::Mono { 1 } else { 2 };
    let bits_dados = match modo {
        Modo::Mono | Modo::Dual => blocos * nch * bitpool as u32,
        Modo::Joint => subbandas + blocos * bitpool as u32,
        Modo::Stereo => blocos * bitpool as u32,
    };
    let quadro = 4 + (4 * subbandas * nch) / 8 + bits_dados.div_ceil(8);
    let kbps = (8 * quadro as u64 * rate as u64 / (subbandas * blocos) as u64 / 1000) as u32;
    Some(Config { rate, modo, blocos, subbandas, loudness, bitpool, kbps })
}

/// Quando o PC escolhe: a maior qualidade dentro do que o transmissor anuncia,
/// preferindo dual channel (bitpool por canal = o dobro de bitrate). Com
/// `dual = false` (link instável) prefere joint stereo, o modo nativo dos chips.
pub fn escolher(caps: &[u8], dual: bool) -> Option<[u8; 4]> {
    if caps.len() < 4 {
        return None;
    }
    let freq = bit(caps[0] & 0xF0, &[0x10, 0x20, 0x40, 0x80])?;
    let ordem: &[u8] = if dual { &[0x04, 0x02, 0x01, 0x08] } else { &[0x01, 0x02, 0x04, 0x08] };
    let modo = bit(caps[0] & 0x0F, ordem)?;
    let blocos = bit(caps[1] & 0xF0, &[0x10, 0x20, 0x40, 0x80])?;
    let sub = bit(caps[1] & 0x0C, &[0x04, 0x08])?;
    let aloc = bit(caps[1] & 0x03, &[0x01, 0x02])?;
    let max = caps[3].min(BITPOOL_MAX_DUAL);
    let min = caps[2].max(2).min(max);
    Some([freq | modo, blocos | sub | aloc, min, max])
}

/// Tira o cabeçalho RTP (12 bytes + CSRCs) e o byte de cabeçalho do payload SBC.
pub fn payload_rtp(pacote: &[u8]) -> Option<&[u8]> {
    let cc = (*pacote.first()? & 0x0F) as usize;
    let inicio = 12 + 4 * cc + 1;
    (pacote.len() > inicio).then(|| &pacote[inicio..])
}

#[repr(C)]
struct SbcT {
    flags: c_ulong,
    frequency: u8,
    blocks: u8,
    subbands: u8,
    mode: u8,
    allocation: u8,
    bitpool: u8,
    endian: u8,
    priv_: *mut c_void,
    priv_alloc_base: *mut c_void,
}

type FnInit = unsafe extern "C" fn(*mut SbcT, c_ulong) -> c_int;
type FnDecode = unsafe extern "C" fn(*mut SbcT, *const c_void, usize, *mut c_void, usize, *mut usize) -> isize;
type FnEncode = unsafe extern "C" fn(*mut SbcT, *const c_void, usize, *mut c_void, usize, *mut isize) -> isize;
type FnFinish = unsafe extern "C" fn(*mut SbcT);

/// libsbc carregada dinamicamente (libsbc.so.1).
pub struct Libsbc {
    _lib: libloading::Library,
    init: FnInit,
    decode: FnDecode,
    #[cfg_attr(not(test), allow(dead_code))]
    encode: FnEncode,
    finish: FnFinish,
}

impl Libsbc {
    pub fn carregar() -> Result<Self, String> {
        // SAFETY: carregamos a libsbc do sistema; os símbolos têm as assinaturas
        // de <sbc/sbc.h> (estáveis desde a libsbc 1.0).
        unsafe {
            let lib = libloading::Library::new("libsbc.so.1")
                .or_else(|_| libloading::Library::new("libsbc.so"))
                .map_err(|e| format!("libsbc não encontrada ({e}); instale o pacote libsbc1/sbc"))?;
            let init: FnInit = *lib.get(b"sbc_init\0").map_err(|e| e.to_string())?;
            let decode: FnDecode = *lib.get(b"sbc_decode\0").map_err(|e| e.to_string())?;
            let encode: FnEncode = *lib.get(b"sbc_encode\0").map_err(|e| e.to_string())?;
            let finish: FnFinish = *lib.get(b"sbc_finish\0").map_err(|e| e.to_string())?;
            Ok(Libsbc { _lib: lib, init, decode, encode, finish })
        }
    }
}

/// Estado de codec da libsbc (decodifica ou codifica quadros SBC).
pub struct Sbc<'a> {
    lib: &'a Libsbc,
    st: Box<SbcT>,
    saida: Vec<u8>,
}

impl<'a> Sbc<'a> {
    pub fn novo(lib: &'a Libsbc) -> Result<Self, String> {
        let mut st = Box::new(SbcT {
            flags: 0,
            frequency: 0,
            blocks: 0,
            subbands: 0,
            mode: 0,
            allocation: 0,
            bitpool: 0,
            endian: 0,
            priv_: std::ptr::null_mut(),
            priv_alloc_base: std::ptr::null_mut(),
        });
        // SAFETY: st aponta para memória válida e fica no heap até o Drop.
        if unsafe { (lib.init)(&mut *st, 0) } < 0 {
            return Err("sbc_init falhou".into());
        }
        st.endian = 0; // SBC_LE: PCM little-endian
        Ok(Sbc { lib, st, saida: vec![0; 8192] })
    }

    /// Quadros SBC -> PCM s16le intercalado (estéreo).
    pub fn decodificar(&mut self, dados: &[u8], pcm: &mut Vec<u8>) {
        let mut pos = 0;
        while pos < dados.len() {
            let mut escrito = 0usize;
            // SAFETY: ponteiros e tamanhos vêm de slices válidos.
            let n = unsafe {
                (self.lib.decode)(
                    &mut *self.st,
                    dados[pos..].as_ptr() as *const c_void,
                    dados.len() - pos,
                    self.saida.as_mut_ptr() as *mut c_void,
                    self.saida.len(),
                    &mut escrito,
                )
            };
            if n <= 0 {
                break;
            }
            pcm.extend_from_slice(&self.saida[..escrito]);
            pos += n as usize;
        }
    }

    /// Só para testes: PCM -> SBC com os parâmetros dados (códigos da libsbc).
    #[cfg(test)]
    pub fn configurar_codificador(&mut self, freq: u8, blocos: u8, sub: u8, modo: u8, bitpool: u8) {
        self.st.frequency = freq;
        self.st.blocks = blocos;
        self.st.subbands = sub;
        self.st.mode = modo;
        self.st.allocation = 0;
        self.st.bitpool = bitpool;
    }

    #[cfg(test)]
    pub fn codificar(&mut self, pcm: &[u8], quadros: &mut Vec<u8>) -> usize {
        let mut pos = 0;
        let mut tamanho = 0;
        while pos < pcm.len() {
            let mut escrito: isize = 0;
            // SAFETY: ponteiros e tamanhos vêm de slices válidos.
            let n = unsafe {
                (self.lib.encode)(
                    &mut *self.st,
                    pcm[pos..].as_ptr() as *const c_void,
                    pcm.len() - pos,
                    self.saida.as_mut_ptr() as *mut c_void,
                    self.saida.len(),
                    &mut escrito,
                )
            };
            if n <= 0 || escrito <= 0 {
                break;
            }
            tamanho = escrito as usize;
            quadros.extend_from_slice(&self.saida[..tamanho]);
            pos += n as usize;
        }
        tamanho
    }
}

impl Drop for Sbc<'_> {
    fn drop(&mut self) {
        // SAFETY: st foi inicializado por sbc_init.
        unsafe { (self.lib.finish)(&mut *self.st) }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn descreve_config_da_vitrola() {
        let c = descrever(&[0x11, 0x15, 2, 30]).unwrap();
        assert_eq!((c.rate, c.modo, c.bitpool, c.kbps), (48_000, Modo::Joint, 30, 219));
    }

    #[test]
    fn escolhe_dual_channel_nas_caps_do_chip() {
        let cfg = escolher(&[0xFF, 0xFF, 2, 30], true).unwrap();
        let c = descrever(&cfg).unwrap();
        assert_eq!((c.rate, c.modo, c.blocos, c.subbandas, c.bitpool, c.kbps), (48_000, Modo::Dual, 16, 8, 30, 396));
    }

    #[test]
    fn link_instavel_prefere_joint_stereo() {
        let c = descrever(&escolher(&[0xFF, 0xFF, 2, 30], false).unwrap()).unwrap();
        assert_eq!((c.modo, c.kbps), (Modo::Joint, 219));
    }

    #[test]
    fn ida_e_volta_pela_libsbc() {
        let Ok(lib) = Libsbc::carregar() else { return }; // sem libsbc no ambiente: pula
        let mut enc = Sbc::novo(&lib).unwrap();
        // 48 kHz (3), 16 blocos (3), 8 sub-bandas (1), dual channel (1), bitpool 30
        enc.configurar_codificador(3, 3, 1, 1, 30);
        let pcm: Vec<u8> = (0..48_000)
            .flat_map(|i| {
                let v = (12_000.0 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 48_000.0).sin()) as i16;
                [v.to_le_bytes(), v.to_le_bytes()].concat()
            })
            .collect();
        let mut quadros = Vec::new();
        let tam = enc.codificar(&pcm, &mut quadros);
        assert_eq!(8 * tam * 48_000 / 128 / 1000, 396, "quadro dual channel bitpool 30");
        let mut dec = Sbc::novo(&lib).unwrap();
        let mut volta = Vec::new();
        dec.decodificar(&quadros, &mut volta);
        let pico = volta.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs()).max().unwrap();
        assert!((11_000..=13_000).contains(&pico), "pico {pico}");
    }
}
