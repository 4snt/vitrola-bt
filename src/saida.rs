//! Saída de som: um stream do PipeWire (`pw-cat --playback`) alimentado pelo
//! PCM decodificado, com um buffer anti-jitter: o Bluetooth entrega em rajadas,
//! então o cano até o pw-cat é mantido com ~250 ms de reserva. Se esvaziar, o
//! app junta 250 ms de novo antes de voltar a tocar (um silêncio curto em vez de
//! dezenas de picotes). A escrita nunca bloqueia a thread de áudio.

use std::collections::VecDeque;
use std::io;
use std::os::fd::AsRawFd;
use std::process::{Child, Command, Stdio};

use crate::sbc;

pub const PREFIXO_NO: &str = "vitrola_bt.";

/// Reserva que o cano até o pw-cat precisa ter antes de (re)começar a tocar.
const RESERVA_MS: usize = 250;
/// Tamanho do cano (o padrão de 64 KiB é só ~340 ms a 48 kHz estéreo).
const CANO_BYTES: libc::c_int = 256 * 1024;

pub struct Saida {
    filho: Child,
    fila: VecDeque<u8>,
    reserva: usize,
    maximo: usize,
    juntando: bool,
}

impl Saida {
    pub fn abrir(cfg: &sbc::Config, mac: &str) -> io::Result<Self> {
        let mac_no = mac.replace(':', "_");
        let props = [
            ("node.name", format!("{PREFIXO_NO}{mac_no}")),
            ("node.description", format!("Vitrola · SBC {} · {} kbps", cfg.modo.nome(), cfg.kbps)),
            ("application.name", "Vitrola BT".to_string()),
            ("api.bluez5.address", mac.to_string()),
            ("vitrola.modo", cfg.modo.nome().to_string()),
            ("vitrola.kbps", cfg.kbps.to_string()),
        ];
        // pw-cat espera SPA-JSON: { chave = "valor" ... }
        let spa =
            format!("{{ {} }}", props.iter().map(|(k, v)| format!("{k} = \"{v}\"")).collect::<Vec<_>>().join(" "));
        let filho = Command::new("pw-cat")
            // f32: o PCM depois do EQ e do volume vai em ponto flutuante, sem
            // voltar para 16 bits (o PipeWire mistura em float de qualquer jeito)
            .args(["--playback", "--raw", "--format", "f32", "--channels", "2", "--media-role", "Music"])
            .args(["--rate", &cfg.rate.to_string(), "-P", &spa, "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let fd = filho.stdin.as_ref().map(|s| s.as_raw_fd()).unwrap_or(-1);
        // SAFETY: fd é o stdin do filho, aberto por nós.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            libc::fcntl(fd, libc::F_SETPIPE_SZ, CANO_BYTES);
        }
        let por_ms = cfg.rate as usize * 8 / 1000; // f32 estéreo = 8 bytes por quadro
        Ok(Saida { filho, fila: VecDeque::new(), reserva: RESERVA_MS * por_ms, maximo: 1000 * por_ms, juntando: true })
    }

    fn no_cano(&self, fd: i32) -> usize {
        let mut n: libc::c_int = 0;
        // SAFETY: FIONREAD num fd de pipe nosso escreve um int.
        if unsafe { libc::ioctl(fd, libc::FIONREAD, &mut n) } < 0 {
            return 0;
        }
        n.max(0) as usize
    }

    pub fn escrever(&mut self, pcm: &[u8]) -> io::Result<()> {
        let Some(stdin) = self.filho.stdin.as_ref() else {
            return Err(io::ErrorKind::BrokenPipe.into());
        };
        let fd = stdin.as_raw_fd();
        self.fila.extend(pcm);
        if self.fila.len() > self.maximo {
            let sobra = self.fila.len() - self.maximo;
            self.fila.drain(..sobra);
        }
        let no_cano = self.no_cano(fd);
        if !self.juntando && no_cano == 0 {
            // o cano secou (rajada atrasou demais): junta a reserva de novo
            self.juntando = true;
        }
        if self.juntando {
            if no_cano + self.fila.len() < self.reserva {
                return Ok(());
            }
            self.juntando = false;
        }
        while !self.fila.is_empty() {
            let (a, _) = self.fila.as_slices();
            // SAFETY: escrevemos de um slice válido num fd nosso.
            let n = unsafe { libc::write(fd, a.as_ptr() as *const libc::c_void, a.len()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::WouldBlock {
                    break; // cano cheio: o resto espera na fila
                }
                return Err(e);
            }
            self.fila.drain(..n as usize);
        }
        Ok(())
    }
}

impl Drop for Saida {
    fn drop(&mut self) {
        drop(self.filho.stdin.take());
        let _ = self.filho.kill();
        let _ = self.filho.wait();
    }
}
