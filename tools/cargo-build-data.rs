use serde_json::{Number, Value};
use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

fn main() {
    if let Err(err) = generate_data_json() {
        panic!("failed to generate data JSON from CSV: {err}");
    }
}

fn generate_data_json() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let repo_root = manifest_dir
        .parent()
        .ok_or("CARGO_MANIFEST_DIR must be inside the repository root")?;
    let source_dir = repo_root.join("data-src");
    let data_dir = repo_root.join("data");

    println!(
        "cargo:rerun-if-changed={}",
        repo_root.join("tools/cargo-build-data.rs").display()
    );
    println!("cargo:rerun-if-changed={}", source_dir.display());
    fs::create_dir_all(&data_dir)?;

    for entry in fs::read_dir(&source_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("csv") {
            println!("cargo:rerun-if-changed={}", path.display());
            convert_csv_file(&path, &data_dir)?;
        }
    }

    Ok(())
}

fn convert_csv_file(csv_path: &Path, data_dir: &Path) -> Result<(), Box<dyn Error>> {
    let csv = fs::read_to_string(csv_path)?;
    let mut lines = csv.lines();
    let headers = lines
        .next()
        .ok_or_else(|| format!("{} is empty", csv_path.display()))?
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    let mut entries = Vec::new();

    for (line_index, line) in lines.enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let values = line.split(',').collect::<Vec<_>>();
        if values.len() != headers.len() {
            return Err(format!(
                "{} line {} has {} fields but the header has {}",
                csv_path.display(),
                line_index + 2,
                values.len(),
                headers.len()
            )
            .into());
        }
        let mut fields = Vec::new();
        let mut id = None;

        for (column_index, key) in headers.iter().enumerate() {
            let raw = values[column_index].trim();
            if raw.is_empty() {
                continue;
            }

            if *key == "id" {
                id = Some(raw.to_string());
            }
            fields.push(((*key).to_string(), parse_csv_value(raw)?));
        }

        let id = id.ok_or_else(|| {
            format!(
                "{} line {} is missing a string id",
                csv_path.display(),
                line_index + 2
            )
        })?;
        entries.push((id, fields));
    }

    let json_file_name = csv_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("invalid CSV file name: {}", csv_path.display()))?
        .to_string()
        + ".json";
    let json_path = data_dir.join(json_file_name);
    fs::write(json_path, stringify_entries(&entries)?)?;

    Ok(())
}

fn stringify_entries(entries: &[(String, Vec<(String, Value)>)]) -> Result<String, Box<dyn Error>> {
    let mut json = String::from("{\n");

    for (entry_index, (id, fields)) in entries.iter().enumerate() {
        json.push_str("  ");
        json.push_str(&serde_json::to_string(id)?);
        json.push_str(": {\n");

        for (field_index, (key, value)) in fields.iter().enumerate() {
            json.push_str("    ");
            json.push_str(&serde_json::to_string(key)?);
            json.push_str(": ");
            json.push_str(&serde_json::to_string(value)?);
            if field_index + 1 != fields.len() {
                json.push(',');
            }
            json.push('\n');
        }

        json.push_str("  }");
        if entry_index + 1 != entries.len() {
            json.push(',');
        }
        json.push('\n');
    }

    json.push_str("}\n");
    Ok(json)
}

fn parse_csv_value(raw: &str) -> Result<Value, Box<dyn Error>> {
    if raw == "true" || raw == "false" {
        return Ok(Value::Bool(raw == "true"));
    }

    if let Ok(num) = raw.parse::<f64>() {
        if num.is_finite() {
            if num.fract() == 0.0 && num >= i64::MIN as f64 && num <= i64::MAX as f64 {
                return Ok(Value::Number(Number::from(num as i64)));
            }

            if let Some(number) = Number::from_f64(num) {
                return Ok(Value::Number(number));
            }
        }
    }

    Ok(Value::String(raw.to_string()))
}
