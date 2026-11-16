//! Flux data files (`src/core/flux-data.js`): a solution written as reaction
//! identifiers and flux values, and read back onto a model. The JSON form is
//! Escher's reaction data, one object such as `{"PGI": 4.86, "PFK": 7.48}`, so
//! files move between Escher, COBRApy and Coxeter-MFV. A CSV with a reaction
//! column and a flux column (the Flux table export, for instance) is read too.
//! This module only reads, writes and matches identifiers: it never changes a
//! model or solves anything.

use std::collections::HashSet;

use coxeter_core::community::split_id;
use coxeter_core::js;
use indexmap::IndexMap;
use serde_json::Value;

use crate::error::{Error, ErrorKind, Result};

/// Header names of the reaction identifier column of a flux CSV, compared in lower case.
const ID_HEADERS: [&str; 4] = ["reaction", "reaction_id", "rxn", "id"];

/// Header names of the flux column of a flux CSV, compared in lower case.
const FLUX_HEADERS: [&str; 3] = ["flux", "fluxes", "value"];

fn fail(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::FluxData, message)
}

/// `parseFluxData(text)`: read a flux data file, Escher reaction data JSON
/// or a CSV (or tab-separated file) with a reaction column and a flux
/// column. Values that are not finite numbers are skipped. Returns the flux
/// of each reaction id, in file order.
pub fn parse_flux_data(text: &str) -> Result<IndexMap<String, f64>> {
    let trimmed = text.strip_prefix('\u{FEFF}').unwrap_or(text).trim();
    if trimmed.is_empty() {
        return Err(fail("The file is empty."));
    }
    let data = if trimmed.starts_with('{') || trimmed.starts_with('[') { flux_data_from_json(trimmed)? } else { flux_data_from_csv(trimmed)? };
    if data.is_empty() {
        return Err(fail("No reaction fluxes were found in this file."));
    }
    Ok(data)
}

/// Escher reaction data: one object of fluxes, or a list holding one.
fn flux_data_from_json(text: &str) -> Result<IndexMap<String, f64>> {
    let mut parsed: Value = serde_json::from_str(text).map_err(|e| fail(format!("Not a readable JSON file ({e}).")))?;
    if let Value::Array(items) = &parsed {
        if items.len() > 1 {
            return Err(fail(format!("This file holds {} data sets, as Escher uses for a comparison. Import one set at a time.", items.len())));
        }
        parsed = items.first().cloned().unwrap_or(Value::Null);
    }
    let Value::Object(mut object) = parsed else {
        return Err(fail("Flux data JSON must be an object of reaction ids and fluxes, such as {\"PGI\": 4.86}."));
    };
    // Object.entries lists integer-like keys first.
    js::js_object_key_order(&mut object);
    let mut data = IndexMap::new();
    for (id, value) in object {
        let number = json_number(&value);
        if number.is_finite() {
            data.insert(id, number);
        }
    }
    Ok(data)
}

/// A table with a reaction column and a flux column, found by header or else the first two columns.
fn flux_data_from_csv(text: &str) -> Result<IndexMap<String, f64>> {
    let rows = parse_delimited_rows(text);
    let header: Vec<String> = rows[0].iter().map(|cell| cell.trim().to_lowercase()).collect();
    let mut id_index = header.iter().position(|name| ID_HEADERS.contains(&name.as_str()));
    let mut flux_index = header.iter().position(|name| FLUX_HEADERS.contains(&name.as_str()));
    let mut first = 1;
    if id_index.is_none() || flux_index.is_none() {
        // No header: the first column names the reaction and the second holds its flux.
        if rows[0].len() < 2 || !cell_number(&rows[0][1]).is_finite() {
            return Err(fail("The table needs a reaction column and a flux column."));
        }
        id_index = Some(0);
        flux_index = Some(1);
        first = 0;
    }
    let (id_index, flux_index) = (id_index.expect("set above"), flux_index.expect("set above"));
    let mut data = IndexMap::new();
    for row in &rows[first..] {
        let id = row.get(id_index).map_or("", |cell| cell.trim());
        let number = row.get(flux_index).map_or(f64::NAN, |cell| cell_number(cell));
        if !id.is_empty() && number.is_finite() {
            data.insert(id.to_string(), number);
        }
    }
    Ok(data)
}

/// `parseDelimitedRows(text)`: split delimited text into rows of cells. The
/// delimiter is a tab when the first line has one, else a semicolon when it
/// has a semicolon and no comma, else a comma. Double quotes enclose cells
/// that hold the delimiter, and two double quotes inside them stand for one.
pub fn parse_delimited_rows(text: &str) -> Vec<Vec<String>> {
    let first_line = text.split('\n').next().unwrap_or("").trim_end_matches('\r');
    let delimiter = if first_line.contains('\t') {
        '\t'
    } else if first_line.contains(';') && !first_line.contains(',') {
        ';'
    } else {
        ','
    };
    let keep = |row: &[String]| row.iter().any(|value| !value.trim().is_empty());
    let chars: Vec<char> = text.chars().collect();
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if quoted {
            if c == '"' && chars.get(i + 1) == Some(&'"') {
                cell.push('"');
                i += 1;
            } else if c == '"' {
                quoted = false;
            } else {
                cell.push(c);
            }
            i += 1;
            continue;
        }
        if c == '"' {
            quoted = true;
        } else if c == delimiter {
            row.push(std::mem::take(&mut cell));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.get(i + 1) == Some(&'\n') {
                i += 1;
            }
            row.push(std::mem::take(&mut cell));
            if keep(&row) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else {
            cell.push(c);
        }
        i += 1;
    }
    row.push(cell);
    if keep(&row) {
        rows.push(row);
    }
    rows
}

/// `toNumber(value)` of a JSON value: a number, or text holding one; empty text is not zero.
fn json_number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => cell_number(s),
        _ => f64::NAN,
    }
}

/// `toNumber(cell)` of a table cell.
fn cell_number(cell: &str) -> f64 {
    if cell.trim().is_empty() {
        return f64::NAN;
    }
    js::string_to_number(cell)
}

/// What [`match_flux_data`] found: the flux of each model reaction, and the
/// ids that match none.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FluxMatch {
    pub fluxes: IndexMap<String, f64>,
    pub unmatched: Vec<String>,
}

/// `matchFluxData(data, reactionIds)`: which model reaction each value
/// belongs to. An identifier matches the model reaction with the same id, or,
/// in a community, a species' copy of it (`PGI` matches `Ec1@PGI`) when
/// exactly one species has that reaction.
pub fn match_flux_data<'a>(data: &IndexMap<String, f64>, reaction_ids: impl IntoIterator<Item = &'a str>) -> FluxMatch {
    let mut ids: HashSet<&str> = HashSet::new();
    let mut copies: IndexMap<&str, Vec<&str>> = IndexMap::new();
    for rid in reaction_ids {
        if !ids.insert(rid) {
            continue;
        }
        let (species, id) = split_id(rid);
        if species.is_none() {
            continue;
        }
        copies.entry(id).or_default().push(rid);
    }
    let mut found = FluxMatch::default();
    for (id, value) in data {
        if ids.contains(id.as_str()) {
            found.fluxes.insert(id.clone(), *value);
            continue;
        }
        match copies.get(id.as_str()) {
            Some(candidates) if candidates.len() == 1 => {
                found.fluxes.insert(candidates[0].to_string(), *value);
            }
            _ => found.unmatched.push(id.clone()),
        }
    }
    found
}

/// `fluxDataJSON(fluxes)`: fluxes as Escher reaction data JSON, keyed by the
/// model's reaction ids.
pub fn flux_data_json(fluxes: &IndexMap<String, f64>) -> String {
    let mut object = serde_json::Map::new();
    for (id, value) in fluxes {
        // JSON.stringify writes a non-finite number as null.
        let number = serde_json::Number::from_f64(*value).map_or(Value::Null, Value::Number);
        object.insert(id.clone(), number);
    }
    js::js_object_key_order(&mut object);
    js::stringify(&Value::Object(object), Some(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(data: &IndexMap<String, f64>) -> Vec<(&str, f64)> {
        data.iter().map(|(k, v)| (k.as_str(), *v)).collect()
    }

    #[test]
    fn escher_reaction_data_json_reads_as_a_flux_for_each_reaction_and_values_that_are_not_numbers_are_skipped() {
        let data = parse_flux_data(r#"{"PGI": 4.86, "PFK": "7.5", "GAPD": null, "TPI": ""}"#).unwrap();
        assert_eq!(pairs(&data), [("PGI", 4.86), ("PFK", 7.5)]);
        assert_eq!(pairs(&parse_flux_data(r#"[{"PGI": -1}]"#).unwrap()), [("PGI", -1.0)]);
    }

    #[test]
    fn a_csv_with_reaction_and_flux_columns_reads_as_does_the_flux_table_export_with_quoted_names_and_a_two_column_table_without_a_header() {
        let exported = "reaction,species,local_id,name,subsystem,flux,lower_bound,upper_bound\n\
            eco@PGI,eco,PGI,\"glucose-6-phosphate isomerase, cytosolic\",Glycolysis,4.860000,-1000,1000\n";
        assert_eq!(pairs(&parse_flux_data(exported).unwrap()), [("eco@PGI", 4.86)]);
        assert_eq!(pairs(&parse_flux_data("PGI\t4.86\r\nPFK\t-2\r\n").unwrap()), [("PGI", 4.86), ("PFK", -2.0)]);
        assert_eq!(parse_delimited_rows("a;b\n\"x;y\";\"say \"\"hi\"\"\"\n"), [vec!["a", "b"], vec!["x;y", "say \"hi\""]]);
    }

    #[test]
    fn a_comparison_file_with_two_data_sets_is_refused_and_a_file_with_no_fluxes_says_so() {
        assert!(parse_flux_data(r#"[{"PGI": 1}, {"PGI": 2}]"#).unwrap_err().message.contains("2 data sets"));
        assert!(parse_flux_data(r#"{"PGI": "fast"}"#).unwrap_err().message.contains("No reaction fluxes"));
        assert!(parse_flux_data("name,comment\nPGI,fast\n").unwrap_err().message.contains("reaction column and a flux column"));
        assert!(parse_flux_data("   ").unwrap_err().message.contains("empty"));
    }

    #[test]
    fn ids_match_the_model_reaction_or_the_one_species_copy_of_it_and_ids_shared_by_several_species_stay_unmatched() {
        let data: IndexMap<String, f64> = [("Ec1@PGI", 1.0), ("PFK", 2.0), ("ATPM", 3.0), ("MISSING", 4.0)].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        let ids = ["Ec1@PGI", "Ec2@PGI", "Ec1@PFK", "Ec1@ATPM", "Ec2@ATPM"];
        let found = match_flux_data(&data, ids);
        assert_eq!(pairs(&found.fluxes), [("Ec1@PGI", 1.0), ("Ec1@PFK", 2.0)]);
        assert_eq!(found.unmatched, ["ATPM", "MISSING"]);
    }

    #[test]
    fn flux_data_json_lists_each_reaction_id_with_its_flux_and_reads_back_unchanged() {
        let fluxes: IndexMap<String, f64> = [("Ec1@PGI", 4.86), ("EX_glc__D_s", -10.0), ("ATPM", 8.39)].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        assert_eq!(flux_data_json(&fluxes), "{\n  \"Ec1@PGI\": 4.86,\n  \"EX_glc__D_s\": -10,\n  \"ATPM\": 8.39\n}");
        assert_eq!(parse_flux_data(&flux_data_json(&fluxes)).unwrap(), fluxes);
    }
}
