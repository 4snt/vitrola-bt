//! Medição em tempo real do PCM decodificado: RMS/pico por canal (para os VU)
//! e espectro em 10 bandas de oitava (FFT de 4096 pontos, janela Hann).

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

pub const CENTROS: [f32; 10] = [31.0, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
pub const PISO_DB: f32 = -90.0;
const N_FFT: usize = 4096;
/// Medições por segundo (25 = a cada 40 ms).
const TAXA_MEDIDAS: u32 = 25;

#[derive(Clone, Copy, Debug)]
pub struct Medidas {
    pub rms_db: [f32; 2],
    pub pico_db: [f32; 2],
    pub bandas_db: [f32; 10],
}

impl Default for Medidas {
    fn default() -> Self {
        Medidas { rms_db: [PISO_DB; 2], pico_db: [PISO_DB; 2], bandas_db: [PISO_DB; 10] }
    }
}

pub struct Medidor {
    rate: u32,
    fft: Arc<dyn Fft<f32>>,
    janela: Vec<f32>,
    historico: Vec<f32>,
    pos: usize,
    soma_q: [f64; 2],
    pico: [i32; 2],
    contagem: u32,
    por_medida: u32,
    buf: Vec<Complex32>,
}

fn db(x: f64) -> f32 {
    if x <= 0.0 { PISO_DB } else { (10.0 * x.log10()) as f32 }.max(PISO_DB)
}

impl Medidor {
    pub fn novo(rate: u32) -> Self {
        let janela =
            (0..N_FFT).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N_FFT as f32).cos()).collect();
        Medidor {
            rate,
            fft: FftPlanner::new().plan_fft_forward(N_FFT),
            janela,
            historico: vec![0.0; N_FFT],
            pos: 0,
            soma_q: [0.0; 2],
            pico: [0; 2],
            contagem: 0,
            por_medida: rate / TAXA_MEDIDAS,
            buf: vec![Complex32::new(0.0, 0.0); N_FFT],
        }
    }

    /// Alimenta PCM s16le estéreo; devolve uma medição a cada 40 ms de áudio.
    pub fn alimentar(&mut self, pcm: &[u8]) -> Option<Medidas> {
        let mut saida = None;
        for q in pcm.chunks_exact(4) {
            let l = i16::from_le_bytes([q[0], q[1]]) as i32;
            let r = i16::from_le_bytes([q[2], q[3]]) as i32;
            self.soma_q[0] += (l * l) as f64;
            self.soma_q[1] += (r * r) as f64;
            self.pico[0] = self.pico[0].max(l.abs());
            self.pico[1] = self.pico[1].max(r.abs());
            self.historico[self.pos] = (l + r) as f32 / 65536.0;
            self.pos = (self.pos + 1) % N_FFT;
            self.contagem += 1;
            if self.contagem >= self.por_medida {
                saida = Some(self.medir());
            }
        }
        saida
    }

    fn medir(&mut self) -> Medidas {
        let fs = 32768.0f64 * 32768.0;
        let n = self.contagem.max(1) as f64;
        let rms_db = [db(self.soma_q[0] / n / fs), db(self.soma_q[1] / n / fs)];
        let pico_db = [db((self.pico[0] as f64 / 32768.0).powi(2)), db((self.pico[1] as f64 / 32768.0).powi(2))];
        self.soma_q = [0.0; 2];
        self.pico = [0; 2];
        self.contagem = 0;

        for i in 0..N_FFT {
            let v = self.historico[(self.pos + i) % N_FFT] * self.janela[i];
            self.buf[i] = Complex32::new(v, 0.0);
        }
        self.fft.process(&mut self.buf);
        // Seno de amplitude A (escala cheia = 1) dá |X| ~ A*N/4 com Hann:
        // normaliza para dBFS de seno
        let ref_ = (N_FFT as f64 / 4.0).powi(2);
        let largura = self.rate as f32 / N_FFT as f32;
        let mut bandas_db = [PISO_DB; 10];
        for (b, c) in CENTROS.iter().enumerate() {
            let lo = (c / std::f32::consts::SQRT_2 / largura).floor().max(1.0) as usize;
            let hi = ((c * std::f32::consts::SQRT_2 / largura).ceil() as usize).clamp(lo + 1, N_FFT / 2);
            let energia: f64 = self.buf[lo..hi].iter().map(|x| x.norm_sqr() as f64).sum();
            bandas_db[b] = db(energia / ref_);
        }
        Medidas { rms_db, pico_db, bandas_db }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn seno_de_1khz() {
        let mut m = Medidor::novo(48_000);
        let pcm: Vec<u8> = (0..48_000)
            .flat_map(|i| {
                let v = (16384.0 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 48_000.0).sin()) as i16;
                [v.to_le_bytes(), v.to_le_bytes()].concat()
            })
            .collect();
        let mut ultima = None;
        for bloco in pcm.chunks(1024) {
            if let Some(md) = m.alimentar(bloco) {
                ultima = Some(md);
            }
        }
        let md = ultima.unwrap();
        // seno de meia escala: RMS = -9 dBFS
        assert!((md.rms_db[0] + 9.03).abs() < 0.2, "rms {}", md.rms_db[0]);
        let maior = md.bandas_db.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(CENTROS[maior], 1000.0);
        assert!(md.bandas_db[maior] > -8.0 && md.bandas_db[maior] < -2.0, "banda {}", md.bandas_db[maior]);
    }
}
