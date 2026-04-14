mod keyboard;
mod bus;
mod display;
mod cpu;

use wasm_bindgen::prelude::*;
use bus::Bus;
use cpu::Cpu;
use display::Display;
use keyboard::map_key;

/// ZX Spectrum 48K — T-states per frame at 3.5 MHz / 50 Hz
const T_PER_FRAME: u32 = 69888;

/// Beeper audio: 3.5 MHz CPU → 44100 Hz output
const CPU_HZ: u32 = 3_500_000;
const SAMPLE_RATE: u32 = 44_100;

#[wasm_bindgen]
pub struct ZxSpectrum {
    cpu: Cpu,
    bus: Bus,
    display: Display,
    audio_buf: Vec<f32>,
}

#[wasm_bindgen]
impl ZxSpectrum {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        ZxSpectrum {
            cpu: Cpu::new(),
            bus: Bus::new(),
            display: Display::new(),
            audio_buf: Vec::new(),
        }
    }

    /// Load the 16 KB Spectrum ROM image.
    pub fn load_rom(&mut self, data: &[u8]) {
        self.bus.load_rom(data);
        self.reset();
    }

    /// Load a 48 K .SNA snapshot (49179 bytes).
    /// Does NOT require a ROM — the entire machine state is in the file.
    pub fn load_sna(&mut self, data: &[u8]) -> bool {
        if data.len() < 27 + 49152 { return false; }

        self.cpu.i   = data[0];
        self.cpu.l2  = data[1]; self.cpu.h2  = data[2];
        self.cpu.e2  = data[3]; self.cpu.d2  = data[4];
        self.cpu.c2  = data[5]; self.cpu.b2  = data[6];
        self.cpu.f2  = data[7]; self.cpu.a2  = data[8];
        self.cpu.l   = data[9]; self.cpu.h   = data[10];
        self.cpu.e   = data[11]; self.cpu.d  = data[12];
        self.cpu.c   = data[13]; self.cpu.b  = data[14];
        self.cpu.iy  = u16::from_le_bytes([data[15], data[16]]);
        self.cpu.ix  = u16::from_le_bytes([data[17], data[18]]);
        self.cpu.iff2 = data[19] & 0x04 != 0;
        self.cpu.iff1 = self.cpu.iff2;
        self.cpu.r   = data[20];
        self.cpu.f   = data[21]; self.cpu.a  = data[22];
        self.cpu.sp  = u16::from_le_bytes([data[23], data[24]]);
        self.cpu.im  = data[25] & 0x03;
        self.bus.border_color = data[26] & 0x07;

        // Load RAM (0x4000–0xFFFF)
        for (i, &b) in data[27..27 + 49152].iter().enumerate() {
            self.bus.write(0x4000u16.wrapping_add(i as u16), b);
        }

        // PC is on the stack in SNA format — pop it
        let sp = self.cpu.sp;
        let pc_lo = self.bus.read(sp);
        let pc_hi = self.bus.read(sp.wrapping_add(1));
        self.cpu.pc = (pc_hi as u16) << 8 | pc_lo as u16;
        self.cpu.sp = sp.wrapping_add(2);

        self.cpu.halted = false;
        self.cpu.t = 0;
        true
    }

    /// Run one video frame (69888 T-states). Fires the 50 Hz maskable interrupt.
    pub fn step_frame(&mut self) {
        self.bus.clear_beeper_log();
        self.cpu.t = 0;

        while self.cpu.t < T_PER_FRAME {
            // Fire the 50 Hz interrupt at T=0 of each frame
            if self.cpu.t == 0 {
                let irq_cycles = self.cpu.interrupt(&mut self.bus);
                if irq_cycles > 0 {
                    self.cpu.t += irq_cycles;
                    continue;
                }
            }
            let cycles = self.cpu.step(&mut self.bus);
            let _ = cycles; // t is updated inside step()
        }

        self.display.render(&self.bus);
        self.generate_audio();
    }

    /// Returns the 320×240 RGBA framebuffer.
    pub fn get_framebuffer(&self) -> Vec<u8> {
        self.display.framebuffer.clone()
    }

    /// Returns accumulated PCM samples (stereo interleaved f32, 44100 Hz).
    pub fn get_audio_buffer(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.audio_buf)
    }

    /// Key press/release. Pass the JS `event.key` string.
    pub fn key_down(&mut self, key: &str) {
        // Cursor keys: map to CAPS SHIFT + 5/6/7/8
        match key {
            "ArrowLeft"  => { self.bus.keyboard.press(keyboard::ZxKey::CapsShift); self.bus.keyboard.press(keyboard::ZxKey::K5); }
            "ArrowDown"  => { self.bus.keyboard.press(keyboard::ZxKey::CapsShift); self.bus.keyboard.press(keyboard::ZxKey::K6); }
            "ArrowUp"    => { self.bus.keyboard.press(keyboard::ZxKey::CapsShift); self.bus.keyboard.press(keyboard::ZxKey::K7); }
            "ArrowRight" => { self.bus.keyboard.press(keyboard::ZxKey::CapsShift); self.bus.keyboard.press(keyboard::ZxKey::K8); }
            _ => { if let Some(k) = map_key(key) { self.bus.keyboard.press(k); } }
        }
    }

    pub fn key_up(&mut self, key: &str) {
        match key {
            "ArrowLeft"  => { self.bus.keyboard.release(keyboard::ZxKey::CapsShift); self.bus.keyboard.release(keyboard::ZxKey::K5); }
            "ArrowDown"  => { self.bus.keyboard.release(keyboard::ZxKey::CapsShift); self.bus.keyboard.release(keyboard::ZxKey::K6); }
            "ArrowUp"    => { self.bus.keyboard.release(keyboard::ZxKey::CapsShift); self.bus.keyboard.release(keyboard::ZxKey::K7); }
            "ArrowRight" => { self.bus.keyboard.release(keyboard::ZxKey::CapsShift); self.bus.keyboard.release(keyboard::ZxKey::K8); }
            _ => { if let Some(k) = map_key(key) { self.bus.keyboard.release(k); } }
        }
    }

    pub fn screen_width()  -> u32 { display::SCREEN_W as u32 }
    pub fn screen_height() -> u32 { display::SCREEN_H as u32 }

    fn reset(&mut self) {
        self.cpu = Cpu::new();
        self.cpu.t = 0;
        self.bus.border_color = 7;
        self.bus.beeper = false;
    }

    /// Convert the frame's beeper transition log into square-wave PCM samples.
    fn generate_audio(&mut self) {
        // Number of samples for one frame
        let samples_per_frame = SAMPLE_RATE / 50; // 882

        let mut level = if self.bus.beeper { 0.5f32 } else { -0.5f32 };
        let mut log_iter = self.bus.beeper_log.iter().peekable();

        for s in 0..samples_per_frame {
            // T-cycle position corresponding to this sample
            let t_pos = (s as u64 * CPU_HZ as u64 / SAMPLE_RATE as u64) as u32;

            // Advance past any transitions that occurred before this sample
            while log_iter.peek().map_or(false, |&&(t, _)| t <= t_pos) {
                let (_, state) = *log_iter.next().unwrap();
                level = if state { 0.5 } else { -0.5 };
            }

            self.audio_buf.push(level); // left
            self.audio_buf.push(level); // right (mono)
        }
    }
}
