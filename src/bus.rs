/// ZX Spectrum 48K memory bus and I/O
///
/// Memory map:
///   0x0000–0x3FFF  ROM (16 KB, read-only)
///   0x4000–0x7FFF  Video RAM + attributes (16 KB)
///   0x8000–0xFFFF  RAM (32 KB)
///
/// I/O (port 0xFE, selected when A0=0):
///   Write → bits 2:0 = border colour, bit 3 = MIC, bit 4 = EAR/beeper
///   Read  → bits 4:0 = keyboard half-row, bit 6 = EAR input

use crate::keyboard::Keyboard;

const ROM_SIZE: usize = 0x4000;  // 16 KB
const RAM_SIZE: usize = 0xC000;  // 48 KB (0x4000–0xFFFF)

pub struct Bus {
    rom: Vec<u8>,
    ram: [u8; RAM_SIZE],
    pub keyboard: Keyboard,
    pub border_color: u8,
    // Beeper: 1-bit audio output driven by OUT 0xFE bit 4
    pub beeper: bool,
    // Sequence of (t_cycle_within_frame, new_state) transitions for audio
    pub beeper_log: Vec<(u32, bool)>,
}

impl Bus {
    pub fn new() -> Self {
        Bus {
            rom: vec![0xFF; ROM_SIZE],
            ram: [0u8; RAM_SIZE],
            keyboard: Keyboard::new(),
            border_color: 7,
            beeper: false,
            beeper_log: Vec::new(),
        }
    }

    pub fn load_rom(&mut self, data: &[u8]) {
        let len = data.len().min(ROM_SIZE);
        self.rom[..len].copy_from_slice(&data[..len]);
        // Pad with 0xFF if shorter
        for b in self.rom[len..].iter_mut() { *b = 0xFF; }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3FFF => *self.rom.get(addr as usize).unwrap_or(&0xFF),
            _ => self.ram[(addr - 0x4000) as usize],
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        if addr >= 0x4000 {
            self.ram[(addr - 0x4000) as usize] = val;
        }
        // Writes to ROM are silently ignored
    }

    pub fn read16(&self, addr: u16) -> u16 {
        self.read(addr) as u16 | ((self.read(addr.wrapping_add(1)) as u16) << 8)
    }

    pub fn write16(&mut self, addr: u16, val: u16) {
        self.write(addr, val as u8);
        self.write(addr.wrapping_add(1), (val >> 8) as u8);
    }

    /// I/O read. The Spectrum uses A0=0 to select ULA (port 0xFE family).
    /// The upper byte selects keyboard rows.
    pub fn port_in(&self, port: u16) -> u8 {
        if port & 0x0001 == 0 {
            // ULA port — return keyboard + EAR bit
            let port_hi = (port >> 8) as u8;
            let keys = self.keyboard.read(port_hi);
            // Bit 6 = EAR (tape input), held high here; bits 5,7 = 1
            0xA0 | keys
        } else {
            0xFF
        }
    }

    /// I/O write. Port 0xFE: border colour (bits 2:0), MIC (bit 3), EAR/beeper (bit 4).
    pub fn port_out(&mut self, port: u16, val: u8, t_cycle: u32) {
        if port & 0x0001 == 0 {
            self.border_color = val & 0x07;
            let new_beeper = val & 0x10 != 0;
            if new_beeper != self.beeper {
                self.beeper = new_beeper;
                self.beeper_log.push((t_cycle, new_beeper));
            }
        }
    }

    /// Clear per-frame beeper log (called at start of each frame).
    pub fn clear_beeper_log(&mut self) {
        self.beeper_log.clear();
    }

    /// Direct RAM read (used by display — bypasses ROM check, always RAM).
    #[inline]
    pub fn vram_byte(&self, addr: u16) -> u8 {
        if addr >= 0x4000 {
            self.ram[(addr - 0x4000) as usize]
        } else {
            0xFF
        }
    }
}
