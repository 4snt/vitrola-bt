//! Codificador FLAC mínimo em Rust puro (16 bits, estéreo): preditores fixos
//! de ordem 0 a 4 + resíduo Rice particionado. Comprime ~40-60% do WAV e é
//! lido por qualquer player. O STREAMINFO é reescrito no fim com o total.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

const BLOCO: usize = 4096;
const BPS: u32 = 16;

struct Bits {
    buf: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn new() -> Self {
        Bits { buf: Vec::with_capacity(BLOCO * 4), acc: 0, n: 0 }
    }
    fn put(&mut self, valor: u64, bits: u32) {
        debug_assert!(bits <= 32);
        if bits == 0 {
            return;
        }
        self.acc = (self.acc << bits) | (valor & ((1u64 << bits) - 1));
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            self.buf.push((self.acc >> self.n) as u8);
        }
    }
    fn zeros(&mut self, mut q: u64) {
        while q >= 32 {
            self.put(0, 32);
            q -= 32;
        }
        self.put(0, q as u32);
    }
    fn alinhar(&mut self) {
        if self.n > 0 {
            self.put(0, 8 - self.n);
        }
    }
}

fn crc8(dados: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in dados {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

fn crc16(dados: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in dados {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x8005 } else { crc << 1 };
        }
    }
    crc
}

fn utf8_num(b: &mut Bits, v: u64) {
    if v < 0x80 {
        b.put(v, 8);
        return;
    }
    let (bytes, prefixo) = match v {
        0..=0x7FF => (2, 0xC0),
        0x800..=0xFFFF => (3, 0xE0),
        0x1_0000..=0x1F_FFFF => (4, 0xF0),
        0x20_0000..=0x3FF_FFFF => (5, 0xF8),
        _ => (6, 0xFC),
    };
    let resto = 6 * (bytes - 1);
    b.put(prefixo | (v >> resto), 8);
    for i in (0..bytes - 1).rev() {
        b.put(0x80 | ((v >> (6 * i)) & 0x3F), 8);
    }
}

fn residuo(x: &[i32], ordem: usize) -> Vec<i32> {
    (ordem..x.len())
        .map(|i| {
            let s = |k: usize| x[i - k] as i64;
            (match ordem {
                0 => s(0),
                1 => s(0) - s(1),
                2 => s(0) - 2 * s(1) + s(2),
                3 => s(0) - 3 * s(1) + 3 * s(2) - s(3),
                _ => s(0) - 4 * s(1) + 6 * s(2) - 4 * s(3) + s(4),
            }) as i32
        })
        .collect()
}

fn zigzag(r: i32) -> u64 {
    ((r << 1) ^ (r >> 31)) as u32 as u64
}

/// Melhor parâmetro Rice para uma partição e o custo em bits.
fn rice(part: &[i32]) -> (u32, u64) {
    let soma: u64 = part.iter().map(|&r| zigzag(r)).sum();
    let media = soma / part.len().max(1) as u64;
    let k0 = if media == 0 { 0 } else { 63 - media.leading_zeros() };
    let mut melhor = (0, u64::MAX);
    for k in k0.saturating_sub(1)..=(k0 + 1).min(14) {
        let bits = part.len() as u64 * (k as u64 + 1) + part.iter().map(|&r| zigzag(r) >> k).sum::<u64>();
        if bits < melhor.1 {
            melhor = (k, bits);
        }
    }
    melhor
}

/// (ordem de partição, parâmetros, custo) para o resíduo de um preditor.
fn particionar(res: &[i32], n: usize, ordem: usize) -> (u32, Vec<u32>, u64) {
    let mut melhor = (0, vec![], u64::MAX);
    for p in 0..=4u32 {
        let parts = 1usize << p;
        if n % parts != 0 || (n >> p) <= ordem {
            break;
        }
        let mut ks = Vec::with_capacity(parts);
        let mut custo = 6u64; // método + ordem de partição
        let mut ini = 0;
        for i in 0..parts {
            let tam = (n >> p) - if i == 0 { ordem } else { 0 };
            let (k, bits) = rice(&res[ini..ini + tam]);
            ks.push(k);
            custo += 4 + bits;
            ini += tam;
        }
        if custo < melhor.2 {
            melhor = (p, ks, custo);
        }
    }
    melhor
}

fn subframe(b: &mut Bits, x: &[i32]) {
    let n = x.len();
    if x.iter().all(|&v| v == x[0]) {
        b.put(0, 8); // pad + CONSTANT + sem wasted bits
        b.put(x[0] as u32 as u64, BPS);
        return;
    }
    struct Candidato {
        ordem: usize,
        res: Vec<i32>,
        p: u32,
        ks: Vec<u32>,
        custo: u64,
    }
    let mut melhor: Option<Candidato> = None;
    for ordem in 0..=4usize.min(n.saturating_sub(1)) {
        let res = residuo(x, ordem);
        let (p, ks, custo) = particionar(&res, n, ordem);
        let custo = custo + (ordem as u64) * BPS as u64;
        if melhor.as_ref().is_none_or(|m| custo < m.custo) {
            melhor = Some(Candidato { ordem, res, p, ks, custo });
        }
    }
    let Candidato { ordem, res, p, ks, custo } = melhor.unwrap();
    if custo >= (n as u64) * BPS as u64 {
        b.put(0b0000_0010, 8); // VERBATIM
        for &v in x {
            b.put(v as u32 as u64, BPS);
        }
        return;
    }
    b.put(((0b001000 | ordem as u64) << 1) & 0x7E, 8); // pad 0 + FIXED(ordem) + wasted 0
    for &v in &x[..ordem] {
        b.put(v as u32 as u64, BPS);
    }
    b.put(0b00, 2); // Rice com parâmetro de 4 bits
    b.put(p as u64, 4);
    let mut ini = 0;
    for (i, &k) in ks.iter().enumerate() {
        let tam = (n >> p) - if i == 0 { ordem } else { 0 };
        b.put(k as u64, 4);
        for &r in &res[ini..ini + tam] {
            let u = zigzag(r);
            b.zeros(u >> k);
            b.put(1, 1);
            b.put(u, k);
        }
        ini += tam;
    }
}

pub struct Flac {
    arq: BufWriter<File>,
    rate: u32,
    l: Vec<i32>,
    r: Vec<i32>,
    quadro: u64,
    amostras: u64,
    min_quadro: u32,
    max_quadro: u32,
}

impl Flac {
    pub fn criar(caminho: &Path, rate: u32) -> io::Result<Self> {
        let mut arq = BufWriter::new(File::create(caminho)?);
        arq.write_all(b"fLaC")?;
        let mut f = Flac {
            arq,
            rate,
            l: Vec::with_capacity(BLOCO),
            r: Vec::with_capacity(BLOCO),
            quadro: 0,
            amostras: 0,
            min_quadro: u32::MAX,
            max_quadro: 0,
        };
        f.escrever_streaminfo()?;
        Ok(f)
    }

    fn escrever_streaminfo(&mut self) -> io::Result<()> {
        let mut b = Bits::new();
        b.put(1, 1); // último bloco de metadados
        b.put(0, 7); // STREAMINFO
        b.put(34, 24);
        b.put(BLOCO as u64, 16);
        b.put(BLOCO as u64, 16);
        b.put(if self.min_quadro == u32::MAX { 0 } else { self.min_quadro as u64 }, 24);
        b.put(self.max_quadro as u64, 24);
        b.put(self.rate as u64, 20);
        b.put(1, 3); // 2 canais
        b.put((BPS - 1) as u64, 5);
        b.put(self.amostras >> 32, 4);
        b.put(self.amostras & 0xFFFF_FFFF, 32);
        b.put(0, 32); // MD5 desconhecido (permitido)
        b.put(0, 32);
        b.put(0, 32);
        b.put(0, 32);
        self.arq.write_all(&b.buf)
    }

    /// PCM s16le estéreo intercalado.
    pub fn escrever(&mut self, pcm: &[u8]) -> io::Result<()> {
        for q in pcm.chunks_exact(4) {
            self.l.push(i16::from_le_bytes([q[0], q[1]]) as i32);
            self.r.push(i16::from_le_bytes([q[2], q[3]]) as i32);
            if self.l.len() == BLOCO {
                self.escrever_quadro()?;
            }
        }
        Ok(())
    }

    fn escrever_quadro(&mut self) -> io::Result<()> {
        let n = self.l.len();
        if n == 0 {
            return Ok(());
        }
        let mut b = Bits::new();
        b.put(0xFFF8, 16); // sync + bloco de tamanho fixo
        let codigo_bloco = if n == BLOCO { 12 } else { 7 };
        b.put(codigo_bloco, 4);
        b.put(
            match self.rate {
                48_000 => 0b1010,
                44_100 => 0b1001,
                32_000 => 0b1000,
                16_000 => 0b0101,
                _ => 0b0000,
            },
            4,
        );
        b.put(0b0001, 4); // 2 canais independentes
        b.put(0b100, 3); // 16 bits
        b.put(0, 1);
        utf8_num(&mut b, self.quadro);
        if codigo_bloco == 7 {
            b.put((n - 1) as u64, 16);
        }
        let crc = crc8(&b.buf);
        b.put(crc as u64, 8);
        subframe(&mut b, &self.l);
        subframe(&mut b, &self.r);
        b.alinhar();
        let crc = crc16(&b.buf);
        b.put(crc as u64, 16);
        self.arq.write_all(&b.buf)?;
        let tam = b.buf.len() as u32;
        self.min_quadro = self.min_quadro.min(tam);
        self.max_quadro = self.max_quadro.max(tam);
        self.quadro += 1;
        self.amostras += n as u64;
        self.l.clear();
        self.r.clear();
        Ok(())
    }

    /// Fecha o arquivo: último quadro e STREAMINFO com o total de amostras.
    pub fn finalizar(mut self) -> io::Result<u64> {
        self.escrever_quadro()?;
        self.arq.seek(SeekFrom::Start(4))?;
        self.escrever_streaminfo()?;
        self.arq.flush()?;
        Ok(self.amostras)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn crc_conhecidos() {
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc16(b"123456789"), 0xFEE8);
    }

    #[test]
    fn grava_e_fecha() {
        let dir = std::env::temp_dir().join(format!("vitrola-flac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let caminho = dir.join("teste.flac");
        let mut f = Flac::criar(&caminho, 48_000).unwrap();
        let pcm: Vec<u8> = (0..100_000)
            .flat_map(|i| {
                let t = i as f64 / 48_000.0;
                let l = (9000.0 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as i16;
                let r = (5000.0 * (2.0 * std::f64::consts::PI * 660.0 * t).sin()) as i16;
                [l.to_le_bytes(), r.to_le_bytes()].concat()
            })
            .collect();
        f.escrever(&pcm).unwrap();
        assert_eq!(f.finalizar().unwrap(), 100_000);
        let tamanho = std::fs::metadata(&caminho).unwrap().len();
        assert!(tamanho < pcm.len() as u64 / 2, "comprimiu para {tamanho} bytes");
    }
}
