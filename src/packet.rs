use crate::cache_bail as bail;
use crate::error::{Context, Result};
use encoding_rs::WINDOWS_1252;

// ── Packet reader (read-only, references external buffer) ──

#[derive(Clone, Debug)]
pub struct Packet<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Packet<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn with_pos(data: &'a [u8], pos: usize) -> Result<Self> {
        if pos > data.len() {
            bail!("packet position out of bounds: {pos} > {}", data.len());
        }
        Ok(Self { data, pos })
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            bail!("packet position out of bounds: {pos} > {}", self.data.len());
        }
        self.pos = pos;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn is_done(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub fn g1(&mut self) -> Result<u8> {
        let byte = *self
            .data
            .get(self.pos)
            .with_context(|| format!("g1 out of bounds at {}", self.pos))?;
        self.pos += 1;
        Ok(byte)
    }

    pub fn peek1(&self) -> Result<u8> {
        self.data
            .get(self.pos)
            .copied()
            .with_context(|| format!("peek1 out of bounds at {}", self.pos))
    }

    pub fn g1s(&mut self) -> Result<i8> {
        Ok(i8::from_ne_bytes([self.g1()?]))
    }

    pub fn g2(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn g2null(&mut self) -> Result<i32> {
        let value = self.g2()?;
        Ok(if value == 0xFFFF {
            -1
        } else {
            i32::from(value)
        })
    }

    pub fn g2s(&mut self) -> Result<i16> {
        let bytes = self.take(2)?;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn g2s_le(&mut self) -> Result<i16> {
        let bytes = self.take(2)?;
        Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn g2_le(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn g3(&mut self) -> Result<u32> {
        let bytes = self.take(3)?;
        Ok(u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2]))
    }

    pub fn g4s(&mut self) -> Result<i32> {
        let bytes = self.take(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn g4s_le(&mut self) -> Result<i32> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn g8s(&mut self) -> Result<i64> {
        let bytes = self.take(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub fn gfloat_le(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.g4s_le()? as u32))
    }

    pub fn gdata(&mut self, length: usize) -> Result<Vec<u8>> {
        Ok(self.take(length)?.to_vec())
    }

    pub fn gjstrnull(&mut self) -> Result<Option<String>> {
        if self
            .data
            .get(self.pos)
            .with_context(|| format!("gjstrnull out of bounds at {}", self.pos))?
            == &0
        {
            self.pos += 1;
            Ok(None)
        } else {
            Ok(Some(self.gjstr()?))
        }
    }

    pub fn gjstr(&mut self) -> Result<String> {
        let start = self.pos;
        while self.pos < self.data.len() {
            let value = self.data[self.pos];
            self.pos += 1;
            if value == 0 || value == 10 {
                let bytes = &self.data[start..self.pos - 1];
                let (decoded, _, had_errors) = WINDOWS_1252.decode(bytes);
                if had_errors {
                    bail!("cp1252 decode error");
                }
                return Ok(decoded.into_owned());
            }
        }
        bail!("unterminated gjstr at offset {start}");
    }

    pub fn gjstr2(&mut self) -> Result<String> {
        let marker = self.g1()?;
        if marker != 0 {
            bail!("gjstr2 marker mismatch: expected 0, got {marker}");
        }
        let start = self.pos;
        while self.pos < self.data.len() {
            if self.data[self.pos] == 0 {
                let bytes = &self.data[start..self.pos];
                self.pos += 1;
                let (decoded, _, had_errors) = WINDOWS_1252.decode(bytes);
                if had_errors {
                    bail!("cp1252 decode error");
                }
                return Ok(decoded.into_owned());
            }
            self.pos += 1;
        }
        bail!("unterminated gjstr2 at offset {start}");
    }

    pub fn gsmart2or4null(&mut self) -> Result<i32> {
        let first = *self
            .data
            .get(self.pos)
            .with_context(|| format!("gsmart2or4null out of bounds at {}", self.pos))?;
        if (first as i8) < 0 {
            Ok(self.g4s()? & i32::MAX)
        } else {
            let value = i32::from(self.g2()?);
            Ok(if value == 32767 { -1 } else { value })
        }
    }

    pub fn gsmart2or4(&mut self) -> Result<i32> {
        let first = *self
            .data
            .get(self.pos)
            .with_context(|| format!("gsmart2or4 out of bounds at {}", self.pos))?;
        if (first as i8) < 0 {
            Ok(self.g4s()? & i32::MAX)
        } else {
            Ok(i32::from(self.g2()?))
        }
    }

    pub fn gsmart1or2(&mut self) -> Result<u16> {
        let first = *self
            .data
            .get(self.pos)
            .with_context(|| format!("gsmart1or2 out of bounds at {}", self.pos))?;
        if first < 128 {
            Ok(u16::from(self.g1()?))
        } else {
            Ok(self.g2()?.saturating_sub(32_768))
        }
    }

    pub fn gsmart1or2s(&mut self) -> Result<i32> {
        let first = *self
            .data
            .get(self.pos)
            .with_context(|| format!("gsmart1or2s out of bounds at {}", self.pos))?;
        if first < 128 {
            Ok(i32::from(self.g1()?) - 64)
        } else {
            Ok(i32::from(self.g2()?) - 49_152)
        }
    }

    pub fn g_extended_1or2(&mut self) -> Result<i32> {
        let mut value = 0_i32;
        loop {
            let next = i32::from(self.gsmart1or2()?);
            value += next;
            if next != 32_767 {
                return Ok(value);
            }
        }
    }

    pub fn gvarint2(&mut self) -> Result<u32> {
        let mut value = 0_u32;
        let mut shift = 0_u32;
        loop {
            let byte = u32::from(self.g1()?);
            value |= (byte & 0x7F) << shift;
            if byte <= 0x7F {
                return Ok(value);
            }
            shift = shift.checked_add(7).context("gvarint2 shift overflow")?;
            if shift >= 32 {
                bail!("gvarint2 too large");
            }
        }
    }

    pub fn slice(&self, start: usize, end: usize) -> Result<&'a [u8]> {
        if start > end || end > self.data.len() {
            bail!("slice out of bounds: {start}..{end}");
        }
        Ok(&self.data[start..end])
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(count)
            .context("packet position overflow")?;
        if end > self.data.len() {
            bail!(
                "packet read out of bounds: {}..{} (len {})",
                self.pos,
                end,
                self.data.len()
            );
        }
        let start = self.pos;
        self.pos = end;
        Ok(&self.data[start..end])
    }
}

// ── ByteWriter (write-only, owns its buffer) ──

#[derive(Clone, Debug, Default)]
pub struct ByteWriter {
    pub data: Vec<u8>,
}

impl ByteWriter {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            data: Vec::with_capacity(cap),
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn p1(&mut self, value: u8) {
        self.data.push(value);
    }

    pub fn p2(&mut self, value: u16) {
        self.data.extend_from_slice(&value.to_be_bytes());
    }

    pub fn p2_le(&mut self, value: u16) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }

    pub fn p3(&mut self, value: u32) {
        let bytes = value.to_be_bytes();
        self.data.push(bytes[1]);
        self.data.push(bytes[2]);
        self.data.push(bytes[3]);
    }

    pub fn p4s(&mut self, value: i32) {
        self.data.extend_from_slice(&value.to_be_bytes());
    }

    pub fn p4s_le(&mut self, value: i32) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }

    pub fn p8s(&mut self, value: i64) {
        self.data.extend_from_slice(&value.to_be_bytes());
    }

    pub fn pfloat_le(&mut self, value: f32) {
        self.p4s_le(value.to_bits() as i32);
    }

    pub fn pdata(&mut self, bytes: &[u8]) {
        self.data.extend_from_slice(bytes);
    }

    pub fn pjstr(&mut self, s: &str) {
        let (encoded, _, had_errors) = WINDOWS_1252.encode(s);
        if had_errors {
            self.data.extend_from_slice(s.as_bytes());
        } else {
            self.data.extend_from_slice(&encoded);
        }
        self.data.push(0);
    }

    pub fn pjstrnull(&mut self, s: Option<&str>) {
        match s {
            None => self.data.push(0),
            Some(s) => self.pjstr(s),
        }
    }
}
