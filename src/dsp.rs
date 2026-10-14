//! Processamento de escuta: equalizador gráfico de 10 bandas e as funções de
//! um receiver de vinil (RUMBLE, SCRATCH, MONO, LOUDNESS, BALANCE). Filtros
//! biquad do "Audio EQ Cookbook" (R. Bristow-Johnson). Só afeta o que se ouve:
//! a gravação continua com o sinal original.

use std::f32::consts::PI;

use serde::{Deserialize, Serialize};

use crate::medidor::CENTROS;

/// Ajustes de som (persistidos nas preferências).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Som {
    /// Ganho de cada banda do equalizador gráfico, em dB (-12..+12).
    pub ganhos: [f32; 10],
    /// Passa-altas em 35 Hz: tira o ronco do motor e do prato.
    pub rumble: bool,
    /// Passa-baixas em 8 kHz: atenua chiado e estalos de superfície.
    pub scratch: bool,
    /// Soma L+R: discos mono antigos ficam com menos ruído de sulco.
    pub mono: bool,
    /// Reforço de graves e agudos para ouvir em volume baixo.
    pub loudness: bool,
    /// -1 (só esquerda) .. 0 (centro) .. +1 (só direita).
    pub balanco: f32,
    /// Volume de escuta, 0..1 (curva de áudio aplicada no processamento).
    pub volume: f32,
}

impl Default for Som {
    fn default() -> Self {
        Som {
            ganhos: [0.0; 10],
            rumble: false,
            scratch: false,
            mono: false,
            loudness: false,
            balanco: 0.0,
            volume: 1.0,
        }
    }
}

impl Som {
    pub fn neutro(&self) -> bool {
        self.ganhos.iter().all(|g| g.abs() < 0.05)
            && !self.rumble
            && !self.scratch
            && !self.mono
            && !self.loudness
            && self.balanco.abs() < 0.01
            && self.volume >= 0.999
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    fn normalizar(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    fn pico(fs: f32, f0: f32, q: f32, db: f32) -> Self {
        let a = 10f32.powf(db / 40.0);
        let w = 2.0 * PI * f0 / fs;
        let alfa = w.sin() / (2.0 * q);
        let c = w.cos();
        Self::normalizar(1.0 + alfa * a, -2.0 * c, 1.0 - alfa * a, 1.0 + alfa / a, -2.0 * c, 1.0 - alfa / a)
    }

    fn prateleira(fs: f32, f0: f32, db: f32, alta: bool) -> Self {
        let a = 10f32.powf(db / 40.0);
        let w = 2.0 * PI * f0 / fs;
        let (s, c) = w.sin_cos();
        let alfa = s / 2.0 * std::f32::consts::SQRT_2; // inclinação S = 1
        let r = 2.0 * a.sqrt() * alfa;
        if alta {
            Self::normalizar(
                a * ((a + 1.0) + (a - 1.0) * c + r),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - r),
                (a + 1.0) - (a - 1.0) * c + r,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - r,
            )
        } else {
            Self::normalizar(
                a * ((a + 1.0) - (a - 1.0) * c + r),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - r),
                (a + 1.0) + (a - 1.0) * c + r,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - r,
            )
        }
    }

    fn passa(fs: f32, f0: f32, altas: bool) -> Self {
        let w = 2.0 * PI * f0 / fs;
        let (s, c) = w.sin_cos();
        let alfa = s / (2.0 * std::f32::consts::FRAC_1_SQRT_2); // Butterworth
        if altas {
            Self::normalizar((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alfa, -2.0 * c, 1.0 - alfa)
        } else {
            Self::normalizar((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alfa, -2.0 * c, 1.0 - alfa)
        }
    }
}

/// Estado de um filtro (forma direta II transposta) para um canal.
#[derive(Clone, Copy, Default)]
struct Estado {
    z1: f32,
    z2: f32,
}

impl Estado {
    #[inline]
    fn passar(&mut self, f: &Biquad, x: f32) -> f32 {
        let y = f.b0 * x + self.z1;
        self.z1 = f.b1 * x - f.a1 * y + self.z2;
        self.z2 = f.b2 * x - f.a2 * y;
        y
    }
}

pub struct Processador {
    som: Som,
    filtros: Vec<Biquad>,
    estados: Vec<[Estado; 2]>,
    ganho_l: f32,
    ganho_r: f32,
    ativo: bool,
    rate: u32,
}

impl Processador {
    pub fn novo(rate: u32, som: &Som) -> Self {
        let mut p = Processador {
            som: Som::default(),
            filtros: vec![],
            estados: vec![],
            ganho_l: 1.0,
            ganho_r: 1.0,
            ativo: false,
            rate,
        };
        p.definir(som);
        p
    }

    pub fn definir(&mut self, som: &Som) {
        if *som == self.som && !self.filtros.is_empty() {
            return;
        }
        let fs = self.rate as f32;
        let mut f = Vec::new();
        if som.rumble {
            f.push(Biquad::passa(fs, 35.0, true));
            f.push(Biquad::passa(fs, 35.0, true)); // 4ª ordem: corta firme abaixo
        }
        for (i, &g) in som.ganhos.iter().enumerate() {
            if g.abs() >= 0.05 && CENTROS[i] < fs / 2.0 {
                f.push(Biquad::pico(fs, CENTROS[i], 1.41, g)); // ~1 oitava por banda
            }
        }
        if som.loudness {
            f.push(Biquad::prateleira(fs, 100.0, 8.0, false));
            f.push(Biquad::prateleira(fs, 10_000.0, 4.0, true));
        }
        if som.scratch {
            f.push(Biquad::passa(fs, 8_000.0, false));
            f.push(Biquad::passa(fs, 8_000.0, false));
        }
        // Os estados são preservados quando a quantidade de filtros não muda
        // (arrastar um cursor não estala)
        self.estados.resize(f.len(), [Estado::default(); 2]);
        self.filtros = f;
        let b = som.balanco.clamp(-1.0, 1.0);
        self.ganho_l = if b > 0.0 { 1.0 - b } else { 1.0 };
        self.ganho_r = if b < 0.0 { 1.0 + b } else { 1.0 };
        // Folga para não saturar com reforço: tira o maior ganho positivo
        let reforco = som.ganhos.iter().fold(0f32, |m, g| m.max(*g)) + if som.loudness { 6.0 } else { 0.0 };
        let folga = 10f32.powf(-reforco.max(0.0) / 20.0 * 0.7);
        // volume com curva de potenciômetro logarítmico (~60 dB de excursão)
        let v = som.volume.clamp(0.0, 1.0);
        let volume = if v <= 0.0 { 0.0 } else { 10f32.powf((v - 1.0) * 3.0) };
        self.ganho_l *= folga * volume;
        self.ganho_r *= folga * volume;
        self.ativo = !som.neutro();
        self.som = som.clone();
    }

    /// PCM s16le estéreo (entrada) -> f32le estéreo (saída, sem requantizar:
    /// EQ e volume não jogam bits fora; o PipeWire trabalha em float).
    pub fn processar_f32(&mut self, pcm: &[u8], saida: &mut Vec<u8>) {
        saida.clear();
        saida.reserve(pcm.len() * 2);
        for q in pcm.chunks_exact(4) {
            let (l, r) = self.amostra(q);
            saida.extend_from_slice(&l.to_le_bytes());
            saida.extend_from_slice(&r.to_le_bytes());
        }
    }

    #[inline]
    fn amostra(&mut self, q: &[u8]) -> (f32, f32) {
        let mut l = i16::from_le_bytes([q[0], q[1]]) as f32 / 32768.0;
        let mut r = i16::from_le_bytes([q[2], q[3]]) as f32 / 32768.0;
        if !self.ativo {
            return (l, r);
        }
        if self.som.mono {
            let m = (l + r) * 0.5;
            l = m;
            r = m;
        }
        for (f, e) in self.filtros.iter().zip(self.estados.iter_mut()) {
            l = e[0].passar(f, l);
            r = e[1].passar(f, r);
        }
        ((l * self.ganho_l).clamp(-1.0, 1.0), (r * self.ganho_r).clamp(-1.0, 1.0))
    }

    /// Versão 16 bits no lugar (usada nos testes).
    #[cfg(test)]
    pub fn processar(&mut self, pcm: &mut [u8]) {
        if !self.ativo {
            return;
        }
        for q in pcm.chunks_exact_mut(4) {
            let (l, r) = self.amostra(q);
            let l = (l * 32767.0) as i16;
            let r = (r * 32767.0) as i16;
            q[..2].copy_from_slice(&l.to_le_bytes());
            q[2..].copy_from_slice(&r.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn seno(f: f32, n: usize) -> Vec<u8> {
        (0..n)
            .flat_map(|i| {
                let v = (8000.0 * (2.0 * PI * f * i as f32 / 48_000.0).sin()) as i16;
                [v.to_le_bytes(), v.to_le_bytes()].concat()
            })
            .collect()
    }

    fn pico(pcm: &[u8]) -> i32 {
        pcm.chunks_exact(4).skip(4800).map(|q| (i16::from_le_bytes([q[0], q[1]]) as i32).abs()).max().unwrap()
    }

    #[test]
    fn neutro_nao_mexe() {
        let mut p = Processador::novo(48_000, &Som::default());
        let original = seno(1000.0, 9600);
        let mut x = original.clone();
        p.processar(&mut x);
        assert_eq!(x, original);
    }

    #[test]
    fn banda_de_1khz_corta_12db() {
        let mut som = Som::default();
        som.ganhos[5] = -12.0;
        let mut p = Processador::novo(48_000, &som);
        let mut x = seno(1000.0, 24_000);
        p.processar(&mut x);
        let razao = pico(&x) as f32 / 8000.0;
        assert!((razao - 0.25).abs() < 0.03, "razão {razao}"); // -12 dB = 0.25
    }

    #[test]
    fn rumble_mata_20hz_e_preserva_1khz() {
        let som = Som { rumble: true, ..Default::default() };
        let mut p = Processador::novo(48_000, &som);
        let mut grave = seno(20.0, 48_000);
        p.processar(&mut grave);
        assert!(pico(&grave) < 3000, "20 Hz passou com {}", pico(&grave));
        let mut p = Processador::novo(48_000, &som);
        let mut meio = seno(1000.0, 24_000);
        p.processar(&mut meio);
        assert!(pico(&meio) > 7600, "1 kHz caiu para {}", pico(&meio));
    }

    #[test]
    fn balanco_todo_a_esquerda() {
        let som = Som { balanco: -1.0, ..Default::default() };
        let mut p = Processador::novo(48_000, &som);
        let mut x = seno(1000.0, 9600);
        p.processar(&mut x);
        let r_max = x.chunks_exact(4).map(|q| (i16::from_le_bytes([q[2], q[3]]) as i32).abs()).max().unwrap();
        assert_eq!(r_max, 0);
    }
}
