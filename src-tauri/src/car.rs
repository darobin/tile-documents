use anyhow::{anyhow, bail, Result};
use cid::Cid;
use ciborium::value::Value as CborValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

// ── MASL types ───────────────────────────────────────────────────────────────
//
// Resources are stored as flat maps: "src" → CID string, other keys → HTTP
// header values.  This mirrors the MASL structure directly (headers are
// siblings of `src`, not nested under a "headers" key).

pub type Resource = HashMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Masl {
    pub name: String,
    pub resources: HashMap<String, Resource>,
    #[serde(default)]
    pub icons: Vec<Icon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Icon {
    pub src: String,
    #[serde(default)]
    pub sizes: String,
    #[serde(default)]
    pub purpose: String,
}

// ── Tile content ─────────────────────────────────────────────────────────────

/// Parsed tile: keeps the file path + MASL + a CID→(offset, len) index so
/// individual blocks can be served by seeking into the file on demand.
#[derive(Debug)]
pub struct TileContent {
    pub path: PathBuf,
    pub masl: Masl,
    /// CID (canonical string form) → (byte offset of block data, byte length)
    pub index: HashMap<String, (u64, u64)>,
}

impl TileContent {
    /// Read the raw bytes of the block identified by `cid_str`.
    pub fn read_block(&self, cid_str: &str) -> Result<Vec<u8>> {
        let &(offset, len) = self
            .index
            .get(cid_str)
            .ok_or_else(|| anyhow!("block not found for CID {cid_str}"))?;
        let mut f = File::open(&self.path)?;
        f.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len as usize];
        f.read_exact(&mut buf)?;
        Ok(buf)
    }
}

// ── CAR parsing ──────────────────────────────────────────────────────────────

/// Parse a `.tile` (CARv1) file. Returns `TileContent` with MASL metadata and
/// a CID→offset index built from the file's blocks.
pub fn parse_tile(path: &Path) -> Result<TileContent> {
    let mut f = File::open(path)?;
    let mut data = Vec::new();
    f.read_to_end(&mut data)?;

    let mut pos = 0usize;

    // ── header ────────────────────────────────────────────────────────────
    let (header_len, n) = read_uvarint(&data[pos..])
        .ok_or_else(|| anyhow!("failed to read CAR header varint"))?;
    pos += n;

    let header_end = pos + header_len as usize;
    if header_end > data.len() {
        bail!("CAR header length exceeds file size");
    }

    let masl = parse_masl(&data[pos..header_end])?;
    pos = header_end;

    // ── blocks ────────────────────────────────────────────────────────────
    let mut index: HashMap<String, (u64, u64)> = HashMap::new();

    while pos < data.len() {
        let (block_len, n) = read_uvarint(&data[pos..])
            .ok_or_else(|| anyhow!("failed to read block varint at pos {pos}"))?;
        pos += n;

        if block_len == 0 {
            break;
        }

        let block_end = pos + block_len as usize;
        if block_end > data.len() {
            bail!("block extends beyond file at pos {pos}");
        }

        let (cid, cid_len) = read_cid(&data[pos..])
            .ok_or_else(|| anyhow!("failed to parse CID at pos {pos}"))?;

        let data_offset = (pos + cid_len) as u64;
        let data_len = (block_len as usize - cid_len) as u64;
        index.insert(cid.to_string(), (data_offset, data_len));

        pos = block_end;
    }

    Ok(TileContent { path: path.to_path_buf(), masl, index })
}

// ── MASL extraction from CBOR header ─────────────────────────────────────────

fn parse_masl(header_bytes: &[u8]) -> Result<Masl> {
    let value: CborValue = ciborium::de::from_reader(header_bytes)
        .map_err(|e| anyhow!("CBOR decode error: {e}"))?;

    let map = match value {
        CborValue::Map(m) => m,
        _ => bail!("CAR header is not a CBOR map"),
    };

    let mut name: Option<String> = None;
    let mut resources: HashMap<String, Resource> = HashMap::new();
    let mut icons: Vec<Icon> = Vec::new();
    let mut description: Option<String> = None;
    let mut short_name: Option<String> = None;
    let mut theme_color: Option<String> = None;
    let mut background_color: Option<String> = None;

    for (k, v) in &map {
        let key = cbor_to_string(k).unwrap_or_default();
        match key.as_str() {
            "name" => name = cbor_to_string(v),
            "description" => description = cbor_to_string(v),
            "short_name" => short_name = cbor_to_string(v),
            "theme_color" => theme_color = cbor_to_string(v),
            "background_color" => background_color = cbor_to_string(v),
            "resources" => resources = parse_resources(v)?,
            "icons" => icons = parse_icons(v)?,
            _ => {} // skip `version`, `roots`, and unknown fields
        }
    }

    Ok(Masl {
        name: name.ok_or_else(|| anyhow!("MASL missing `name` field"))?,
        resources,
        icons,
        description,
        short_name,
        theme_color,
        background_color,
    })
}

fn parse_resources(v: &CborValue) -> Result<HashMap<String, Resource>> {
    let map = match v {
        CborValue::Map(m) => m,
        _ => bail!("`resources` is not a CBOR map"),
    };
    let mut out = HashMap::new();
    for (k, rv) in map {
        let path = cbor_to_string(k).ok_or_else(|| anyhow!("resource key is not a string"))?;
        out.insert(path, parse_resource(rv)?);
    }
    Ok(out)
}

/// A resource entry is a flat map: `"src"` → CID string, other keys → header
/// values.  This matches the MASL format where headers are siblings of `src`.
fn parse_resource(v: &CborValue) -> Result<Resource> {
    let map = match v {
        CborValue::Map(m) => m,
        _ => bail!("resource entry is not a CBOR map"),
    };

    let mut out: Resource = HashMap::new();

    for (k, rv) in map {
        let key = cbor_to_string(k).unwrap_or_default();
        let value = if key == "src" {
            cbor_to_cid_string(rv)
                .ok_or_else(|| anyhow!("resource `src` is not a CID"))?
        } else if let Some(s) = cbor_to_string(rv) {
            s
        } else {
            continue; // skip non-string header values
        };
        out.insert(key, value);
    }

    if !out.contains_key("src") {
        bail!("resource missing `src` field");
    }
    Ok(out)
}

fn parse_icons(v: &CborValue) -> Result<Vec<Icon>> {
    let arr = match v {
        CborValue::Array(a) => a,
        _ => bail!("`icons` is not a CBOR array"),
    };
    let mut out = Vec::new();
    for item in arr {
        let map = match item {
            CborValue::Map(m) => m,
            _ => continue,
        };
        let mut src: Option<String> = None;
        let mut sizes = String::new();
        let mut purpose = String::new();
        for (k, iv) in map {
            match cbor_to_string(k).unwrap_or_default().as_str() {
                "src" => src = cbor_to_string(iv),
                "sizes" => sizes = cbor_to_string(iv).unwrap_or_default(),
                "purpose" => purpose = cbor_to_string(iv).unwrap_or_default(),
                _ => {}
            }
        }
        if let Some(src) = src {
            out.push(Icon { src, sizes, purpose });
        }
    }
    Ok(out)
}

// ── CBOR helpers ──────────────────────────────────────────────────────────────

fn cbor_to_string(v: &CborValue) -> Option<String> {
    match v {
        CborValue::Text(s) => Some(s.clone()),
        _ => None,
    }
}

/// Extract a CID from a DAG-CBOR CID link: `Tag(42, Bytes(0x00 || raw_cid))`.
/// The leading `0x00` byte is the identity multibase prefix.
fn cbor_to_cid_string(v: &CborValue) -> Option<String> {
    match v {
        CborValue::Tag(42, inner) => {
            if let CborValue::Bytes(bytes) = inner.as_ref() {
                let raw = if bytes.first() == Some(&0x00) { &bytes[1..] } else { bytes };
                Cid::try_from(raw).ok().map(|c| c.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

// ── Varint / CID helpers ──────────────────────────────────────────────────────

/// Decode an unsigned LEB128 varint. Returns `(value, bytes_consumed)`.
fn read_uvarint(data: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    for (i, &byte) in data.iter().enumerate() {
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

/// Parse a CID from the start of a slice. Returns `(cid, bytes_consumed)`.
fn read_cid(data: &[u8]) -> Option<(Cid, usize)> {
    let mut cursor = std::io::Cursor::new(data);
    let cid = Cid::read_bytes(&mut cursor).ok()?;
    Some((cid, cursor.position() as usize))
}

// ── Authority helper ──────────────────────────────────────────────────────────

/// Derive a `tile:` URI authority from the full file name.
/// e.g. `"My Document.tile"` → `"my-document.tile"`.
pub fn authority_from_path(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("tile");
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
