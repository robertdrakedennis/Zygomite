use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub enum AudioKind {
    Jaga,
    Ogg,
    Midi,
    Wav,
    Flac,
    Unknown,
}

impl AudioKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jaga => "jaga",
            Self::Ogg => "ogg",
            Self::Midi => "midi",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct JagaHeader {
    pub field0: u32,
    pub field1: u32,
    pub field2: u32,
    pub field3: u32,
    pub field4: u32,
    pub field5: u32,
    pub field6: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AudioInspection {
    pub kind: AudioKind,
    pub extension: &'static str,
    pub embedded_ogg_offset: Option<usize>,
    pub jaga_header: Option<JagaHeader>,
}

impl AudioInspection {
    pub fn embedded_ogg_slice<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        self.embedded_ogg_offset
            .and_then(|offset| bytes.get(offset..))
            .filter(|slice| !slice.is_empty())
    }
}

pub fn inspect_audio_file(bytes: &[u8]) -> AudioInspection {
    if bytes.starts_with(b"JAGA") {
        return AudioInspection {
            kind: AudioKind::Jaga,
            extension: "jaga",
            embedded_ogg_offset: find_subsequence(bytes, b"OggS"),
            jaga_header: decode_jaga_header(bytes),
        };
    }

    if bytes.starts_with(b"OggS") {
        return AudioInspection {
            kind: AudioKind::Ogg,
            extension: "ogg",
            embedded_ogg_offset: Some(0),
            jaga_header: None,
        };
    }

    if bytes.starts_with(b"MThd") {
        return AudioInspection {
            kind: AudioKind::Midi,
            extension: "mid",
            embedded_ogg_offset: None,
            jaga_header: None,
        };
    }

    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        return AudioInspection {
            kind: AudioKind::Wav,
            extension: "wav",
            embedded_ogg_offset: None,
            jaga_header: None,
        };
    }

    if bytes.starts_with(b"fLaC") {
        return AudioInspection {
            kind: AudioKind::Flac,
            extension: "flac",
            embedded_ogg_offset: None,
            jaga_header: None,
        };
    }

    AudioInspection {
        kind: AudioKind::Unknown,
        extension: "bin",
        embedded_ogg_offset: find_subsequence(bytes, b"OggS"),
        jaga_header: None,
    }
}

fn decode_jaga_header(bytes: &[u8]) -> Option<JagaHeader> {
    if bytes.len() < 32 {
        return None;
    }
    Some(JagaHeader {
        field0: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        field1: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        field2: u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
        field3: u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
        field4: u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
        field5: u32::from_be_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        field6: u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]),
    })
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|chunk| chunk == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_jaga_and_embedded_ogg() {
        let mut bytes = vec![0_u8; 40];
        bytes[0..4].copy_from_slice(b"JAGA");
        bytes[4..8].copy_from_slice(&1_u32.to_be_bytes());
        bytes[8..12].copy_from_slice(&2_u32.to_be_bytes());
        bytes[12..16].copy_from_slice(&3_u32.to_be_bytes());
        bytes[16..20].copy_from_slice(&4_u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&5_u32.to_be_bytes());
        bytes[24..28].copy_from_slice(&6_u32.to_be_bytes());
        bytes[28..32].copy_from_slice(&7_u32.to_be_bytes());
        bytes[32..36].copy_from_slice(b"OggS");

        let inspection = inspect_audio_file(&bytes);
        assert_eq!(AudioKind::Jaga, inspection.kind);
        assert_eq!("jaga", inspection.extension);
        assert_eq!(Some(32), inspection.embedded_ogg_offset);
        assert_eq!(Some(&bytes[32..]), inspection.embedded_ogg_slice(&bytes));
        let header = inspection.jaga_header.expect("jaga header");
        assert_eq!(1, header.field0);
        assert_eq!(7, header.field6);
    }

    #[test]
    fn detects_known_magic_formats() {
        assert_eq!(AudioKind::Ogg, inspect_audio_file(b"OggSrest").kind);
        assert_eq!(AudioKind::Midi, inspect_audio_file(b"MThdrest").kind);
        assert_eq!(
            AudioKind::Wav,
            inspect_audio_file(b"RIFF\x00\x00\x00\x00WAVErest").kind
        );
        assert_eq!(AudioKind::Flac, inspect_audio_file(b"fLaCrest").kind);
        assert_eq!(
            AudioKind::Unknown,
            inspect_audio_file(b"\x01\x02\x03\x04").kind
        );
    }
}
