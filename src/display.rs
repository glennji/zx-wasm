/// ZX Spectrum display renderer
///
/// Output: 320×240 RGBA framebuffer (256×192 game area + 32px border on
/// left/right and 24px border on top/bottom).
///
/// Pixel memory layout (0x4000–0x57FF, 6144 bytes):
///   The screen is divided into three vertical thirds (0–63, 64–127, 128–191).
///   Within each third, the address encodes rows in a non-linear order:
///     addr = 0x4000 | (third<<11) | (line_in_char<<8) | (char_row<<5) | col_byte
///   where:
///     third        = scanline >> 6          (0–2)
///     line_in_char = scanline & 7           (0–7, pixel row within 8-line block)
///     char_row     = (scanline >> 3) & 7    (0–7, which character row in the third)
///     col_byte     = 0–31                   (byte column)
///
/// Attribute memory (0x5800–0x5AFF, 768 bytes):
///   Linear: attr[y/8 * 32 + x_byte]
///   Byte:  FBPPPIII  (F=flash, B=bright, PPP=paper, III=ink)

use crate::bus::Bus;

pub const SCREEN_W: usize = 320;
pub const SCREEN_H: usize = 240;
const BORDER_X: usize = 32;
const BORDER_Y: usize = 24;
const GAME_W: usize = 256;
const GAME_H: usize = 192;

/// Standard Spectrum palette: 8 colours × normal/bright
static PALETTE: [(u8, u8, u8); 16] = [
    // Normal
    (0x00, 0x00, 0x00), // 0 Black
    (0x00, 0x00, 0xCD), // 1 Blue
    (0xCD, 0x00, 0x00), // 2 Red
    (0xCD, 0x00, 0xCD), // 3 Magenta
    (0x00, 0xCD, 0x00), // 4 Green
    (0x00, 0xCD, 0xCD), // 5 Cyan
    (0xCD, 0xCD, 0x00), // 6 Yellow
    (0xCD, 0xCD, 0xCD), // 7 White
    // Bright
    (0x00, 0x00, 0x00), // 8 Bright Black (same as black)
    (0x00, 0x00, 0xFF), // 9 Bright Blue
    (0xFF, 0x00, 0x00), // 10 Bright Red
    (0xFF, 0x00, 0xFF), // 11 Bright Magenta
    (0x00, 0xFF, 0x00), // 12 Bright Green
    (0x00, 0xFF, 0xFF), // 13 Bright Cyan
    (0xFF, 0xFF, 0x00), // 14 Bright Yellow
    (0xFF, 0xFF, 0xFF), // 15 Bright White
];

pub struct Display {
    pub framebuffer: Vec<u8>, // SCREEN_W × SCREEN_H × 4 (RGBA)
    /// Incremented every frame; drives the flash attribute (toggles at 16 frames ≈ 1.5 Hz at 50 Hz)
    pub frame_counter: u32,
}

impl Display {
    pub fn new() -> Self {
        Display {
            framebuffer: vec![0xFF; SCREEN_W * SCREEN_H * 4],
            frame_counter: 0,
        }
    }

    pub fn render(&mut self, bus: &Bus) {
        self.frame_counter += 1;
        let flash_phase = (self.frame_counter / 16) & 1 == 1;

        let border = PALETTE[bus.border_color as usize & 7];

        // Fill entire framebuffer with border colour
        for y in 0..SCREEN_H {
            for x in 0..SCREEN_W {
                let in_game = x >= BORDER_X
                    && x < BORDER_X + GAME_W
                    && y >= BORDER_Y
                    && y < BORDER_Y + GAME_H;

                let (r, g, b) = if in_game {
                    let gx = x - BORDER_X;
                    let gy = y - BORDER_Y;
                    self.pixel_color(bus, gx, gy, flash_phase)
                } else {
                    border
                };

                let idx = (y * SCREEN_W + x) * 4;
                self.framebuffer[idx]     = r;
                self.framebuffer[idx + 1] = g;
                self.framebuffer[idx + 2] = b;
                self.framebuffer[idx + 3] = 0xFF;
            }
        }
    }

    fn pixel_color(&self, bus: &Bus, gx: usize, gy: usize, flash_phase: bool) -> (u8, u8, u8) {
        // Pixel byte address
        let pixel_addr = pixel_addr(gy, gx >> 3);
        let pixel_byte = bus.vram_byte(pixel_addr as u16);
        let pixel_bit = 7 - (gx & 7);
        let ink_on = (pixel_byte >> pixel_bit) & 1 != 0;

        // Attribute byte
        let attr_addr = 0x5800u16 + ((gy / 8) * 32 + gx / 8) as u16;
        let attr = bus.vram_byte(attr_addr);

        let flash  = attr & 0x80 != 0;
        let bright = (attr & 0x40) >> 3; // shifts to bit 3, making it the bright nibble
        let paper  = (attr >> 3) & 0x07;
        let ink    = attr & 0x07;

        // If flash is set and we're in the active flash phase, swap ink/paper
        let (ink_idx, paper_idx) = if flash && flash_phase {
            (paper | bright, ink | bright)
        } else {
            (ink | bright, paper | bright)
        };

        let color_idx = if ink_on { ink_idx } else { paper_idx } as usize;
        PALETTE[color_idx]
    }
}

/// Calculate the VRAM address for pixel byte at game scanline `y` (0–191), column byte `x` (0–31).
#[inline]
fn pixel_addr(y: usize, x: usize) -> usize {
    let third        = (y >> 6) & 0x03;
    let line_in_char = y & 0x07;
    let char_row     = (y >> 3) & 0x07;
    0x4000 | (third << 11) | (line_in_char << 8) | (char_row << 5) | x
}
