//! Port of `src/core/xlsx.js`: a minimal XLSX reader.
//!
//! Published community models sometimes arrive as a workbook rather than SBML
//! (the nine-species gut microbiota model of the SteadyCom paper is
//! distributed that way), so this reads one the same way the SBML reader reads
//! a COBRA file.
//!
//! An .xlsx is a ZIP of XML. Two things about the format matter for
//! correctness and are easy to get wrong:
//!
//! * Empty cells are omitted entirely. A row is a sparse list, so reading cells
//!   by their position in the row silently shifts every value after the first
//!   gap into the wrong column. Cells are therefore keyed by the column letter
//!   from their `r` reference, never by order.
//! * Text usually lives in a shared-string table and the cell holds an index
//!   into it (`t="s"`). A cell with `t="str"` or `t="inlineStr"` holds its own
//!   text instead, and a cell with no `t` is a number. All four appear in real
//!   files.
//!
//! Rows are yielded one at a time rather than collected, because these sheets
//! run to tens of thousands of rows.
//!
//! The archive is read from its central directory as the JS reads it (no
//! checksum is verified, as the JS verifies none). Entries are stored or
//! deflated.

use std::path::Path;
use std::sync::{Arc, LazyLock};

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};

use coxeter_core::js;

use crate::error::{Error, ErrorKind, Result};
use crate::jscompat::{B, S};

fn fail(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Xlsx, message)
}

// ZIP archive

const EOCD: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;

#[derive(Debug, Clone, Copy)]
struct ZipEntry {
    method: u16,
    offset: usize,
    size: usize,
}

fn u16_at(buf: &[u8], at: usize) -> Result<u16> {
    buf.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| fail(format!("corrupt ZIP archive: read past the end at {at}")))
}

fn u32_at(buf: &[u8], at: usize) -> Result<u32> {
    buf.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| fail(format!("corrupt ZIP archive: read past the end at {at}")))
}

/// Index a ZIP archive by reading its central directory.
fn read_zip(buf: &[u8]) -> Result<IndexMap<String, ZipEntry>> {
    // The end-of-central-directory record is last, but a trailing comment may
    // follow it, so scan back for the signature rather than assuming the offset.
    let mut eocd = None;
    if buf.len() >= 22 {
        let last = buf.len() - 22;
        let first = last.saturating_sub(65535);
        for i in (first..=last).rev() {
            if u32_at(buf, i)? == EOCD {
                eocd = Some(i);
                break;
            }
        }
    }
    let Some(eocd) = eocd else {
        return Err(fail("not a ZIP archive: no end-of-central-directory record"));
    };

    let count = u16_at(buf, eocd + 10)?;
    let mut at = u32_at(buf, eocd + 16)? as usize;
    let mut entries = IndexMap::new();
    for _ in 0..count {
        if u32_at(buf, at)? != CENTRAL {
            return Err(fail(format!("corrupt central directory at {at}")));
        }
        let method = u16_at(buf, at + 10)?;
        let size = u32_at(buf, at + 24)? as usize;
        let name_length = u16_at(buf, at + 28)? as usize;
        let extra_length = u16_at(buf, at + 30)? as usize;
        let comment_length = u16_at(buf, at + 32)? as usize;
        let offset = u32_at(buf, at + 42)? as usize;
        let name_end = (at + 46 + name_length).min(buf.len());
        let name = String::from_utf8_lossy(&buf[(at + 46).min(name_end)..name_end]).into_owned();
        entries.insert(name, ZipEntry { method, offset, size });
        at += 46 + name_length + extra_length + comment_length;
    }
    Ok(entries)
}

/// Decompress one entry. The local header's lengths are the ones to trust.
fn read_entry(buf: &[u8], name: &str, entry: ZipEntry) -> Result<Vec<u8>> {
    // The central directory's name and extra lengths can differ from the local
    // header's, so the data offset is computed from the local header.
    let name_length = u16_at(buf, entry.offset + 26)? as usize;
    let extra_length = u16_at(buf, entry.offset + 28)? as usize;
    let start = (entry.offset + 30 + name_length + extra_length).min(buf.len());
    match entry.method {
        0 => Ok(buf[start..(start + entry.size).min(buf.len())].to_vec()),
        8 => miniz_oxide::inflate::decompress_to_vec(&buf[start..])
            .map_err(|e| fail(format!("{name}: invalid deflate data ({:?})", e.status))),
        m => Err(fail(format!("unsupported ZIP compression method {m}"))),
    }
}

// XML

static DECIMAL_ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#([0-9]+);").expect("pattern"));
static HEX_ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#x([0-9a-fA-F]+);").expect("pattern"));

/// `String.fromCodePoint(n)`. JS throws a RangeError past U+10FFFF. A lone
/// surrogate, which a JS string can hold, becomes U+FFFD.
fn from_code_point(n: f64) -> Result<char> {
    if !(0.0..=1_114_111.0).contains(&n) || n.fract() != 0.0 {
        return Err(fail(format!("Invalid code point {}", js::number_to_string(n))));
    }
    Ok(char::from_u32(n as u32).unwrap_or('\u{FFFD}'))
}

fn replace_entities(s: &str, re: &Regex, value: impl Fn(&str) -> f64) -> Result<String> {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for c in re.captures_iter(s) {
        let m = c.get(0).expect("whole match");
        out.push_str(&s[last..m.start()]);
        out.push(from_code_point(value(&c[1]))?);
        last = m.end();
    }
    out.push_str(&s[last..]);
    Ok(out)
}

fn decode(s: &str) -> Result<String> {
    let s = s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'");
    let s = replace_entities(&s, &DECIMAL_ENTITY, js::string_to_number)?;
    let s = replace_entities(&s, &HEX_ENTITY, |h| {
        // parseInt(h, 16): exact up to 2^53, then a nearby double.
        h.chars().fold(0.0, |acc, d| acc * 16.0 + d.to_digit(16).unwrap_or(0) as f64)
    })?;
    Ok(s.replace("&amp;", "&"))
}

static TEXT_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"<t(?:{S}[^>]*)?>((?s:.)*?)</t>|<t{S}*/>", S = *S)).expect("pattern"));

/// All the text of an element, with its runs joined: `<t>a</t><t>b</t>` → `ab`.
fn text_of(xml: &str) -> Result<String> {
    let mut out = String::new();
    for m in TEXT_RUN.captures_iter(xml) {
        out.push_str(&decode(m.get(1).map_or("", |g| g.as_str()))?);
    }
    Ok(out)
}

// Workbook

static SHARED_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<si>((?s:.)*?)</si>").expect("pattern"));
static RELATIONSHIP: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<Relationship{B}([^>]*)/>")).expect("pattern"));
static REL_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"{B}Id="([^"]*)""#)).expect("pattern"));
static REL_TARGET: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"{B}Target="([^"]*)""#)).expect("pattern"));
static SHEET: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<sheet{B}([^>]*)/>")).expect("pattern"));
static SHEET_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"{B}name="([^"]*)""#)).expect("pattern"));
static SHEET_RID: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"{B}r:id="([^"]*)""#)).expect("pattern"));
static ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<row{B}([^>]*)>((?s:.)*?)</row>")).expect("pattern"));
static ROW_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"{B}r="([0-9]+)""#)).expect("pattern"));
static CELL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"<c{B}([^>]*?)(?:/>|>((?s:.)*?)</c>)")).expect("pattern"));
static CELL_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"{B}r="([A-Z]+)[0-9]+""#)).expect("pattern"));
static CELL_TYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r#"{B}t="([^"]*)""#)).expect("pattern"));
static CELL_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<v>((?s:.)*?)</v>").expect("pattern"));

/// An open workbook: its sheet names and, per sheet, its rows.
#[derive(Debug, Clone)]
pub struct Workbook {
    data: Arc<Vec<u8>>,
    zip: IndexMap<String, ZipEntry>,
    shared: Arc<Vec<String>>,
    sheets: IndexMap<String, String>,
}

/// One row: its number (`__row`) and its cells by column letter. Columns with
/// no cell are simply absent, which is what the file says and what callers
/// must handle.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub row: f64,
    pub cells: IndexMap<String, String>,
}

impl Row {
    /// `{__row, A: …, C: …}` as the JS yields it.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("__row".into(), json!(self.row));
        for (k, v) in &self.cells {
            out.insert(k.clone(), json!(v));
        }
        Value::Object(out)
    }
}

/// Open a workbook from the bytes of an .xlsx file.
pub fn read_workbook(bytes: impl Into<Vec<u8>>) -> Result<Workbook> {
    let data = Arc::new(bytes.into());
    let zip = read_zip(&data)?;
    let part = |name: &str| -> Result<String> {
        let entry = zip.get(name).ok_or_else(|| fail(format!("{name} is missing from the workbook")))?;
        Ok(String::from_utf8_lossy(&read_entry(&data, name, *entry)?).into_owned())
    };

    // Shared strings, in order: a cell with t="s" holds an index into this.
    let mut shared = Vec::new();
    if zip.contains_key("xl/sharedStrings.xml") {
        let xml = part("xl/sharedStrings.xml")?;
        for m in SHARED_ITEM.captures_iter(&xml) {
            shared.push(text_of(&m[1])?);
        }
    }

    // Sheet name -> part name, through the relationship id the workbook cites.
    let mut rels: IndexMap<String, String> = IndexMap::new();
    let rels_xml = part("xl/_rels/workbook.xml.rels")?;
    for m in RELATIONSHIP.captures_iter(&rels_xml) {
        let id = REL_ID.captures(&m[1]).map(|c| c[1].to_string()).filter(|s| !s.is_empty());
        let target = REL_TARGET.captures(&m[1]).map(|c| c[1].to_string()).filter(|s| !s.is_empty());
        let (Some(id), Some(target)) = (id, target) else { continue };
        let target = target.strip_prefix('/').unwrap_or(&target);
        let target = target.strip_prefix("xl/").unwrap_or(target);
        rels.insert(id, format!("xl/{target}"));
    }

    let mut sheets = IndexMap::new();
    let workbook_xml = part("xl/workbook.xml")?;
    for m in SHEET.captures_iter(&workbook_xml) {
        let name = decode(SHEET_NAME.captures(&m[1]).map_or("", |c| c.get(1).expect("group").as_str()))?;
        let rid = SHEET_RID.captures(&m[1]).map(|c| c[1].to_string()).filter(|s| !s.is_empty());
        if let Some(target) = rid.and_then(|r| rels.get(&r)) {
            if !name.is_empty() {
                sheets.insert(name, target.clone());
            }
        }
    }

    Ok(Workbook { data, zip, shared: Arc::new(shared), sheets })
}

/// Read a whole .xlsx from disk.
pub fn read_workbook_file(path: impl AsRef<Path>) -> Result<Workbook> {
    read_workbook(std::fs::read(path)?)
}

impl Workbook {
    /// The sheet names, in workbook order.
    pub fn sheet_names(&self) -> Vec<String> {
        self.sheets.keys().cloned().collect()
    }

    /// The rows of a sheet, one at a time.
    pub fn rows(&self, sheet_name: &str) -> Result<Rows> {
        let Some(target) = self.sheets.get(sheet_name) else {
            let have = self.sheets.keys().cloned().collect::<Vec<_>>().join(", ");
            return Err(fail(format!("no sheet named \"{sheet_name}\" (have: {have})")));
        };
        let entry = self.zip.get(target).ok_or_else(|| fail(format!("{target} is missing from the workbook")))?;
        let xml = String::from_utf8_lossy(&read_entry(&self.data, target, *entry)?).into_owned();
        Ok(Rows { xml, at: 0, shared: self.shared.clone() })
    }
}

/// The rows of one sheet. Each item is a `Result` because a cell can hold a
/// character reference JavaScript refuses (`&#99999999;`).
#[derive(Debug, Clone)]
pub struct Rows {
    xml: String,
    at: usize,
    shared: Arc<Vec<String>>,
}

impl Rows {
    fn cell_value(&self, kind: &str, body: &str) -> Result<Option<String>> {
        Ok(match kind {
            "s" => {
                let raw = CELL_VALUE.captures(body).map(|c| c.get(1).expect("group").as_str().to_string());
                let index = raw.as_deref().map_or(f64::NAN, js::string_to_number);
                let found = (index >= 0.0 && index.fract() == 0.0).then(|| self.shared.get(index as usize)).flatten();
                Some(found.cloned().unwrap_or_default())
            }
            "inlineStr" => Some(text_of(body)?),
            _ => match CELL_VALUE.captures(body) {
                Some(c) => Some(decode(c.get(1).expect("group").as_str())?),
                None => None,
            },
        })
    }

    fn read_row(&self, attrs: &str, body: &str) -> Result<Row> {
        let row = ROW_NUMBER.captures(attrs).map_or(0.0, |c| js::string_to_number(&c[1]));
        let mut cells = IndexMap::new();
        for cell in CELL.captures_iter(body) {
            let attrs = cell.get(1).map_or("", |g| g.as_str());
            let body = cell.get(2).map_or("", |g| g.as_str());
            let Some(reference) = CELL_REF.captures(attrs).map(|c| c[1].to_string()) else { continue };
            let kind = CELL_TYPE.captures(attrs).map_or_else(|| "n".to_string(), |c| c[1].to_string());
            if let Some(value) = self.cell_value(&kind, body)? {
                if !value.is_empty() {
                    cells.insert(reference, value);
                }
            }
        }
        Ok(Row { row, cells })
    }
}

impl Iterator for Rows {
    type Item = Result<Row>;

    fn next(&mut self) -> Option<Result<Row>> {
        let caps = ROW.captures_at(&self.xml, self.at)?;
        let whole = caps.get(0).expect("whole match");
        let (start, end) = (whole.start(), whole.end());
        let attrs = caps.get(1).map_or("", |g| g.as_str()).to_string();
        let body = caps.get(2).map_or("", |g| g.as_str()).to_string();
        // An empty match cannot happen (`<row` is consumed), but never stall.
        self.at = if end > start { end } else { end + 1 };
        Some(self.read_row(&attrs, &body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_decode_in_the_js_order() {
        assert_eq!(decode("a &amp;lt; b &#65;&#x42;").unwrap(), "a &lt; b AB");
        assert!(decode("&#99999999;").is_err());
    }
}
