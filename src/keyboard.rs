/// ZX Spectrum keyboard — 8 half-rows of 5 keys each.
///
/// The keyboard is read via port 0xFE. The upper byte of the port address
/// selects which half-rows to read (bit = 0 means selected). Bits 4:0 of
/// the returned byte are the key states (0 = pressed).
///
/// Half-row map (port addr → keys, left-to-right = bit 0 to bit 4):
///   0xFEFE : CAPS-SHIFT  Z  X  C  V
///   0xFDFE : A           S  D  F  G
///   0xFBFE : Q           W  E  R  T
///   0xF7FE : 1           2  3  4  5
///   0xEFFE : 0           9  8  7  6
///   0xDFFE : P           O  I  U  Y
///   0xBFFE : ENTER       L  K  J  H
///   0x7FFE : SPACE  SYM-SHIFT  M  N  B

pub struct Keyboard {
    // rows[0..8] — bit set means key is pressed
    rows: [u8; 8],
}

impl Keyboard {
    pub fn new() -> Self {
        Keyboard { rows: [0u8; 8] }
    }

    /// Read the combined state of all half-rows selected by `port_hi` (upper byte).
    /// Returns bits 4:0 with 0 = pressed, 1 = released (active-low).
    pub fn read(&self, port_hi: u8) -> u8 {
        let mut result = 0x1Fu8; // all 5 bits high (not pressed)
        for row in 0..8 {
            if port_hi & (1 << row) == 0 {
                // This row is selected — OR in its pressed bits (inverted)
                result &= !self.rows[row];
            }
        }
        result
    }

    pub fn press(&mut self, key: ZxKey) {
        let (row, bit) = key.location();
        self.rows[row] |= 1 << bit;
    }

    pub fn release(&mut self, key: ZxKey) {
        let (row, bit) = key.location();
        self.rows[row] &= !(1 << bit);
    }
}

/// Every key on the ZX Spectrum keyboard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZxKey {
    // Row 0 (0xFEFE)
    CapsShift, Z, X, C, V,
    // Row 1 (0xFDFE)
    A, S, D, F, G,
    // Row 2 (0xFBFE)
    Q, W, E, R, T,
    // Row 3 (0xF7FE)
    K1, K2, K3, K4, K5,
    // Row 4 (0xEFFE)
    K0, K9, K8, K7, K6,
    // Row 5 (0xDFFE)
    P, O, I, U, Y,
    // Row 6 (0xBFFE)
    Enter, L, K, J, H,
    // Row 7 (0x7FFE)
    Space, SymShift, M, N, B,
}

impl ZxKey {
    /// Returns (row, bit) for this key.
    fn location(self) -> (usize, usize) {
        match self {
            ZxKey::CapsShift => (0, 0), ZxKey::Z => (0, 1), ZxKey::X => (0, 2),
            ZxKey::C => (0, 3), ZxKey::V => (0, 4),

            ZxKey::A => (1, 0), ZxKey::S => (1, 1), ZxKey::D => (1, 2),
            ZxKey::F => (1, 3), ZxKey::G => (1, 4),

            ZxKey::Q => (2, 0), ZxKey::W => (2, 1), ZxKey::E => (2, 2),
            ZxKey::R => (2, 3), ZxKey::T => (2, 4),

            ZxKey::K1 => (3, 0), ZxKey::K2 => (3, 1), ZxKey::K3 => (3, 2),
            ZxKey::K4 => (3, 3), ZxKey::K5 => (3, 4),

            ZxKey::K0 => (4, 0), ZxKey::K9 => (4, 1), ZxKey::K8 => (4, 2),
            ZxKey::K7 => (4, 3), ZxKey::K6 => (4, 4),

            ZxKey::P => (5, 0), ZxKey::O => (5, 1), ZxKey::I => (5, 2),
            ZxKey::U => (5, 3), ZxKey::Y => (5, 4),

            ZxKey::Enter => (6, 0), ZxKey::L => (6, 1), ZxKey::K => (6, 2),
            ZxKey::J => (6, 3), ZxKey::H => (6, 4),

            ZxKey::Space => (7, 0), ZxKey::SymShift => (7, 1), ZxKey::M => (7, 2),
            ZxKey::N => (7, 3), ZxKey::B => (7, 4),
        }
    }
}

/// Map a JS `event.code` string to a ZxKey (or None if unmapped).
/// Using `e.code` rather than `e.key` means the mapping is layout-independent
/// and modifier keys have unambiguous values (e.g. "ShiftLeft" not "Shift").
pub fn map_key(code: &str) -> Option<ZxKey> {
    Some(match code {
        "ShiftLeft" | "ShiftRight" | "CapsLock" => ZxKey::CapsShift,
        "ControlLeft" | "ControlRight"          => ZxKey::SymShift,
        "Space"  => ZxKey::Space,
        "Enter"  => ZxKey::Enter,
        "KeyA"   => ZxKey::A,
        "KeyB"   => ZxKey::B,
        "KeyC"   => ZxKey::C,
        "KeyD"   => ZxKey::D,
        "KeyE"   => ZxKey::E,
        "KeyF"   => ZxKey::F,
        "KeyG"   => ZxKey::G,
        "KeyH"   => ZxKey::H,
        "KeyI"   => ZxKey::I,
        "KeyJ"   => ZxKey::J,
        "KeyK"   => ZxKey::K,
        "KeyL"   => ZxKey::L,
        "KeyM"   => ZxKey::M,
        "KeyN"   => ZxKey::N,
        "KeyO"   => ZxKey::O,
        "KeyP"   => ZxKey::P,
        "KeyQ"   => ZxKey::Q,
        "KeyR"   => ZxKey::R,
        "KeyS"   => ZxKey::S,
        "KeyT"   => ZxKey::T,
        "KeyU"   => ZxKey::U,
        "KeyV"   => ZxKey::V,
        "KeyW"   => ZxKey::W,
        "KeyX"   => ZxKey::X,
        "KeyY"   => ZxKey::Y,
        "KeyZ"   => ZxKey::Z,
        "Digit0" => ZxKey::K0,
        "Digit1" => ZxKey::K1,
        "Digit2" => ZxKey::K2,
        "Digit3" => ZxKey::K3,
        "Digit4" => ZxKey::K4,
        "Digit5" => ZxKey::K5,
        "Digit6" => ZxKey::K6,
        "Digit7" => ZxKey::K7,
        "Digit8" => ZxKey::K8,
        "Digit9" => ZxKey::K9,
        // Arrow keys handled separately in lib.rs (they need CAPS SHIFT chorded)
        _ => return None,
    })
}
