/// Zilog Z80 CPU — complete instruction set including undocumented opcodes
///
/// Registers:
///   Main:      A  F  B  C  D  E  H  L
///   Alternate: A' F' B' C' D' E' H' L'
///   Index:     IX  IY
///   Special:   I  R  SP  PC
///
/// Flags (F): S Z Y H X PV N C  (Y=bit5, X=bit3 are undocumented copy bits)
///
/// Clock: 3.5 MHz.  One frame = 69888 T-states at 50 Hz.

use crate::bus::Bus;

// ── Flag bit positions ───────────────────────────────────────────────────────
const SF: u8 = 0x80; // Sign
const ZF: u8 = 0x40; // Zero
const YF: u8 = 0x20; // undocumented (bit 5 of result)
const HF: u8 = 0x10; // Half carry
const XF: u8 = 0x08; // undocumented (bit 3 of result)
const PF: u8 = 0x04; // Parity / Overflow
const NF: u8 = 0x02; // Add/Subtract
const CF: u8 = 0x01; // Carry

pub struct Cpu {
    // Main registers
    pub a: u8, pub f: u8,
    pub b: u8, pub c: u8,
    pub d: u8, pub e: u8,
    pub h: u8, pub l: u8,
    // Alternate registers
    pub a2: u8, pub f2: u8,
    pub b2: u8, pub c2: u8,
    pub d2: u8, pub e2: u8,
    pub h2: u8, pub l2: u8,
    // Index registers
    pub ix: u16,
    pub iy: u16,
    // Special
    pub i: u8,
    pub r: u8,
    pub sp: u16,
    pub pc: u16,
    // Interrupt state
    pub iff1: bool,
    pub iff2: bool,
    pub im: u8,
    pub halted: bool,
    ei_pending: bool, // EI enables interrupts after the *next* instruction
    // T-cycle counter within the current frame (reset each frame)
    pub t: u32,
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            a: 0xFF, f: 0xFF,
            b: 0xFF, c: 0xFF,
            d: 0xFF, e: 0xFF,
            h: 0xFF, l: 0xFF,
            a2: 0xFF, f2: 0xFF,
            b2: 0xFF, c2: 0xFF,
            d2: 0xFF, e2: 0xFF,
            h2: 0xFF, l2: 0xFF,
            ix: 0xFFFF, iy: 0xFFFF,
            i: 0x3F, r: 0x00,
            sp: 0xFFFF, pc: 0x0000,
            iff1: false, iff2: false,
            im: 1,
            halted: false,
            ei_pending: false,
            t: 0,
        }
    }

    // ── 16-bit register pairs ─────────────────────────────────────────────────

    pub fn af(&self)  -> u16 { (self.a as u16) << 8 | self.f as u16 }
    pub fn bc(&self)  -> u16 { (self.b as u16) << 8 | self.c as u16 }
    pub fn de(&self)  -> u16 { (self.d as u16) << 8 | self.e as u16 }
    pub fn hl(&self)  -> u16 { (self.h as u16) << 8 | self.l as u16 }
    pub fn ixh(&self) -> u8  { (self.ix >> 8) as u8 }
    pub fn ixl(&self) -> u8  { self.ix as u8 }
    pub fn iyh(&self) -> u8  { (self.iy >> 8) as u8 }
    pub fn iyl(&self) -> u8  { self.iy as u8 }

    fn set_af(&mut self, v: u16)  { self.a = (v >> 8) as u8; self.f = v as u8; }
    fn set_bc(&mut self, v: u16)  { self.b = (v >> 8) as u8; self.c = v as u8; }
    fn set_de(&mut self, v: u16)  { self.d = (v >> 8) as u8; self.e = v as u8; }
    fn set_hl(&mut self, v: u16)  { self.h = (v >> 8) as u8; self.l = v as u8; }
    fn set_ixh(&mut self, v: u8)  { self.ix = (self.ix & 0x00FF) | ((v as u16) << 8); }
    fn set_ixl(&mut self, v: u8)  { self.ix = (self.ix & 0xFF00) | v as u16; }
    fn set_iyh(&mut self, v: u8)  { self.iy = (self.iy & 0x00FF) | ((v as u16) << 8); }
    fn set_iyl(&mut self, v: u8)  { self.iy = (self.iy & 0xFF00) | v as u16; }

    // ── Flag helpers ──────────────────────────────────────────────────────────

    fn sf(&self) -> bool { self.f & SF != 0 }
    fn zf(&self) -> bool { self.f & ZF != 0 }
    fn hf(&self) -> bool { self.f & HF != 0 }
    fn pf(&self) -> bool { self.f & PF != 0 }
    fn nf(&self) -> bool { self.f & NF != 0 }
    fn cf(&self) -> bool { self.f & CF != 0 }

    /// Evaluate condition code 0-7 (NZ Z NC C PO PE P M)
    fn cond(&self, cc: u8) -> bool {
        match cc & 7 {
            0 => !self.zf(),
            1 =>  self.zf(),
            2 => !self.cf(),
            3 =>  self.cf(),
            4 => !self.pf(),
            5 =>  self.pf(),
            6 => !self.sf(),
            7 =>  self.sf(),
            _ => unreachable!(),
        }
    }

    // ── Fetch helpers ─────────────────────────────────────────────────────────

    fn fetch(&mut self, bus: &Bus) -> u8 {
        let v = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        self.r = (self.r & 0x80) | ((self.r.wrapping_add(1)) & 0x7F);
        v
    }

    fn fetch16(&mut self, bus: &Bus) -> u16 {
        let lo = self.fetch(bus) as u16;
        let hi = self.fetch(bus) as u16;
        hi << 8 | lo
    }

    fn fetch_disp(&mut self, bus: &Bus) -> i8 {
        self.fetch(bus) as i8
    }

    // ── Stack ─────────────────────────────────────────────────────────────────

    fn push16(&mut self, val: u16, bus: &mut Bus) {
        self.sp = self.sp.wrapping_sub(1); bus.write(self.sp, (val >> 8) as u8);
        self.sp = self.sp.wrapping_sub(1); bus.write(self.sp, val as u8);
    }

    fn pop16(&mut self, bus: &Bus) -> u16 {
        let lo = bus.read(self.sp) as u16; self.sp = self.sp.wrapping_add(1);
        let hi = bus.read(self.sp) as u16; self.sp = self.sp.wrapping_add(1);
        hi << 8 | lo
    }

    // ── ALU ───────────────────────────────────────────────────────────────────

    fn add8(&mut self, val: u8, carry: u8) {
        let a = self.a;
        let res16 = a as u16 + val as u16 + carry as u16;
        let res = res16 as u8;
        let half = (a & 0xF) + (val & 0xF) + carry > 0xF;
        let overflow = (a ^ val) & 0x80 == 0 && (a ^ res) & 0x80 != 0;
        self.f = flags_szxy(res)
            | if half     { HF } else { 0 }
            | if overflow { PF } else { 0 }
            | if res16 > 0xFF { CF } else { 0 };
        self.a = res;
    }

    fn sub8(&mut self, val: u8, borrow: u8) {
        let a = self.a;
        let res = a.wrapping_sub(val).wrapping_sub(borrow);
        let half = (a & 0xF) < (val & 0xF) + borrow;
        let overflow = (a ^ val) & 0x80 != 0 && (a ^ res) & 0x80 != 0;
        let carry = (a as u16) < val as u16 + borrow as u16;
        self.f = flags_szxy(res) | NF
            | if half     { HF } else { 0 }
            | if overflow { PF } else { 0 }
            | if carry    { CF } else { 0 };
        self.a = res;
    }

    fn and8(&mut self, val: u8) {
        self.a &= val;
        self.f = flags_szxy(self.a) | HF | parity_flag(self.a);
    }

    fn or8(&mut self, val: u8) {
        self.a |= val;
        self.f = flags_szxy(self.a) | parity_flag(self.a);
    }

    fn xor8(&mut self, val: u8) {
        self.a ^= val;
        self.f = flags_szxy(self.a) | parity_flag(self.a);
    }

    fn cp8(&mut self, val: u8) {
        let a = self.a;
        let res = a.wrapping_sub(val);
        let half = (a & 0xF) < (val & 0xF);
        let overflow = (a ^ val) & 0x80 != 0 && (a ^ res) & 0x80 != 0;
        let carry = a < val;
        // CP is like SUB but result not stored; XF/YF come from the *operand* not result
        self.f = (flags_szxy(res) & !(YF | XF)) | (val & (YF | XF)) | NF
            | if half     { HF } else { 0 }
            | if overflow { PF } else { 0 }
            | if carry    { CF } else { 0 };
    }

    fn inc8(&mut self, val: u8) -> u8 {
        let res = val.wrapping_add(1);
        let overflow = val == 0x7F;
        let old_c = self.f & CF;
        self.f = flags_szxy(res)
            | if (val & 0xF) == 0xF { HF } else { 0 }
            | if overflow { PF } else { 0 }
            | old_c; // carry unaffected
        res
    }

    fn dec8(&mut self, val: u8) -> u8 {
        let res = val.wrapping_sub(1);
        let overflow = val == 0x80;
        let old_c = self.f & CF;
        self.f = flags_szxy(res) | NF
            | if (val & 0xF) == 0x00 { HF } else { 0 }
            | if overflow { PF } else { 0 }
            | old_c;
        res
    }

    fn add_hl16(&mut self, rr: u16) {
        let hl = self.hl();
        let res = hl.wrapping_add(rr);
        let half = (hl & 0xFFF) + (rr & 0xFFF) > 0xFFF;
        let carry = hl as u32 + rr as u32 > 0xFFFF;
        // Only N, H, C, and the undocumented XY bits from result high byte change
        self.f = (self.f & (SF | ZF | PF))
            | ((res >> 8) as u8 & (YF | XF))
            | if half  { HF } else { 0 }
            | if carry { CF } else { 0 };
        self.set_hl(res);
    }

    fn add_ix16(&mut self, rr: u16) {
        let ix = self.ix;
        let res = ix.wrapping_add(rr);
        let half = (ix & 0xFFF) + (rr & 0xFFF) > 0xFFF;
        let carry = ix as u32 + rr as u32 > 0xFFFF;
        self.f = (self.f & (SF | ZF | PF))
            | ((res >> 8) as u8 & (YF | XF))
            | if half  { HF } else { 0 }
            | if carry { CF } else { 0 };
        self.ix = res;
    }

    fn add_iy16(&mut self, rr: u16) {
        let iy = self.iy;
        let res = iy.wrapping_add(rr);
        let half = (iy & 0xFFF) + (rr & 0xFFF) > 0xFFF;
        let carry = iy as u32 + rr as u32 > 0xFFFF;
        self.f = (self.f & (SF | ZF | PF))
            | ((res >> 8) as u8 & (YF | XF))
            | if half  { HF } else { 0 }
            | if carry { CF } else { 0 };
        self.iy = res;
    }

    fn sbc_hl16(&mut self, rr: u16) {
        let hl = self.hl();
        let c = self.cf() as u16;
        let res = hl.wrapping_sub(rr).wrapping_sub(c);
        let half = (hl & 0xFFF) < (rr & 0xFFF) + c;
        let overflow = (hl ^ rr) & 0x8000 != 0 && (hl ^ res) & 0x8000 != 0;
        let carry = (hl as u32) < rr as u32 + c as u32;
        self.f = (if res & 0x8000 != 0 { SF } else { 0 })
            | (if res == 0 { ZF } else { 0 })
            | ((res >> 8) as u8 & (YF | XF))
            | if half     { HF } else { 0 }
            | if overflow { PF } else { 0 }
            | NF
            | if carry    { CF } else { 0 };
        self.set_hl(res);
    }

    fn adc_hl16(&mut self, rr: u16) {
        let hl = self.hl();
        let c = self.cf() as u16;
        let res = hl.wrapping_add(rr).wrapping_add(c);
        let half = (hl & 0xFFF) + (rr & 0xFFF) + c > 0xFFF;
        let overflow = (hl ^ rr) & 0x8000 == 0 && (hl ^ res) & 0x8000 != 0;
        let carry = hl as u32 + rr as u32 + c as u32 > 0xFFFF;
        self.f = (if res & 0x8000 != 0 { SF } else { 0 })
            | (if res == 0 { ZF } else { 0 })
            | ((res >> 8) as u8 & (YF | XF))
            | if half     { HF } else { 0 }
            | if overflow { PF } else { 0 }
            | if carry    { CF } else { 0 };
        self.set_hl(res);
    }

    // ── Rotates & shifts ──────────────────────────────────────────────────────

    fn rlc(&mut self, v: u8) -> u8 {
        let c = v >> 7; let r = (v << 1) | c;
        self.f = flags_szxy(r) | parity_flag(r) | c; r
    }
    fn rrc(&mut self, v: u8) -> u8 {
        let c = v & 1; let r = (v >> 1) | (c << 7);
        self.f = flags_szxy(r) | parity_flag(r) | c; r
    }
    fn rl(&mut self, v: u8) -> u8 {
        let old_c = self.cf() as u8; let new_c = v >> 7;
        let r = (v << 1) | old_c;
        self.f = flags_szxy(r) | parity_flag(r) | new_c; r
    }
    fn rr(&mut self, v: u8) -> u8 {
        let old_c = self.cf() as u8; let new_c = v & 1;
        let r = (v >> 1) | (old_c << 7);
        self.f = flags_szxy(r) | parity_flag(r) | new_c; r
    }
    fn sla(&mut self, v: u8) -> u8 {
        let c = v >> 7; let r = v << 1;
        self.f = flags_szxy(r) | parity_flag(r) | c; r
    }
    fn sra(&mut self, v: u8) -> u8 {
        let c = v & 1; let r = (v >> 1) | (v & 0x80);
        self.f = flags_szxy(r) | parity_flag(r) | c; r
    }
    fn sll(&mut self, v: u8) -> u8 { // undocumented
        let c = v >> 7; let r = (v << 1) | 1;
        self.f = flags_szxy(r) | parity_flag(r) | c; r
    }
    fn srl(&mut self, v: u8) -> u8 {
        let c = v & 1; let r = v >> 1;
        self.f = flags_szxy(r) | parity_flag(r) | c; r
    }

    fn rlca(&mut self) {
        let c = self.a >> 7; self.a = (self.a << 1) | c;
        self.f = (self.f & (SF | ZF | PF)) | (self.a & (YF | XF)) | c;
    }
    fn rrca(&mut self) {
        let c = self.a & 1; self.a = (self.a >> 1) | (c << 7);
        self.f = (self.f & (SF | ZF | PF)) | (self.a & (YF | XF)) | c;
    }
    fn rla(&mut self) {
        let old_c = self.cf() as u8; let new_c = self.a >> 7;
        self.a = (self.a << 1) | old_c;
        self.f = (self.f & (SF | ZF | PF)) | (self.a & (YF | XF)) | new_c;
    }
    fn rra(&mut self) {
        let old_c = self.cf() as u8; let new_c = self.a & 1;
        self.a = (self.a >> 1) | (old_c << 7);
        self.f = (self.f & (SF | ZF | PF)) | (self.a & (YF | XF)) | new_c;
    }

    fn daa(&mut self) {
        let mut a = self.a;
        let c = self.cf();
        let h = self.hf();
        let n = self.nf();
        let mut new_c = c;
        if !n {
            if c || a > 0x99 { a = a.wrapping_add(0x60); new_c = true; }
            if h || (a & 0x0F) > 0x09 { a = a.wrapping_add(0x06); }
        } else {
            if c { a = a.wrapping_sub(0x60); new_c = true; }
            if h { a = a.wrapping_sub(0x06); }
        }
        self.f = flags_szxy(a) | parity_flag(a)
            | (self.f & NF)
            | if new_c { CF } else { 0 }
            | if (self.a ^ a) & 0x10 != 0 { HF } else { 0 };
        self.a = a;
    }

    fn bit_test(&mut self, bit: u8, val: u8) {
        let test = val & (1 << bit);
        self.f = (self.f & CF)
            | HF
            | if test == 0 { ZF | PF } else { 0 }
            | if bit == 7 && test != 0 { SF } else { 0 }
            | (val & (YF | XF));
    }

    // ── Register array access (B C D E H L (HL) A = index 0-7) ───────────────
    // Used for generic CB and plain unprefixed ops.

    fn r8(&self, idx: u8, bus: &Bus) -> u8 {
        match idx {
            0 => self.b, 1 => self.c, 2 => self.d, 3 => self.e,
            4 => self.h, 5 => self.l,
            6 => bus.read(self.hl()),
            7 => self.a,
            _ => unreachable!(),
        }
    }

    fn set_r8(&mut self, idx: u8, val: u8, bus: &mut Bus) {
        match idx {
            0 => self.b = val, 1 => self.c = val, 2 => self.d = val, 3 => self.e = val,
            4 => self.h = val, 5 => self.l = val,
            6 => bus.write(self.hl(), val),
            7 => self.a = val,
            _ => unreachable!(),
        }
    }

    // Like r8 but H/L are replaced by IXH/IXL (for DD prefix)
    #[allow(dead_code)]
    fn r8_ix(&self, idx: u8, bus: &Bus) -> u8 {
        match idx {
            4 => self.ixh(),
            5 => self.ixl(),
            6 => bus.read(self.ix), // caller provides real (IX+d) address
            _ => self.r8(idx, bus),
        }
    }

    #[allow(dead_code)]
    fn set_r8_ix(&mut self, idx: u8, val: u8) {
        match idx {
            4 => self.set_ixh(val),
            5 => self.set_ixl(val),
            _ => match idx {
                0 => self.b = val, 1 => self.c = val, 2 => self.d = val,
                3 => self.e = val, 7 => self.a = val,
                _ => {}
            }
        }
    }

    #[allow(dead_code)]
    fn r8_iy(&self, idx: u8, bus: &Bus) -> u8 {
        match idx {
            4 => self.iyh(),
            5 => self.iyl(),
            6 => bus.read(self.iy),
            _ => self.r8(idx, bus),
        }
    }

    #[allow(dead_code)]
    fn set_r8_iy(&mut self, idx: u8, val: u8) {
        match idx {
            4 => self.set_iyh(val),
            5 => self.set_iyl(val),
            _ => match idx {
                0 => self.b = val, 1 => self.c = val, 2 => self.d = val,
                3 => self.e = val, 7 => self.a = val,
                _ => {}
            }
        }
    }

    // ── Interrupt handling ────────────────────────────────────────────────────

    /// Service a maskable interrupt (INT). Returns T-cycles used.
    pub fn interrupt(&mut self, bus: &mut Bus) -> u32 {
        if !self.iff1 { return 0; }
        if self.halted { self.halted = false; self.pc = self.pc.wrapping_add(1); }
        self.iff1 = false;
        self.iff2 = false;
        self.r = (self.r & 0x80) | ((self.r.wrapping_add(1)) & 0x7F);
        match self.im {
            0 | 1 => {
                // IM 0 / IM 1 — execute RST 38h
                self.push16(self.pc, bus);
                self.pc = 0x0038;
                13
            }
            2 => {
                // IM 2 — indirect jump via I register table
                let vec_addr = (self.i as u16) << 8 | 0xFF; // data bus = 0xFF when no device
                let target = bus.read16(vec_addr);
                self.push16(self.pc, bus);
                self.pc = target;
                19
            }
            _ => 0,
        }
    }

    // ── Main execute ──────────────────────────────────────────────────────────

    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        if self.ei_pending {
            self.ei_pending = false;
            self.iff1 = true;
            self.iff2 = true;
        }
        if self.halted { return 4; }
        let op = self.fetch(bus);
        let cycles = self.execute(op, bus);
        self.t = self.t.wrapping_add(cycles);
        cycles
    }

    fn execute(&mut self, op: u8, bus: &mut Bus) -> u32 {
        match op {
            0x00 => 4,  // NOP
            0x08 => { let af = self.af(); let af2 = (self.a2 as u16)<<8 | self.f2 as u16; self.set_af(af2); self.a2=(af>>8) as u8; self.f2=af as u8; 4 } // EX AF,AF'
            0x10 => { // DJNZ e
                let e = self.fetch_disp(bus);
                self.b = self.b.wrapping_sub(1);
                if self.b != 0 { self.pc = self.pc.wrapping_add(e as u16); 13 } else { 8 }
            }
            0x18 => { let e = self.fetch_disp(bus); self.pc = self.pc.wrapping_add(e as u16); 12 } // JR e
            0x20 => { let e = self.fetch_disp(bus); if !self.zf() { self.pc = self.pc.wrapping_add(e as u16); 12 } else { 7 } }
            0x28 => { let e = self.fetch_disp(bus); if  self.zf() { self.pc = self.pc.wrapping_add(e as u16); 12 } else { 7 } }
            0x30 => { let e = self.fetch_disp(bus); if !self.cf() { self.pc = self.pc.wrapping_add(e as u16); 12 } else { 7 } }
            0x38 => { let e = self.fetch_disp(bus); if  self.cf() { self.pc = self.pc.wrapping_add(e as u16); 12 } else { 7 } }

            // 16-bit loads
            0x01 => { let v = self.fetch16(bus); self.set_bc(v); 10 }
            0x11 => { let v = self.fetch16(bus); self.set_de(v); 10 }
            0x21 => { let v = self.fetch16(bus); self.set_hl(v); 10 }
            0x31 => { self.sp = self.fetch16(bus); 10 }
            0x22 => { let a = self.fetch16(bus); bus.write16(a, self.hl()); 16 }
            0x2A => { let a = self.fetch16(bus); let v = bus.read16(a); self.set_hl(v); 16 }
            0x32 => { let a = self.fetch16(bus); bus.write(a, self.a); 13 }
            0x3A => { let a = self.fetch16(bus); self.a = bus.read(a); 13 }
            0xF9 => { self.sp = self.hl(); 6 }

            // INC/DEC 16-bit
            0x03 => { let v = self.bc().wrapping_add(1); self.set_bc(v); 6 }
            0x13 => { let v = self.de().wrapping_add(1); self.set_de(v); 6 }
            0x23 => { let v = self.hl().wrapping_add(1); self.set_hl(v); 6 }
            0x33 => { self.sp = self.sp.wrapping_add(1); 6 }
            0x0B => { let v = self.bc().wrapping_sub(1); self.set_bc(v); 6 }
            0x1B => { let v = self.de().wrapping_sub(1); self.set_de(v); 6 }
            0x2B => { let v = self.hl().wrapping_sub(1); self.set_hl(v); 6 }
            0x3B => { self.sp = self.sp.wrapping_sub(1); 6 }

            // ADD HL,rr
            0x09 => { let v = self.bc(); self.add_hl16(v); 11 }
            0x19 => { let v = self.de(); self.add_hl16(v); 11 }
            0x29 => { let v = self.hl(); self.add_hl16(v); 11 }
            0x39 => { let v = self.sp;   self.add_hl16(v); 11 }

            // 8-bit register loads (indirect)
            0x02 => { bus.write(self.bc(), self.a); 7 }
            0x12 => { bus.write(self.de(), self.a); 7 }
            0x0A => { self.a = bus.read(self.bc()); 7 }
            0x1A => { self.a = bus.read(self.de()); 7 }

            // INC/DEC 8-bit
            0x04 => { self.b = self.inc8(self.b); 4 }
            0x0C => { self.c = self.inc8(self.c); 4 }
            0x14 => { self.d = self.inc8(self.d); 4 }
            0x1C => { self.e = self.inc8(self.e); 4 }
            0x24 => { self.h = self.inc8(self.h); 4 }
            0x2C => { self.l = self.inc8(self.l); 4 }
            0x34 => { let hl = self.hl(); let v = bus.read(hl); let r = self.inc8(v); bus.write(hl, r); 11 }
            0x3C => { self.a = self.inc8(self.a); 4 }
            0x05 => { self.b = self.dec8(self.b); 4 }
            0x0D => { self.c = self.dec8(self.c); 4 }
            0x15 => { self.d = self.dec8(self.d); 4 }
            0x1D => { self.e = self.dec8(self.e); 4 }
            0x25 => { self.h = self.dec8(self.h); 4 }
            0x2D => { self.l = self.dec8(self.l); 4 }
            0x35 => { let hl = self.hl(); let v = bus.read(hl); let r = self.dec8(v); bus.write(hl, r); 11 }
            0x3D => { self.a = self.dec8(self.a); 4 }

            // LD r,n
            0x06 => { self.b = self.fetch(bus); 7 }
            0x0E => { self.c = self.fetch(bus); 7 }
            0x16 => { self.d = self.fetch(bus); 7 }
            0x1E => { self.e = self.fetch(bus); 7 }
            0x26 => { self.h = self.fetch(bus); 7 }
            0x2E => { self.l = self.fetch(bus); 7 }
            0x36 => { let n = self.fetch(bus); bus.write(self.hl(), n); 10 }
            0x3E => { self.a = self.fetch(bus); 7 }

            // Rotates (A only, don't affect S/Z/P)
            0x07 => { self.rlca(); 4 }
            0x0F => { self.rrca(); 4 }
            0x17 => { self.rla(); 4 }
            0x1F => { self.rra(); 4 }

            // Misc
            0x27 => { self.daa(); 4 }
            0x2F => { self.a = !self.a; self.f |= HF | NF; 4 } // CPL
            0x37 => { self.f = (self.f & (SF | ZF | PF)) | (self.a & (YF | XF)) | CF; 4 } // SCF
            0x3F => { let c = self.cf(); self.f = (self.f & (SF | ZF | PF)) | (self.a & (YF | XF)) | if c { HF } else { 0 } | if !c { CF } else { 0 }; 4 } // CCF

            // HALT
            0x76 => { self.halted = true; 4 }

            // LD r,r' block (0x40–0x7F)
            0x40..=0x7F => {
                let dst = (op >> 3) & 7;
                let src = op & 7;
                let val = self.r8(src, bus);
                self.set_r8(dst, val, bus);
                if src == 6 || dst == 6 { 7 } else { 4 }
            }

            // ALU A,r  (0x80–0xBF)
            0x80..=0xBF => {
                let src = op & 7;
                let val = self.r8(src, bus);
                self.alu_op((op >> 3) & 7, val);
                if src == 6 { 7 } else { 4 }
            }

            // ALU A,n  (0xC6, 0xCE, 0xD6, 0xDE, 0xE6, 0xEE, 0xF6, 0xFE)
            0xC6 => { let n = self.fetch(bus); self.alu_op(0, n); 7 }
            0xCE => { let n = self.fetch(bus); self.alu_op(1, n); 7 }
            0xD6 => { let n = self.fetch(bus); self.alu_op(2, n); 7 }
            0xDE => { let n = self.fetch(bus); self.alu_op(3, n); 7 }
            0xE6 => { let n = self.fetch(bus); self.alu_op(4, n); 7 }
            0xEE => { let n = self.fetch(bus); self.alu_op(5, n); 7 }
            0xF6 => { let n = self.fetch(bus); self.alu_op(6, n); 7 }
            0xFE => { let n = self.fetch(bus); self.alu_op(7, n); 7 }

            // RET cc
            0xC0 => { if self.cond(0) { self.pc = self.pop16(bus); 11 } else { 5 } }
            0xC8 => { if self.cond(1) { self.pc = self.pop16(bus); 11 } else { 5 } }
            0xD0 => { if self.cond(2) { self.pc = self.pop16(bus); 11 } else { 5 } }
            0xD8 => { if self.cond(3) { self.pc = self.pop16(bus); 11 } else { 5 } }
            0xE0 => { if self.cond(4) { self.pc = self.pop16(bus); 11 } else { 5 } }
            0xE8 => { if self.cond(5) { self.pc = self.pop16(bus); 11 } else { 5 } }
            0xF0 => { if self.cond(6) { self.pc = self.pop16(bus); 11 } else { 5 } }
            0xF8 => { if self.cond(7) { self.pc = self.pop16(bus); 11 } else { 5 } }

            // RET / RETI / RETN
            0xC9 => { self.pc = self.pop16(bus); 10 }

            // JP cc,nn
            0xC2 => { let a = self.fetch16(bus); if self.cond(0) { self.pc = a; } 10 }
            0xCA => { let a = self.fetch16(bus); if self.cond(1) { self.pc = a; } 10 }
            0xD2 => { let a = self.fetch16(bus); if self.cond(2) { self.pc = a; } 10 }
            0xDA => { let a = self.fetch16(bus); if self.cond(3) { self.pc = a; } 10 }
            0xE2 => { let a = self.fetch16(bus); if self.cond(4) { self.pc = a; } 10 }
            0xEA => { let a = self.fetch16(bus); if self.cond(5) { self.pc = a; } 10 }
            0xF2 => { let a = self.fetch16(bus); if self.cond(6) { self.pc = a; } 10 }
            0xFA => { let a = self.fetch16(bus); if self.cond(7) { self.pc = a; } 10 }

            // JP nn / JP (HL)
            0xC3 => { self.pc = self.fetch16(bus); 10 }
            0xE9 => { self.pc = self.hl(); 4 }

            // CALL cc,nn
            0xC4 => { let a = self.fetch16(bus); if self.cond(0) { self.push16(self.pc, bus); self.pc = a; 17 } else { 10 } }
            0xCC => { let a = self.fetch16(bus); if self.cond(1) { self.push16(self.pc, bus); self.pc = a; 17 } else { 10 } }
            0xD4 => { let a = self.fetch16(bus); if self.cond(2) { self.push16(self.pc, bus); self.pc = a; 17 } else { 10 } }
            0xDC => { let a = self.fetch16(bus); if self.cond(3) { self.push16(self.pc, bus); self.pc = a; 17 } else { 10 } }
            0xE4 => { let a = self.fetch16(bus); if self.cond(4) { self.push16(self.pc, bus); self.pc = a; 17 } else { 10 } }
            0xEC => { let a = self.fetch16(bus); if self.cond(5) { self.push16(self.pc, bus); self.pc = a; 17 } else { 10 } }
            0xF4 => { let a = self.fetch16(bus); if self.cond(6) { self.push16(self.pc, bus); self.pc = a; 17 } else { 10 } }
            0xFC => { let a = self.fetch16(bus); if self.cond(7) { self.push16(self.pc, bus); self.pc = a; 17 } else { 10 } }

            // CALL nn
            0xCD => { let a = self.fetch16(bus); self.push16(self.pc, bus); self.pc = a; 17 }

            // RST
            0xC7 => { self.push16(self.pc, bus); self.pc = 0x00; 11 }
            0xCF => { self.push16(self.pc, bus); self.pc = 0x08; 11 }
            0xD7 => { self.push16(self.pc, bus); self.pc = 0x10; 11 }
            0xDF => { self.push16(self.pc, bus); self.pc = 0x18; 11 }
            0xE7 => { self.push16(self.pc, bus); self.pc = 0x20; 11 }
            0xEF => { self.push16(self.pc, bus); self.pc = 0x28; 11 }
            0xF7 => { self.push16(self.pc, bus); self.pc = 0x30; 11 }
            0xFF => { self.push16(self.pc, bus); self.pc = 0x38; 11 }

            // PUSH / POP
            0xC1 => { let v = self.pop16(bus); self.set_bc(v); 10 }
            0xD1 => { let v = self.pop16(bus); self.set_de(v); 10 }
            0xE1 => { let v = self.pop16(bus); self.set_hl(v); 10 }
            0xF1 => { let v = self.pop16(bus); self.set_af(v); 10 }
            0xC5 => { let v = self.bc(); self.push16(v, bus); 11 }
            0xD5 => { let v = self.de(); self.push16(v, bus); 11 }
            0xE5 => { let v = self.hl(); self.push16(v, bus); 11 }
            0xF5 => { let v = self.af(); self.push16(v, bus); 11 }

            // Exchange
            0xD9 => { // EXX
                let (b,c,d,e,h,l) = (self.b,self.c,self.d,self.e,self.h,self.l);
                self.b=self.b2; self.c=self.c2; self.d=self.d2;
                self.e=self.e2; self.h=self.h2; self.l=self.l2;
                self.b2=b; self.c2=c; self.d2=d; self.e2=e; self.h2=h; self.l2=l;
                4
            }
            0xEB => { // EX DE,HL
                let de = self.de(); let hl = self.hl();
                self.set_de(hl); self.set_hl(de); 4
            }
            0xE3 => { // EX (SP),HL
                let sp = self.sp;
                let mem = bus.read16(sp);
                bus.write16(sp, self.hl());
                self.set_hl(mem);
                19
            }

            // IN / OUT
            0xDB => { let n = self.fetch(bus); let port = (self.a as u16) << 8 | n as u16; self.a = bus.port_in(port); 11 }
            0xD3 => { let n = self.fetch(bus); let port = (self.a as u16) << 8 | n as u16; bus.port_out(port, self.a, self.t); 11 }

            // Interrupt control
            0xF3 => { self.iff1 = false; self.iff2 = false; 4 }  // DI
            0xFB => { self.ei_pending = true; 4 }                  // EI

            // Prefixes
            0xCB => self.execute_cb(bus),
            0xDD => self.execute_dd(bus),
            0xED => self.execute_ed(bus),
            0xFD => self.execute_fd(bus),
        }
    }

    fn alu_op(&mut self, op: u8, val: u8) {
        match op {
            0 => self.add8(val, 0),
            1 => { let c = self.cf() as u8; self.add8(val, c) }
            2 => self.sub8(val, 0),
            3 => { let c = self.cf() as u8; self.sub8(val, c) }
            4 => self.and8(val),
            5 => self.xor8(val),
            6 => self.or8(val),
            7 => self.cp8(val),
            _ => unreachable!(),
        }
    }

    // ── CB prefix ─────────────────────────────────────────────────────────────

    fn execute_cb(&mut self, bus: &mut Bus) -> u32 {
        let op = self.fetch(bus);
        let reg = op & 7;
        let val = self.r8(reg, bus);
        let row = op >> 3;

        let res = match row {
            0x00 => self.rlc(val),
            0x01 => self.rrc(val),
            0x02 => self.rl(val),
            0x03 => self.rr(val),
            0x04 => self.sla(val),
            0x05 => self.sra(val),
            0x06 => self.sll(val), // undocumented
            0x07 => self.srl(val),
            // BIT b,r (rows 8–15)
            r @ 0x08..=0x0F => {
                self.bit_test(r - 8, val);
                return if reg == 6 { 12 } else { 8 };
            }
            // RES b,r (rows 16–23)
            r @ 0x10..=0x17 => val & !(1 << (r - 0x10)),
            // SET b,r (rows 24–31)
            r @ 0x18..=0x1F => val | (1 << (r - 0x18)),
            _ => unreachable!(),
        };

        self.set_r8(reg, res, bus);
        if reg == 6 { 15 } else { 8 }
    }

    // ── DD prefix (IX instructions) ───────────────────────────────────────────

    fn execute_dd(&mut self, bus: &mut Bus) -> u32 {
        let op = self.fetch(bus);
        self.r = (self.r & 0x80) | ((self.r.wrapping_add(1)) & 0x7F);
        match op {
            0x09 => { let v = self.bc(); self.add_ix16(v); 15 }
            0x19 => { let v = self.de(); self.add_ix16(v); 15 }
            0x21 => { self.ix = self.fetch16(bus); 14 }
            0x22 => { let a = self.fetch16(bus); bus.write16(a, self.ix); 20 }
            0x23 => { self.ix = self.ix.wrapping_add(1); 10 }
            0x24 => { let v = self.ixh(); self.ix = (self.inc8(v) as u16) << 8 | self.ixl() as u16; 8 }
            0x25 => { let v = self.ixh(); self.ix = (self.dec8(v) as u16) << 8 | self.ixl() as u16; 8 }
            0x26 => { let n = self.fetch(bus); self.set_ixh(n); 11 }
            0x29 => { let v = self.ix;  self.add_ix16(v); 15 }
            0x2A => { let a = self.fetch16(bus); self.ix = bus.read16(a); 20 }
            0x2B => { self.ix = self.ix.wrapping_sub(1); 10 }
            0x2C => { let v = self.ixl(); self.ix = self.ixh() as u16 * 256 | self.inc8(v) as u16; 8 }
            0x2D => { let v = self.ixl(); self.ix = self.ixh() as u16 * 256 | self.dec8(v) as u16; 8 }
            0x2E => { let n = self.fetch(bus); self.set_ixl(n); 11 }
            0x34 => { let d = self.fetch_disp(bus); let a = self.ix.wrapping_add(d as u16); let v = bus.read(a); let r = self.inc8(v); bus.write(a, r); 23 }
            0x35 => { let d = self.fetch_disp(bus); let a = self.ix.wrapping_add(d as u16); let v = bus.read(a); let r = self.dec8(v); bus.write(a, r); 23 }
            0x36 => { let d = self.fetch_disp(bus); let n = self.fetch(bus); bus.write(self.ix.wrapping_add(d as u16), n); 19 }
            0x39 => { let v = self.sp; self.add_ix16(v); 15 }
            0xE1 => { self.ix = self.pop16(bus); 14 }
            0xE3 => { let sp = self.sp; let m = bus.read16(sp); bus.write16(sp, self.ix); self.ix = m; 23 }
            0xE5 => { let v = self.ix; self.push16(v, bus); 15 }
            0xE9 => { self.pc = self.ix; 8 }
            0xF9 => { self.sp = self.ix; 10 }
            0xCB => self.execute_ddcb(bus),
            // LD r,(IX+d) and LD (IX+d),r
            0x46 => { let d = self.fetch_disp(bus); self.b = bus.read(self.ix.wrapping_add(d as u16)); 19 }
            0x4E => { let d = self.fetch_disp(bus); self.c = bus.read(self.ix.wrapping_add(d as u16)); 19 }
            0x56 => { let d = self.fetch_disp(bus); self.d = bus.read(self.ix.wrapping_add(d as u16)); 19 }
            0x5E => { let d = self.fetch_disp(bus); self.e = bus.read(self.ix.wrapping_add(d as u16)); 19 }
            0x66 => { let d = self.fetch_disp(bus); self.h = bus.read(self.ix.wrapping_add(d as u16)); 19 }
            0x6E => { let d = self.fetch_disp(bus); self.l = bus.read(self.ix.wrapping_add(d as u16)); 19 }
            0x7E => { let d = self.fetch_disp(bus); self.a = bus.read(self.ix.wrapping_add(d as u16)); 19 }
            0x70 => { let d = self.fetch_disp(bus); bus.write(self.ix.wrapping_add(d as u16), self.b); 19 }
            0x71 => { let d = self.fetch_disp(bus); bus.write(self.ix.wrapping_add(d as u16), self.c); 19 }
            0x72 => { let d = self.fetch_disp(bus); bus.write(self.ix.wrapping_add(d as u16), self.d); 19 }
            0x73 => { let d = self.fetch_disp(bus); bus.write(self.ix.wrapping_add(d as u16), self.e); 19 }
            0x74 => { let d = self.fetch_disp(bus); bus.write(self.ix.wrapping_add(d as u16), self.h); 19 }
            0x75 => { let d = self.fetch_disp(bus); bus.write(self.ix.wrapping_add(d as u16), self.l); 19 }
            0x77 => { let d = self.fetch_disp(bus); bus.write(self.ix.wrapping_add(d as u16), self.a); 19 }
            // Undocumented: LD r,IXH / LD r,IXL and LD IXH/IXL,r
            0x44 => { self.b = self.ixh(); 8 } 0x45 => { self.b = self.ixl(); 8 }
            0x4C => { self.c = self.ixh(); 8 } 0x4D => { self.c = self.ixl(); 8 }
            0x54 => { self.d = self.ixh(); 8 } 0x55 => { self.d = self.ixl(); 8 }
            0x5C => { self.e = self.ixh(); 8 } 0x5D => { self.e = self.ixl(); 8 }
            0x60 => { let v=self.b; self.set_ixh(v); 8 } 0x61 => { let v=self.c; self.set_ixh(v); 8 }
            0x62 => { let v=self.d; self.set_ixh(v); 8 } 0x63 => { let v=self.e; self.set_ixh(v); 8 }
            0x64 => { 8 } // LD IXH,IXH (NOP)
            0x65 => { let v=self.ixl(); self.set_ixh(v); 8 }
            0x67 => { let v=self.a; self.set_ixh(v); 8 }
            0x68 => { let v=self.b; self.set_ixl(v); 8 } 0x69 => { let v=self.c; self.set_ixl(v); 8 }
            0x6A => { let v=self.d; self.set_ixl(v); 8 } 0x6B => { let v=self.e; self.set_ixl(v); 8 }
            0x6C => { let v=self.ixh(); self.set_ixl(v); 8 }
            0x6D => { 8 } // LD IXL,IXL (NOP)
            0x6F => { let v=self.a; self.set_ixl(v); 8 }
            0x7C => { self.a = self.ixh(); 8 } 0x7D => { self.a = self.ixl(); 8 }
            // ALU with IXH/IXL/mem
            0x84 => { let v=self.ixh(); self.add8(v,0); 8 } 0x85 => { let v=self.ixl(); self.add8(v,0); 8 }
            0x86 => { let d=self.fetch_disp(bus); let v=bus.read(self.ix.wrapping_add(d as u16)); self.add8(v,0); 19 }
            0x8C => { let v=self.ixh(); let c=self.cf() as u8; self.add8(v,c); 8 }
            0x8D => { let v=self.ixl(); let c=self.cf() as u8; self.add8(v,c); 8 }
            0x8E => { let d=self.fetch_disp(bus); let v=bus.read(self.ix.wrapping_add(d as u16)); let c=self.cf() as u8; self.add8(v,c); 19 }
            0x94 => { let v=self.ixh(); self.sub8(v,0); 8 } 0x95 => { let v=self.ixl(); self.sub8(v,0); 8 }
            0x96 => { let d=self.fetch_disp(bus); let v=bus.read(self.ix.wrapping_add(d as u16)); self.sub8(v,0); 19 }
            0x9C => { let v=self.ixh(); let c=self.cf() as u8; self.sub8(v,c); 8 }
            0x9D => { let v=self.ixl(); let c=self.cf() as u8; self.sub8(v,c); 8 }
            0x9E => { let d=self.fetch_disp(bus); let v=bus.read(self.ix.wrapping_add(d as u16)); let c=self.cf() as u8; self.sub8(v,c); 19 }
            0xA4 => { let v=self.ixh(); self.and8(v); 8 } 0xA5 => { let v=self.ixl(); self.and8(v); 8 }
            0xA6 => { let d=self.fetch_disp(bus); let v=bus.read(self.ix.wrapping_add(d as u16)); self.and8(v); 19 }
            0xAC => { let v=self.ixh(); self.xor8(v); 8 } 0xAD => { let v=self.ixl(); self.xor8(v); 8 }
            0xAE => { let d=self.fetch_disp(bus); let v=bus.read(self.ix.wrapping_add(d as u16)); self.xor8(v); 19 }
            0xB4 => { let v=self.ixh(); self.or8(v); 8 } 0xB5 => { let v=self.ixl(); self.or8(v); 8 }
            0xB6 => { let d=self.fetch_disp(bus); let v=bus.read(self.ix.wrapping_add(d as u16)); self.or8(v); 19 }
            0xBC => { let v=self.ixh(); self.cp8(v); 8 } 0xBD => { let v=self.ixl(); self.cp8(v); 8 }
            0xBE => { let d=self.fetch_disp(bus); let v=bus.read(self.ix.wrapping_add(d as u16)); self.cp8(v); 19 }
            _ => 8, // Treat remaining DD xx as NOP-like
        }
    }

    fn execute_ddcb(&mut self, bus: &mut Bus) -> u32 {
        let d = self.fetch_disp(bus);
        let op = self.fetch(bus);
        let addr = self.ix.wrapping_add(d as u16);
        let val = bus.read(addr);
        let reg = op & 7;
        let row = op >> 3;

        let res = match row {
            0x00 => self.rlc(val),
            0x01 => self.rrc(val),
            0x02 => self.rl(val),
            0x03 => self.rr(val),
            0x04 => self.sla(val),
            0x05 => self.sra(val),
            0x06 => self.sll(val),
            0x07 => self.srl(val),
            r @ 0x08..=0x0F => { self.bit_test(r - 8, val); return 20; }
            r @ 0x10..=0x17 => val & !(1 << (r - 0x10)),
            r @ 0x18..=0x1F => val |  (1 << (r - 0x18)),
            _ => unreachable!(),
        };

        bus.write(addr, res);
        if reg != 6 { self.set_r8(reg, res, bus); } // also store in register (undocumented)
        23
    }

    // ── FD prefix (IY instructions) — mirrors DD with IY ─────────────────────

    fn execute_fd(&mut self, bus: &mut Bus) -> u32 {
        let op = self.fetch(bus);
        self.r = (self.r & 0x80) | ((self.r.wrapping_add(1)) & 0x7F);
        match op {
            0x09 => { let v = self.bc(); self.add_iy16(v); 15 }
            0x19 => { let v = self.de(); self.add_iy16(v); 15 }
            0x21 => { self.iy = self.fetch16(bus); 14 }
            0x22 => { let a = self.fetch16(bus); bus.write16(a, self.iy); 20 }
            0x23 => { self.iy = self.iy.wrapping_add(1); 10 }
            0x24 => { let v = self.iyh(); self.iy = (self.inc8(v) as u16) << 8 | self.iyl() as u16; 8 }
            0x25 => { let v = self.iyh(); self.iy = (self.dec8(v) as u16) << 8 | self.iyl() as u16; 8 }
            0x26 => { let n = self.fetch(bus); self.set_iyh(n); 11 }
            0x29 => { let v = self.iy; self.add_iy16(v); 15 }
            0x2A => { let a = self.fetch16(bus); self.iy = bus.read16(a); 20 }
            0x2B => { self.iy = self.iy.wrapping_sub(1); 10 }
            0x2C => { let v = self.iyl(); self.iy = self.iyh() as u16 * 256 | self.inc8(v) as u16; 8 }
            0x2D => { let v = self.iyl(); self.iy = self.iyh() as u16 * 256 | self.dec8(v) as u16; 8 }
            0x2E => { let n = self.fetch(bus); self.set_iyl(n); 11 }
            0x34 => { let d=self.fetch_disp(bus); let a=self.iy.wrapping_add(d as u16); let v=bus.read(a); let r=self.inc8(v); bus.write(a,r); 23 }
            0x35 => { let d=self.fetch_disp(bus); let a=self.iy.wrapping_add(d as u16); let v=bus.read(a); let r=self.dec8(v); bus.write(a,r); 23 }
            0x36 => { let d=self.fetch_disp(bus); let n=self.fetch(bus); bus.write(self.iy.wrapping_add(d as u16), n); 19 }
            0x39 => { let v = self.sp; self.add_iy16(v); 15 }
            0xCB => self.execute_fdcb(bus),
            0xE1 => { self.iy = self.pop16(bus); 14 }
            0xE3 => { let sp=self.sp; let m=bus.read16(sp); bus.write16(sp, self.iy); self.iy=m; 23 }
            0xE5 => { let v = self.iy; self.push16(v, bus); 15 }
            0xE9 => { self.pc = self.iy; 8 }
            0xF9 => { self.sp = self.iy; 10 }
            0x44 => { self.b = self.iyh(); 8 } 0x45 => { self.b = self.iyl(); 8 }
            0x4C => { self.c = self.iyh(); 8 } 0x4D => { self.c = self.iyl(); 8 }
            0x54 => { self.d = self.iyh(); 8 } 0x55 => { self.d = self.iyl(); 8 }
            0x5C => { self.e = self.iyh(); 8 } 0x5D => { self.e = self.iyl(); 8 }
            0x60 => { let v=self.b; self.set_iyh(v); 8 } 0x61 => { let v=self.c; self.set_iyh(v); 8 }
            0x62 => { let v=self.d; self.set_iyh(v); 8 } 0x63 => { let v=self.e; self.set_iyh(v); 8 }
            0x64 => { 8 }
            0x65 => { let v=self.iyl(); self.set_iyh(v); 8 }
            0x67 => { let v=self.a; self.set_iyh(v); 8 }
            0x68 => { let v=self.b; self.set_iyl(v); 8 } 0x69 => { let v=self.c; self.set_iyl(v); 8 }
            0x6A => { let v=self.d; self.set_iyl(v); 8 } 0x6B => { let v=self.e; self.set_iyl(v); 8 }
            0x6C => { let v=self.iyh(); self.set_iyl(v); 8 }
            0x6D => { 8 }
            0x6F => { let v=self.a; self.set_iyl(v); 8 }
            0x7C => { self.a = self.iyh(); 8 } 0x7D => { self.a = self.iyl(); 8 }
            0x46 => { let d=self.fetch_disp(bus); self.b=bus.read(self.iy.wrapping_add(d as u16)); 19 }
            0x4E => { let d=self.fetch_disp(bus); self.c=bus.read(self.iy.wrapping_add(d as u16)); 19 }
            0x56 => { let d=self.fetch_disp(bus); self.d=bus.read(self.iy.wrapping_add(d as u16)); 19 }
            0x5E => { let d=self.fetch_disp(bus); self.e=bus.read(self.iy.wrapping_add(d as u16)); 19 }
            0x66 => { let d=self.fetch_disp(bus); self.h=bus.read(self.iy.wrapping_add(d as u16)); 19 }
            0x6E => { let d=self.fetch_disp(bus); self.l=bus.read(self.iy.wrapping_add(d as u16)); 19 }
            0x7E => { let d=self.fetch_disp(bus); self.a=bus.read(self.iy.wrapping_add(d as u16)); 19 }
            0x70 => { let d=self.fetch_disp(bus); bus.write(self.iy.wrapping_add(d as u16), self.b); 19 }
            0x71 => { let d=self.fetch_disp(bus); bus.write(self.iy.wrapping_add(d as u16), self.c); 19 }
            0x72 => { let d=self.fetch_disp(bus); bus.write(self.iy.wrapping_add(d as u16), self.d); 19 }
            0x73 => { let d=self.fetch_disp(bus); bus.write(self.iy.wrapping_add(d as u16), self.e); 19 }
            0x74 => { let d=self.fetch_disp(bus); bus.write(self.iy.wrapping_add(d as u16), self.h); 19 }
            0x75 => { let d=self.fetch_disp(bus); bus.write(self.iy.wrapping_add(d as u16), self.l); 19 }
            0x77 => { let d=self.fetch_disp(bus); bus.write(self.iy.wrapping_add(d as u16), self.a); 19 }
            0x84 => { let v=self.iyh(); self.add8(v,0); 8 } 0x85 => { let v=self.iyl(); self.add8(v,0); 8 }
            0x86 => { let d=self.fetch_disp(bus); let v=bus.read(self.iy.wrapping_add(d as u16)); self.add8(v,0); 19 }
            0x8C => { let v=self.iyh(); let c=self.cf() as u8; self.add8(v,c); 8 }
            0x8D => { let v=self.iyl(); let c=self.cf() as u8; self.add8(v,c); 8 }
            0x8E => { let d=self.fetch_disp(bus); let v=bus.read(self.iy.wrapping_add(d as u16)); let c=self.cf() as u8; self.add8(v,c); 19 }
            0x94 => { let v=self.iyh(); self.sub8(v,0); 8 } 0x95 => { let v=self.iyl(); self.sub8(v,0); 8 }
            0x96 => { let d=self.fetch_disp(bus); let v=bus.read(self.iy.wrapping_add(d as u16)); self.sub8(v,0); 19 }
            0x9C => { let v=self.iyh(); let c=self.cf() as u8; self.sub8(v,c); 8 }
            0x9D => { let v=self.iyl(); let c=self.cf() as u8; self.sub8(v,c); 8 }
            0x9E => { let d=self.fetch_disp(bus); let v=bus.read(self.iy.wrapping_add(d as u16)); let c=self.cf() as u8; self.sub8(v,c); 19 }
            0xA4 => { let v=self.iyh(); self.and8(v); 8 } 0xA5 => { let v=self.iyl(); self.and8(v); 8 }
            0xA6 => { let d=self.fetch_disp(bus); let v=bus.read(self.iy.wrapping_add(d as u16)); self.and8(v); 19 }
            0xAC => { let v=self.iyh(); self.xor8(v); 8 } 0xAD => { let v=self.iyl(); self.xor8(v); 8 }
            0xAE => { let d=self.fetch_disp(bus); let v=bus.read(self.iy.wrapping_add(d as u16)); self.xor8(v); 19 }
            0xB4 => { let v=self.iyh(); self.or8(v); 8 } 0xB5 => { let v=self.iyl(); self.or8(v); 8 }
            0xB6 => { let d=self.fetch_disp(bus); let v=bus.read(self.iy.wrapping_add(d as u16)); self.or8(v); 19 }
            0xBC => { let v=self.iyh(); self.cp8(v); 8 } 0xBD => { let v=self.iyl(); self.cp8(v); 8 }
            0xBE => { let d=self.fetch_disp(bus); let v=bus.read(self.iy.wrapping_add(d as u16)); self.cp8(v); 19 }
            _ => 8,
        }
    }

    fn execute_fdcb(&mut self, bus: &mut Bus) -> u32 {
        let d = self.fetch_disp(bus);
        let op = self.fetch(bus);
        let addr = self.iy.wrapping_add(d as u16);
        let val = bus.read(addr);
        let reg = op & 7;
        let row = op >> 3;

        let res = match row {
            0x00 => self.rlc(val), 0x01 => self.rrc(val),
            0x02 => self.rl(val),  0x03 => self.rr(val),
            0x04 => self.sla(val), 0x05 => self.sra(val),
            0x06 => self.sll(val), 0x07 => self.srl(val),
            r @ 0x08..=0x0F => { self.bit_test(r - 8, val); return 20; }
            r @ 0x10..=0x17 => val & !(1 << (r - 0x10)),
            r @ 0x18..=0x1F => val |  (1 << (r - 0x18)),
            _ => unreachable!(),
        };

        bus.write(addr, res);
        if reg != 6 { self.set_r8(reg, res, bus); }
        23
    }

    // ── ED prefix (extended instructions) ────────────────────────────────────

    fn execute_ed(&mut self, bus: &mut Bus) -> u32 {
        let op = self.fetch(bus);
        self.r = (self.r & 0x80) | ((self.r.wrapping_add(1)) & 0x7F);
        match op {
            // IN r,(C)
            0x40 => { let v = bus.port_in(self.bc()); self.b = v; self.f = (self.f & CF) | flags_szxy(v) | parity_flag(v); 12 }
            0x48 => { let v = bus.port_in(self.bc()); self.c = v; self.f = (self.f & CF) | flags_szxy(v) | parity_flag(v); 12 }
            0x50 => { let v = bus.port_in(self.bc()); self.d = v; self.f = (self.f & CF) | flags_szxy(v) | parity_flag(v); 12 }
            0x58 => { let v = bus.port_in(self.bc()); self.e = v; self.f = (self.f & CF) | flags_szxy(v) | parity_flag(v); 12 }
            0x60 => { let v = bus.port_in(self.bc()); self.h = v; self.f = (self.f & CF) | flags_szxy(v) | parity_flag(v); 12 }
            0x68 => { let v = bus.port_in(self.bc()); self.l = v; self.f = (self.f & CF) | flags_szxy(v) | parity_flag(v); 12 }
            0x70 => { let v = bus.port_in(self.bc()); self.f = (self.f & CF) | flags_szxy(v) | parity_flag(v); 12 } // IN F,(C)
            0x78 => { let v = bus.port_in(self.bc()); self.a = v; self.f = (self.f & CF) | flags_szxy(v) | parity_flag(v); 12 }
            // OUT (C),r
            0x41 => { bus.port_out(self.bc(), self.b, self.t); 12 }
            0x49 => { bus.port_out(self.bc(), self.c, self.t); 12 }
            0x51 => { bus.port_out(self.bc(), self.d, self.t); 12 }
            0x59 => { bus.port_out(self.bc(), self.e, self.t); 12 }
            0x61 => { bus.port_out(self.bc(), self.h, self.t); 12 }
            0x69 => { bus.port_out(self.bc(), self.l, self.t); 12 }
            0x71 => { bus.port_out(self.bc(), 0, self.t); 12 } // OUT (C),0
            0x79 => { bus.port_out(self.bc(), self.a, self.t); 12 }
            // SBC/ADC HL,rr
            0x42 => { let v = self.bc(); self.sbc_hl16(v); 15 }
            0x52 => { let v = self.de(); self.sbc_hl16(v); 15 }
            0x62 => { let v = self.hl(); self.sbc_hl16(v); 15 }
            0x72 => { let v = self.sp;  self.sbc_hl16(v); 15 }
            0x4A => { let v = self.bc(); self.adc_hl16(v); 15 }
            0x5A => { let v = self.de(); self.adc_hl16(v); 15 }
            0x6A => { let v = self.hl(); self.adc_hl16(v); 15 }
            0x7A => { let v = self.sp;  self.adc_hl16(v); 15 }
            // LD (nn),rr / LD rr,(nn)
            0x43 => { let a = self.fetch16(bus); bus.write16(a, self.bc()); 20 }
            0x53 => { let a = self.fetch16(bus); bus.write16(a, self.de()); 20 }
            0x63 => { let a = self.fetch16(bus); bus.write16(a, self.hl()); 20 }
            0x73 => { let a = self.fetch16(bus); bus.write16(a, self.sp); 20 }
            0x4B => { let a = self.fetch16(bus); let v = bus.read16(a); self.set_bc(v); 20 }
            0x5B => { let a = self.fetch16(bus); let v = bus.read16(a); self.set_de(v); 20 }
            0x6B => { let a = self.fetch16(bus); let v = bus.read16(a); self.set_hl(v); 20 }
            0x7B => { let a = self.fetch16(bus); self.sp = bus.read16(a); 20 }
            // NEG
            0x44 | 0x4C | 0x54 | 0x5C | 0x64 | 0x6C | 0x74 | 0x7C => {
                let a = self.a; self.a = 0; self.sub8(a, 0); 8
            }
            // RETN / RETI
            0x45 | 0x55 | 0x5D | 0x65 | 0x6D | 0x75 | 0x7D => {
                self.iff1 = self.iff2; self.pc = self.pop16(bus); 14
            }
            0x4D => { self.iff1 = self.iff2; self.pc = self.pop16(bus); 14 } // RETI
            // IM 0/1/2
            0x46 | 0x4E | 0x66 | 0x6E => { self.im = 0; 8 }
            0x56 | 0x76 => { self.im = 1; 8 }
            0x5E | 0x7E => { self.im = 2; 8 }
            // LD I/R,A / LD A,I/R
            0x47 => { self.i = self.a; 9 }
            0x4F => { self.r = self.a; 9 }
            0x57 => {
                let v = self.i;
                self.f = (self.f & CF) | flags_szxy(v) | if v & 0x80 != 0 { SF } else { 0 }
                    | if self.iff2 { PF } else { 0 };
                self.a = v; 9
            }
            0x5F => {
                let v = self.r;
                self.f = (self.f & CF) | flags_szxy(v) | if v & 0x80 != 0 { SF } else { 0 }
                    | if self.iff2 { PF } else { 0 };
                self.a = v; 9
            }
            // RLD / RRD
            0x6F => {
                let hl = self.hl();
                let m = bus.read(hl);
                bus.write(hl, (m << 4) | (self.a & 0xF));
                self.a = (self.a & 0xF0) | (m >> 4);
                self.f = (self.f & CF) | flags_szxy(self.a) | parity_flag(self.a); 18
            }
            0x67 => {
                let hl = self.hl();
                let m = bus.read(hl);
                bus.write(hl, (m >> 4) | (self.a << 4));
                self.a = (self.a & 0xF0) | (m & 0xF);
                self.f = (self.f & CF) | flags_szxy(self.a) | parity_flag(self.a); 18
            }
            // Block instructions
            0xA0 => { self.ldi(bus); 16 }   // LDI
            0xA8 => { self.ldd(bus); 16 }   // LDD
            0xB0 => { // LDIR
                self.ldi(bus);
                if self.pf() { self.pc = self.pc.wrapping_sub(2); 21 } else { 16 }
            }
            0xB8 => { // LDDR
                self.ldd(bus);
                if self.pf() { self.pc = self.pc.wrapping_sub(2); 21 } else { 16 }
            }
            0xA1 => { self.cpi(bus); 16 }   // CPI
            0xA9 => { self.cpd(bus); 16 }   // CPD
            0xB1 => { // CPIR
                self.cpi(bus);
                if self.pf() && !self.zf() { self.pc = self.pc.wrapping_sub(2); 21 } else { 16 }
            }
            0xB9 => { // CPDR
                self.cpd(bus);
                if self.pf() && !self.zf() { self.pc = self.pc.wrapping_sub(2); 21 } else { 16 }
            }
            _ => 8,
        }
    }

    // ── Block instruction helpers ─────────────────────────────────────────────

    fn ldi(&mut self, bus: &mut Bus) {
        let v = bus.read(self.hl());
        bus.write(self.de(), v);
        let n = v.wrapping_add(self.a);
        let bc = self.bc().wrapping_sub(1);
        self.set_hl(self.hl().wrapping_add(1));
        self.set_de(self.de().wrapping_add(1));
        self.set_bc(bc);
        self.f = (self.f & (SF | ZF | CF))
            | (n & YF) | if n & 0x02 != 0 { XF } else { 0 }
            | if bc != 0 { PF } else { 0 };
    }

    fn ldd(&mut self, bus: &mut Bus) {
        let v = bus.read(self.hl());
        bus.write(self.de(), v);
        let n = v.wrapping_add(self.a);
        let bc = self.bc().wrapping_sub(1);
        self.set_hl(self.hl().wrapping_sub(1));
        self.set_de(self.de().wrapping_sub(1));
        self.set_bc(bc);
        self.f = (self.f & (SF | ZF | CF))
            | (n & YF) | if n & 0x02 != 0 { XF } else { 0 }
            | if bc != 0 { PF } else { 0 };
    }

    fn cpi(&mut self, bus: &mut Bus) {
        let v = bus.read(self.hl());
        let res = self.a.wrapping_sub(v);
        let half = (self.a & 0xF) < (v & 0xF);
        let bc = self.bc().wrapping_sub(1);
        self.set_hl(self.hl().wrapping_add(1));
        self.set_bc(bc);
        let n = res.wrapping_sub(if half { 1 } else { 0 });
        self.f = (self.f & CF)
            | (if res & 0x80 != 0 { SF } else { 0 })
            | (if res == 0 { ZF } else { 0 })
            | (n & YF) | if n & 0x02 != 0 { XF } else { 0 }
            | if half { HF } else { 0 }
            | if bc != 0 { PF } else { 0 }
            | NF;
    }

    fn cpd(&mut self, bus: &mut Bus) {
        let v = bus.read(self.hl());
        let res = self.a.wrapping_sub(v);
        let half = (self.a & 0xF) < (v & 0xF);
        let bc = self.bc().wrapping_sub(1);
        self.set_hl(self.hl().wrapping_sub(1));
        self.set_bc(bc);
        let n = res.wrapping_sub(if half { 1 } else { 0 });
        self.f = (self.f & CF)
            | (if res & 0x80 != 0 { SF } else { 0 })
            | (if res == 0 { ZF } else { 0 })
            | (n & YF) | if n & 0x02 != 0 { XF } else { 0 }
            | if half { HF } else { 0 }
            | if bc != 0 { PF } else { 0 }
            | NF;
    }
}

// ── Utility functions ─────────────────────────────────────────────────────────

/// Build S/Z/Y/X flags from an 8-bit result (the common case).
#[inline]
fn flags_szxy(v: u8) -> u8 {
    (if v & 0x80 != 0 { SF } else { 0 })
    | (if v == 0 { ZF } else { 0 })
    | (v & YF)
    | (v & XF)
}

/// Even-parity flag: PF set if number of set bits is even.
#[inline]
fn parity_flag(v: u8) -> u8 {
    if v.count_ones() % 2 == 0 { PF } else { 0 }
}
