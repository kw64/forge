//! Lecteur PSID : 6502 (mos6502) + SID (reSID). Approximation : les écritures SID d'une image sont
//! appliquées au début de l'image, puis le SID avance d'une période d'image complète.
use super::{Pcm, SAMPLE_RATE};
use mos6502::cpu::CPU;
use mos6502::instruction::Nmos6502;
use mos6502::memory::Bus;
use resid::{ChipModel, SamplingMethod, Sid};

struct Mem {
    ram: Box<[u8; 65536]>,
    writes: Vec<(u8, u8)>,
}

impl Bus for Mem {
    fn get_byte(&mut self, a: u16) -> u8 {
        match a {
            0xD012 => 0x80,                       // trame raster : évite les attentes infinies
            0xDC0D | 0xDD0D => 0,                  // CIA : pas d'IRQ en attente
            _ => self.ram[a as usize],
        }
    }
    fn set_byte(&mut self, a: u16, v: u8) {
        if (0xD400..=0xD41C).contains(&a) {
            self.writes.push(((a - 0xD400) as u8, v));
        }
        self.ram[a as usize] = v;
    }
}

const TRAP: u16 = 0xFFF0;

/// Appelle une sous-routine 6502 (JSR) ; renvoie les cycles consommés.
fn call(cpu: &mut CPU<Mem, Nmos6502>, addr: u16, a: u8) -> u64 {
    let start = cpu.cycles;
    // Adresse de retour = TRAP - 1 (RTS ajoute 1)
    let ret = TRAP.wrapping_sub(1);
    cpu.registers.accumulator = a;
    cpu.registers.index_x = 0;
    cpu.registers.index_y = 0;
    cpu.registers.stack_pointer.0 = 0xFD;
    cpu.memory.ram[0x01FD] = (ret >> 8) as u8;
    cpu.memory.ram[0x01FC] = (ret & 0xFF) as u8;
    cpu.registers.stack_pointer.0 = 0xFB;
    cpu.registers.program_counter = addr;
    let limit = start + 3_000_000;
    while cpu.registers.program_counter != TRAP && cpu.cycles < limit {
        if !cpu.single_step() {
            break;
        }
    }
    cpu.cycles - start
}

pub fn render(data: &[u8], max_secs: f32) -> Result<Pcm, String> {
    if data.len() < 0x7C {
        return Err("SID : fichier trop court".into());
    }
    if &data[0..4] == b"RSID" {
        return Err("SID : les fichiers RSID exigent un C64 complet (non pris en charge) ; utilisez un PSID".into());
    }
    if &data[0..4] != b"PSID" {
        return Err("SID : en-tête PSID absent".into());
    }
    let be16 = |o: usize| u16::from_be_bytes([data[o], data[o + 1]]);
    let version = be16(4);
    let off = be16(6) as usize;
    let mut load = be16(8);
    let init = be16(0x0A);
    let mut play = be16(0x0C);
    let start = be16(0x10).max(1);
    let speed = u32::from_be_bytes([data[0x12], data[0x13], data[0x14], data[0x15]]);
    let flags = if version >= 2 && data.len() >= 0x78 { be16(0x76) } else { 0 };
    let ntsc = (flags >> 2) & 3 == 2;
    let model = if (flags >> 4) & 3 == 2 { ChipModel::Mos8580 } else { ChipModel::Mos6581 };
    let mut body = data.get(off..).ok_or("SID : décalage des données invalide")?;
    if load == 0 {
        if body.len() < 2 {
            return Err("SID : données absentes".into());
        }
        load = u16::from_le_bytes([body[0], body[1]]);
        body = &body[2..];
    }
    if load as usize + body.len() > 0x10000 {
        return Err("SID : programme hors mémoire".into());
    }
    let mut mem = Mem { ram: Box::new([0; 65536]), writes: Vec::new() };
    mem.ram[load as usize..load as usize + body.len()].copy_from_slice(body);
    mem.ram[TRAP as usize] = 0xEA;
    let mut cpu = CPU::new(mem, Nmos6502);

    let (clock, frame_cycles): (u32, u32) = if ntsc { (1_022_727, 17_095) } else { (985_248, 19_656) };
    let mut sid = Sid::new(model);
    sid.set_sampling_parameters(SamplingMethod::Fast, clock, SAMPLE_RATE);

    call(&mut cpu, init, (start - 1) as u8);
    if play == 0 {
        // Le morceau installe son propre gestionnaire d'interruption.
        play = u16::from_le_bytes([cpu.memory.ram[0x0314], cpu.memory.ram[0x0315]]);
        if play == 0 {
            return Err("SID : routine de lecture introuvable".into());
        }
    }
    let _ = speed; // 50/60 Hz : on suit l'horloge vidéo du fichier
    let total = (max_secs.max(1.0) * SAMPLE_RATE as f32) as usize;
    let mut out: Vec<i16> = Vec::with_capacity(total * 2);
    let mut buf = vec![0i16; 4096];
    let mut silent = 0usize;
    while out.len() < total * 2 {
        for (r, v) in cpu.memory.writes.drain(..).collect::<Vec<_>>() {
            sid.write(r, v);
        }
        let used = call(&mut cpu, play, 0) as u32;
        let writes: Vec<(u8, u8)> = cpu.memory.writes.drain(..).collect();
        for (r, v) in writes {
            sid.write(r, v);
        }
        let mut remaining = frame_cycles.max(used.min(frame_cycles));
        while remaining > 0 {
            let (n, left) = sid.sample(remaining, &mut buf, 1);
            for &s in &buf[..n] {
                out.push(s);
                out.push(s);
            }
            if left == remaining && n == 0 {
                break;
            }
            remaining = left;
        }
        // Arrêt anticipé si le morceau est muet depuis longtemps (> 15 s)
        let tail = &out[out.len().saturating_sub(2 * 882)..];
        if tail.iter().all(|s| s.abs() < 8) {
            silent += 882;
        } else {
            silent = 0;
        }
        if silent > SAMPLE_RATE as usize * 15 {
            let keep = out.len().saturating_sub(silent * 2).max(SAMPLE_RATE as usize * 2);
            out.truncate(keep);
            break;
        }
    }
    out.truncate(total * 2);
    Ok(Pcm { rate: SAMPLE_RATE, samples: out })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PSID minimal : init règle le volume, play fait varier la fréquence de la voix 1.
    fn tiny() -> Vec<u8> {
        let mut h = vec![0u8; 0x7C];
        h[0..4].copy_from_slice(b"PSID");
        h[5] = 2;
        h[7] = 0x7C; // offset
        h[0x0A] = 0x10; // init = $1000
        h[0x0B] = 0x00;
        h[0x0C] = 0x10; // play = $1020
        h[0x0D] = 0x20;
        h[0x0F] = 1; // songs
        h[0x11] = 1; // start
        let mut code = vec![0x00, 0x10]; // adresse de chargement $1000
        let mut m = vec![0xEA; 0x30];
        // init : LDA #$0F ; STA $D418 ; LDA #$21 ; STA $D404 ; LDA #$00 ; STA $D405 ; LDA #$F0 ; STA $D406 ; RTS
        let init = [0xA9, 0x0F, 0x8D, 0x18, 0xD4, 0xA9, 0x00, 0x8D, 0x05, 0xD4, 0xA9, 0xF0, 0x8D, 0x06, 0xD4, 0xA9, 0x21, 0x8D, 0x04, 0xD4, 0x60];
        m[..init.len()].copy_from_slice(&init);
        // play : INC $1040 ; LDA $1040 ; STA $D401 ; RTS
        let play = [0xEE, 0x40, 0x10, 0xAD, 0x40, 0x10, 0x8D, 0x01, 0xD4, 0x60];
        m[0x20..0x20 + play.len()].copy_from_slice(&play);
        code.extend(m);
        code.extend(vec![0u8; 0x20]);
        h.extend(code);
        h
    }

    #[test]
    fn plays_tiny_psid() {
        let pcm = render(&tiny(), 2.0).unwrap();
        assert_eq!(pcm.samples.len(), 2 * 2 * 44100);
        let peak = pcm.samples.iter().map(|s| s.abs()).max().unwrap();
        assert!(peak > 500, "silence : {peak}");
    }

    #[test]
    fn rejects_rsid() {
        let mut d = tiny();
        d[0..4].copy_from_slice(b"RSID");
        assert!(render(&d, 1.0).is_err());
    }
}
