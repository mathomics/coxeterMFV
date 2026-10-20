//! Port of `src/import/mat5.js`: a reader for the documented MATLAB Level 5
//! interchange format (<https://www.mathworks.com/help/pdf_doc/matlab/matfile_format.pdf>).
//!
//! Cells and structure arrays retain MATLAB column-major order. Numeric arrays
//! are flat typed arrays with a `shape`. A scalar is a one-item array. Scalar
//! structures become ordinary objects, and character rows become strings.
//! Sparse arrays retain zero-based CSC storage: `{shape, ir, jc, values}`.
//! This deliberately rejects objects, complex arrays and Level 7.3/HDF5 files.
//! No model data or third-party parser code is included here.
//!
//! The values mirror what the JS returns, including the element type of each
//! typed array, which follows the type the file stores the data in, not the
//! MATLAB class (a double array saved compactly as `uint16` reads as a
//! `Uint16Array`).

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorKind, Result};

mod mi {
    pub const INT8: u32 = 1;
    pub const UINT8: u32 = 2;
    pub const INT16: u32 = 3;
    pub const UINT16: u32 = 4;
    pub const INT32: u32 = 5;
    pub const UINT32: u32 = 6;
    pub const SINGLE: u32 = 7;
    pub const DOUBLE: u32 = 9;
    pub const INT64: u32 = 12;
    pub const UINT64: u32 = 13;
    pub const MATRIX: u32 = 14;
    pub const COMPRESSED: u32 = 15;
    pub const UTF8: u32 = 16;
    pub const UTF16: u32 = 17;
    pub const UTF32: u32 = 18;
    pub const KNOWN: [u32; 15] =
        [INT8, UINT8, INT16, UINT16, INT32, UINT32, SINGLE, DOUBLE, INT64, UINT64, MATRIX, COMPRESSED, UTF8, UTF16, UTF32];
}

const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_ELEMENTS: f64 = 20_000_000.0;
const MAX_CONTAINERS: f64 = 1_000_000.0;
const MAX_DEPTH: usize = 128;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

fn fail(message: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Mat5, format!("MAT Level 5: {message}"))
}

/// The JavaScript typed array a numeric element becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedArrayKind {
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    /// Also what 64-bit integers become (checked to be exact).
    Float64,
}

impl TypedArrayKind {
    /// The JS constructor name (`Float64Array`…).
    pub fn js_name(self) -> &'static str {
        match self {
            TypedArrayKind::Int8 => "Int8Array",
            TypedArrayKind::Uint8 => "Uint8Array",
            TypedArrayKind::Int16 => "Int16Array",
            TypedArrayKind::Uint16 => "Uint16Array",
            TypedArrayKind::Int32 => "Int32Array",
            TypedArrayKind::Uint32 => "Uint32Array",
            TypedArrayKind::Float32 => "Float32Array",
            TypedArrayKind::Float64 => "Float64Array",
        }
    }
}

/// A flat numeric array. Every value is exactly representable as an `f64`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedArray {
    pub kind: TypedArrayKind,
    pub values: Vec<f64>,
}

impl TypedArray {
    fn to_json(&self, shape: Option<&[usize]>) -> Value {
        let mut out = Map::new();
        out.insert("$typed".into(), json!(self.kind.js_name()));
        if let Some(shape) = shape {
            out.insert("shape".into(), json!(shape));
        }
        out.insert("values".into(), Value::Array(self.values.iter().map(|v| number_json(*v)).collect()));
        Value::Object(out)
    }
}

/// Non-finite numbers as the strings JS prints for them (JSON has no NaN).
fn number_json(v: f64) -> Value {
    if v.is_finite() {
        json!(v)
    } else if v.is_nan() {
        json!("NaN")
    } else if v > 0.0 {
        json!("Infinity")
    } else {
        json!("-Infinity")
    }
}

/// A sparse matrix in zero-based compressed sparse column storage.
#[derive(Debug, Clone, PartialEq)]
pub struct SparseMatrix {
    pub shape: Vec<usize>,
    /// Row index of each stored entry.
    pub ir: TypedArray,
    /// Column pointers: column `j` holds entries `jc[j]..jc[j + 1]`.
    pub jc: TypedArray,
    pub values: TypedArray,
}

/// One MATLAB value, shaped as the JS reader returns it.
#[derive(Debug, Clone, PartialEq)]
pub enum MatValue {
    /// A numeric or logical array (`shape` is MATLAB's dimensions).
    Numeric { shape: Vec<usize>, data: TypedArray },
    /// A character array of one row.
    Text(String),
    /// A character matrix, one string per row.
    TextRows(Vec<String>),
    /// A cell array, column-major.
    Cell(Vec<MatValue>),
    /// A 1×1 structure.
    Struct(IndexMap<String, MatValue>),
    /// A structure array of any other size, column-major.
    StructArray(Vec<IndexMap<String, MatValue>>),
    Sparse(SparseMatrix),
}

impl MatValue {
    /// A JSON picture of the value: strings, arrays and objects as JS has
    /// them; a typed array as `{"$typed": "Float64Array", "shape", "values"}`;
    /// a sparse matrix as `{"$sparse": {shape, ir, jc, values}}`; non-finite
    /// numbers as `"NaN"`/`"Infinity"`.
    pub fn to_json(&self) -> Value {
        match self {
            MatValue::Numeric { shape, data } => data.to_json(Some(shape)),
            MatValue::Text(s) => json!(s),
            MatValue::TextRows(rows) => json!(rows),
            MatValue::Cell(items) => Value::Array(items.iter().map(MatValue::to_json).collect()),
            MatValue::Struct(fields) => struct_json(fields),
            MatValue::StructArray(items) => Value::Array(items.iter().map(struct_json).collect()),
            MatValue::Sparse(s) => json!({"$sparse": {
                "shape": s.shape,
                "ir": s.ir.to_json(None),
                "jc": s.jc.to_json(None),
                "values": s.values.to_json(None),
            }}),
        }
    }

    /// The field of a scalar structure.
    pub fn field(&self, name: &str) -> Option<&MatValue> {
        match self {
            MatValue::Struct(fields) => fields.get(name),
            _ => None,
        }
    }

    /// The numbers of a numeric array.
    pub fn numbers(&self) -> Option<&[f64]> {
        match self {
            MatValue::Numeric { data, .. } => Some(&data.values),
            _ => None,
        }
    }
}

fn struct_json(fields: &IndexMap<String, MatValue>) -> Value {
    Value::Object(fields.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
}

/// The variables of a MAT file, by name, in file order.
pub type MatVariables = IndexMap<String, MatValue>;

/// Every variable as JSON (see [`MatValue::to_json`]).
pub fn variables_to_json(vars: &MatVariables) -> Value {
    Value::Object(vars.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
}

struct Element<'a> {
    kind: u32,
    bytes: &'a [u8],
}

struct Cursor<'a> {
    bytes: &'a [u8],
    little: bool,
    offset: usize,
}

fn read_u16(b: &[u8], at: usize, little: bool) -> u16 {
    let x = [b[at], b[at + 1]];
    if little { u16::from_le_bytes(x) } else { u16::from_be_bytes(x) }
}

fn read_u32(b: &[u8], at: usize, little: bool) -> u32 {
    let x = [b[at], b[at + 1], b[at + 2], b[at + 3]];
    if little { u32::from_le_bytes(x) } else { u32::from_be_bytes(x) }
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], little: bool) -> Self {
        Cursor { bytes, little, offset: 0 }
    }

    fn tag(&mut self) -> Result<Element<'a>> {
        let start = self.offset;
        if self.bytes.len() - start < 8 {
            return Err(fail("truncated data-element tag."));
        }
        let word = read_u32(self.bytes, start, self.little);
        let (kind, size, payload, next) = if word > 0xffff {
            let kind = read_u16(self.bytes, start, self.little) as u32;
            let size = read_u16(self.bytes, start + 2, self.little) as usize;
            if !(1..=4).contains(&size) {
                return Err(fail("invalid small data-element length."));
            }
            (kind, size, start + 4, start + 8)
        } else {
            let size = read_u32(self.bytes, start + 4, self.little) as usize;
            let payload = start + 8;
            // miCOMPRESSED is not padded. SciPy and MATLAB concatenate it directly.
            let pad = if word == mi::COMPRESSED { 0 } else { (8 - size % 8) % 8 };
            (word, size, payload, payload + size + pad)
        };
        if !mi::KNOWN.contains(&kind) {
            return Err(fail(format!("unsupported data-element type {kind}.")));
        }
        if size > MAX_BYTES || next > self.bytes.len() {
            return Err(fail("data element exceeds its container."));
        }
        self.offset = next;
        Ok(Element { kind, bytes: &self.bytes[payload..payload + size] })
    }

    fn done(&self) -> Result<()> {
        if self.offset != self.bytes.len() {
            return Err(fail("unexpected trailing matrix data."));
        }
        Ok(())
    }
}

fn numbers(element: &Element, little: bool) -> Result<TypedArray> {
    let (width, kind) = match element.kind {
        mi::INT8 => (1, TypedArrayKind::Int8),
        mi::UINT8 => (1, TypedArrayKind::Uint8),
        mi::INT16 => (2, TypedArrayKind::Int16),
        mi::UINT16 => (2, TypedArrayKind::Uint16),
        mi::INT32 => (4, TypedArrayKind::Int32),
        mi::UINT32 => (4, TypedArrayKind::Uint32),
        mi::SINGLE => (4, TypedArrayKind::Float32),
        mi::DOUBLE | mi::INT64 | mi::UINT64 => (8, TypedArrayKind::Float64),
        other => return Err(fail(format!("expected numeric data, found type {other}."))),
    };
    let bytes = element.bytes;
    if bytes.len() % width != 0 {
        return Err(fail("numeric data length is not a whole number of values."));
    }
    let count = bytes.len() / width;
    if count as f64 > MAX_ELEMENTS {
        return Err(fail("numeric array exceeds the element limit."));
    }
    let mut values = Vec::with_capacity(count);
    for chunk in bytes.chunks_exact(width) {
        let value = match element.kind {
            mi::INT8 => chunk[0] as i8 as f64,
            mi::UINT8 => chunk[0] as f64,
            mi::INT16 => read_u16(chunk, 0, little) as i16 as f64,
            mi::UINT16 => read_u16(chunk, 0, little) as f64,
            mi::INT32 => read_u32(chunk, 0, little) as i32 as f64,
            mi::UINT32 => read_u32(chunk, 0, little) as f64,
            mi::SINGLE => f32::from_bits(read_u32(chunk, 0, little)) as f64,
            mi::DOUBLE => f64::from_bits(read_u64(chunk, little)),
            mi::INT64 => {
                let raw = read_u64(chunk, little) as i64;
                if raw.unsigned_abs() > MAX_SAFE_INTEGER as u64 {
                    return Err(fail("64-bit integer cannot be represented exactly as a JavaScript number."));
                }
                raw as f64
            }
            _ => {
                let raw = read_u64(chunk, little);
                if raw > MAX_SAFE_INTEGER as u64 {
                    return Err(fail("64-bit integer cannot be represented exactly as a JavaScript number."));
                }
                raw as f64
            }
        };
        values.push(value);
    }
    Ok(TypedArray { kind, values })
}

fn read_u64(b: &[u8], little: bool) -> u64 {
    let x: [u8; 8] = b[..8].try_into().expect("eight bytes");
    if little { u64::from_le_bytes(x) } else { u64::from_be_bytes(x) }
}

/// `new TextDecoder('utf-8', {fatal: true}).decode(bytes)`, which also drops a
/// leading byte-order mark.
fn utf8(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).map_err(|_| fail("invalid UTF-8 text."))?;
    Ok(text.strip_prefix('\u{FEFF}').unwrap_or(text).to_string())
}

/// Characters of a char array: code points (UTF-8 and UTF-32 storage) or
/// UTF-16 code units (UTF-16 storage), as `Array.from`/`fromCharCode` give them.
enum TextUnits {
    CodePoints(Vec<char>),
    CodeUnits(Vec<u16>),
}

impl TextUnits {
    fn len(&self) -> usize {
        match self {
            TextUnits::CodePoints(c) => c.len(),
            TextUnits::CodeUnits(u) => u.len(),
        }
    }

    fn collect(&self, indices: impl Iterator<Item = usize>) -> String {
        match self {
            TextUnits::CodePoints(c) => indices.map(|i| c[i]).collect(),
            TextUnits::CodeUnits(u) => String::from_utf16_lossy(&indices.map(|i| u[i]).collect::<Vec<_>>()),
        }
    }
}

fn text_units(element: &Element, little: bool) -> Result<TextUnits> {
    let bytes = element.bytes;
    match element.kind {
        mi::UTF8 | mi::INT8 | mi::UINT8 => Ok(TextUnits::CodePoints(utf8(bytes)?.chars().collect())),
        mi::UTF16 | mi::UINT16 | mi::INT16 => {
            if bytes.len() % 2 != 0 {
                return Err(fail("invalid UTF-16 byte length."));
            }
            Ok(TextUnits::CodeUnits((0..bytes.len() / 2).map(|i| read_u16(bytes, i * 2, little)).collect()))
        }
        mi::UTF32 | mi::UINT32 => {
            if bytes.len() % 4 != 0 {
                return Err(fail("invalid UTF-32 byte length."));
            }
            let mut chars = Vec::with_capacity(bytes.len() / 4);
            for i in 0..bytes.len() / 4 {
                let code = read_u32(bytes, i * 4, little);
                let c = char::from_u32(code).ok_or_else(|| fail("invalid UTF-32 code point."))?;
                chars.push(c);
            }
            Ok(TextUnits::CodePoints(chars))
        }
        other => Err(fail(format!("unsupported character storage type {other}."))),
    }
}

fn matrix(element: &Element, little: bool, depth: usize) -> Result<(String, MatValue)> {
    if element.kind != mi::MATRIX {
        return Err(fail("expected a matrix data element."));
    }
    if depth > MAX_DEPTH {
        return Err(fail("matrix nesting exceeds the depth limit."));
    }
    // MATLAB writes an empty array inside a cell or structure (the empty
    // comments of an AGORA model) as a matrix element with no bytes at all.
    if element.bytes.is_empty() {
        let data = TypedArray { kind: TypedArrayKind::Float64, values: Vec::new() };
        return Ok((String::new(), MatValue::Numeric { shape: vec![0, 0], data }));
    }
    let mut cursor = Cursor::new(element.bytes, little);
    let flags_tag = cursor.tag()?;
    if flags_tag.kind != mi::UINT32 || flags_tag.bytes.len() != 8 {
        return Err(fail("invalid array flags."));
    }
    let flags = numbers(&flags_tag, little)?.values;
    let flag_word = flags[0] as u32;
    let array_class = flag_word & 0xff;
    if flag_word & 0x800 != 0 {
        return Err(fail("complex arrays are not supported."));
    }
    let dims_tag = cursor.tag()?;
    if dims_tag.kind != mi::INT32 {
        return Err(fail("invalid array dimensions type."));
    }
    let dims = numbers(&dims_tag, little)?.values;
    if dims.len() < 2 || dims.len() > 32 || dims.iter().any(|x| *x < 0.0) {
        return Err(fail("invalid array dimensions."));
    }
    let mut count = 1.0f64;
    for size in &dims {
        count *= size;
        if count > MAX_SAFE_INTEGER || (array_class != 5 && count > MAX_ELEMENTS) || *size > MAX_ELEMENTS {
            return Err(fail("array dimensions exceed the element limit."));
        }
    }
    let shape: Vec<usize> = dims.iter().map(|d| *d as usize).collect();
    let count_n = count as usize;
    let name_tag = cursor.tag()?;
    if ![mi::INT8, mi::UINT8, mi::UTF8].contains(&name_tag.kind) {
        return Err(fail("invalid array name type."));
    }
    let name = utf8(name_tag.bytes)?;
    let remaining = |c: &Cursor| (c.bytes.len() - c.offset) as f64 / 8.0;

    let value = match array_class {
        1 => {
            if count > MAX_CONTAINERS || count > remaining(&cursor) {
                return Err(fail("cell array exceeds its container or element limit."));
            }
            let mut items = Vec::with_capacity(count_n);
            for _ in 0..count_n {
                let tag = cursor.tag()?;
                items.push(matrix(&tag, little, depth + 1)?.1);
            }
            MatValue::Cell(items)
        }
        2 => {
            let length_tag = cursor.tag()?;
            if length_tag.kind != mi::INT32 {
                return Err(fail("invalid structure field-name length type."));
            }
            let lengths = numbers(&length_tag, little)?.values;
            if lengths.len() != 1 || lengths[0] < 1.0 || lengths[0] > 1024.0 {
                return Err(fail("invalid structure field-name length."));
            }
            let width = lengths[0] as usize;
            let names_tag = cursor.tag()?;
            if names_tag.kind != mi::INT8 || names_tag.bytes.len() % width != 0 {
                return Err(fail("invalid structure field names."));
            }
            let mut fields: Vec<String> = Vec::new();
            for slot in names_tag.bytes.chunks(width) {
                let end = slot.iter().position(|b| *b == 0).unwrap_or(slot.len());
                let field = utf8(&slot[..end])?;
                if field.is_empty() || fields.contains(&field) {
                    return Err(fail("empty or duplicate structure field name."));
                }
                fields.push(field);
            }
            if count > MAX_CONTAINERS || count * fields.len() as f64 > remaining(&cursor) {
                return Err(fail("structure array exceeds its container or element limit."));
            }
            let mut structures = Vec::with_capacity(count_n);
            for _ in 0..count_n {
                let mut object = IndexMap::new();
                for field in &fields {
                    let tag = cursor.tag()?;
                    object.insert(field.clone(), matrix(&tag, little, depth + 1)?.1);
                }
                structures.push(object);
            }
            if count_n == 1 {
                MatValue::Struct(structures.pop().expect("one structure"))
            } else {
                MatValue::StructArray(structures)
            }
        }
        4 => {
            if shape.len() > 2 {
                return Err(fail("character arrays with more than two dimensions are not supported."));
            }
            let tag = cursor.tag()?;
            let units = text_units(&tag, little)?;
            if units.len() != count_n {
                return Err(fail("character data does not match its dimensions."));
            }
            if shape[0] <= 1 {
                MatValue::Text(units.collect(0..units.len()))
            } else {
                let (rows, cols) = (shape[0], shape[1]);
                MatValue::TextRows((0..rows).map(|row| units.collect((0..cols).map(|col| row + col * rows))).collect())
            }
        }
        5 => {
            if shape.len() != 2 {
                return Err(fail("sparse arrays must have two dimensions."));
            }
            let ir_tag = cursor.tag()?;
            let jc_tag = cursor.tag()?;
            let index_type = |k: u32| k == mi::INT32 || k == mi::UINT32;
            if !index_type(ir_tag.kind) || !index_type(jc_tag.kind) {
                return Err(fail("invalid sparse index storage type."));
            }
            let ir = numbers(&ir_tag, little)?;
            let jc = numbers(&jc_tag, little)?;
            let values_tag = cursor.tag()?;
            let values = numbers(&values_tag, little)?;
            if jc.values.len() != shape[1] + 1 || jc.values[0] != 0.0 {
                return Err(fail("invalid sparse column pointers."));
            }
            let nnz = jc.values[jc.values.len() - 1];
            if nnz < 0.0 || nnz > ir.values.len() as f64 || nnz > values.values.len() as f64 || nnz > flags[1] {
                return Err(fail("sparse entries exceed allocated storage."));
            }
            for col in 1..jc.values.len() {
                if jc.values[col] < jc.values[col - 1] || jc.values[col] > nnz {
                    return Err(fail("sparse column pointers are not monotone."));
                }
            }
            let nnz = nnz as usize;
            for i in 0..nnz {
                if ir.values[i] < 0.0 || ir.values[i] >= shape[0] as f64 {
                    return Err(fail("sparse row index exceeds its dimensions."));
                }
            }
            MatValue::Sparse(SparseMatrix {
                shape: shape.clone(),
                ir: TypedArray { kind: ir.kind, values: ir.values[..nnz].to_vec() },
                jc,
                values: TypedArray { kind: values.kind, values: values.values[..nnz].to_vec() },
            })
        }
        6..=15 => {
            let tag = cursor.tag()?;
            let data = numbers(&tag, little)?;
            if data.values.len() != count_n {
                return Err(fail("numeric data does not match its dimensions."));
            }
            MatValue::Numeric { shape: shape.clone(), data }
        }
        other => return Err(fail(format!("unsupported MATLAB array class {other}."))),
    };
    cursor.done()?;
    Ok((name, value))
}

fn inflate(bytes: &[u8], budget: &mut usize) -> Result<Vec<u8>> {
    use miniz_oxide::inflate::TINFLStatus;
    match miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(bytes, *budget) {
        Ok(out) => {
            *budget -= out.len();
            Ok(out)
        }
        Err(e) if e.status == TINFLStatus::HasMoreOutput => Err(fail("decompressed data exceeds the byte limit.")),
        Err(_) => Err(fail("invalid compressed zlib data.")),
    }
}

/// Read the bytes of a MAT file into its named variables.
pub fn read_mat5(bytes: &[u8]) -> Result<MatVariables> {
    if bytes.len() < 128 {
        return Err(fail("truncated Level 5 header."));
    }
    if bytes.len() > MAX_BYTES {
        return Err(fail("file exceeds the byte limit."));
    }
    let little = match (bytes[126], bytes[127]) {
        (b'I', b'M') => true,
        (b'M', b'I') => false,
        _ => return Err(fail("invalid endian indicator; Level 7.3/HDF5 is not supported.")),
    };
    let version = read_u16(bytes, 124, little);
    if version != 0x100 {
        return Err(fail(format!("unsupported MAT-file version {version}.")));
    }
    let mut variables = MatVariables::new();
    let mut budget = MAX_BYTES;
    read_elements(&bytes[128..], little, false, &mut variables, &mut budget)?;
    Ok(variables)
}

fn read_elements(data: &[u8], little: bool, compressed: bool, variables: &mut MatVariables, budget: &mut usize) -> Result<()> {
    let mut cursor = Cursor::new(data, little);
    while cursor.offset < data.len() {
        let element = cursor.tag()?;
        if element.kind == mi::COMPRESSED {
            if compressed {
                return Err(fail("nested compressed data elements are not supported."));
            }
            let expanded = inflate(element.bytes, budget)?;
            read_elements(&expanded, little, true, variables, budget)?;
        } else {
            let (name, value) = matrix(&element, little, 0)?;
            if name.is_empty() || variables.contains_key(&name) {
                return Err(fail("empty or duplicate variable name."));
            }
            variables.insert(name, value);
        }
    }
    Ok(())
}

/// Read a MAT file from disk.
pub fn read_mat5_file(path: impl AsRef<std::path::Path>) -> Result<MatVariables> {
    read_mat5(&std::fs::read(path)?)
}
