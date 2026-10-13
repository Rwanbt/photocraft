//! Minimal ZIP container: we write STORE entries (the payloads are already
//! zstd-compressed) and read STORE or DEFLATE entries (so a bundle that was
//! unzipped and re-zipped by another tool still loads). No ZIP64, no
//! encryption, no multi-disk archives.

use std::collections::HashSet;
use std::io::Read;

use crate::FormatError;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;

/// CRC-32 (IEEE), table-driven.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

/// Streaming writer for STORE entries.
pub struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
    names: HashSet<String>,
}

impl Default for ZipWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ZipWriter {
    pub fn new() -> Self {
        ZipWriter { out: Vec::new(), central: Vec::new(), count: 0, names: HashSet::new() }
    }

    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<(), FormatError> {
        if self.count == u16::MAX {
            return Err(FormatError::Unsupported("more than 65535 zip entries (ZIP64 not supported)".into()));
        }
        // The ZIP local and central headers encode the UTF-8 filename length as u16.
        // Validate before writing either header; a cast would truncate and corrupt the archive.
        if self.names.contains(name) {
            return Err(FormatError::Unsupported(format!("duplicate zip entry name `{name}`")));
        }
        let name_b = name.as_bytes();
        let name_len = u16::try_from(name_b.len()).map_err(|_| FormatError::Unsupported("zip entry filename exceeds 65535 UTF-8 bytes".into()))?;
        let size = u32::try_from(data.len()).map_err(|_| FormatError::Unsupported("zip entry over 4 GiB".into()))?;
        let offset = u32::try_from(self.out.len()).map_err(|_| FormatError::Unsupported("zip over 4 GiB".into()))?;
        let crc = crc32(data);
        // Local file header.
        let o = &mut self.out;
        o.extend_from_slice(&LOCAL_SIG.to_le_bytes());
        o.extend_from_slice(&20u16.to_le_bytes()); // version needed
        o.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: UTF-8 names
        o.extend_from_slice(&0u16.to_le_bytes()); // method: store
        o.extend_from_slice(&0u16.to_le_bytes()); // time
        o.extend_from_slice(&0x21u16.to_le_bytes()); // date: 1980-01-01
        o.extend_from_slice(&crc.to_le_bytes());
        o.extend_from_slice(&size.to_le_bytes());
        o.extend_from_slice(&size.to_le_bytes());
        o.extend_from_slice(&name_len.to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        o.extend_from_slice(name_b);
        o.extend_from_slice(data);
        // Central directory entry.
        let c = &mut self.central;
        c.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes()); // made by
        c.extend_from_slice(&20u16.to_le_bytes()); // needed
        c.extend_from_slice(&0x0800u16.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&0u16.to_le_bytes());
        c.extend_from_slice(&0x21u16.to_le_bytes());
        c.extend_from_slice(&crc.to_le_bytes());
        c.extend_from_slice(&size.to_le_bytes());
        c.extend_from_slice(&size.to_le_bytes());
        c.extend_from_slice(&name_len.to_le_bytes());
        c.extend_from_slice(&[0; 8]); // extra len, comment len, disk, internal attrs
        c.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name_b);
        self.count += 1;
        self.names.insert(name.to_owned());
        Ok(())
    }

    pub fn finish(mut self) -> Result<Vec<u8>, FormatError> {
        let cd_offset = u32::try_from(self.out.len()).map_err(|_| FormatError::Unsupported("zip over 4 GiB".into()))?;
        let cd_size = u32::try_from(self.central.len()).map_err(|_| FormatError::Unsupported("zip central directory over 4 GiB".into()))?;
        if cd_offset.checked_add(cd_size).is_none() {
            return Err(FormatError::Unsupported("zip over 4 GiB (ZIP64 not supported)".into()));
        }
        self.out.extend_from_slice(&self.central);
        self.out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]); // disk numbers
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&cd_size.to_le_bytes());
        self.out.extend_from_slice(&cd_offset.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        Ok(self.out)
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    method: u16,
    crc: u32,
    compressed: usize,
    uncompressed: usize,
    data_offset: usize,
}

/// Read-only view of an archive in memory.
pub struct ZipReader<'a> {
    bytes: &'a [u8],
    pub entries: Vec<Entry>,
}

fn u16_at(b: &[u8], o: usize) -> Result<u16, FormatError> {
    o.checked_add(2).and_then(|end| b.get(o..end)).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| FormatError::corrupt("truncated zip"))
}
fn u32_at(b: &[u8], o: usize) -> Result<u32, FormatError> {
    o.checked_add(4).and_then(|end| b.get(o..end)).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| FormatError::corrupt("truncated zip"))
}

impl<'a> ZipReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, FormatError> {
        if bytes.len() < 22 {
            return Err(FormatError::corrupt("not a zip archive (too short)"));
        }
        // End of central directory: scan back over a possible comment.
        let min = bytes.len().saturating_sub(22 + 65535);
        let eocd = (min..=bytes.len() - 22)
            .rev()
            .find(|&i| u32_at(bytes, i).ok() == Some(EOCD_SIG) && u16_at(bytes, i + 20).is_ok_and(|comment_len| bytes.len() - i - 22 == comment_len as usize))
            .ok_or_else(|| FormatError::corrupt("zip end-of-central-directory not found"))?;
        let count = u16_at(bytes, eocd + 10)? as usize;
        let cd_size = u32_at(bytes, eocd + 12)? as usize;
        let cd_off = u32_at(bytes, eocd + 16)? as usize;
        let cd_end = cd_off.checked_add(cd_size).filter(|end| *end <= eocd).ok_or_else(|| FormatError::corrupt("zip central directory out of range"))?;
        let mut entries = Vec::with_capacity(count.min(bytes.len() / 46));
        let mut seen = HashSet::with_capacity(count.min(bytes.len() / 46));
        let mut p = cd_off;
        for _ in 0..count {
            let header_end = p.checked_add(46).filter(|end| *end <= cd_end).ok_or_else(|| FormatError::corrupt("truncated zip central directory entry"))?;
            if u32_at(bytes, p)? != CENTRAL_SIG {
                return Err(FormatError::corrupt("bad zip central directory entry"));
            }
            let flags = u16_at(bytes, p + 8)?;
            let method = u16_at(bytes, p + 10)?;
            let crc = u32_at(bytes, p + 16)?;
            let compressed = u32_at(bytes, p + 20)? as usize;
            let uncompressed = u32_at(bytes, p + 24)? as usize;
            let nlen = u16_at(bytes, p + 28)? as usize;
            let elen = u16_at(bytes, p + 30)? as usize;
            let clen = u16_at(bytes, p + 32)? as usize;
            let local = u32_at(bytes, p + 42)? as usize;
            let name_end = header_end.checked_add(nlen).filter(|end| *end <= cd_end).ok_or_else(|| FormatError::corrupt("zip entry name out of range"))?;
            let name_bytes = bytes.get(header_end..name_end).ok_or_else(|| FormatError::corrupt("truncated zip name"))?;
            let name = String::from_utf8_lossy(name_bytes).into_owned();
            if !seen.insert(name.clone()) {
                return Err(FormatError::corrupt(format!("duplicate zip entry `{name}`")));
            }
            if flags & 1 != 0 {
                return Err(FormatError::Unsupported("encrypted zip entries".into()));
            }
            // Bound each entry by the declared central-directory extent before advancing.
            let entry_end = name_end
                .checked_add(elen)
                .and_then(|end| end.checked_add(clen))
                .filter(|end| *end <= cd_end)
                .ok_or_else(|| FormatError::corrupt("zip central directory entry out of range"))?;
            // The central and local headers must describe the same file.
            // Otherwise different ZIP readers can disagree about its name or
            // compression method, permitting ambiguous project contents.
            let local_header_end = local
                .checked_add(30)
                .filter(|end| *end <= cd_off)
                .ok_or_else(|| FormatError::corrupt("zip local header out of range"))?;
            if u32_at(bytes, local)? != LOCAL_SIG {
                return Err(FormatError::corrupt("bad zip local header"));
            }
            if u16_at(bytes, local + 6)? != flags || u16_at(bytes, local + 8)? != method {
                return Err(FormatError::corrupt(format!("zip entry `{name}` has mismatched local header flags or method")));
            }
            // ZIP bit 3 permits a streaming writer to leave local CRC and sizes
            // empty and place the real values in a data descriptor. The central
            // header still supplies the authoritative, checked values.
            if flags & 0x0008 == 0
                && (u32_at(bytes, local + 14)? != crc
                    || u32_at(bytes, local + 18)? as usize != compressed
                    || u32_at(bytes, local + 22)? as usize != uncompressed)
            {
                return Err(FormatError::corrupt(format!("zip entry `{name}` has mismatched local CRC or size")));
            }
            let lnlen = u16_at(bytes, local + 26)? as usize;
            let lelen = u16_at(bytes, local + 28)? as usize;
            let local_name_end = local_header_end
                .checked_add(lnlen)
                .filter(|end| *end <= cd_off)
                .ok_or_else(|| FormatError::corrupt("zip local filename out of range"))?;
            if bytes.get(local_header_end..local_name_end) != Some(name_bytes) {
                return Err(FormatError::corrupt(format!("zip entry `{name}` has mismatched local filename")));
            }
            let data_offset = local_name_end
                .checked_add(lelen)
                .filter(|end| *end <= cd_off)
                .ok_or_else(|| FormatError::corrupt("zip local extra field out of range"))?;
            if data_offset.checked_add(compressed).is_none_or(|end| end > cd_off) {
                return Err(FormatError::corrupt(format!("zip entry `{name}` out of range")));
            }
            entries.push(Entry { name, method, crc, compressed, uncompressed, data_offset });
            p = entry_end;
        }
        Ok(ZipReader { bytes, entries })
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Entry payload (decompressed), at most `max` bytes, CRC-checked.
    pub fn read(&self, e: &Entry, max: usize) -> Result<Vec<u8>, FormatError> {
        if e.uncompressed > max {
            return Err(FormatError::LimitExceeded(format!("zip entry `{}` is {} bytes (max {max})", e.name, e.uncompressed)));
        }
        let raw = &self.bytes[e.data_offset..e.data_offset + e.compressed];
        let data = match e.method {
            0 => raw.to_vec(),
            8 => {
                // The uncompressed length comes from an untrusted ZIP header: a
                // small forged archive must not reserve gigabytes up front, so the
                // preallocation is capped (DEFLATE expands at most ~1032x).
                let mut out = Vec::with_capacity(e.uncompressed.min(raw.len().saturating_mul(1032)).min(64 << 20));
                flate2::read::DeflateDecoder::new(raw)
                    .take((max as u64).saturating_add(1))
                    .read_to_end(&mut out)
                    .map_err(|err| FormatError::corrupt(format!("zip entry `{}`: {err}", e.name)))?;
                out
            }
            m => {
                return Err(FormatError::Unsupported(format!("zip compression method {m} for `{}`", e.name)));
            }
        };
        if data.len() != e.uncompressed || data.len() > max {
            return Err(FormatError::corrupt(format!("zip entry `{}` has wrong size", e.name)));
        }
        if crc32(&data) != e.crc {
            return Err(FormatError::corrupt(format!("zip entry `{}` failed its CRC check", e.name)));
        }
        Ok(data)
    }

    pub fn read_by_name(&self, name: &str, max: usize) -> Result<Vec<u8>, FormatError> {
        let e = self.find(name).ok_or_else(|| FormatError::corrupt(format!("missing `{name}`")))?;
        self.read(e, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn roundtrip() {
        let mut w = ZipWriter::new();
        w.add("a.txt", b"hello").unwrap();
        w.add("dir/b.bin", &[0, 1, 2, 3]).unwrap();
        w.add("empty", &[]).unwrap();
        let bytes = w.finish().unwrap();
        let r = ZipReader::new(&bytes).unwrap();
        assert_eq!(r.entries.len(), 3);
        assert_eq!(r.read_by_name("a.txt", 100).unwrap(), b"hello");
        assert_eq!(r.read_by_name("dir/b.bin", 100).unwrap(), [0, 1, 2, 3]);
        assert!(r.read_by_name("empty", 0).unwrap().is_empty());
        assert!(matches!(r.read_by_name("a.txt", 3), Err(FormatError::LimitExceeded(_))));
    }

    #[test]
    fn crc_mismatch_detected() {
        let mut w = ZipWriter::new();
        w.add("a", b"hello").unwrap();
        let mut bytes = w.finish().unwrap();
        let pos = bytes.windows(5).position(|x| x == b"hello").unwrap();
        bytes[pos] = b'j';
        let r = ZipReader::new(&bytes).unwrap();
        assert!(r.read_by_name("a", 100).is_err());
    }

    /// A valid DEFLATE-compressed bundle may be read with an effectively
    /// unlimited caller budget (e.g. corruption/round-trip checks). On 64-bit
    /// targets `usize::MAX as u64 + 1` overflows, so the decoder must saturate.
    #[test]
    fn deflated_entry_with_unbounded_limit_round_trips() {
        use std::io::Write as _;

        let content = b"DEFLATE-compressed PhotoCraft project content, with a valid CRC.";
        let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(content).unwrap();
        let compressed = encoder.finish().unwrap();

        // Construct a standards-conforming DEFLATE entry from the store-only
        // writer: keep its actual compressed bytes, but set the ZIP method,
        // original length and CRC in *both* the local and central headers.
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", &compressed).unwrap();
        let mut bytes = writer.finish().unwrap();
        let eocd = bytes.len() - 22;
        let central = u32_at(&bytes, eocd + 16).unwrap() as usize;
        let crc = crc32(content).to_le_bytes();
        let length = (content.len() as u32).to_le_bytes();

        bytes[8..10].copy_from_slice(&8u16.to_le_bytes());
        bytes[14..18].copy_from_slice(&crc);
        bytes[22..26].copy_from_slice(&length);
        bytes[central + 10..central + 12].copy_from_slice(&8u16.to_le_bytes());
        bytes[central + 16..central + 20].copy_from_slice(&crc);
        bytes[central + 24..central + 28].copy_from_slice(&length);

        let archive = ZipReader::new(&bytes).unwrap();
        assert_eq!(archive.read_by_name("manifest.json", usize::MAX).unwrap(), content);
        assert_eq!(archive.read_by_name("manifest.json", content.len()).unwrap(), content);
        assert!(matches!(archive.read_by_name("manifest.json", content.len() - 1), Err(FormatError::LimitExceeded(_))));

        // A tiny archive with a forged 4 GiB size must be rejected without
        // trying to allocate its claimed output capacity in advance.
        let mut forged = bytes;
        forged[22..26].copy_from_slice(&u32::MAX.to_le_bytes());
        forged[central + 24..central + 28].copy_from_slice(&u32::MAX.to_le_bytes());
        let archive = ZipReader::new(&forged).unwrap();
        assert!(matches!(archive.read_by_name("manifest.json", usize::MAX), Err(FormatError::Corrupt(_))));
    }

    #[test]
    fn offset_access_rejects_overflow() {
        assert!(u16_at(&[], usize::MAX).is_err());
        assert!(u32_at(&[], usize::MAX - 3).is_err());
    }
    #[test]
    fn oversized_utf8_entry_name_is_rejected_without_poisoning_writer() {
        let mut writer = ZipWriter::new();
        let overlong_ascii = "x".repeat(u16::MAX as usize + 1);
        assert!(matches!(writer.add(&overlong_ascii, b"bad"), Err(FormatError::Unsupported(_))));
        // The limit applies to the encoded byte length, not Unicode scalar count.
        let overlong_unicode = format!("{}é", "a".repeat(u16::MAX as usize - 1));
        assert_eq!(overlong_unicode.len(), u16::MAX as usize + 1);
        assert!(matches!(writer.add(&overlong_unicode, b"bad"), Err(FormatError::Unsupported(_))));

        // Failed additions must not change the entry count or corrupt the archive.
        writer.add("manifest.json", b"good").unwrap();
        let bytes = writer.finish().unwrap();
        let archive = ZipReader::new(&bytes).unwrap();
        assert_eq!(archive.entries.len(), 1);
        assert_eq!(archive.read_by_name("manifest.json", 4).unwrap(), b"good");
    }

    #[test]
    fn maximum_legal_utf8_entry_name_round_trips() {
        let name = format!("{}é", "a".repeat(u16::MAX as usize - 2));
        assert_eq!(name.len(), u16::MAX as usize);
        let mut writer = ZipWriter::new();
        writer.add(&name, b"ok").unwrap();
        let bytes = writer.finish().unwrap();
        let archive = ZipReader::new(&bytes).unwrap();
        assert_eq!(archive.entries.len(), 1);
        assert_eq!(archive.read_by_name(&name, 2).unwrap(), b"ok");
    }

    #[test]
    fn corrupt_central_directory_offsets_and_counts_are_rejected() {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"good").unwrap();
        let valid = writer.finish().unwrap();
        let eocd = valid.len() - 22;
        let central = u32_at(&valid, eocd + 16).unwrap() as usize;

        let mut wrong_signature = valid.clone();
        wrong_signature[central..central + 4].copy_from_slice(&0u32.to_le_bytes());
        assert!(matches!(ZipReader::new(&wrong_signature), Err(FormatError::Corrupt(_))));

        let mut impossible_local = valid.clone();
        impossible_local[central + 42..central + 46].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(ZipReader::new(&impossible_local), Err(FormatError::Corrupt(_))));

        let mut impossible_size = valid.clone();
        impossible_size[eocd + 12..eocd + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(ZipReader::new(&impossible_size), Err(FormatError::Corrupt(_))));

        let mut wrong_count = valid;
        wrong_count[eocd + 10..eocd + 12].copy_from_slice(&2u16.to_le_bytes());
        assert!(matches!(ZipReader::new(&wrong_count), Err(FormatError::Corrupt(_))));
    }

    #[test]
    fn encrypted_entries_and_unsupported_compression_are_rejected() {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"data").unwrap();
        let valid = writer.finish().unwrap();
        let central = u32_at(&valid, valid.len() - 22 + 16).unwrap() as usize;

        let mut encrypted = valid.clone();
        encrypted[central + 8..central + 10].copy_from_slice(&1u16.to_le_bytes());
        assert!(matches!(ZipReader::new(&encrypted), Err(FormatError::Unsupported(_))));

        let mut unsupported = valid;
        unsupported[central + 10..central + 12].copy_from_slice(&99u16.to_le_bytes());
        let archive = ZipReader::new(&unsupported).unwrap();
        assert!(matches!(archive.read_by_name("manifest.json", 4), Err(FormatError::Unsupported(_))));
        assert!(matches!(archive.read_by_name("missing", 4), Err(FormatError::Corrupt(_))));
    }

    #[test]
    fn writer_empty_archive_and_zero_byte_entry_are_readable() {
        let empty = ZipWriter::new().finish().unwrap();
        assert!(ZipReader::new(&empty).unwrap().entries.is_empty());

        let mut writer = ZipWriter::new();
        writer.add("zero", b"").unwrap();
        let bytes = writer.finish().unwrap();
        let archive = ZipReader::new(&bytes).unwrap();
        assert_eq!(archive.read_by_name("zero", 0).unwrap(), b"");
    }

    #[test]
    fn zip_comment_containing_eocd_magic_does_not_mask_the_real_directory() {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"valid").unwrap();
        let mut bytes = writer.finish().unwrap();
        let eocd = bytes.len() - 22;
        let comment = b"visible-comment-PK\x05\x06-not-an-end-of-central-directory";
        bytes[eocd + 20..eocd + 22].copy_from_slice(&(comment.len() as u16).to_le_bytes());
        bytes.extend_from_slice(comment);
        let archive = ZipReader::new(&bytes).unwrap();
        assert_eq!(archive.read_by_name("manifest.json", 5).unwrap(), b"valid");
    }

    #[test]
    fn mismatched_local_and_central_names_are_rejected() {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"valid").unwrap();
        let mut archive = writer.finish().unwrap();
        // The central directory retains "manifest.json", but the local header
        // points at a different same-length filename.
        archive[30..43].copy_from_slice(b"malifest.json");
        assert!(matches!(ZipReader::new(&archive), Err(FormatError::Corrupt(_))));
    }

    #[test]
    fn mismatched_local_and_central_compression_are_rejected() {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"valid").unwrap();
        let mut archive = writer.finish().unwrap();
        archive[8..10].copy_from_slice(&8u16.to_le_bytes());
        assert!(matches!(ZipReader::new(&archive), Err(FormatError::Corrupt(_))));
    }

    #[test]
    fn encrypted_local_header_is_rejected_even_if_central_header_is_clear() {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"valid").unwrap();
        let mut archive = writer.finish().unwrap();
        archive[6..8].copy_from_slice(&0x0801u16.to_le_bytes());
        assert!(ZipReader::new(&archive).is_err());
    }

    #[test]
    fn duplicate_entry_names_must_not_make_zip_lookup_ambiguous() {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"first").unwrap();
        assert!(writer.add("manifest.json", b"second").is_err());
        let archive = writer.finish().unwrap();
        assert_eq!(ZipReader::new(&archive).unwrap().entries.len(), 1);
    }

    #[test]
    fn local_crc_or_length_disagreement_is_rejected_without_data_descriptor() {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"valid").unwrap();
        let good = writer.finish().unwrap();
        for (start, value) in [(14, 0u32), (18, 0u32), (22, 0u32)] {
            let mut modified = good.clone();
            modified[start..start + 4].copy_from_slice(&value.to_le_bytes());
            assert!(matches!(ZipReader::new(&modified), Err(FormatError::Corrupt(_))), "local header offset {start}");
        }
    }

    #[test]
    fn valid_streaming_zip_data_descriptor_round_trips() {
        let payload = b"valid";
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", payload).unwrap();
        let mut bytes = writer.finish().unwrap();
        let old_eocd = bytes.len() - 22;
        let old_central = u32_at(&bytes, old_eocd + 16).unwrap() as usize;

        // A streamed ZIP can put zero CRC/sizes in the local header, append
        // PK\\x07\\x08 + the real descriptor, and mark bit 3 in both headers.
        let mut descriptor = Vec::new();
        descriptor.extend_from_slice(&0x0807_4b50u32.to_le_bytes());
        descriptor.extend_from_slice(&crc32(payload).to_le_bytes());
        descriptor.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        descriptor.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.splice(old_central..old_central, descriptor);
        let central = old_central + 16;
        let eocd = old_eocd + 16;
        bytes[6..8].copy_from_slice(&0x0808u16.to_le_bytes());
        bytes[14..26].fill(0);
        bytes[central + 8..central + 10].copy_from_slice(&0x0808u16.to_le_bytes());
        bytes[eocd + 16..eocd + 20].copy_from_slice(&(central as u32).to_le_bytes());

        let reader = ZipReader::new(&bytes).unwrap();
        assert_eq!(reader.read_by_name("manifest.json", payload.len()).unwrap(), payload);
    }

    #[test]
    fn truncated_zip_comments_and_false_comment_lengths_are_rejected() {
        let mut writer = ZipWriter::new();
        writer.add("manifest.json", b"valid").unwrap();
        let mut bytes = writer.finish().unwrap();
        let eocd = bytes.len() - 22;
        bytes[eocd + 20..eocd + 22].copy_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(b"note");
        assert!(ZipReader::new(&bytes).is_ok());

        let mut truncated = bytes.clone();
        truncated.pop();
        assert!(matches!(ZipReader::new(&truncated), Err(FormatError::Corrupt(_))));

        let mut forged = bytes;
        forged[eocd + 20..eocd + 22].copy_from_slice(&5u16.to_le_bytes());
        assert!(matches!(ZipReader::new(&forged), Err(FormatError::Corrupt(_))));
    }
}
